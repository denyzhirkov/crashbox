//! Heartbeat sweep.
//!
//! Periodically flips `up` monitors whose ping deadline (`last_ping_at + period + grace`) has
//! passed to `down`, and fires a `heartbeat_down` notification once per transition. Recovery
//! (`down` → `up` + `heartbeat_recovered`) happens in the ping endpoint, not here.
//!
//! Tradeoffs / notes:
//! - Unlike the spike job, the sweep runs even with no notifiers configured — the status flip
//!   itself is user-visible state in the UI, not just an alert trigger.
//! - The deadline is computed in Rust from the fetched row, not in SQL: SQLite date-function
//!   arithmetic on RFC 3339 strings is version-sensitive, and we need the fetched
//!   `last_ping_at` anyway for the race guard below.
//! - The flip UPDATE re-checks `status = 'up' AND last_ping_at = <the value we judged>`.
//!   If a ping lands between SELECT and UPDATE it changes `last_ping_at` (and possibly
//!   nothing else), so `rows_affected = 0` and we skip — no false down, no double alert on
//!   overlapping ticks.
//! - `pending` monitors are never swept: a monitor that has not pinged yet has no deadline
//!   to be late against.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use crate::config::HeartbeatConfig;
use crate::db::heartbeats::{STATUS_DOWN, STATUS_UP};
use crate::notify::{HeartbeatKind, HeartbeatNotification, Notification, NotifyHub};

pub fn spawn(
    pool: SqlitePool,
    cfg: Arc<HeartbeatConfig>,
    notify_hub: Arc<NotifyHub>,
    cancel: CancellationToken,
) {
    if cfg.sweep_interval_seconds == 0 {
        tracing::info!("heartbeat: sweep_interval_seconds=0, sweep disabled");
        return;
    }
    tokio::spawn(async move {
        run_loop(pool, cfg, notify_hub, cancel).await;
    });
}

async fn run_loop(
    pool: SqlitePool,
    cfg: Arc<HeartbeatConfig>,
    notify_hub: Arc<NotifyHub>,
    cancel: CancellationToken,
) {
    let mut tick = interval(Duration::from_secs(cfg.sweep_interval_seconds));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if let Err(e) = run_once(&pool, &notify_hub, Utc::now()).await {
                    tracing::error!(error = %e, "heartbeat sweep failed");
                }
            }
            () = cancel.cancelled() => {
                tracing::info!("heartbeat: shutdown signal received, exiting");
                return;
            }
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct UpMonitor {
    id: i64,
    project_id: i64,
    name: String,
    period_seconds: i64,
    grace_seconds: i64,
    last_ping_at: String,
    project_name: String,
    project_slug: String,
}

/// Run a single sweep at the given instant. Takes `now` as a parameter so tests can fabricate
/// overdue monitors without sleeping. Returns the number of monitors flipped to `down`.
pub async fn run_once(
    pool: &SqlitePool,
    notify_hub: &Arc<NotifyHub>,
    now: DateTime<Utc>,
) -> sqlx::Result<u64> {
    let candidates = sqlx::query_as::<_, UpMonitor>(
        "SELECT m.id, m.project_id, m.name, m.period_seconds, m.grace_seconds, \
                m.last_ping_at, p.name AS project_name, p.slug AS project_slug \
         FROM heartbeat_monitors m \
         JOIN projects p ON p.id = m.project_id \
         WHERE m.status = ?1 AND m.last_ping_at IS NOT NULL",
    )
    .bind(STATUS_UP)
    .fetch_all(pool)
    .await?;

    let mut flipped = 0u64;
    let now_iso = now.to_rfc3339();
    for m in candidates {
        let Ok(last_ping) = DateTime::parse_from_rfc3339(&m.last_ping_at) else {
            tracing::warn!(
                monitor_id = m.id,
                last_ping_at = %m.last_ping_at,
                "heartbeat: unparseable last_ping_at, skipping"
            );
            continue;
        };
        let deadline = last_ping.with_timezone(&Utc)
            + chrono::Duration::seconds(m.period_seconds + m.grace_seconds);
        if now <= deadline {
            continue;
        }

        let res = sqlx::query(
            "UPDATE heartbeat_monitors \
             SET status = ?, last_transition_at = ?, updated_at = ? \
             WHERE id = ? AND status = ? AND last_ping_at = ?",
        )
        .bind(STATUS_DOWN)
        .bind(&now_iso)
        .bind(&now_iso)
        .bind(m.id)
        .bind(STATUS_UP)
        .bind(&m.last_ping_at)
        .execute(pool)
        .await?;
        if res.rows_affected() == 0 {
            // A fresh ping (or a manual pause) won the race — the monitor is not late anymore.
            continue;
        }

        metrics::counter!("crashbox_heartbeat_transitions_total", "to" => "down").increment(1);
        let overdue = (now - deadline).num_seconds();
        tracing::info!(
            monitor_id = m.id,
            project_id = m.project_id,
            overdue_seconds = overdue,
            "heartbeat: monitor down"
        );
        notify_hub.fire(Notification::Heartbeat(HeartbeatNotification {
            kind: HeartbeatKind::HeartbeatDown,
            project_name: m.project_name,
            project_slug: m.project_slug,
            monitor_id: m.id,
            monitor_name: m.name,
            overdue_seconds: Some(overdue),
            downtime_seconds: None,
            link: notify_hub.build_heartbeat_link(m.project_id),
        }));
        flipped += 1;
    }
    if flipped > 0 {
        tracing::info!(flipped, "heartbeat: sweep flipped monitors down");
    } else {
        tracing::debug!("heartbeat: sweep found no overdue monitors");
    }
    Ok(flipped)
}
