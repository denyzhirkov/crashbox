use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app_state::AppState;
use crate::db::issues::UpsertOutcome;
use crate::db::projects::Project;
use crate::db::{events, issues};
use crate::http::decompress::{self, DecodeError};
use crate::http::dsn_auth::{self, DsnAuthError};
use crate::http::livelog;
use crate::notify::{IssueNotification, Kind as NotifyKind, Notification};
use crate::sentry::{envelope, grouping, normalize};

#[derive(Debug, Deserialize)]
pub struct IngestQuery {
    #[serde(default)]
    pub sentry_key: Option<String>,
    #[serde(default)]
    pub sentry_version: Option<String>,
    #[serde(default)]
    pub sentry_client: Option<String>,
}

/// POST /api/:project_id/envelope[/]
pub async fn envelope_endpoint(
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    Query(q): Query<IngestQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let limit = state.config.ingest.max_envelope_bytes;
    if body.len() > limit {
        metrics::counter!(
            "crashbox_events_dropped_total",
            "reason" => "too_large_envelope"
        )
        .increment(1);
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            "envelope exceeds CRASHBOX_MAX_ENVELOPE_BYTES",
        );
    }

    let project = match auth_project(&state, project_id, &headers, &q).await {
        Ok(p) => p,
        Err(resp) => return resp,
    };

    metrics::counter!(
        "crashbox_envelope_bytes_total",
        "project" => project.slug.clone()
    )
    .increment(body.len() as u64);

    if let Err(resp) = check_rate_limit(&state, &project) {
        return resp;
    }

    let decoded = match decode_body(&headers, &body, limit, project.id) {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    let body: &[u8] = decoded.as_deref().unwrap_or(&body);

    let env = match envelope::parse(body) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(project_id = project.id, error = %e, "envelope parse failed");
            metrics::counter!(
                "crashbox_events_dropped_total",
                "reason" => "bad_envelope"
            )
            .increment(1);
            return refuse(StatusCode::BAD_REQUEST, &format!("invalid envelope: {e}"));
        }
    };

    let mut stored_event_id: Option<String> = None;

    for item in &env.items {
        // Sentry structured-log items feed the ephemeral Live Logs channel, never the DB.
        if item.header.ty.as_deref() == Some("log") {
            if state.config.livelog.enabled {
                let published = livelog::ingest_log_item(&state, project.id, &item.payload);
                if published > 0 {
                    metrics::counter!(
                        "crashbox_livelog_received_total",
                        "project" => project.slug.clone()
                    )
                    .increment(published as u64);
                }
            }
            continue;
        }
        if !item.is_event() {
            tracing::debug!(
                ty = item.header.ty.as_deref().unwrap_or("unknown"),
                "skipping non-event item"
            );
            continue;
        }
        // Envelope header may carry event_id when the event payload omits it.
        match ingest_event_payload(
            &state,
            &project,
            &item.payload,
            env.header.event_id.as_ref(),
        )
        .await
        {
            Ok(event_id) => {
                stored_event_id = Some(event_id.unwrap_or_default());
                // MVP: process only the first event item per envelope.
                break;
            }
            Err(resp) => return resp,
        }
    }

    let id_for_response = stored_event_id.unwrap_or_default();
    (StatusCode::OK, Json(json!({ "id": id_for_response }))).into_response()
}

