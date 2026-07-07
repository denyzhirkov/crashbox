#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for Live Logs ingest: the dedicated `/logs` endpoint and the Sentry `log`
//! envelope item both publish into the in-memory hub. Nothing is persisted.

use crashbox::app_state::AppState;
use crashbox::config::Config;
use crashbox::livelog::LiveLogHub;
use crashbox::{db, http};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_stream::StreamExt;

static ENV_LOCK: Mutex<()> = Mutex::new(());

const KEY: &str = "logspublickey";

async fn spawn_app() -> (SocketAddr, Arc<LiveLogHub>) {
    spawn_app_with(true).await
}

async fn spawn_app_with(live_logs_enabled: bool) -> (SocketAddr, Arc<LiveLogHub>) {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let db_path = tmp.path().join("crashbox.db");

    let cfg = {
        let _guard = ENV_LOCK
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
        std::env::set_var("CRASHBOX_PROJECT_NAME", "test");
        std::env::set_var("CRASHBOX_PROJECT_PUBLIC_KEY", KEY);
        std::env::set_var(
            "CRASHBOX_LIVE_LOGS_ENABLED",
            if live_logs_enabled { "true" } else { "false" },
        );
        Config::from_env().expect("config")
    };
    let pool = db::connect(&cfg.database_url).await.expect("pool");
    db::migrate(&pool).await.expect("migrate");
    crashbox::bootstrap::run(&pool, &cfg)
        .await
        .expect("bootstrap");

    let state = AppState::new(
        cfg,
        pool.clone(),
        crashbox::metrics_layer::MetricsHandle::dummy(),
    );
    let hub = state.livelog.clone();
    let app = http::routes::build(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("addr");
    Box::leak(Box::new(tmp));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (addr, hub)
}

fn auth_header() -> String {
    format!("Sentry sentry_version=7, sentry_key={KEY}, sentry_client=test/0")
}

/// A cookie-aware client logged in as the bootstrapped admin (a@b.c / x).
async fn logged_in_client(addr: SocketAddr) -> reqwest::Client {
    let client = reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .expect("client");
    let resp = client
        .post(format!("http://{addr}/api/auth/login"))
        .json(&serde_json::json!({ "email": "a@b.c", "password": "x" }))
        .send()
        .await
        .expect("login");
    assert_eq!(resp.status().as_u16(), 200, "login should succeed");
    client
}

/// Drain an SSE response into a String for up to `dur`, then return what arrived.
async fn read_stream_for(resp: reqwest::Response, dur: Duration) -> String {
    let mut stream = Box::pin(resp.bytes_stream());
    let mut buf = String::new();
    let deadline = tokio::time::Instant::now() + dur;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(Ok(chunk))) => buf.push_str(&String::from_utf8_lossy(&chunk)),
            _ => break,
        }
    }
    buf
}

#[tokio::test]
async fn logs_endpoint_accepts_array_and_publishes_to_hub() {
    let (addr, hub) = spawn_app().await;

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/logs"))
        .header("x-sentry-auth", auth_header())
        .json(&serde_json::json!([
            { "level": "info", "message": "first", "logger": "auth" },
            { "level": "warn", "message": "second" }
        ]))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 202, "body: {:?}", resp.text().await);

    let snapshot = hub.subscribe(1).expect("subscribe").snapshot;
    let msgs: Vec<_> = snapshot.iter().map(|r| r.message.clone()).collect();
    assert_eq!(msgs, vec!["first", "second"]);
}

#[tokio::test]
async fn logs_endpoint_streams_live_to_an_existing_subscriber() {
    let (addr, hub) = spawn_app().await;
    let mut sub = hub.subscribe(1).expect("subscribe");
    assert!(sub.snapshot.is_empty());

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/logs/"))
        .header("x-sentry-auth", auth_header())
        .body("{\"message\":\"live one\"}\n{\"message\":\"live two\"}\n")
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 202);

    let first = tokio::time::timeout(std::time::Duration::from_secs(1), sub.rx.recv())
        .await
        .expect("not timed out")
        .expect("recv");
    assert_eq!(first.message, "live one");
}

#[tokio::test]
async fn logs_endpoint_rejects_unknown_key() {
    let (addr, _hub) = spawn_app().await;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/logs"))
        .header("x-sentry-auth", "Sentry sentry_key=WRONG")
        .json(&serde_json::json!([{ "message": "x" }]))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 401);
}

