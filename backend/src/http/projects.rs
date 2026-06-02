use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use ulid::Ulid;

use crate::app_state::AppState;
use crate::db::projects;
use crate::http::error::{AppError, AppResult};
use crate::security::sessions::AuthUser;
use crate::sentry::dsn::Dsn;

#[derive(Debug, Deserialize)]
pub struct CreateProject {
    pub name: String,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub default_environment: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProject {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub default_environment: Option<String>,
}

/// GET /api/projects
pub async fn list(
    _user: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<projects::Project>>> {
    Ok(Json(projects::list(&state.db).await?))
}

/// GET /api/projects/overview
/// Returns each project with unresolved-issue count, 24h event count, and the 3 most-recent
/// issues. Designed as the single round-trip for the Projects dashboard page.
pub async fn overview(
    _user: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<projects::ProjectOverview>>> {
    Ok(Json(projects::list_with_overview(&state.db).await?))
}

/// POST /api/projects (admin only)
pub async fn create(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateProject>,
) -> AppResult<impl IntoResponse> {
    if !user.is_admin {
        return Err(AppError::Forbidden);
    }
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    let slug = slugify(name);
    let public_key = Ulid::new().to_string().to_lowercase();
    let id = projects::insert(
        &state.db,
        name,
        &slug,
        body.platform.as_deref(),
        body.default_environment.as_deref(),
        &public_key,
        None,
    )
    .await?;
    let project = projects::find_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok((StatusCode::CREATED, Json(project)))
}

/// GET /api/projects/:id
pub async fn get(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<projects::Project>> {
    let project = projects::find_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(project))
}

/// PATCH /api/projects/:id (admin only)
pub async fn patch(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateProject>,
) -> AppResult<Json<projects::Project>> {
    if !user.is_admin {
        return Err(AppError::Forbidden);
    }
    let affected = projects::update(
        &state.db,
        id,
        body.name.as_deref(),
        body.platform.as_deref(),
        body.default_environment.as_deref(),
    )
    .await?;
    if affected == 0 {
        return Err(AppError::NotFound);
    }
    let project = projects::find_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(project))
}

/// GET /api/projects/:id/dsn
pub async fn dsn(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let project = projects::find_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    let url = Dsn::build(&state.config.public_url, &project.public_key, project.id);
    Ok(Json(
        json!({ "dsn": url, "public_key": project.public_key }),
    ))
}

/// POST /api/projects/:id/rotate-key (admin only)
pub async fn rotate_key(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    if !user.is_admin {
        return Err(AppError::Forbidden);
    }
    let new_key = Ulid::new().to_string().to_lowercase();
    let affected = projects::rotate_key(&state.db, id, &new_key).await?;
    if affected == 0 {
        return Err(AppError::NotFound);
    }
    let dsn = Dsn::build(&state.config.public_url, &new_key, id);
    tracing::warn!(project_id = id, "project key rotated");
    Ok(Json(json!({ "public_key": new_key, "dsn": dsn })))
}

fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for ch in s.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_end_matches('-').to_string();
    if trimmed.is_empty() {
        format!("project-{}", Ulid::new().to_string().to_lowercase())
    } else {
        trimmed
    }
}