/// POST /api/:project_id/store[/] — legacy (pre-envelope) Sentry store API. The body is a bare
/// event JSON object; auth, rate limiting, compression, and the per-event pipeline are shared
/// with the envelope endpoint.
pub async fn store_endpoint(
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    Query(q): Query<IngestQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let limit = state.config.ingest.max_envelope_bytes;
    if body.len() > limit {
        metrics::counter!(
            "crashbox_events_dropped_total",
            "reason" => "too_large_envelope"
        )
        .increment(1);
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            "body exceeds CRASHBOX_MAX_ENVELOPE_BYTES",
        );
    }

    let project = match auth_project(&state, project_id, &headers, &q).await {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    if let Err(resp) = check_rate_limit(&state, &project) {
        return resp;
    }
    let decoded = match decode_body(&headers, &body, limit, project.id) {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    let payload: &[u8] = decoded.as_deref().unwrap_or(&body);

    match ingest_event_payload(&state, &project, payload, None).await {
        Ok(event_id) => (
            StatusCode::OK,
            Json(json!({ "id": event_id.unwrap_or_default() })),
        )
            .into_response(),
        Err(resp) => resp,
    }
}

async fn auth_project(
    state: &AppState,
    project_id: i64,
    headers: &HeaderMap,
    q: &IngestQuery,
) -> Result<Project, Response> {
    let auth_header = headers.get("x-sentry-auth").and_then(|v| v.to_str().ok());
    match dsn_auth::resolve_project(&state.db, project_id, auth_header, q.sentry_key.as_deref())
        .await
    {
        Ok(p) => Ok(p),
        Err(DsnAuthError::MissingKey) => {
            metrics::counter!("crashbox_events_dropped_total", "reason" => "bad_key").increment(1);
            Err(refuse(StatusCode::UNAUTHORIZED, "missing sentry_key"))
        }
        Err(DsnAuthError::UnknownKey) => {
            metrics::counter!("crashbox_events_dropped_total", "reason" => "bad_key").increment(1);
            Err(refuse(StatusCode::UNAUTHORIZED, "unknown sentry_key"))
        }
        Err(DsnAuthError::ProjectMismatch) => {
            metrics::counter!("crashbox_events_dropped_total", "reason" => "bad_key").increment(1);
            Err(refuse(
                StatusCode::UNAUTHORIZED,
                "sentry_key does not match project_id in path",
            ))
        }
        Err(DsnAuthError::Db(e)) => {
            tracing::error!(error = %e, "db error looking up project");
            metrics::counter!("crashbox_events_dropped_total", "reason" => "db_error").increment(1);
            Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))
        }
    }
}

// Err carries a fully-built HTTP response returned to the client immediately; boxing it
// would add indirection without saving anything on this path.
#[allow(clippy::result_large_err)]
fn check_rate_limit(state: &AppState, project: &Project) -> Result<(), Response> {
    let decision = state.rate_limiter.check(project.id);
    if decision.allowed {
        return Ok(());
    }
    metrics::counter!(
        "crashbox_events_dropped_total",
        "reason" => "rate_limit",
        "project" => project.slug.clone()
    )
    .increment(1);
    Err(rate_limited_response(
        "rate limit exceeded",
        decision.retry_after,
        "error",
    ))
}

/// 429 with the backoff headers SDKs understand: `Retry-After` plus Sentry's
/// `X-Sentry-Rate-Limits: <seconds>:<category>:<scope>`. `category` is a Sentry data
/// category (`error`, `log_item`); our limiter is per-project, hence scope `project`.
pub(crate) fn rate_limited_response(msg: &str, retry_after: u32, category: &str) -> Response {
    let mut resp = (StatusCode::TOO_MANY_REQUESTS, Json(json!({"error": msg}))).into_response();
    let headers = resp.headers_mut();
    if let Ok(v) = retry_after.to_string().parse() {
        headers.insert("retry-after", v);
    }
    if let Ok(v) = format!("{retry_after}:{category}:project").parse() {
        headers.insert("x-sentry-rate-limits", v);
    }
    resp
}

/// Decode the body per `Content-Encoding`, only after auth + rate limiting so unauthenticated
/// traffic can't burn CPU. The decompressed size is bounded by the same
/// CRASHBOX_MAX_ENVELOPE_BYTES as the raw body.
// Err carries a fully-built HTTP response returned to the client immediately; boxing it
// would add indirection without saving anything on this path.
#[allow(clippy::result_large_err)]
fn decode_body(
    headers: &HeaderMap,
    body: &Bytes,
    limit: usize,
    project_id: i64,
) -> Result<Option<Vec<u8>>, Response> {
    match headers.get("content-encoding").map(|v| v.to_str()) {
        None => Ok(None),
        Some(Err(_)) => {
            metrics::counter!("crashbox_events_dropped_total", "reason" => "bad_encoding")
                .increment(1);
            Err(refuse(
                StatusCode::BAD_REQUEST,
                "Content-Encoding header is not valid UTF-8",
            ))
        }
        Some(Ok(enc)) => match decompress::decode(enc, body, limit) {
            Ok(d) => Ok(d),
            Err(DecodeError::TooLarge) => {
                metrics::counter!(
                    "crashbox_events_dropped_total",
                    "reason" => "too_large_envelope"
                )
                .increment(1);
                Err(refuse(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "decompressed envelope exceeds CRASHBOX_MAX_ENVELOPE_BYTES",
                ))
            }
            Err(e) => {
                tracing::debug!(project_id, error = %e, "body decode failed");
                metrics::counter!("crashbox_events_dropped_total", "reason" => "bad_encoding")
                    .increment(1);
                Err(refuse(StatusCode::BAD_REQUEST, &e.to_string()))
            }
        },
    }
}

