#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for A2 spike detection.

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, Utc};
use crashbox::config::SpikeConfig;
use crashbox::jobs::spike;
use crashbox::notify::NotifyHub;
use serde_json::Value;
use sqlx::SqlitePool;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
struct Inbox(Arc<Mutex<Vec<Value>>>);

async fn spawn_inbox() -> (SocketAddr, Inbox) {
    let inbox = Inbox::default();
    let app = Router::new()
        .route(
            "/hook",
            post(
                |State(state): State<Inbox>, Json(body): Json<Value>| async move {
                    state.0.lock().unwrap().push(body);
                    (axum::http::StatusCode::OK, "ok")
                },
            ),
        )
        .with_state(inbox.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    (addr, inbox)
}

async fn fresh_pool() -> SqlitePool {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("spike.db");
    Box::leak(Box::new(tmp));
    let url = format!("sqlite://{}", path.display());
    let pool = crashbox::db::connect(&url).await.unwrap();
    crashbox::db::migrate(&pool).await.unwrap();
    pool
}

async fn seed_project_and_issue(pool: &SqlitePool) -> i64 {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO projects (id, name, slug, public_key, created_at, updated_at) \
         VALUES (1, 'demo', 'demo', 'pk', ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .unwrap();
    let row = sqlx::query(
        "INSERT INTO issues \
            (project_id, fingerprint, title, status, level, platform, \
             first_seen, last_seen, event_count, created_at, updated_at) \
         VALUES (1, 'fp1', 'BurstError: x', 'unresolved', 'error', 'node', ?, ?, 0, ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .unwrap();
    row.last_insert_rowid()
}

async fn insert_events_at_offset(
    pool: &SqlitePool,
    issue_id: i64,
    count: i64,
    ago: ChronoDuration,
) {
    for _ in 0..count {
        let ts = (Utc::now() - ago).to_rfc3339();
        sqlx::query(
            "INSERT INTO events (project_id, issue_id, received_at, raw_json) \
             VALUES (1, ?, ?, '{}')",
        )
        .bind(issue_id)
        .bind(&ts)
        .execute(pool)
        .await
        .unwrap();
    }
}

fn spike_cfg() -> SpikeConfig {
    SpikeConfig {
        check_interval_seconds: 0, // irrelevant — we call run_once directly
        min_events_per_hour: 10,
        ratio_threshold: 5.0,
        cooldown_seconds: 3600,
    }
}

fn hub_pointing_at(webhook_url: String) -> Arc<NotifyHub> {
    use crashbox::notify::webhook::GenericWebhook;
    let notifiers: Vec<Arc<dyn crashbox::notify::Notifier>> =
        vec![Arc::new(GenericWebhook::new(webhook_url))];
    let limiters = vec![Arc::new(tokio::sync::Mutex::new(
        crashbox::notify::TokenBucket::new(60),
    ))];
    Arc::new(NotifyHub {
        notifiers,
        limiters,
        public_url: "http://localhost".to_string(),
    })
}

async fn drain() {
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test]
async fn detects_a_real_spike_and_respects_cooldown() {
    let pool = fresh_pool().await;
    let issue_id = seed_project_and_issue(&pool).await;
    let (addr, inbox) = spawn_inbox().await;
    let hub = hub_pointing_at(format!("http://{addr}/hook"));
    let cfg = spike_cfg();

    // Baseline: 5 events spread across the prior 23h (~0.22/h)
    for h in 2..=23 {
        if h <= 6 {
            insert_events_at_offset(&pool, issue_id, 1, ChronoDuration::hours(h)).await;
        }
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        5
    );

    // Spike: 30 events in the last 10 minutes
    insert_events_at_offset(&pool, issue_id, 30, ChronoDuration::minutes(10)).await;

    let fired = spike::run_once(&pool, &cfg, &hub).await.unwrap();
    assert_eq!(fired, 1);
    drain().await;

    let msgs = inbox.0.lock().unwrap().clone();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["kind"], "spike");
    assert_eq!(msgs[0]["current_hour"], 30);
    assert!(msgs[0]["baseline_per_hour"].as_f64().unwrap() > 0.2);
    assert_eq!(msgs[0]["issue_title"], "BurstError: x");

    // Immediate second run: cooldown should prevent a duplicate alert.
    let fired2 = spike::run_once(&pool, &cfg, &hub).await.unwrap();
    assert_eq!(fired2, 0, "cooldown must suppress second alert");
    drain().await;
    let msgs2 = inbox.0.lock().unwrap().clone();
    assert_eq!(msgs2.len(), 1, "no duplicate alert");
}

#[tokio::test]
async fn no_alert_when_below_min_events() {
    let pool = fresh_pool().await;
    let issue_id = seed_project_and_issue(&pool).await;
    let (addr, inbox) = spawn_inbox().await;
    let hub = hub_pointing_at(format!("http://{addr}/hook"));
    let cfg = spike_cfg();

    // Baseline of 1 event, current 9 events (below the 10/h floor)
    insert_events_at_offset(&pool, issue_id, 1, ChronoDuration::hours(12)).await;
    insert_events_at_offset(&pool, issue_id, 9, ChronoDuration::minutes(5)).await;

    let fired = spike::run_once(&pool, &cfg, &hub).await.unwrap();
    assert_eq!(fired, 0);
    drain().await;
    assert!(inbox.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn no_alert_when_baseline_is_zero() {
    // A brand-new issue with current burst has no baseline — that's a NewIssue case,
    // not a spike. Spike SQL must skip baseline=0.
    let pool = fresh_pool().await;
    let issue_id = seed_project_and_issue(&pool).await;
    let (addr, inbox) = spawn_inbox().await;
    let hub = hub_pointing_at(format!("http://{addr}/hook"));
    let cfg = spike_cfg();

    insert_events_at_offset(&pool, issue_id, 50, ChronoDuration::minutes(2)).await;

    let fired = spike::run_once(&pool, &cfg, &hub).await.unwrap();
    assert_eq!(fired, 0);
    drain().await;
    assert!(inbox.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn no_alert_when_ratio_below_threshold() {
    let pool = fresh_pool().await;
    let issue_id = seed_project_and_issue(&pool).await;
    let (addr, inbox) = spawn_inbox().await;
    let hub = hub_pointing_at(format!("http://{addr}/hook"));
    let cfg = spike_cfg();

    // High baseline (200 in baseline window = ~8.7/h), current only 15/h — ratio < 2× < 5×.
    for h in 2..=23 {
        insert_events_at_offset(&pool, issue_id, 9, ChronoDuration::hours(h)).await;
    }
    insert_events_at_offset(&pool, issue_id, 15, ChronoDuration::minutes(5)).await;

    let fired = spike::run_once(&pool, &cfg, &hub).await.unwrap();
    assert_eq!(fired, 0, "ratio under threshold must not alert");
    drain().await;
    assert!(inbox.0.lock().unwrap().is_empty());
}
