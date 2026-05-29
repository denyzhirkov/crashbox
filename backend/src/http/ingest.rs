use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app_state::AppState;
use crate::db::{events, issues, projects};
use crate::db::issues::UpsertOutcome;
use crate::notify::{Kind as NotifyKind, Notification};
use crate::sentry::{auth, envelope, grouping, normalize};

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
) -> impl IntoResponse {
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

    let auth_header = headers.get("x-sentry-auth").and_then(|v| v.to_str().ok());
    let raw_query_key = q.sentry_key.as_deref();
    let key = auth::extract_sentry_key(auth_header, None)
        .or_else(|| raw_query_key.map(str::to_string));
    let Some(public_key) = key else {
        metrics::counter!("crashbox_events_dropped_total", "reason" => "bad_key").increment(1);
        return refuse(StatusCode::UNAUTHORIZED, "missing sentry_key");
    };

    let project = match projects::find_by_public_key(&state.db, &public_key).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            metrics::counter!("crashbox_events_dropped_total", "reason" => "bad_key")
                .increment(1);
            return refuse(StatusCode::UNAUTHORIZED, "unknown sentry_key");
        }
        Err(e) => {
            tracing::error!(error = %e, "db error looking up project");
            metrics::counter!("crashbox_events_dropped_total", "reason" => "db_error")
                .increment(1);
            return refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
        }
    };
    if project.id != project_id {
        metrics::counter!("crashbox_events_dropped_total", "reason" => "bad_key").increment(1);
        return refuse(
            StatusCode::UNAUTHORIZED,
            "sentry_key does not match project_id in path",
        );
    }

    metrics::counter!(
        "crashbox_envelope_bytes_total",
        "project" => project.slug.clone()
    )
    .increment(body.len() as u64);

    let decision = state.rate_limiter.check(project.id);
    if !decision.allowed {
        metrics::counter!(
            "crashbox_events_dropped_total",
            "reason" => "rate_limit",
            "project" => project.slug.clone()
        )
        .increment(1);
        let mut resp = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limit exceeded"})),
        )
            .into_response();
        if let Ok(v) = decision.retry_after.to_string().parse() {
            resp.headers_mut().insert("retry-after", v);
        }
        return resp;
    }

    let env = match envelope::parse(&body) {
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
    let max_event_bytes = state.config.ingest.max_event_bytes;

    for item in &env.items {
        if !item.is_event() {
            tracing::debug!(
                ty = item.header.ty.as_deref().unwrap_or("unknown"),
                "skipping non-event item"
            );
            continue;
        }
        if item.payload.len() > max_event_bytes {
            return refuse(
                StatusCode::PAYLOAD_TOO_LARGE,
                "event exceeds CRASHBOX_MAX_EVENT_BYTES",
            );
        }
        let raw_str = match std::str::from_utf8(&item.payload) {
            Ok(s) => s,
            Err(_) => return refuse(StatusCode::BAD_REQUEST, "event payload is not valid UTF-8"),
        };
        let parsed: Value = match serde_json::from_str(raw_str) {
            Ok(v) => v,
            Err(_) => return refuse(StatusCode::BAD_REQUEST, "event payload is not valid JSON"),
        };

        let mut ev = normalize::from_value(&parsed);
        // Envelope header may carry event_id when the event payload omits it.
        if ev.event_id.is_none() {
            ev.event_id = env.header.event_id.clone();
        }

        let fp = grouping::fingerprint(&ev);
        let title = normalize::title_for(&ev);
        let event_ts = ev.timestamp.clone().unwrap_or_else(|| Utc::now().to_rfc3339());

        match store_event(&state, project.id, &ev, &fp, &title, &event_ts, raw_str).await {
            Ok((issue_id, outcome, new_event_count)) => {
                metrics::counter!(
                    "crashbox_events_ingested_total",
                    "project" => project.slug.clone(),
                    "level" => ev.level.clone().unwrap_or_else(|| "unknown".to_string()),
                )
                .increment(1);
                stored_event_id = ev.event_id.clone().or_else(|| Some(String::new()));
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
                        state.notify.fire(Notification {
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
                        });
                    }
                }

                // MVP: process only the first event item per envelope.
                break;
            }
            Err(e) => {
                tracing::error!(error = %e, "store_event failed");
                metrics::counter!(
                    "crashbox_events_dropped_total",
                    "reason" => "db_error"
                )
                .increment(1);
                return refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
            }
        }
    }

    let id_for_response = stored_event_id.unwrap_or_default();
    (StatusCode::OK, Json(json!({ "id": id_for_response }))).into_response()
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
    let new_event_count: i64 =
        sqlx::query_scalar("SELECT event_count FROM issues WHERE id = ?")
            .bind(issue_id)
            .fetch_one(tx.acquire())
            .await?;
    tx.commit().await?;
    Ok((issue_id, outcome, new_event_count))
}

fn refuse(status: StatusCode, msg: &str) -> axum::response::Response {
    (status, Json(json!({"error": msg}))).into_response()
}
