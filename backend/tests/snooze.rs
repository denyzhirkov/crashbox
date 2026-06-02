#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for A4 issue snooze.
//!
//! Covers:
//! - PATCH /api/issues/:id with {snooze: "1h"} removes the issue from the default unresolved list
//! - PATCH with {snooze: "forever"} also hides it; auto-wakes on the next ingested event
//! - status=snoozed filter shows snoozed issues
//! - PATCH with {snooze: "wake"} returns the issue to the default list
//! - Invalid snooze value returns 400

use crashbox::app_state::AppState;
use crashbox::config::Config;
use crashbox::{db, http};
use reqwest::redirect::Policy;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn spawn_app() -> SocketAddr {
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
        std::env::set_var("CRASHBOX_PROJECT_NAME", "snooze");
        std::env::set_var("CRASHBOX_PROJECT_PUBLIC_KEY", "snoozekey");
        std::env::remove_var("CRASHBOX_TELEGRAM_BOT_TOKEN");
        std::env::remove_var("CRASHBOX_DISCORD_WEBHOOK_URL");
        std::env::remove_var("CRASHBOX_GENERIC_WEBHOOK_URL");
        Config::from_env().unwrap()
    };
    let pool = db::connect(&cfg.database_url).await.unwrap();
    db::migrate(&pool).await.unwrap();
    crashbox::bootstrap::run(&pool, &cfg).await.unwrap();

    let state = AppState::new(cfg, pool, crashbox::metrics_layer::MetricsHandle::dummy());
    let app = http::routes::build(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    Box::leak(Box::new(tmp));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    addr
}

fn admin_client() -> reqwest::Client {
    reqwest::Client::builder()
        .cookie_store(true)
        .redirect(Policy::none())
        .build()
        .unwrap()
}

async fn login(c: &reqwest::Client, addr: SocketAddr) {
    c.post(format!("http://{addr}/api/auth/login"))
        .json(&json!({"email": "a@b.c", "password": "x"}))
        .send()
        .await
        .unwrap();
}

async fn ingest(addr: SocketAddr, ty: &str, value: &str) {
    let payload = json!({
        "platform": "node",
        "exception": {"values": [{"type": ty, "value": value}]}
    })
    .to_string();
    let envelope = format!(
        "{{}}\n{{\"type\":\"event\",\"length\":{}}}\n{}\n",
        payload.len(),
        payload
    );
    reqwest::Client::new()
        .post(format!("http://{addr}/api/1/envelope/"))
        .header("x-sentry-auth", "Sentry sentry_key=snoozekey")
        .body(envelope)
        .send()
        .await
        .unwrap();
}

async fn list(c: &reqwest::Client, addr: SocketAddr, status: &str) -> Vec<Value> {
    let url = format!("http://{addr}/api/projects/1/issues?status={status}");
    c.get(url).send().await.unwrap().json().await.unwrap()
}

async fn list_default(c: &reqwest::Client, addr: SocketAddr) -> Vec<Value> {
    let url = format!("http://{addr}/api/projects/1/issues");
    c.get(url).send().await.unwrap().json().await.unwrap()
}

#[tokio::test]
async fn time_snooze_hides_from_default_list_but_visible_under_snoozed_filter() {
    let addr = spawn_app().await;
    let c = admin_client();
    login(&c, addr).await;
    ingest(addr, "Foo", "x").await;
    ingest(addr, "Bar", "y").await;

    assert_eq!(list_default(&c, addr).await.len(), 2);

    // Snooze issue #1 for an hour.
    let r = c
        .patch(format!("http://{addr}/api/issues/1"))
        .json(&json!({"snooze": "1h"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let issue: Value = r.json().await.unwrap();
    assert!(issue["snoozed_until"].as_str().is_some());

    // Default list now shows only the un-snoozed issue.
    let visible = list_default(&c, addr).await;
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0]["id"], 2);

    // status=snoozed shows the muted one.
    let snoozed = list(&c, addr, "snoozed").await;
    assert_eq!(snoozed.len(), 1);
    assert_eq!(snoozed[0]["id"], 1);

    // Wake it back up.
    c.patch(format!("http://{addr}/api/issues/1"))
        .json(&json!({"snooze": "wake"}))
        .send()
        .await
        .unwrap();
    assert_eq!(list_default(&c, addr).await.len(), 2);
}

#[tokio::test]
async fn forever_snooze_auto_wakes_on_next_event() {
    let addr = spawn_app().await;
    let c = admin_client();
    login(&c, addr).await;
    ingest(addr, "Foo", "x").await;

    // forever-snooze
    c.patch(format!("http://{addr}/api/issues/1"))
        .json(&json!({"snooze": "forever"}))
        .send()
        .await
        .unwrap();
    assert_eq!(list_default(&c, addr).await.len(), 0);

    // Another event on the same fingerprint should auto-wake.
    ingest(addr, "Foo", "x").await;
    let visible = list_default(&c, addr).await;
    assert_eq!(visible.len(), 1, "auto-wake on event");
    assert!(visible[0]["snoozed_until"].is_null());
    assert_eq!(visible[0]["event_count"], 2);
}

#[tokio::test]
async fn invalid_snooze_value_returns_400() {
    let addr = spawn_app().await;
    let c = admin_client();
    login(&c, addr).await;
    ingest(addr, "Foo", "x").await;

    let r = c
        .patch(format!("http://{addr}/api/issues/1"))
        .json(&json!({"snooze": "decades"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 400);
}

#[tokio::test]
async fn patch_without_status_or_snooze_returns_400() {
    let addr = spawn_app().await;
    let c = admin_client();
    login(&c, addr).await;
    ingest(addr, "Foo", "x").await;

    let r = c
        .patch(format!("http://{addr}/api/issues/1"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 400);
}
