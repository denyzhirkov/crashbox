//! Integration test for A1 webhooks: spin up Crashbox + a tiny axum receiver, point
//! `CRASHBOX_GENERIC_WEBHOOK_URL` at the receiver, push events through the ingest endpoint, and
//! assert which notifications the receiver got.
//!
//! Covers:
//! - new issue → 1 notification (NewIssue)
//! - second event of same unresolved issue → no extra notification
//! - resolve via API then send another event → reopen notification (Reopened)
//! - rate limit: bursts beyond `CRASHBOX_NOTIFY_MAX_PER_MINUTE` are dropped (logged), not queued

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use crashbox::app_state::AppState;
use crashbox::config::Config;
use crashbox::{db, http as http_mod};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Default)]
struct ReceivedNotifications(Arc<Mutex<Vec<Value>>>);

async fn spawn_webhook_receiver() -> (SocketAddr, ReceivedNotifications) {
    let inbox = ReceivedNotifications::default();
    let app = Router::new()
        .route(
            "/hook",
            post(
                |State(state): State<ReceivedNotifications>, Json(body): Json<Value>| async move {
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

async fn spawn_crashbox(
    webhook_url: &str,
    max_per_minute: &str,
) -> (SocketAddr, sqlx::SqlitePool) {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("cb.db");

    let cfg = {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var(
            "CRASHBOX_DATABASE_URL",
            format!("sqlite://{}", db_path.display()),
        );
        std::env::set_var("CRASHBOX_PORT", "0");
        std::env::set_var("CRASHBOX_PUBLIC_URL", "http://localhost");
        std::env::set_var("CRASHBOX_ADMIN_EMAIL", "a@b.c");
        std::env::set_var("CRASHBOX_ADMIN_PASSWORD", "x");
        std::env::set_var("CRASHBOX_PROJECT_NAME", "notify-test");
        std::env::set_var("CRASHBOX_PROJECT_PUBLIC_KEY", "notifkey");
        std::env::set_var("CRASHBOX_GENERIC_WEBHOOK_URL", webhook_url);
        std::env::set_var("CRASHBOX_NOTIFY_MAX_PER_MINUTE", max_per_minute);
        // Drop these so we don't accidentally hit real Telegram / Discord from CI.
        std::env::remove_var("CRASHBOX_TELEGRAM_BOT_TOKEN");
        std::env::remove_var("CRASHBOX_TELEGRAM_CHAT_ID");
        std::env::remove_var("CRASHBOX_DISCORD_WEBHOOK_URL");
        Config::from_env().unwrap()
    };

    let pool = db::connect(&cfg.database_url).await.unwrap();
    db::migrate(&pool).await.unwrap();
    crashbox::bootstrap::run(&pool, &cfg).await.unwrap();

    let state = AppState::new(cfg, pool.clone());
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

fn make_envelope(payload: &str) -> String {
    format!(
        "{{}}\n{{\"type\":\"event\",\"length\":{}}}\n{}\n",
        payload.len(),
        payload
    )
}

async fn send(client: &reqwest::Client, cb: SocketAddr, payload: &str) {
    let resp = client
        .post(format!("http://{cb}/api/1/envelope/"))
        .header("x-sentry-auth", "Sentry sentry_key=notifkey")
        .body(make_envelope(payload))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
}

/// Wait briefly for the fire-and-forget notify task to deliver. We don't have hooks into the
/// hub, so we sleep just enough for a localhost POST round-trip.
async fn drain() {
    tokio::time::sleep(Duration::from_millis(150)).await;
}

#[tokio::test]
async fn notifies_on_new_issue_then_silent_on_followups() {
    let (hook_addr, received) = spawn_webhook_receiver().await;
    let webhook_url = format!("http://{hook_addr}/hook");
    let (cb, _pool) = spawn_crashbox(&webhook_url, "30").await;
    let client = reqwest::Client::new();

    let payload = json!({
        "platform": "node",
        "exception": {"values": [{"type": "X", "value": "boom"}]}
    })
    .to_string();

    send(&client, cb, &payload).await;
    drain().await;
    send(&client, cb, &payload).await;
    drain().await;
    send(&client, cb, &payload).await;
    drain().await;

    let messages = received.0.lock().unwrap().clone();
    assert_eq!(
        messages.len(),
        1,
        "exactly one NewIssue notification expected; got {messages:?}"
    );
    assert_eq!(messages[0]["kind"], "new_issue");
    assert_eq!(messages[0]["project_slug"], "notify-test");
    assert_eq!(messages[0]["issue_title"], "X: boom");
    assert_eq!(messages[0]["event_count"], 1);
    assert!(messages[0]["link"]
        .as_str()
        .unwrap()
        .ends_with("/issues/1"));
}

#[tokio::test]
async fn notifies_on_reopen_after_manual_resolve() {
    let (hook_addr, received) = spawn_webhook_receiver().await;
    let webhook_url = format!("http://{hook_addr}/hook");
    let (cb, _pool) = spawn_crashbox(&webhook_url, "30").await;
    let client = reqwest::Client::new();

    let payload = json!({
        "platform": "node",
        "exception": {"values": [{"type": "Y", "value": "oops"}]}
    })
    .to_string();

    // 1. First event → NewIssue
    send(&client, cb, &payload).await;
    drain().await;

    // 2. Log in and resolve the issue via admin API
    let auth = reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .unwrap();
    auth.post(format!("http://{cb}/api/auth/login"))
        .json(&json!({"email": "a@b.c", "password": "x"}))
        .send()
        .await
        .unwrap();
    auth.patch(format!("http://{cb}/api/issues/1"))
        .json(&json!({"status": "resolved"}))
        .send()
        .await
        .unwrap();

    // 3. Another event lands → should reopen + notify
    send(&client, cb, &payload).await;
    drain().await;

    let messages = received.0.lock().unwrap().clone();
    assert_eq!(messages.len(), 2, "expected NewIssue + Reopened; got {messages:?}");
    assert_eq!(messages[0]["kind"], "new_issue");
    assert_eq!(messages[1]["kind"], "reopened");
    assert_eq!(messages[1]["event_count"], 2);
}

#[tokio::test]
async fn rate_limit_drops_excess_notifications() {
    let (hook_addr, received) = spawn_webhook_receiver().await;
    let webhook_url = format!("http://{hook_addr}/hook");
    // Tight cap: only 3 notifications/min get through.
    let (cb, _pool) = spawn_crashbox(&webhook_url, "3").await;
    let client = reqwest::Client::new();

    // Five distinct exception types → five NewIssue events, but bucket capacity = 3.
    for t in ["A", "B", "C", "D", "E"] {
        let p = json!({
            "platform": "node",
            "exception": {"values": [{"type": t, "value": "x"}]}
        })
        .to_string();
        send(&client, cb, &p).await;
    }
    drain().await;
    drain().await;

    let messages = received.0.lock().unwrap().clone();
    assert_eq!(messages.len(), 3, "rate limit must cap at 3; got {messages:?}");
}
