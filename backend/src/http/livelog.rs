//! HTTP adapter for Live Logs ingest. Thin: authenticate by DSN, rate-limit, parse the batch, and
//! publish each record into the in-memory hub. Nothing is written to the database.
//!
//! The streaming (read) side is added in a later slice; this module only handles ingest plus the
//! shared helper used by the envelope endpoint for Sentry `log` items.

use std::convert::Infallible;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::app_state::AppState;
use crate::db::projects;
use crate::http::dsn_auth::{self, DsnAuthError};
use crate::http::ingest::IngestQuery;
use crate::livelog::{LogLevel, LogRecord, SubscribeError};
use crate::security::sessions::AuthUser;

/// `POST /api/:project_id/logs[/]` — accept a batch of loose log records (a JSON array, a single
/// object, or newline-delimited JSON) and fan them out to the project's live stream. Returns 202
/// with how many records were accepted vs. skipped (malformed entries are skipped, not fatal).
pub async fn logs_ingest(
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    Query(q): Query<IngestQuery>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    if !state.config.livelog.enabled {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, "live logs are disabled");
    }
    if body.len() > state.config.livelog.max_batch_bytes {
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            "log batch exceeds CRASHBOX_MAX_LOG_BATCH_BYTES",
        );
    }

    let auth_header = headers.get("x-sentry-auth").and_then(|v| v.to_str().ok());
    let project = match dsn_auth::resolve_project(
        &state.db,
        project_id,
        auth_header,
        q.sentry_key.as_deref(),
    )
    .await
    {
        Ok(p) => p,
        Err(DsnAuthError::MissingKey) => {
            return refuse(StatusCode::UNAUTHORIZED, "missing sentry_key")
        }
        Err(DsnAuthError::UnknownKey) => {
            return refuse(StatusCode::UNAUTHORIZED, "unknown sentry_key")
        }
        Err(DsnAuthError::ProjectMismatch) => {
            return refuse(
                StatusCode::UNAUTHORIZED,
                "sentry_key does not match project_id in path",
            )
        }
        Err(DsnAuthError::Db(e)) => {
            tracing::error!(error = %e, "db error looking up project for logs");
            return refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
        }
    };

    let decision = state.log_rate_limiter.check(project.id);
    if !decision.allowed {
        let mut resp = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "log rate limit exceeded"})),
        )
            .into_response();
        if let Ok(v) = decision.retry_after.to_string().parse() {
            resp.headers_mut().insert("retry-after", v);
        }
        return resp;
    }

    let max_msg = state.config.livelog.message_max_bytes;
    let mut accepted = 0u32;
    let mut skipped = 0u32;
    for value in parse_batch(&body) {
        match LogRecord::from_loose(&value, max_msg) {
            Some(rec) => {
                state.livelog.publish(project.id, rec);
                accepted += 1;
            }
            None => skipped += 1,
        }
    }

    if accepted > 0 {
        metrics::counter!("crashbox_livelog_received_total", "project" => project.slug.clone())
            .increment(u64::from(accepted));
    }
    if skipped > 0 {
        metrics::counter!(
            "crashbox_livelog_dropped_total",
            "project" => project.slug.clone(),
            "reason" => "bad_record"
        )
        .increment(u64::from(skipped));
    }

    (
        StatusCode::ACCEPTED,
        Json(json!({ "accepted": accepted, "skipped": skipped })),
    )
        .into_response()
}

/// Server-side filters for the live stream — applied before bytes hit the wire so a noisy project
/// doesn't flood the browser. All are optional and combine with AND.
#[derive(Debug, Default, Deserialize)]
pub struct StreamFilter {
    /// Minimum severity to emit (e.g. `warn` drops trace/debug/info).
    level: Option<String>,
    /// Case-insensitive substring match on the record's `logger`.
    logger: Option<String>,
    /// Case-insensitive substring match on the record's `message`.
    q: Option<String>,
}

struct CompiledFilter {
    min_rank: u8,
    logger: Option<String>,
    q: Option<String>,
}

impl StreamFilter {
    fn compile(self) -> CompiledFilter {
        CompiledFilter {
            min_rank: self
                .level
                .as_deref()
                .map_or(0, |l| LogLevel::parse(l).rank()),
            logger: self.logger.map(|s| s.to_lowercase()),
            q: self.q.map(|s| s.to_lowercase()),
        }
    }
}

impl CompiledFilter {
    fn matches(&self, rec: &LogRecord) -> bool {
        if rec.level.rank() < self.min_rank {
            return false;
        }
        if let Some(needle) = &self.logger {
            let hit = rec
                .logger
                .as_deref()
                .is_some_and(|l| l.to_lowercase().contains(needle));
            if !hit {
                return false;
            }
        }
        if let Some(needle) = &self.q {
            if !rec.message.to_lowercase().contains(needle) {
                return false;
            }
        }
        true
    }
}

