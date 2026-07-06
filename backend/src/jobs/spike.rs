//! Spike detection.
//!
//! Periodically scans the events table and fires a `Spike` notification for any issue whose
//! last-hour event rate is N× higher than its 23h baseline (defaults: 10 events/h minimum,
//! 5× ratio, 1h cooldown per issue).
//!
//! Tradeoffs / notes:
//! - We only consider *known* issues — orphan events (no issue_id) are ignored. A spike on an
//!   orphan would be a new fingerprint anyway, which fires `NewIssue` via the ingest path.
//! - Baseline = sum of events in the window `[-24h, -1h)`, divided by 23. If baseline is 0
//!   the issue is brand-new or has been silent for a day; we skip — those cases are covered by
//!   `NewIssue` and `Reopened` triggers.
//! - The cooldown is enforced via `issues.spike_alerted_at`; we UPDATE inside the same task
//!   so a missed tick can't double-alert.
//! - Hard-coded `min_events_per_hour` floor (10 by default) prevents noisy alerts when both
//!   numbers are tiny: e.g. baseline 0.04/h × 5× = 0.2/h, but a single retry burst would trip
//!   that — not useful.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sqlx::SqlitePool;
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use crate::config::SpikeConfig;
use crate::notify::{IssueNotification, Kind, Notification, NotifyHub};

pub fn spawn(
    pool: SqlitePool,
    spike_cfg: Arc<SpikeConfig>,
    notify_hub: Arc<NotifyHub>,
    cancel: CancellationToken,
) {
    if spike_cfg.check_interval_seconds == 0 {
        tracing::info!("spike: check_interval_seconds=0, job disabled");
        return;
    }
    if notify_hub.is_empty() {
        tracing::info!("spike: no notifiers configured, job disabled");
        return;
    }
    tokio::spawn(async move {
        run_loop(pool, spike_cfg, notify_hub, cancel).await;
    });
}

async fn run_loop(
    pool: SqlitePool,
    cfg: Arc<SpikeConfig>,
    notify_hub: Arc<NotifyHub>,
    cancel: CancellationToken,
) {
    let mut tick = interval(Duration::from_secs(cfg.check_interval_seconds));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if let Err(e) = run_once(&pool, cfg.as_ref(), &notify_hub).await {
                    tracing::error!(error = %e, "spike sweep failed");
                }
            }
            () = cancel.cancelled() => {
                tracing::info!("spike: shutdown signal received, exiting");
                return;
            }
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct SpikeCandidate {
    issue_id: i64,
    project_name: String,
    project_slug: String,
    issue_title: String,
    level: Option<String>,
    current_count: i64,
    baseline_count: i64,
}

/// Run a single spike check. Returns the number of alerts fired.
pub async fn run_once(
    pool: &SqlitePool,
    cfg: &SpikeConfig,
    notify_hub: &Arc<NotifyHub>,
) -> sqlx::Result<u64> {
    let now = Utc::now();
    let cutoff_1h = (now - chrono::Duration::hours(1)).to_rfc3339();
    let cutoff_24h = (now - chrono::Duration::hours(24)).to_rfc3339();
    let cooldown_cutoff =
        (now - chrono::Duration::seconds(cfg.cooldown_seconds as i64)).to_rfc3339();
    let min_events = cfg.min_events_per_hour as i64;
    // Multiply baseline by (23 * threshold) on the right side instead of dividing on the left,
    // so the comparison stays in integer arithmetic until the very end. Cleaner SQL.
    let ratio_x_23 = cfg.ratio_threshold * 23.0;

    let rows = sqlx::query_as::<_, SpikeCandidate>(
        "WITH last_hour AS ( \
            SELECT issue_id, COUNT(*) AS cnt \
            FROM events \
            WHERE issue_id IS NOT NULL AND received_at >= ?1 \
            GROUP BY issue_id \
         ), baseline AS ( \
            SELECT issue_id, COUNT(*) AS cnt \
            FROM events \
            WHERE issue_id IS NOT NULL AND received_at >= ?2 AND received_at < ?1 \
            GROUP BY issue_id \
         ) \
         SELECT \
            i.id AS issue_id, \
            p.name AS project_name, \
            p.slug AS project_slug, \
            i.title AS issue_title, \
            i.level, \
            last_hour.cnt AS current_count, \
            baseline.cnt AS baseline_count \
         FROM issues i \
         JOIN projects p ON p.id = i.project_id \
         JOIN last_hour ON last_hour.issue_id = i.id \
         JOIN baseline ON baseline.issue_id = i.id \
         WHERE last_hour.cnt >= ?3 \
           AND baseline.cnt > 0 \
           AND CAST(last_hour.cnt AS REAL) * 23.0 >= ?4 * CAST(baseline.cnt AS REAL) \
           AND (i.spike_alerted_at IS NULL OR i.spike_alerted_at < ?5)",
    )
    .bind(&cutoff_1h)
    .bind(&cutoff_24h)
    .bind(min_events)
    .bind(ratio_x_23)
    .bind(&cooldown_cutoff)
    .fetch_all(pool)
    .await?;

    let mut fired = 0u64;
    let now_iso = now.to_rfc3339();
    for row in rows {
        // Mark cooldown BEFORE notifying so a slow notifier can't cause double-firing on
        // overlapping ticks.
        let res = sqlx::query("UPDATE issues SET spike_alerted_at = ? WHERE id = ?")
            .bind(&now_iso)
            .bind(row.issue_id)
            .execute(pool)
            .await?;
        if res.rows_affected() == 0 {
            continue;
        }

        let baseline_per_hour = (row.baseline_count as f64) / 23.0;
        let link = notify_hub.build_link(row.issue_id);
        notify_hub.fire(Notification::Issue(IssueNotification {
            kind: Kind::Spike,
            project_name: row.project_name,
            project_slug: row.project_slug,
            issue_id: row.issue_id,
            issue_title: row.issue_title,
            event_count: row.current_count,
            level: row.level,
            environment: None,
            release: None,
            link,
            current_hour: Some(row.current_count),
            baseline_per_hour: Some(baseline_per_hour),
        }));
        fired += 1;
    }
    if fired > 0 {
        tracing::info!(fired, "spike: alerts dispatched");
    } else {
        tracing::debug!("spike: no candidates");
    }
    Ok(fired)
}
