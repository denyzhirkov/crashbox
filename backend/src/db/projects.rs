use chrono::{Duration, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::db::issues::Issue;

#[derive(Debug, sqlx::FromRow, Clone, serde::Serialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub platform: Option<String>,
    pub default_environment: Option<String>,
    pub public_key: String,
    #[serde(skip)]
    pub secret_key_hash: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn count(pool: &SqlitePool) -> sqlx::Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM projects")
        .fetch_one(pool)
        .await
}

pub async fn list(pool: &SqlitePool) -> sqlx::Result<Vec<Project>> {
    sqlx::query_as::<_, Project>(
        "SELECT id, name, slug, platform, default_environment, public_key, \
                secret_key_hash, created_at, updated_at \
         FROM projects ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn find_by_public_key(
    pool: &SqlitePool,
    public_key: &str,
) -> sqlx::Result<Option<Project>> {
    sqlx::query_as::<_, Project>(
        "SELECT id, name, slug, platform, default_environment, public_key, \
                secret_key_hash, created_at, updated_at \
         FROM projects WHERE public_key = ?",
    )
    .bind(public_key)
    .fetch_optional(pool)
    .await
}

pub async fn find_by_id(pool: &SqlitePool, id: i64) -> sqlx::Result<Option<Project>> {
    sqlx::query_as::<_, Project>(
        "SELECT id, name, slug, platform, default_environment, public_key, \
                secret_key_hash, created_at, updated_at \
         FROM projects WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn insert(
    pool: &SqlitePool,
    name: &str,
    slug: &str,
    platform: Option<&str>,
    default_environment: Option<&str>,
    public_key: &str,
    secret_key_hash: Option<&str>,
) -> sqlx::Result<i64> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO projects \
            (name, slug, platform, default_environment, public_key, secret_key_hash, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(name)
    .bind(slug)
    .bind(platform)
    .bind(default_environment)
    .bind(public_key)
    .bind(secret_key_hash)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(row.last_insert_rowid())
}

pub async fn update(
    pool: &SqlitePool,
    id: i64,
    name: Option<&str>,
    platform: Option<&str>,
    default_environment: Option<&str>,
) -> sqlx::Result<u64> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE projects SET \
            name = COALESCE(?, name), \
            platform = COALESCE(?, platform), \
            default_environment = COALESCE(?, default_environment), \
            updated_at = ? \
         WHERE id = ?",
    )
    .bind(name)
    .bind(platform)
    .bind(default_environment)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Per-project summary for the Projects dashboard. Joins three small indexed queries; cheap
/// enough that we don't bother with a single window-function aggregate.
#[derive(Debug, Serialize)]
pub struct ProjectOverview {
    #[serde(flatten)]
    pub project: Project,
    pub unresolved_count: i64,
    pub events_24h: i64,
    pub recent_issues: Vec<Issue>,
}

pub async fn list_with_overview(pool: &SqlitePool) -> sqlx::Result<Vec<ProjectOverview>> {
    let projects = list(pool).await?;
    let cutoff_24h = (Utc::now() - Duration::hours(24)).to_rfc3339();
    let mut out = Vec::with_capacity(projects.len());
    for p in projects {
        let unresolved_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM issues WHERE project_id = ? AND status = 'unresolved'",
        )
        .bind(p.id)
        .fetch_one(pool)
        .await?;
        let events_24h: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE project_id = ? AND received_at >= ?",
        )
        .bind(p.id)
        .bind(&cutoff_24h)
        .fetch_one(pool)
        .await?;
        let recent_issues = sqlx::query_as::<_, Issue>(
            "SELECT * FROM issues WHERE project_id = ? \
             ORDER BY last_seen DESC LIMIT 3",
        )
        .bind(p.id)
        .fetch_all(pool)
        .await?;
        out.push(ProjectOverview {
            project: p,
            unresolved_count,
            events_24h,
            recent_issues,
        });
    }
    Ok(out)
}

pub async fn rotate_key(pool: &SqlitePool, id: i64, new_public_key: &str) -> sqlx::Result<u64> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE projects SET public_key = ?, updated_at = ? WHERE id = ?",
    )
    .bind(new_public_key)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