/// `GET /api/projects/:id/logs/stream` — session-authed SSE tail. Replays the scrollback snapshot
/// then streams live records, both subject to the query filters. A lagging client is dropped by the
/// broadcast layer (records are skipped, the stream stays open). Heartbeats keep proxies from
/// timing the connection out.
pub async fn logs_stream(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    Query(filter): Query<StreamFilter>,
) -> axum::response::Response {
    if !state.config.livelog.enabled {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, "live logs are disabled");
    }

    match projects::find_by_id(&state.db, project_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return refuse(StatusCode::NOT_FOUND, "unknown project"),
        Err(e) => {
            tracing::error!(error = %e, "db error resolving project for log stream");
            return refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
        }
    }

    let subscription = match state.livelog.subscribe(project_id) {
        Ok(s) => s,
        Err(SubscribeError::TooManySubscribers) => {
            return refuse(
                StatusCode::TOO_MANY_REQUESTS,
                "too many concurrent log subscribers for this project",
            )
        }
    };

    let filter = filter.compile();
    // Tracks active subscribers as a gauge; decremented when the stream (and thus this guard,
    // captured in the closure below) is dropped on client disconnect.
    let guard = SubscriberGuard::new();

    // Snapshot replay (already in memory) chained with the live broadcast. Broadcast `Lagged`
    // errors are dropped — that's the intended lossy behavior for a live tail.
    let snapshot = tokio_stream::iter(subscription.snapshot);
    let live = BroadcastStream::new(subscription.rx).filter_map(Result::ok);
    let events = snapshot.chain(live).filter_map(move |rec| {
        let _keep = &guard;
        if filter.matches(&rec) {
            Some(sse_event(&rec))
        } else {
            None
        }
    });

    Sse::new(events)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response()
}

/// `GET /api/projects/:id/logs/recent` — one-shot snapshot of the in-RAM scrollback, oldest
/// first, same filters as the stream plus `limit` (keeps the newest N after filtering).
/// The fetch-once counterpart to the SSE tail: an API client gets current logs in a single
/// request instead of opening a stream and cutting it off.
pub async fn logs_recent(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    Query(q): Query<RecentQuery>,
) -> axum::response::Response {
    if !state.config.livelog.enabled {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, "live logs are disabled");
    }
    match projects::find_by_id(&state.db, project_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return refuse(StatusCode::NOT_FOUND, "unknown project"),
        Err(e) => {
            tracing::error!(error = %e, "db error resolving project for recent logs");
            return refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
        }
    }

    let filter = StreamFilter {
        level: q.level,
        logger: q.logger,
        q: q.q,
    }
    .compile();
    let mut items: Vec<_> = state
        .livelog
        .snapshot(project_id)
        .into_iter()
        .filter(|rec| filter.matches(rec))
        .collect();
    let limit = q.limit.unwrap_or(usize::MAX).max(1);
    if items.len() > limit {
        items.drain(..items.len() - limit);
    }
    Json(json!({ "items": items, "count": items.len() })).into_response()
}

/// Same knobs as `StreamFilter` plus `limit`. Kept as a flat struct — `serde(flatten)` breaks
/// numeric fields under axum's urlencoded `Query` deserializer.
#[derive(Debug, Default, Deserialize)]
pub struct RecentQuery {
    level: Option<String>,
    logger: Option<String>,
    q: Option<String>,
    limit: Option<usize>,
}

// `Sse` requires `Item = Result<Event, E>`, so the wrap is mandatory despite always being `Ok`.
#[allow(clippy::unnecessary_wraps)]
fn sse_event(rec: &LogRecord) -> Result<Event, Infallible> {
    Ok(Event::default()
        .json_data(rec)
        .unwrap_or_else(|_| Event::default().data("{}")))
}

/// Publish the records carried by a Sentry `log` envelope item. Best-effort: a payload that doesn't
/// parse is silently dropped (the rest of the envelope is unaffected). Returns how many records were
/// published so the caller can record the metric with the project label it already holds.
pub(crate) fn ingest_log_item(state: &AppState, project_id: i64, payload: &[u8]) -> usize {
    let Ok(value) = serde_json::from_slice::<Value>(payload) else {
        return 0;
    };
    let max_msg = state.config.livelog.message_max_bytes;
    let records = LogRecord::from_sentry_batch(&value, max_msg);
    let count = records.len();
    for rec in records {
        state.livelog.publish(project_id, rec);
    }
    count
}

/// RAII gauge for active SSE subscribers. Incremented on construction, decremented on drop.
struct SubscriberGuard;

impl SubscriberGuard {
    fn new() -> Self {
        metrics::gauge!("crashbox_livelog_active_subscribers").increment(1.0);
        Self
    }
}

impl Drop for SubscriberGuard {
    fn drop(&mut self) {
        metrics::gauge!("crashbox_livelog_active_subscribers").decrement(1.0);
    }
}

/// Accept a JSON array, a single JSON object, or newline-delimited JSON. Whole-body JSON is tried
/// first; on failure we fall back to per-line parsing so one bad line can't poison the batch.
fn parse_batch(body: &[u8]) -> Vec<Value> {
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        return match value {
            Value::Array(items) => items,
            other => vec![other],
        };
    }
    body.split(|b| *b == b'\n')
        .filter_map(|line| {
            let trimmed = trim_ascii(line);
            if trimmed.is_empty() {
                None
            } else {
                serde_json::from_slice::<Value>(trimmed).ok()
            }
        })
        .collect()
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = bytes {
        if first.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    while let [rest @ .., last] = bytes {
        if last.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

fn refuse(status: StatusCode, msg: &str) -> axum::response::Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_batch_handles_array_object_and_ndjson() {
        assert_eq!(parse_batch(br#"[{"a":1},{"b":2}]"#).len(), 2);
        assert_eq!(parse_batch(br#"{"a":1}"#).len(), 1);
        assert_eq!(parse_batch(b"{\"a\":1}\n{\"b\":2}\n").len(), 2);
    }

    #[test]
    fn parse_batch_skips_blank_and_bad_ndjson_lines() {
        let recs = parse_batch(b"{\"a\":1}\n\nnot json\n{\"b\":2}\n");
        assert_eq!(recs.len(), 2);
    }
}
