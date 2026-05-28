use chrono::Utc;
use sqlx::{QueryBuilder, Sqlite, SqliteConnection, SqlitePool};

#[derive(Debug, sqlx::FromRow, serde::Serialize, Clone)]
pub struct Issue {
    pub id: i64,
    pub project_id: i64,
    pub fingerprint: String,
    pub title: String,
    pub status: String,
    pub level: Option<String>,
    pub platform: Option<String>,
    pub first_seen: String,
    pub last_seen: String,
    pub event_count: i64,
    pub last_event_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Default, Clone)]
pub struct IssueFilters {
    pub status: Option<String>,
    pub level: Option<String>,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub query: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn find_by_id(pool: &SqlitePool, id: i64) -> sqlx::Result<Option<Issue>> {
    sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn list(
    pool: &SqlitePool,
    project_id: i64,
    f: &IssueFilters,
) -> sqlx::Result<Vec<Issue>> {
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT DISTINCT issues.* FROM issues ",
    );
    let needs_event_join = f.environment.is_some() || f.release.is_some();
    if needs_event_join {
        qb.push("JOIN events ON events.issue_id = issues.id ");
    }
    qb.push("WHERE issues.project_id = ");
    qb.push_bind(project_id);

    match f.status.as_deref() {
        Some("all") | None => {}
        Some(s) => {
            qb.push(" AND issues.status = ");
            qb.push_bind(s.to_string());
        }
    }
    if let Some(level) = &f.level {
        qb.push(" AND issues.level = ");
        qb.push_bind(level.clone());
    }
    if let Some(env) = &f.environment {
        qb.push(" AND events.environment = ");
        qb.push_bind(env.clone());
    }
    if let Some(rel) = &f.release {
        qb.push(" AND events.release = ");
        qb.push_bind(rel.clone());
    }
    if let Some(q) = &f.query {
        // Case-insensitive substring match on the issue title.
        qb.push(" AND issues.title LIKE ");
        qb.push_bind(format!("%{q}%"));
    }
    qb.push(" ORDER BY issues.last_seen DESC LIMIT ");
    qb.push_bind(f.limit.clamp(1, 500));
    qb.push(" OFFSET ");
    qb.push_bind(f.offset.max(0));

    qb.build_query_as::<Issue>().fetch_all(pool).await
}

pub async fn set_status(pool: &SqlitePool, id: i64, status: &str) -> sqlx::Result<u64> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query("UPDATE issues SET status = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Find an existing issue's id for (project_id, fingerprint), or insert a new one. Always returns
/// the issue id. The caller is expected to bump `event_count` / `last_seen` separately via
/// [`bump_after_event`] once the event row exists (so `last_event_id` can be set).
pub async fn upsert(
    conn: &mut SqliteConnection,
    project_id: i64,
    fingerprint: &str,
    title: &str,
    level: Option<&str>,
    platform: Option<&str>,
    timestamp_iso: &str,
) -> sqlx::Result<i64> {
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM issues WHERE project_id = ? AND fingerprint = ?",
    )
    .bind(project_id)
    .bind(fingerprint)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = existing {
        return Ok(id);
    }

    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO issues \
            (project_id, fingerprint, title, status, level, platform, \
             first_seen, last_seen, event_count, created_at, updated_at) \
         VALUES (?, ?, ?, 'unresolved', ?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(project_id)
    .bind(fingerprint)
    .bind(title)
    .bind(level)
    .bind(platform)
    .bind(timestamp_iso)
    .bind(timestamp_iso)
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;
    Ok(row.last_insert_rowid())
}

/// After the event row is inserted, update the issue's counters and pointers.
pub async fn bump_after_event(
    conn: &mut SqliteConnection,
    issue_id: i64,
    event_id_row: i64,
    timestamp_iso: &str,
) -> sqlx::Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE issues SET \
            event_count = event_count + 1, \
            last_seen = ?, \
            last_event_id = ?, \
            updated_at = ? \
         WHERE id = ?",
    )
    .bind(timestamp_iso)
    .bind(event_id_row)
    .bind(&now)
    .bind(issue_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