#[tokio::test]
async fn sentry_log_envelope_item_feeds_live_logs() {
    let (addr, hub) = spawn_app().await;

    let log_payload =
        r#"{"items":[{"timestamp":1700000000.0,"level":"error","body":"from envelope"}]}"#;
    let envelope = format!(
        "{{\"event_id\":\"00000000000000000000000000000000\"}}\n\
         {{\"type\":\"log\",\"length\":{}}}\n\
         {log_payload}\n",
        log_payload.len()
    );

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/envelope/"))
        .header("x-sentry-auth", auth_header())
        .header("content-type", "application/x-sentry-envelope")
        .body(envelope)
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 200, "body: {:?}", resp.text().await);

    let snapshot = hub.subscribe(1).expect("subscribe").snapshot;
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].message, "from envelope");
}

#[tokio::test]
async fn stream_requires_a_session() {
    let (addr, _hub) = spawn_app().await;
    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/api/projects/1/logs/stream"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 401);
}

#[tokio::test]
async fn stream_replays_snapshot_and_applies_level_filter() {
    let (addr, hub) = spawn_app().await;
    hub.publish(
        1,
        crashbox::livelog::LogRecord::from_loose(
            &serde_json::json!({ "level": "info", "message": "alpha" }),
            1024,
        )
        .expect("rec"),
    );
    hub.publish(
        1,
        crashbox::livelog::LogRecord::from_loose(
            &serde_json::json!({ "level": "error", "message": "gamma" }),
            1024,
        )
        .expect("rec"),
    );

    let client = logged_in_client(addr).await;
    let resp = client
        .get(format!(
            "http://{addr}/api/projects/1/logs/stream?level=warn"
        ))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    let body = read_stream_for(resp, Duration::from_millis(400)).await;
    assert!(body.contains("gamma"), "error line should pass: {body}");
    assert!(
        !body.contains("alpha"),
        "info line should be filtered: {body}"
    );
}

#[tokio::test]
async fn recent_returns_filtered_snapshot_without_streaming() {
    let (addr, hub) = spawn_app().await;
    for (level, msg) in [("info", "alpha"), ("error", "beta"), ("error", "gamma")] {
        hub.publish(
            1,
            crashbox::livelog::LogRecord::from_loose(
                &serde_json::json!({ "level": level, "message": msg }),
                1024,
            )
            .expect("rec"),
        );
    }

    let client = logged_in_client(addr).await;
    let body: serde_json::Value = client
        .get(format!(
            "http://{addr}/api/projects/1/logs/recent?level=error&limit=1"
        ))
        .send()
        .await
        .expect("send")
        .json()
        .await
        .expect("json");
    // level=error drops alpha; limit=1 keeps only the newest of the remaining two.
    assert_eq!(body["count"], 1);
    assert_eq!(body["items"][0]["message"], "gamma");

    // Unauthenticated → 401; unknown project → 404.
    let resp = reqwest::get(format!("http://{addr}/api/projects/1/logs/recent"))
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 401);
    let resp = client
        .get(format!("http://{addr}/api/projects/999/logs/recent"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 404);
}

#[tokio::test]
async fn disabled_feature_unmounts_routes() {
    let (addr, _hub) = spawn_app_with(false).await;
    let client = logged_in_client(addr).await;

    // The /me flag tells the UI the feature is off.
    let me: serde_json::Value = client
        .get(format!("http://{addr}/api/auth/me"))
        .send()
        .await
        .expect("me")
        .json()
        .await
        .expect("json");
    assert_eq!(me["live_logs_enabled"], serde_json::json!(false));

    // With the routes unmounted, requests fall through to the SPA fallback (HTML), never the API
    // handlers — so the ingest endpoint cannot accept logs and the stream isn't an event-stream.
    let ingest = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/logs"))
        .header("x-sentry-auth", auth_header())
        .json(&serde_json::json!([{ "message": "x" }]))
        .send()
        .await
        .expect("send");
    assert_ne!(ingest.status().as_u16(), 202, "ingest must not accept logs");
    let ct = ingest
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("text/html"),
        "expected SPA fallback, got {ct}"
    );

    let stream = client
        .get(format!("http://{addr}/api/projects/1/logs/stream"))
        .send()
        .await
        .expect("send");
    let ct = stream
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        !ct.starts_with("text/event-stream"),
        "stream route should be unmounted, got {ct}"
    );
}

#[tokio::test]
async fn stream_rejects_unknown_project() {
    let (addr, _hub) = spawn_app().await;
    let client = logged_in_client(addr).await;
    let resp = client
        .get(format!("http://{addr}/api/projects/9999/logs/stream"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 404);
}
