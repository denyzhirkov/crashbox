//! Shared DSN public-key authentication for public ingest endpoints (envelope + logs).
//!
//! Both endpoints authenticate the same way: extract `sentry_key` from the `X-Sentry-Auth` header
//! or query string, resolve the owning project, and assert it matches the `project_id` in the path.
//! Each caller maps the error variants to its own response and metrics, so this stays metric-free.

use sqlx::SqlitePool;

use crate::db::projects::{self, Project};
use crate::sentry::auth;

#[derive(Debug)]
pub enum DsnAuthError {
    MissingKey,
    UnknownKey,
    ProjectMismatch,
    Db(sqlx::Error),
}

/// Resolve and authorize the project for a DSN-authenticated request.
pub async fn resolve_project(
    db: &SqlitePool,
    project_id: i64,
    auth_header: Option<&str>,
    query_key: Option<&str>,
) -> Result<Project, DsnAuthError> {
    let key = auth::extract_sentry_key(auth_header, None)
        .or_else(|| query_key.map(str::to_string))
        .ok_or(DsnAuthError::MissingKey)?;

    let project = projects::find_by_public_key(db, &key)
        .await
        .map_err(DsnAuthError::Db)?
        .ok_or(DsnAuthError::UnknownKey)?;

    if project.id != project_id {
        return Err(DsnAuthError::ProjectMismatch);
    }
    Ok(project)
}
