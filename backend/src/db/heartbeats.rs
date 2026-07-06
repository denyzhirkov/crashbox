//! Heartbeat monitor repository.
//!
//! State machine (the whole thing):
//! - `pending` → `up` on first ping
//! - `up` → `down` by the sweep job once `last_ping_at + period + grace` has passed
//! - `down` → `up` on ping (recovery)
//! - `* → paused` by hand; a ping from `paused` resumes to `up`
//!
//! `pending` never times out — a monitor that was never pinged has nothing to be "late"
//! against. Resume from `paused` goes back to `pending`, not `up`, so a stale
//! `last_ping_at` can't trigger an instant down-alert on the next sweep tick.

use chrono::Utc;
use sqlx::SqlitePool;

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_UP: &str = "up";
pub const STATUS_DOWN: &str = "down";
pub const STATUS_PAUSED: &str = "paused";

const COLUMNS: &str = "id, project_id, name, ping_key, period_seconds, grace_seconds, \
                       status, last_ping_at, last_transition_at, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct HeartbeatMonitor {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub ping_key: String,
    pub period_seconds: i64,
    pub grace_seconds: i64,
    pub status: String,
    pub last_ping_at: Option<String>,
    pub last_transition_at: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Result of a recorded ping. `was_down` drives the recovery notification.
#[derive(Debug)]
pub struct PingOutcome {
    pub monitor: HeartbeatMonitor,
    pub was_down: bool,
    /// Set only when `was_down`: seconds between the down-transition and this ping.
    pub downtime_seconds: Option<i64>,
}

pub async fn list_for_project(
    pool: &SqlitePool,
    project_id: i64,
) -> sqlx::Result<Vec<HeartbeatMonitor>> {
    sqlx::query_as::<_, HeartbeatMonitor>(&format!(
        "SELECT {COLUMNS} FROM heartbeat_monitors WHERE project_id = ? ORDER BY id ASC"
    ))
    .bind(project_id)
    .fetch_all(pool)
    .await
}

pub async fn find_by_id(pool: &SqlitePool, id: i64) -> sqlx::Result<Option<HeartbeatMonitor>> {
    sqlx::query_as::<_, HeartbeatMonitor>(&format!(
        "SELECT {COLUMNS} FROM heartbeat_monitors WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn find_by_ping_key(
    pool: &SqlitePool,
    ping_key: &str,
) -> sqlx::Result<Option<HeartbeatMonitor>> {
    sqlx::query_as::<_, HeartbeatMonitor>(&format!(
        "SELECT {COLUMNS} FROM heartbeat_monitors WHERE ping_key = ?"
    ))
    .bind(ping_key)
    .fetch_optional(pool)
    .await
}

pub async fn insert(
    pool: &SqlitePool,
    project_id: i64,
    name: &str,
    ping_key: &str,
    period_seconds: i64,
    grace_seconds: i64,
) -> sqlx::Result<i64> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO heartbeat_monitors \
            (project_id, name, ping_key, period_seconds, grace_seconds, status, \
             last_transition_at, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(project_id)
    .bind(name)
    .bind(ping_key)
    .bind(period_seconds)
    .bind(grace_seconds)
    .bind(STATUS_PENDING)
    .bind(&now)
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
    period_seconds: Option<i64>,
    grace_seconds: Option<i64>,
) -> sqlx::Result<u64> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE heartbeat_monitors SET \
            name = COALESCE(?, name), \
            period_seconds = COALESCE(?, period_seconds), \
            grace_seconds = COALESCE(?, grace_seconds), \
            updated_at = ? \
         WHERE id = ?",
    )
    .bind(name)
    .bind(period_seconds)
    .bind(grace_seconds)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Set `status` by hand (pause/resume). No-op when the monitor is already in that status, so
/// repeated PATCHes don't churn `last_transition_at`.
pub async fn set_status(pool: &SqlitePool, id: i64, status: &str) -> sqlx::Result<u64> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE heartbeat_monitors \
         SET status = ?, last_transition_at = ?, updated_at = ? \
         WHERE id = ? AND status != ?",
    )
    .bind(status)
    .bind(&now)
    .bind(&now)
    .bind(id)
    .bind(status)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub async fn delete(pool: &SqlitePool, id: i64) -> sqlx::Result<u64> {
    let result = sqlx::query("DELETE FROM heartbeat_monitors WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Record a ping: bump `last_ping_at`, flip any non-`up` status to `up`. Runs in a write
/// transaction with a re-read so the transition decision can't race the sweep job.
/// Returns `None` if the monitor was deleted between lookup and ping.
pub async fn record_ping(pool: &SqlitePool, id: i64) -> sqlx::Result<Option<PingOutcome>> {
    let mut tx = crate::db::begin_write(pool).await?;
    let monitor = sqlx::query_as::<_, HeartbeatMonitor>(&format!(
        "SELECT {COLUMNS} FROM heartbeat_monitors WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(tx.acquire())
    .await?;
    let Some(m) = monitor else {
        return Ok(None);
    };

    let now_ts = Utc::now();
    let now = now_ts.to_rfc3339();
    let was_down = m.status == STATUS_DOWN;
    let goes_up = m.status != STATUS_UP;
    let downtime_seconds = if was_down {
        chrono::DateTime::parse_from_rfc3339(&m.last_transition_at)
            .ok()
            .map(|t| (now_ts - t.with_timezone(&Utc)).num_seconds().max(0))
    } else {
        None
    };
    if goes_up {
        sqlx::query(
            "UPDATE heartbeat_monitors \
             SET last_ping_at = ?, status = ?, last_transition_at = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&now)
        .bind(STATUS_UP)
        .bind(&now)
        .bind(&now)
        .bind(m.id)
        .execute(tx.acquire())
        .await?;
    } else {
        sqlx::query("UPDATE heartbeat_monitors SET last_ping_at = ?, updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(&now)
            .bind(m.id)
            .execute(tx.acquire())
            .await?;
    }
    tx.commit().await?;

    let monitor = HeartbeatMonitor {
        status: STATUS_UP.to_string(),
        last_ping_at: Some(now.clone()),
        last_transition_at: if goes_up {
            now.clone()
        } else {
            m.last_transition_at.clone()
        },
        updated_at: now,
        ..m
    };
    Ok(Some(PingOutcome {
        monitor,
        was_down,
        downtime_seconds,
    }))
}
