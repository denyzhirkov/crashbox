use chrono::Utc;
use sqlx::{SqliteConnection, SqlitePool};

use crate::sentry::normalize::{Breadcrumb, NormalizedEvent};

#[derive(Debug, sqlx::FromRow, serde::Serialize, Clone)]
pub struct EventRow {
    pub id: i64,
    pub event_id: Option<String>,
    pub project_id: i64,
    pub issue_id: Option<i64>,
    pub timestamp: Option<String>,
    pub received_at: String,
    pub level: Option<String>,
    pub platform: Option<String>,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub logger: Option<String>,
    pub transaction_name: Option<String>,
    pub message: Option<String>,
    pub exception_type: Option<String>,
    pub exception_value: Option<String>,
    pub culprit: Option<String>,
    pub server_name: Option<String>,
    pub request_url: Option<String>,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub fingerprint: Option<String>,
    pub raw_json: String,
}

pub async fn find_by_id(pool: &SqlitePool, id: i64) -> sqlx::Result<Option<EventRow>> {
    sqlx::query_as::<_, EventRow>("SELECT * FROM events WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn list_by_issue(
    pool: &SqlitePool,
    issue_id: i64,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<EventRow>> {
    sqlx::query_as::<_, EventRow>(
        "SELECT * FROM events WHERE issue_id = ? \
         ORDER BY received_at DESC LIMIT ? OFFSET ?",
    )
    .bind(issue_id)
    .bind(limit.clamp(1, 500))
    .bind(offset.max(0))
    .fetch_all(pool)
    .await
}

/// Persistent caps so a malicious or buggy SDK can't bloat the DB.
const MAX_TAGS_PER_EVENT: usize = 100;
const MAX_BREADCRUMBS_PER_EVENT: usize = 200;

/// Insert a normalized event with all its rows (event + tags + breadcrumbs) on the given
/// connection. Caller owns the transaction (so the surrounding issue upsert / bump can be atomic
/// with the event insert).
pub async fn insert_full(
    conn: &mut SqliteConnection,
    project_id: i64,
    issue_id: Option<i64>,
    ev: &NormalizedEvent,
    raw_json: &str,
) -> sqlx::Result<i64> {
    let received_at = Utc::now().to_rfc3339();
    let (exc_type, exc_value) = match &ev.exception {
        Some(e) => (e.ty.clone(), e.value.clone()),
        None => (None, None),
    };

    let row = sqlx::query(
        "INSERT INTO events ( \
            event_id, project_id, issue_id, timestamp, received_at, \
            level, platform, environment, release, logger, transaction_name, \
            message, exception_type, exception_value, culprit, server_name, \
            request_url, user_id, user_email, fingerprint, raw_json) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(ev.event_id.as_deref())
    .bind(project_id)
    .bind(issue_id)
    .bind(ev.timestamp.as_deref())
    .bind(&received_at)
    .bind(ev.level.as_deref())
    .bind(ev.platform.as_deref())
    .bind(ev.environment.as_deref())
    .bind(ev.release.as_deref())
    .bind(ev.logger.as_deref())
    .bind(ev.transaction_name.as_deref())
    .bind(ev.message.as_deref())
    .bind(exc_type.as_deref())
    .bind(exc_value.as_deref())
    .bind(ev.culprit.as_deref())
    .bind(ev.server_name.as_deref())
    .bind(ev.request_url.as_deref())
    .bind(ev.user_id.as_deref())
    .bind(ev.user_email.as_deref())
    .bind(ev.custom_fingerprint.as_ref().map(|p| p.join("|")))
    .bind(raw_json)
    .execute(&mut *conn)
    .await?;
    let event_row_id = row.last_insert_rowid();

    insert_tags(&mut *conn, event_row_id, &ev.tags).await?;
    insert_breadcrumbs(&mut *conn, event_row_id, &ev.breadcrumbs).await?;
    Ok(event_row_id)
}

async fn insert_tags(
    conn: &mut SqliteConnection,
    event_row_id: i64,
    tags: &[(String, String)],
) -> sqlx::Result<()> {
    for (k, v) in tags.iter().take(MAX_TAGS_PER_EVENT) {
        sqlx::query("INSERT INTO event_tags (event_id, key, value) VALUES (?, ?, ?)")
            .bind(event_row_id)
            .bind(k)
            .bind(v)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

async fn insert_breadcrumbs(
    conn: &mut SqliteConnection,
    event_row_id: i64,
    crumbs: &[Breadcrumb],
) -> sqlx::Result<()> {
    for b in crumbs.iter().take(MAX_BREADCRUMBS_PER_EVENT) {
        sqlx::query(
            "INSERT INTO event_breadcrumbs \
                (event_id, timestamp, category, level, message, data_json) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(event_row_id)
        .bind(b.timestamp.as_deref())
        .bind(b.category.as_deref())
        .bind(b.level.as_deref())
        .bind(b.message.as_deref())
        .bind(b.data_json.as_deref())
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}
