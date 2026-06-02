use axum::extract::{Path, Query, RawQuery, State};
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::app_state::AppState;
use crate::db::{events, issues};
use crate::http::error::{AppError, AppResult};
use crate::security::sessions::AuthUser;

#[derive(Debug, Deserialize)]
pub struct IssueListQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// `?tag=k=v` can appear multiple times. `serde_urlencoded` (axum's default) doesn't decode
/// repeated keys into a Vec, so we extract them from the raw query string instead.
fn parse_tag_query(raw: Option<&str>) -> Vec<(String, String)> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    raw.split('&')
        .filter_map(|p| {
            let v = p.strip_prefix("tag=")?;
            // Both keys and values may be URL-encoded. Decode once, then split on the
            // FIRST '=' so values containing '=' stay intact.
            let decoded = percent_decode(v);
            let (k, val) = decoded.split_once('=')?;
            Some((k.to_string(), val.to_string()))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b as char);
                    i += 3;
                    continue;
                }
            }
        }
        if bytes[i] == b'+' {
            out.push(' ');
        } else {
            out.push(bytes[i] as char);
        }
        i += 1;
    }
    out
}

#[derive(Debug, Deserialize)]
pub struct PatchIssue {
    /// "resolved" or "unresolved". Optional — a request can change only the snooze.
    #[serde(default)]
    pub status: Option<String>,
    /// Snooze action: "1h" | "1d" | "1w" | "forever" | "wake". Optional.
    #[serde(default)]
    pub snooze: Option<String>,
}

/// GET /api/projects/:project_id/issues
pub async fn list(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    Query(q): Query<IssueListQuery>,
    RawQuery(raw): RawQuery,
) -> AppResult<Json<Vec<issues::IssueWithSparkline>>> {
    let tags = parse_tag_query(raw.as_deref());
    let filters = issues::IssueFilters {
        status: q.status,
        level: q.level,
        environment: q.environment,
        release: q.release,
        query: q.query,
        tags,
        limit: q.limit.unwrap_or(50),
        offset: q.offset.unwrap_or(0),
    };
    let rows = issues::list(&state.db, project_id, &filters).await?;
    Ok(Json(issues::with_sparklines(&state.db, rows).await?))
}

/// GET /api/issues/:id
pub async fn get(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<issues::Issue>> {
    issues::find_by_id(&state.db, id)
        .await?
        .map(Json)
        .ok_or(AppError::NotFound)
}

/// PATCH /api/issues/:id
pub async fn patch(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<PatchIssue>,
) -> AppResult<Json<issues::Issue>> {
    if body.status.is_none() && body.snooze.is_none() {
        return Err(AppError::BadRequest("status or snooze required".into()));
    }

    if let Some(status) = &body.status {
        let normalized = match status.as_str() {
            "resolved" | "unresolved" => status,
            _ => {
                return Err(AppError::BadRequest(
                    "status must be resolved or unresolved".into(),
                ));
            }
        };
        let affected = issues::set_status(&state.db, id, normalized).await?;
        if affected == 0 {
            return Err(AppError::NotFound);
        }
    }

    if let Some(snooze) = &body.snooze {
        let Some(snoozed_until) = parse_snooze(snooze) else {
            return Err(AppError::BadRequest(format!(
                "snooze must be one of 1h, 1d, 1w, forever, wake; got {snooze:?}"
            )));
        };
        let affected = issues::set_snooze(&state.db, id, snoozed_until.as_deref()).await?;
        if affected == 0 {
            return Err(AppError::NotFound);
        }
    }

    let issue = issues::find_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(issue))
}

/// Returns the value to store in `snoozed_until`:
/// - `Some(Some("forever"))` — forever-snooze
/// - `Some(Some("<rfc3339>"))` — time-bound snooze
/// - `Some(None)` — wake (clear snoozed_until)
/// - `None` — invalid input
fn parse_snooze(s: &str) -> Option<Option<String>> {
    match s {
        "wake" => Some(None),
        "forever" => Some(Some("forever".to_string())),
        "1h" => Some(Some(
            (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        )),
        "1d" => Some(Some(
            (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339(),
        )),
        "1w" => Some(Some(
            (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339(),
        )),
        _ => None,
    }
}

/// GET /api/issues/:id/events
pub async fn list_events(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<IssueListQuery>,
) -> AppResult<Json<Vec<events::EventRow>>> {
    let limit = q.limit.unwrap_or(50);
    let offset = q.offset.unwrap_or(0);
    Ok(Json(
        events::list_by_issue(&state.db, id, limit, offset).await?,
    ))
}

/// GET /api/events/:id
pub async fn get_event(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let row = events::find_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    // Surface event + parsed raw payload as `data` for the UI.
    let raw: Value = serde_json::from_str(&row.raw_json).unwrap_or(Value::Null);
    Ok(Json(serde_json::json!({
        "event": row,
        "data": raw,
    })))
}