/// Shared per-event pipeline: size/UTF-8/JSON checks, normalize → fingerprint → store → notify.
/// Returns the event's own id (if any) for the ingest response.
async fn ingest_event_payload(
    state: &AppState,
    project: &Project,
    payload: &[u8],
    fallback_event_id: Option<&String>,
) -> Result<Option<String>, Response> {
    if payload.len() > state.config.ingest.max_event_bytes {
        return Err(refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            "event exceeds CRASHBOX_MAX_EVENT_BYTES",
        ));
    }
    let Ok(raw_str) = std::str::from_utf8(payload) else {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "event payload is not valid UTF-8",
        ));
    };
    let Ok(parsed) = serde_json::from_str::<Value>(raw_str) else {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "event payload is not valid JSON",
        ));
    };

    let mut ev = normalize::from_value(&parsed);
    if ev.event_id.is_none() {
        ev.event_id = fallback_event_id.cloned();
    }

    let fp = grouping::fingerprint(&ev);
    let title = normalize::title_for(&ev);
    let event_ts = ev
        .timestamp
        .clone()
        .unwrap_or_else(|| Utc::now().to_rfc3339());

    match store_event(state, project.id, &ev, &fp, &title, &event_ts, raw_str).await {
        Ok((issue_id, outcome, new_event_count)) => {
            metrics::counter!(
                "crashbox_events_ingested_total",
                "project" => project.slug.clone(),
                "level" => ev.level.clone().unwrap_or_else(|| "unknown".to_string()),
            )
            .increment(1);
            tracing::info!(
                project_id = project.id,
                event_id = ev.event_id.as_deref().unwrap_or("(none)"),
                fingerprint = %fp,
                outcome = ?outcome,
                "ingested event"
            );

            // Fire notifications only on issue-level transitions: brand new issue or
            // resolved → unresolved reopen. Bursts of a known unresolved issue do NOT
            // trigger here; that's what spike detection (A2) is for.
            let notify_kind = match outcome {
                UpsertOutcome::Created => Some(NotifyKind::NewIssue),
                UpsertOutcome::Reopened => Some(NotifyKind::Reopened),
                UpsertOutcome::Existing => None,
            };
            if let Some(kind) = notify_kind {
                if !state.notify.is_empty() {
                    let link = state.notify.build_link(issue_id);
                    state.notify.fire(Notification::Issue(IssueNotification {
                        kind,
                        project_name: project.name.clone(),
                        project_slug: project.slug.clone(),
                        issue_id,
                        issue_title: title.clone(),
                        event_count: new_event_count,
                        level: ev.level.clone(),
                        environment: ev.environment.clone(),
                        release: ev.release.clone(),
                        link,
                        current_hour: None,
                        baseline_per_hour: None,
                    }));
                }
            }

            Ok(ev.event_id)
        }
        Err(e) => {
            tracing::error!(error = %e, "store_event failed");
            metrics::counter!(
                "crashbox_events_dropped_total",
                "reason" => "db_error"
            )
            .increment(1);
            Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))
        }
    }
}

/// Returns (issue_id, upsert outcome, new event_count after this event).
async fn store_event(
    state: &AppState,
    project_id: i64,
    ev: &normalize::NormalizedEvent,
    fingerprint: &str,
    title: &str,
    timestamp_iso: &str,
    raw_json: &str,
) -> anyhow::Result<(i64, UpsertOutcome, i64)> {
    // BEGIN IMMEDIATE serializes concurrent writers cleanly via busy_timeout. See
    // `db::begin_write` for the full rationale — `pool.begin()` (= BEGIN DEFERRED) causes
    // SQLITE_BUSY under bursts because two transactions both hold SHARED and race to upgrade.
    let mut tx = crate::db::begin_write(&state.db).await?;
    let (issue_id, outcome) = issues::upsert(
        tx.acquire(),
        project_id,
        fingerprint,
        title,
        ev.level.as_deref(),
        ev.platform.as_deref(),
        timestamp_iso,
    )
    .await?;
    let event_row_id =
        events::insert_full(tx.acquire(), project_id, Some(issue_id), ev, raw_json).await?;
    issues::bump_after_event(tx.acquire(), issue_id, event_row_id, timestamp_iso).await?;
    let new_event_count: i64 = sqlx::query_scalar("SELECT event_count FROM issues WHERE id = ?")
        .bind(issue_id)
        .fetch_one(tx.acquire())
        .await?;
    tx.commit().await?;
    Ok((issue_id, outcome, new_event_count))
}

fn refuse(status: StatusCode, msg: &str) -> axum::response::Response {
    (status, Json(json!({"error": msg}))).into_response()
}
