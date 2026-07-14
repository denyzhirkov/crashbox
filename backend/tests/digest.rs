#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for the periodic digest job.

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, Utc};
use crashbox::config::DigestConfig;
use crashbox::jobs::digest;
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
    let path = tmp.path().join("digest.db");
    Box::leak(Box::new(tmp));
    let url = format!("sqlite://{}", path.display());
    let pool = crashbox::db::connect(&url).await.unwrap();
    crashbox::db::migrate(&pool).await.unwrap();
    pool
}

async fn seed_project(pool: &SqlitePool) {
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
}

async fn seed_issue_with_events(pool: &SqlitePool, fingerprint: &str, title: &str, events: i64) {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO issues \
            (project_id, fingerprint, title, status, level, platform, \
             first_seen, last_seen, event_count, created_at, updated_at) \
         VALUES (1, ?, ?, 'unresolved', 'error', 'node', ?, ?, ?, ?, ?)",
    )
    .bind(fingerprint)
    .bind(title)
    .bind(&now)
    .bind(&now)
    .bind(events)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .unwrap();
    let issue_id = row.last_insert_rowid();
    for _ in 0..events {
        sqlx::query(
            "INSERT INTO events (project_id, issue_id, received_at, raw_json) \
             VALUES (1, ?, ?, '{}')",
        )
        .bind(issue_id)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
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

fn digest_cfg() -> DigestConfig {
    DigestConfig {
        enabled: true,
        interval_hours: 24,
    }
}

async fn drain() {
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test]
async fn first_run_initializes_anchor_without_sending() {
    let pool = fresh_pool().await;
    seed_project(&pool).await;
    seed_issue_with_events(&pool, "fp1", "Boom", 5).await;
    let (addr, inbox) = spawn_inbox().await;
    let hub = hub_pointing_at(format!("http://{addr}/hook"));

    let fired = digest::run_once(&pool, &digest_cfg(), &hub, Utc::now())
        .await
        .unwrap();
    assert_eq!(fired, 0, "first run only sets the anchor");

    let anchor: Option<String> = crashbox::db::meta::get(&pool, "digest_last_at")
        .await
        .unwrap();
    assert!(anchor.is_some());

    // Interval hasn't elapsed → still nothing, even with data present.
    let fired = digest::run_once(&pool, &digest_cfg(), &hub, Utc::now())
        .await
        .unwrap();
    assert_eq!(fired, 0);
    drain().await;
    assert!(inbox.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn fires_after_interval_with_correct_counts_and_advances_anchor() {
    let pool = fresh_pool().await;
    seed_project(&pool).await;
    let (addr, inbox) = spawn_inbox().await;
    let hub = hub_pointing_at(format!("http://{addr}/hook"));
    let cfg = digest_cfg();

    // Anchor 25h in the past, then activity inside the window.
    let anchor = (Utc::now() - ChronoDuration::hours(25)).to_rfc3339();
    crashbox::db::meta::set(&pool, "digest_last_at", &anchor)
        .await
        .unwrap();
    seed_issue_with_events(&pool, "fp1", "TypeError: boom", 7).await;
    seed_issue_with_events(&pool, "fp2", "RangeError: oops", 2).await;

    let now = Utc::now();
    let fired = digest::run_once(&pool, &cfg, &hub, now).await.unwrap();
    assert_eq!(fired, 1);
    drain().await;

    {
        let inbox = inbox.0.lock().unwrap();
        assert_eq!(inbox.len(), 1);
        let msg = &inbox[0];
        assert_eq!(msg["kind"], "digest");
        assert_eq!(msg["project_slug"], "demo");
        assert_eq!(msg["new_issues"], 2);
        assert_eq!(msg["events"], 9);
        assert_eq!(msg["window_hours"], 25);
        let top = msg["top_issues"].as_array().unwrap();
        assert_eq!(top[0]["title"], "TypeError: boom");
        assert_eq!(top[0]["events"], 7);
        assert!(msg["link"]
            .as_str()
            .unwrap()
            .ends_with("/projects/1/issues"));
    }

    // Anchor advanced to `now` — the next check within the interval is silent.
    let stored = crashbox::db::meta::get(&pool, "digest_last_at")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored, now.to_rfc3339());
}

#[tokio::test]
async fn quiet_window_advances_anchor_but_sends_nothing() {
    let pool = fresh_pool().await;
    seed_project(&pool).await;
    let (addr, inbox) = spawn_inbox().await;
    let hub = hub_pointing_at(format!("http://{addr}/hook"));

    let anchor = (Utc::now() - ChronoDuration::hours(30)).to_rfc3339();
    crashbox::db::meta::set(&pool, "digest_last_at", &anchor)
        .await
        .unwrap();

    let now = Utc::now();
    let fired = digest::run_once(&pool, &digest_cfg(), &hub, now)
        .await
        .unwrap();
    assert_eq!(fired, 0, "no activity → no digest");
    drain().await;
    assert!(inbox.0.lock().unwrap().is_empty());

    let stored = crashbox::db::meta::get(&pool, "digest_last_at")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored,
        now.to_rfc3339(),
        "anchor still advances so the next window isn't huge"
    );
}
