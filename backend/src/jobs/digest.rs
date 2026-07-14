//! Periodic per-project digest: new issues + events since the last digest, through the
//! existing notification channels.
//!
//! The window anchor (`app_meta` key `digest_last_at`) persists across restarts, so a
//! container bounce neither double-sends nor resets the window. The job ticks every minute
//! and fires only once the configured interval has elapsed past the anchor; after downtime
//! the digest simply covers the longer window (reported as `window_hours`). Projects with
//! nothing to report are skipped — an empty digest is noise.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use crate::config::DigestConfig;
use crate::db::meta;
use crate::notify::{DigestKind, DigestNotification, DigestTopIssue, Notification, NotifyHub};

const ANCHOR_KEY: &str = "digest_last_at";
const TICK_SECONDS: u64 = 60;
const TOP_ISSUES: i64 = 3;

pub fn spawn(
    pool: SqlitePool,
    cfg: Arc<DigestConfig>,
    notify_hub: Arc<NotifyHub>,
    cancel: CancellationToken,
) {
    if !cfg.enabled {
        tracing::info!("digest: disabled");
        return;
    }
    if notify_hub.is_empty() {
        tracing::info!("digest: no notifiers configured, job disabled");
        return;
    }
    tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(TICK_SECONDS));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if let Err(e) = run_once(&pool, cfg.as_ref(), &notify_hub, Utc::now()).await {
                        tracing::error!(error = %e, "digest run failed");
                    }
                }
                () = cancel.cancelled() => {
                    tracing::info!("digest: shutdown signal received, exiting");
                    return;
                }
            }
        }
    });
}

#[derive(Debug, sqlx::FromRow)]
struct ProjectActivity {
    project_id: i64,
    project_name: String,
    project_slug: String,
    new_issues: i64,
    events: i64,
}

/// One digest check at `now` (injected for tests). Returns the number of digests fired;
/// `None`-equivalent runs (anchor initialized, interval not yet elapsed) return 0.
pub async fn run_once(
    pool: &SqlitePool,
    cfg: &DigestConfig,
    notify_hub: &Arc<NotifyHub>,
    now: DateTime<Utc>,
) -> anyhow::Result<u64> {
    let Some(anchor_raw) = meta::get(pool, ANCHOR_KEY).await? else {
        // First boot with the digest enabled: start the window now instead of digesting all
        // of history.
        meta::set(pool, ANCHOR_KEY, &now.to_rfc3339()).await?;
        return Ok(0);
    };
    let anchor = DateTime::parse_from_rfc3339(&anchor_raw)
        .map_err(|e| anyhow::anyhow!("corrupt {ANCHOR_KEY} value {anchor_raw:?}: {e}"))?
        .with_timezone(&Utc);
    let elapsed = now - anchor;
    if elapsed < chrono::Duration::hours(cfg.interval_hours as i64) {
        return Ok(0);
    }

    // Advance the anchor BEFORE sending (mirrors the spike job's cooldown) so a slow
    // notifier can't cause a double-send on overlapping ticks.
    meta::set(pool, ANCHOR_KEY, &now.to_rfc3339()).await?;

    let since = anchor.to_rfc3339();
    let projects = sqlx::query_as::<_, ProjectActivity>(
        "SELECT p.id AS project_id, p.name AS project_name, p.slug AS project_slug, \
            (SELECT COUNT(*) FROM issues i \
              WHERE i.project_id = p.id AND i.created_at >= ?1) AS new_issues, \
            (SELECT COUNT(*) FROM events e \
              WHERE e.project_id = p.id AND e.received_at >= ?1) AS events \
         FROM projects p ORDER BY p.id",
    )
    .bind(&since)
    .fetch_all(pool)
    .await?;

    let window_hours = elapsed.num_hours().max(1);
    let mut fired = 0u64;
    for p in projects {
        if p.new_issues == 0 && p.events == 0 {
            continue;
        }
        let top_issues = sqlx::query_as::<_, (i64, String, i64)>(
            "SELECT i.id, i.title, COUNT(*) AS cnt \
             FROM events e JOIN issues i ON i.id = e.issue_id \
             WHERE e.project_id = ? AND e.received_at >= ? \
             GROUP BY i.id, i.title ORDER BY cnt DESC LIMIT ?",
        )
        .bind(p.project_id)
        .bind(&since)
        .bind(TOP_ISSUES)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(issue_id, title, events)| DigestTopIssue {
            issue_id,
            title,
            events,
        })
        .collect();

        let link = notify_hub.build_project_link(p.project_id);
        notify_hub.fire(Notification::Digest(DigestNotification {
            kind: DigestKind::Digest,
            project_name: p.project_name,
            project_slug: p.project_slug,
            window_hours,
            new_issues: p.new_issues,
            events: p.events,
            top_issues,
            link,
        }));
        fired += 1;
    }
    if fired > 0 {
        tracing::info!(fired, window_hours, "digest: summaries dispatched");
    } else {
        tracing::debug!("digest: nothing to report");
    }
    Ok(fired)
}
