use axum::extract::{Path, Query, RawQuery, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::app_state::AppState;
use crate::db::{events, issues};
use crate::http::error::{AppError, AppResult};
use crate::http::Paginated;
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
    /// One of `last_seen` (default), `first_seen`, `event_count`.
    #[serde(default)]
    pub sort: Option<String>,
    /// `desc` (default) or `asc`.
    #[serde(default)]
    pub order: Option<String>,
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
) -> AppResult<Json<Paginated<issues::IssueWithSparkline>>> {
    if let Some(sort) = q.sort.as_deref() {
        if !issues::SORT_COLUMNS.contains(&sort) {
            return Err(AppError::BadRequest(format!(
                "sort must be one of {}",
                issues::SORT_COLUMNS.join(", ")
            )));
        }
    }
    let tags = parse_tag_query(raw.as_deref());
    let filters = issues::IssueFilters {
        status: q.status,
        level: q.level,
        environment: q.environment,
        release: q.release,
        query: q.query,
        tags,
        sort: q.sort,
        order: q.order,
        limit: q.limit.unwrap_or(50),
        offset: q.offset.unwrap_or(0),
    };
    let total = issues::count(&state.db, project_id, &filters).await?;
    let rows = issues::list(&state.db, project_id, &filters).await?;
    Ok(Json(Paginated {
        items: issues::with_sparklines(&state.db, rows).await?,
        total,
    }))
}

/// GET /api/projects/:project_id/events — project-wide event feed, newest first.
/// `?q=` is full-text over the raw payload (stack frames, breadcrumbs, URLs — everything).
pub async fn project_events(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(project_id): Path<i64>,
    Query(q): Query<EventListQuery>,
) -> AppResult<Json<Paginated<events::EventRow>>> {
    let filters = events::EventFilters {
        q: q.q,
        level: q.level,
        environment: q.environment,
        limit: q.limit.unwrap_or(50),
        offset: q.offset.unwrap_or(0),
    };
    let total = events::count_by_project(&state.db, project_id, &filters).await?;
    let items = events::list_by_project(&state.db, project_id, &filters).await?;
    Ok(Json(Paginated { items, total }))
}

#[derive(Debug, Deserialize)]
pub struct EventListQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
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

/// PATCH /api/issues — bulk status/snooze over a list of issue ids.
pub async fn bulk_patch(
    _user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<BulkPatchIssues>,
) -> AppResult<Json<Value>> {
    if body.ids.is_empty() {
        return Err(AppError::BadRequest("ids must be non-empty".into()));
    }
    if body.ids.len() > BULK_MAX_IDS {
        return Err(AppError::BadRequest(format!(
            "at most {BULK_MAX_IDS} ids per request"
        )));
    }
    if body.status.is_none() && body.snooze.is_none() {
        return Err(AppError::BadRequest("status or snooze required".into()));
    }
    if let Some(status) = body.status.as_deref() {
        if status != "resolved" && status != "unresolved" {
            return Err(AppError::BadRequest(
                "status must be resolved or unresolved".into(),
            ));
        }
    }
    let snoozed_until = match body.snooze.as_deref() {
        None => None,
        Some(s) => Some(parse_snooze(s).ok_or_else(|| {
            AppError::BadRequest(format!(
                "snooze must be one of 1h, 1d, 1w, forever, wake; got {s:?}"
            ))
        })?),
    };

    let mut updated = 0u64;
    for id in &body.ids {
        let mut touched = 0u64;
        if let Some(status) = body.status.as_deref() {
            touched += issues::set_status(&state.db, *id, status).await?;
        }
        if let Some(until) = &snoozed_until {
            touched += issues::set_snooze(&state.db, *id, until.as_deref()).await?;
        }
        if touched > 0 {
            updated += 1;
        }
    }
    Ok(Json(serde_json::json!({ "updated": updated })))
}

const BULK_MAX_IDS: usize = 500;

#[derive(Debug, Deserialize)]
pub struct BulkPatchIssues {
    pub ids: Vec<i64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub snooze: Option<String>,
}

/// DELETE /api/issues/:id (admin only) — removes the issue and all of its events.
pub async fn remove(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    if !user.is_admin {
        return Err(AppError::Forbidden);
    }
    let affected = issues::delete(&state.db, id).await?;
    if affected == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/issues/:id/events
pub async fn list_events(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<IssueListQuery>,
) -> AppResult<Json<Paginated<events::EventRow>>> {
    let limit = q.limit.unwrap_or(50);
    let offset = q.offset.unwrap_or(0);
    let total = events::count_by_issue(&state.db, id).await?;
    let items = events::list_by_issue(&state.db, id, limit, offset).await?;
    Ok(Json(Paginated { items, total }))
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
