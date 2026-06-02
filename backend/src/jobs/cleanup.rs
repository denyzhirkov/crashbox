//! Retention job: deletes events older than `CRASHBOX_RETENTION_DAYS`, while keeping the most
//! recent `CRASHBOX_MAX_EVENTS_PER_ISSUE` events per issue regardless of age.
//!
//! Issue summary rows live forever. Tags and breadcrumbs cascade via `FOREIGN KEY ... ON DELETE
//! CASCADE` defined in the schema, so we only need to touch the `events` table.
//!
//! Run as a Tokio task on a fixed interval; cancellation listens to the same shutdown signal as
//! the HTTP server.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sqlx::SqlitePool;
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use crate::config::Retention;

/// Spawn the retention task. Returns immediately; the task runs until `cancel` is triggered.
pub fn spawn(pool: SqlitePool, retention: Arc<Retention>, cancel: CancellationToken) {
    tokio::spawn(async move {
        run_loop(pool, retention, cancel).await;
    });
}

async fn run_loop(pool: SqlitePool, retention: Arc<Retention>, cancel: CancellationToken) {
    if retention.cleanup_interval_seconds == 0 {
        tracing::info!("retention: cleanup_interval_seconds=0, job disabled");
        return;
    }

    let mut tick = interval(Duration::from_secs(retention.cleanup_interval_seconds));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // First tick fires immediately; do an initial sweep on boot.
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if let Err(e) = run_once(&pool, retention.as_ref()).await {
                    tracing::error!(error = %e, "retention sweep failed");
                }
            }
            () = cancel.cancelled() => {
                tracing::info!("retention: shutdown signal received, exiting");
                return;
            }
        }
    }
}

/// Run a single retention + auto-resolve sweep. Returns the number of event rows deleted.
///
/// Order: auto-resolve runs FIRST, so when retention then deletes events of a freshly
/// auto-resolved issue we don't waste work re-checking the same issue.
pub async fn run_once(pool: &SqlitePool, retention: &Retention) -> sqlx::Result<u64> {
    auto_resolve_stale_issues(pool, retention).await?;

    let cutoff = Utc::now() - chrono::Duration::days(retention.retention_days as i64);
    let cutoff_iso = cutoff.to_rfc3339();
    let max_per_issue = retention.max_events_per_issue as i64;

    // Window-function based delete:
    //   1. For each issue, rank events newest-first.
    //   2. Delete those whose rank exceeds max_per_issue AND received before cutoff.
    //
    // Events without an issue (issue_id NULL) are deleted purely by age — there is no per-issue
    // bucket to count against.
    let res = sqlx::query(
        "DELETE FROM events WHERE id IN ( \
            SELECT id FROM ( \
                SELECT id, received_at, issue_id, \
                       ROW_NUMBER() OVER (PARTITION BY issue_id ORDER BY received_at DESC) AS rn \
                FROM events \
            ) ranked \
            WHERE ranked.received_at < ? \
              AND (ranked.issue_id IS NULL OR ranked.rn > ?) \
        )",
    )
    .bind(&cutoff_iso)
    .bind(max_per_issue)
    .execute(pool)
    .await?;

    let deleted = res.rows_affected();
    if deleted > 0 {
        metrics::counter!("crashbox_retention_events_deleted_total").increment(deleted);
        tracing::info!(
            deleted,
            cutoff = %cutoff_iso,
            max_per_issue,
            "retention: events deleted"
        );
    } else {
        tracing::debug!(cutoff = %cutoff_iso, "retention: no events to delete");
    }
    Ok(deleted)
}

/// Flip status to `resolved` for any `unresolved` issue whose `last_seen` is older than
/// `auto_resolve_days`. Auto-reopen is handled implicitly by [`crate::db::issues::upsert`]:
/// if the next event for an auto-resolved fingerprint arrives, the upsert flips status back to
/// `unresolved` and the notify hub emits a `Reopened` alert.
async fn auto_resolve_stale_issues(pool: &SqlitePool, retention: &Retention) -> sqlx::Result<u64> {
    if retention.auto_resolve_days == 0 {
        return Ok(0);
    }
    let cutoff = Utc::now() - chrono::Duration::days(retention.auto_resolve_days as i64);
    let cutoff_iso = cutoff.to_rfc3339();
    let now_iso = Utc::now().to_rfc3339();
    let res = sqlx::query(
        "UPDATE issues SET status = 'resolved', updated_at = ? \
         WHERE status = 'unresolved' AND last_seen < ?",
    )
    .bind(&now_iso)
    .bind(&cutoff_iso)
    .execute(pool)
    .await?;
    let n = res.rows_affected();
    if n > 0 {
        tracing::info!(auto_resolved = n, cutoff = %cutoff_iso, "auto-resolved stale issues");
    }
    Ok(n)
}
