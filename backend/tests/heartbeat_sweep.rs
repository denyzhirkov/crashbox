#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for the heartbeat sweep job and down/recovery notifications.

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, Utc};
use crashbox::app_state::AppState;
use crashbox::config::Config;
use crashbox::jobs::heartbeat;
use crashbox::notify::NotifyHub;
use crashbox::{db, http as http_mod};
use serde_json::Value;
use sqlx::SqlitePool;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

static ENV_LOCK: Mutex<()> = Mutex::new(());

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
    let path = tmp.path().join("heartbeat.db");
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

/// Insert a monitor directly, bypassing the API, so tests control every field.
async fn seed_monitor(
    pool: &SqlitePool,
    name: &str,
    status: &str,
    last_ping_ago: Option<ChronoDuration>,
    period_seconds: i64,
    grace_seconds: i64,
) -> i64 {
    let now = Utc::now().to_rfc3339();
    let last_ping_at = last_ping_ago.map(|ago| (Utc::now() - ago).to_rfc3339());
    let row = sqlx::query(
        "INSERT INTO heartbeat_monitors \
            (project_id, name, ping_key, period_seconds, grace_seconds, status, \
             last_ping_at, last_transition_at, created_at, updated_at) \
         VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(name)
    .bind(format!("key-{name}"))
    .bind(period_seconds)
    .bind(grace_seconds)
    .bind(status)
    .bind(&last_ping_at)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .unwrap();
    row.last_insert_rowid()
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

async fn monitor_status(pool: &SqlitePool, id: i64) -> String {
    sqlx::query_scalar("SELECT status FROM heartbeat_monitors WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn overdue_monitor_flips_down_once_with_alert() {
    let pool = fresh_pool().await;
    seed_project(&pool).await;
    let (addr, inbox) = spawn_inbox().await;
    let hub = hub_pointing_at(format!("http://{addr}/hook"));

    // Last ping 100s ago, deadline = 60 + 10 → 30s overdue.
    let id = seed_monitor(
        &pool,
        "backup",
        "up",
        Some(ChronoDuration::seconds(100)),
        60,
        10,
    )
    .await;

    let flipped = heartbeat::run_once(&pool, &hub, Utc::now()).await.unwrap();
    assert_eq!(flipped, 1);
    assert_eq!(monitor_status(&pool, id).await, "down");
    drain().await;

    let msgs = inbox.0.lock().unwrap().clone();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["kind"], "heartbeat_down");
    assert_eq!(msgs[0]["monitor_name"], "backup");
    assert_eq!(msgs[0]["project_slug"], "demo");
    let overdue = msgs[0]["overdue_seconds"].as_i64().unwrap();
    assert!((25..=40).contains(&overdue), "got {overdue}");
    assert_eq!(msgs[0]["link"], "http://localhost/projects/1/heartbeats");

    // Second sweep: already down, no candidates, no duplicate alert.
    let flipped2 = heartbeat::run_once(&pool, &hub, Utc::now()).await.unwrap();
    assert_eq!(flipped2, 0, "down-flip must fire exactly once");
    drain().await;
    assert_eq!(inbox.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn healthy_pending_and_paused_monitors_never_flip() {
    let pool = fresh_pool().await;
    seed_project(&pool).await;
    let (addr, inbox) = spawn_inbox().await;
    let hub = hub_pointing_at(format!("http://{addr}/hook"));

    // Healthy: pinged 30s ago with a 60+60 deadline.
    let healthy = seed_monitor(
        &pool,
        "healthy",
        "up",
        Some(ChronoDuration::seconds(30)),
        60,
        60,
    )
    .await;
    // Pending: never pinged — no deadline to be late against.
    let pending = seed_monitor(&pool, "pending", "pending", None, 60, 0).await;
    // Paused: stale last_ping_at must not matter.
    let paused = seed_monitor(
        &pool,
        "paused",
        "paused",
        Some(ChronoDuration::hours(5)),
        60,
        0,
    )
    .await;

    let flipped = heartbeat::run_once(&pool, &hub, Utc::now()).await.unwrap();
    assert_eq!(flipped, 0);
    assert_eq!(monitor_status(&pool, healthy).await, "up");
    assert_eq!(monitor_status(&pool, pending).await, "pending");
    assert_eq!(monitor_status(&pool, paused).await, "paused");
    drain().await;
    assert!(inbox.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sweep_flips_even_without_notifiers() {
    // Unlike spike, the flip is user-visible state — it must happen with an empty hub.
    let pool = fresh_pool().await;
    seed_project(&pool).await;
    let hub = Arc::new(NotifyHub {
        notifiers: vec![],
        limiters: vec![],
        public_url: "http://localhost".to_string(),
    });

    let id = seed_monitor(
        &pool,
        "backup",
        "up",
        Some(ChronoDuration::seconds(100)),
        60,
        10,
    )
    .await;
    let flipped = heartbeat::run_once(&pool, &hub, Utc::now()).await.unwrap();
    assert_eq!(flipped, 1);
    assert_eq!(monitor_status(&pool, id).await, "down");
}

/// Full app for the recovery path: ping endpoint fires `heartbeat_recovered` through the
/// configured webhook when a down monitor comes back.
async fn spawn_crashbox(webhook_url: &str) -> (SocketAddr, SqlitePool) {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("cb.db");

    let cfg = {
        let _g = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::env::set_var(
            "CRASHBOX_DATABASE_URL",
            format!("sqlite://{}", db_path.display()),
        );
        std::env::set_var("CRASHBOX_PORT", "0");
        std::env::set_var("CRASHBOX_PUBLIC_URL", "http://localhost");
        std::env::set_var("CRASHBOX_ADMIN_EMAIL", "a@b.c");
        std::env::set_var("CRASHBOX_ADMIN_PASSWORD", "x");
        std::env::set_var("CRASHBOX_PROJECT_NAME", "hb-test");
        std::env::set_var("CRASHBOX_PROJECT_PUBLIC_KEY", "hbkey");
        std::env::set_var("CRASHBOX_GENERIC_WEBHOOK_URL", webhook_url);
        // Drop these so we don't accidentally hit real Telegram / Discord from CI.
        std::env::remove_var("CRASHBOX_TELEGRAM_BOT_TOKEN");
        std::env::remove_var("CRASHBOX_TELEGRAM_CHAT_ID");
        std::env::remove_var("CRASHBOX_DISCORD_WEBHOOK_URL");
        let cfg = Config::from_env().unwrap();
        std::env::remove_var("CRASHBOX_GENERIC_WEBHOOK_URL");
        cfg
    };

    let pool = db::connect(&cfg.database_url).await.unwrap();
    db::migrate(&pool).await.unwrap();
    crashbox::bootstrap::run(&pool, &cfg).await.unwrap();

    let state = AppState::new(
        cfg,
        pool.clone(),
        crashbox::metrics_layer::MetricsHandle::dummy(),
    );
    let app = http_mod::routes::build(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    Box::leak(Box::new(tmp));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    (addr, pool)
}

#[tokio::test]
async fn ping_from_down_fires_recovery_and_first_ping_does_not() {
    let (hook_addr, inbox) = spawn_inbox().await;
    let (addr, pool) = spawn_crashbox(&format!("http://{hook_addr}/hook")).await;

    // Seed a monitor directly against the app's own DB.
    let down_since = (Utc::now() - ChronoDuration::seconds(300)).to_rfc3339();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO heartbeat_monitors \
            (project_id, name, ping_key, period_seconds, grace_seconds, status, \
             last_ping_at, last_transition_at, created_at, updated_at) \
         VALUES (1, 'backup', 'reco-key', 60, 10, 'down', ?, ?, ?, ?)",
    )
    .bind(&down_since)
    .bind(&down_since)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();

    let c = reqwest::Client::new();
    let resp = c
        .get(format!("http://{addr}/ping/reco-key"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    drain().await;

    let msgs = inbox.0.lock().unwrap().clone();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["kind"], "heartbeat_recovered");
    assert_eq!(msgs[0]["monitor_name"], "backup");
    let downtime = msgs[0]["downtime_seconds"].as_i64().unwrap();
    assert!((295..=310).contains(&downtime), "got {downtime}");

    // A repeat ping on an up monitor stays silent.
    let resp = c
        .get(format!("http://{addr}/ping/reco-key"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    drain().await;
    assert_eq!(inbox.0.lock().unwrap().len(), 1, "no alert on repeat ping");

    // First ping of a pending monitor is also silent (pending → up is not a recovery).
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO heartbeat_monitors \
            (project_id, name, ping_key, period_seconds, grace_seconds, status, \
             last_transition_at, created_at, updated_at) \
         VALUES (1, 'fresh', 'fresh-key', 60, 10, 'pending', ?, ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    let resp = c
        .get(format!("http://{addr}/ping/fresh-key"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    drain().await;
    assert_eq!(
        inbox.0.lock().unwrap().len(),
        1,
        "pending → up must not alert"
    );
}

#[tokio::test]
async fn full_cycle_down_then_recovery() {
    let (hook_addr, inbox) = spawn_inbox().await;
    let (addr, pool) = spawn_crashbox(&format!("http://{hook_addr}/hook")).await;
    let hub = hub_pointing_at(format!("http://{hook_addr}/hook"));

    // An up monitor that is already overdue.
    let last_ping = (Utc::now() - ChronoDuration::seconds(100)).to_rfc3339();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO heartbeat_monitors \
            (project_id, name, ping_key, period_seconds, grace_seconds, status, \
             last_ping_at, last_transition_at, created_at, updated_at) \
         VALUES (1, 'cycle', 'cycle-key', 60, 10, 'up', ?, ?, ?, ?)",
    )
    .bind(&last_ping)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();

    let flipped = heartbeat::run_once(&pool, &hub, Utc::now()).await.unwrap();
    assert_eq!(flipped, 1);
    drain().await;

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/ping/cycle-key"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    drain().await;

    let msgs = inbox.0.lock().unwrap().clone();
    let kinds: Vec<&str> = msgs.iter().filter_map(|m| m["kind"].as_str()).collect();
    assert_eq!(kinds, vec!["heartbeat_down", "heartbeat_recovered"]);
    let status: String =
        sqlx::query_scalar("SELECT status FROM heartbeat_monitors WHERE ping_key = 'cycle-key'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "up");
}
