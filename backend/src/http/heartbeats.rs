//! Heartbeat monitor endpoints: the public ping receiver + admin CRUD.
//!
//! The ping endpoint is authenticated by the unguessable `ping_key` alone (same trust model
//! as DSN-key ingest) and accepts both GET and POST so a bare `curl <url>` at the end of a
//! cron line works. It must never panic on garbage input.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ulid::Ulid;

use crate::app_state::AppState;
use crate::db::heartbeats::{self, HeartbeatMonitor};
use crate::db::projects;
use crate::http::error::{AppError, AppResult};
use crate::http::Paginated;
use crate::notify::{HeartbeatKind, HeartbeatNotification, Notification};
use crate::security::sessions::AuthUser;

const PERIOD_MIN_SECONDS: i64 = 10;
const PERIOD_MAX_SECONDS: i64 = 30 * 24 * 3600;
const GRACE_MAX_SECONDS: i64 = 24 * 3600;
const GRACE_DEFAULT_SECONDS: i64 = 60;
const NAME_MAX_CHARS: usize = 200;
const DESCRIPTION_MAX_CHARS: usize = 500;

#[derive(Debug, Deserialize)]
pub struct CreateMonitor {
    pub name: String,
    /// Optional human note ("what breaks if this stops"). Blank is treated as absent.
    #[serde(default)]
    pub description: Option<String>,
    pub period_seconds: i64,
    #[serde(default)]
    pub grace_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMonitor {
    #[serde(default)]
    pub name: Option<String>,
    /// Present-and-blank clears the note; absent keeps it.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub period_seconds: Option<i64>,
    #[serde(default)]
    pub grace_seconds: Option<i64>,
    /// Only `paused` (pause) and `pending` (resume) are accepted here; `up` and `down` are
    /// owned by pings and the sweep job.
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MonitorResponse {
    #[serde(flatten)]
    pub monitor: HeartbeatMonitor,
    pub ping_url: String,
}

fn to_response(state: &AppState, monitor: HeartbeatMonitor) -> MonitorResponse {
    let base = state.config.public_url.trim_end_matches('/');
    let ping_url = format!("{base}/ping/{}", monitor.ping_key);
    MonitorResponse { monitor, ping_url }
}

/// GET|POST /ping/:ping_key — public. 200 "OK" on success, uniform 404 for unknown keys.
pub async fn ping(State(state): State<AppState>, Path(ping_key): Path<String>) -> Response {
    let monitor = match heartbeats::find_by_ping_key(&state.db, &ping_key).await {
        Ok(Some(m)) => m,
        Ok(None) => return not_found(),
        Err(e) => {
            tracing::error!(error = %e, "heartbeat: ping lookup failed");
            return internal_error();
        }
    };

    // Rate-limit keyed by monitor id — a bounded set. Keying by the raw path string would let
    // random-key floods grow the bucket map without bound.
    let decision = state.heartbeat_rate_limiter.check(monitor.id);
    if !decision.allowed {
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

    match heartbeats::record_ping(&state.db, monitor.id).await {
        Ok(Some(outcome)) => {
            tracing::debug!(
                monitor_id = outcome.monitor.id,
                project_id = outcome.monitor.project_id,
                was_down = outcome.was_down,
                "heartbeat: ping recorded"
            );
            metrics::counter!("crashbox_heartbeat_pings_total").increment(1);
            if outcome.was_down {
                metrics::counter!("crashbox_heartbeat_transitions_total", "to" => "up")
                    .increment(1);
                fire_recovery(&state, &outcome).await;
            }
            (StatusCode::OK, "OK").into_response()
        }
        // Deleted between lookup and write — the URL is already invalid.
        Ok(None) => not_found(),
        Err(e) => {
            tracing::error!(monitor_id = monitor.id, error = %e, "heartbeat: ping write failed");
            internal_error()
        }
    }
}

/// Fire `heartbeat_recovered` for a ping that flipped a monitor out of `down`. Best-effort:
/// a failed project lookup only costs the notification, never the 200 to the pinging cron.
async fn fire_recovery(state: &AppState, outcome: &heartbeats::PingOutcome) {
    if state.notify.is_empty() {
        return;
    }
    let m = &outcome.monitor;
    let project = match projects::find_by_id(&state.db, m.project_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(
                monitor_id = m.id,
                error = %e,
                "heartbeat: project lookup for recovery notification failed"
            );
            return;
        }
    };
    tracing::info!(
        monitor_id = m.id,
        project_id = m.project_id,
        downtime_seconds = outcome.downtime_seconds,
        "heartbeat: monitor recovered"
    );
    state
        .notify
        .fire(Notification::Heartbeat(HeartbeatNotification {
            kind: HeartbeatKind::HeartbeatRecovered,
            project_name: project.name,
            project_slug: project.slug,
            monitor_id: m.id,
            monitor_name: m.name.clone(),
            overdue_seconds: None,
            downtime_seconds: outcome.downtime_seconds,
            link: state.notify.build_heartbeat_link(m.project_id),
        }));
}

/// GET /api/projects/:project_id/heartbeats
pub async fn list(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
) -> AppResult<Json<Vec<MonitorResponse>>> {
    projects::find_by_id(&state.db, project_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let monitors = heartbeats::list_for_project(&state.db, project_id).await?;
    Ok(Json(
        monitors
            .into_iter()
            .map(|m| to_response(&state, m))
            .collect(),
    ))
}

/// POST /api/projects/:project_id/heartbeats (admin only)
pub async fn create(
    user: AuthUser,
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    Json(body): Json<CreateMonitor>,
) -> AppResult<impl IntoResponse> {
    if !user.is_admin {
        return Err(AppError::Forbidden);
    }
    projects::find_by_id(&state.db, project_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let name = validate_name(&body.name)?;
    let description = validate_description(body.description.as_deref())?;
    validate_period(body.period_seconds)?;
    let grace = body.grace_seconds.unwrap_or(GRACE_DEFAULT_SECONDS);
    validate_grace(grace)?;

    let ping_key = Ulid::new().to_string().to_lowercase();
    let id = heartbeats::insert(
        &state.db,
        project_id,
        name,
        description,
        &ping_key,
        body.period_seconds,
        grace,
    )
    .await?;
    let monitor = heartbeats::find_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok((StatusCode::CREATED, Json(to_response(&state, monitor))))
}

/// PATCH /api/heartbeats/:id (admin only)
pub async fn patch(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateMonitor>,
) -> AppResult<Json<MonitorResponse>> {
    if !user.is_admin {
        return Err(AppError::Forbidden);
    }
    heartbeats::find_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;

    let name = match body.name.as_deref() {
        Some(raw) => Some(validate_name(raw)?),
        None => None,
    };
    // Three-state: absent = keep, blank = clear, value = set.
    let description = match body.description.as_deref() {
        Some(raw) => Some(validate_description(Some(raw))?),
        None => None,
    };
    if let Some(period) = body.period_seconds {
        validate_period(period)?;
    }
    if let Some(grace) = body.grace_seconds {
        validate_grace(grace)?;
    }
    if let Some(status) = body.status.as_deref() {
        if status != heartbeats::STATUS_PAUSED && status != heartbeats::STATUS_PENDING {
            return Err(AppError::BadRequest(
                "status must be 'paused' or 'pending' (resume)".into(),
            ));
        }
    }

    if name.is_some()
        || description.is_some()
        || body.period_seconds.is_some()
        || body.grace_seconds.is_some()
    {
        heartbeats::update(
            &state.db,
            id,
            name,
            description,
            body.period_seconds,
            body.grace_seconds,
        )
        .await?;
    }
    if let Some(status) = body.status.as_deref() {
        heartbeats::set_status(&state.db, id, status).await?;
    }

    let monitor = heartbeats::find_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(to_response(&state, monitor)))
}

/// GET /api/heartbeats/:id/history — status transitions, newest first. History depth is
/// bounded by the retention job (CRASHBOX_RETENTION_DAYS).
pub async fn history(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<HistoryQuery>,
) -> AppResult<Json<Paginated<heartbeats::HeartbeatTransition>>> {
    heartbeats::find_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    let total = heartbeats::count_transitions(&state.db, id).await?;
    let items =
        heartbeats::list_transitions(&state.db, id, q.limit.unwrap_or(50), q.offset.unwrap_or(0))
            .await?;
    Ok(Json(Paginated { items, total }))
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// DELETE /api/heartbeats/:id (admin only) — invalidates the ping URL immediately.
pub async fn remove(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    if !user.is_admin {
        return Err(AppError::Forbidden);
    }
    let affected = heartbeats::delete(&state.db, id).await?;
    if affected == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

fn validate_name(raw: &str) -> Result<&str, AppError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    if name.chars().count() > NAME_MAX_CHARS {
        return Err(AppError::BadRequest(format!(
            "name must be at most {NAME_MAX_CHARS} characters"
        )));
    }
    Ok(name)
}

/// Trims and bounds the note; a blank input becomes `None` (create: absent, patch: clear).
fn validate_description(raw: Option<&str>) -> Result<Option<&str>, AppError> {
    let Some(trimmed) = raw.map(str::trim) else {
        return Ok(None);
    };
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > DESCRIPTION_MAX_CHARS {
        return Err(AppError::BadRequest(format!(
            "description must be at most {DESCRIPTION_MAX_CHARS} characters"
        )));
    }
    Ok(Some(trimmed))
}

fn validate_period(v: i64) -> Result<(), AppError> {
    if !(PERIOD_MIN_SECONDS..=PERIOD_MAX_SECONDS).contains(&v) {
        return Err(AppError::BadRequest(format!(
            "period_seconds must be between {PERIOD_MIN_SECONDS} and {PERIOD_MAX_SECONDS}"
        )));
    }
    Ok(())
}

fn validate_grace(v: i64) -> Result<(), AppError> {
    if !(0..=GRACE_MAX_SECONDS).contains(&v) {
        return Err(AppError::BadRequest(format!(
            "grace_seconds must be between 0 and {GRACE_MAX_SECONDS}"
        )));
    }
    Ok(())
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response()
}

fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "internal error"})),
    )
        .into_response()
}
