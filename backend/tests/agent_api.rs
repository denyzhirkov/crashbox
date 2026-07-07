#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end tests for the agent-facing API surface added in 1.9.0: read-scoped tokens,
//! paginated list envelopes with sort, the project-wide event feed with full-text search,
//! bulk issue patch, issue delete, and heartbeat transition history.

use crashbox::app_state::AppState;
use crashbox::config::Config;
use crashbox::{db, http};
use reqwest::redirect::Policy;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn spawn_app() -> SocketAddr {
    let tmp = tempfile::tempdir().expect("tmp");
    let db_path = tmp.path().join("crashbox.db");

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
        std::env::set_var("CRASHBOX_ADMIN_EMAIL", "admin@example.com");
        std::env::set_var("CRASHBOX_ADMIN_PASSWORD", "hunter2");
        std::env::set_var("CRASHBOX_PROJECT_NAME", "main");
        std::env::set_var("CRASHBOX_PROJECT_PUBLIC_KEY", "pkmain");
        Config::from_env().expect("cfg")
    };
    let pool = db::connect(&cfg.database_url).await.expect("pool");
    db::migrate(&pool).await.expect("migrate");
    crashbox::bootstrap::run(&pool, &cfg)
        .await
        .expect("bootstrap");

    let state = AppState::new(cfg, pool, crashbox::metrics_layer::MetricsHandle::dummy());
    let app = http::routes::build(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listen");
    let addr = listener.local_addr().expect("addr");
    Box::leak(Box::new(tmp));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    addr
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .cookie_store(true)
        .redirect(Policy::none())
        .build()
        .expect("client")
}

async fn admin(addr: SocketAddr) -> reqwest::Client {
    let c = client();
    let resp = c
        .post(format!("http://{addr}/api/auth/login"))
        .json(&json!({"email": "admin@example.com", "password": "hunter2"}))
        .send()
        .await
        .expect("login");
    assert_eq!(resp.status().as_u16(), 200);
    c
}

/// Ingest one event whose exception value / stack filename we control.
async fn ingest(addr: SocketAddr, ty: &str, value: &str, filename: &str) {
    let payload = json!({
        "platform": "node",
        "exception": {"values": [{
            "type": ty,
            "value": value,
            "stacktrace": {"frames": [
                {"function": "load", "filename": filename, "lineno": 7, "in_app": true}
            ]}
        }]}
    })
    .to_string();
    let env_body = format!(
        "{{}}\n{{\"type\":\"event\",\"length\":{}}}\n{}\n",
        payload.len(),
        payload
    );
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/envelope/"))
        .header("x-sentry-auth", "Sentry sentry_key=pkmain")
        .body(env_body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
}

#[tokio::test]
async fn read_scoped_token_is_get_only() {
    let addr = spawn_app().await;
    let c = admin(addr).await;

    let created: Value = c
        .post(format!("http://{addr}/api/tokens"))
        .json(&json!({"name": "agent-ro", "scope": "read"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["scope"], "read");
    let token = created["token"].as_str().unwrap().to_string();

    let bearer = client();
    // Reads pass.
    let resp = bearer
        .get(format!("http://{addr}/api/projects"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);

    // Writes are refused with 403 and a self-explanatory message.
    let resp = bearer
        .post(format!("http://{addr}/api/projects"))
        .header("authorization", format!("Bearer {token}"))
        .json(&json!({"name": "nope"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 403);
    let body: Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("read-only"));

    // Unknown scope is rejected at mint time.
    let resp = c
        .post(format!("http://{addr}/api/tokens"))
        .json(&json!({"name": "bad", "scope": "root"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
}

#[tokio::test]
async fn issues_list_sorts_and_paginates() {
    let addr = spawn_app().await;
    let c = admin(addr).await;

    // Three issues; "TypeError" gets two events so event_count differs.
    ingest(addr, "TypeError", "boom", "a.js").await;
    ingest(addr, "TypeError", "boom", "a.js").await;
    ingest(addr, "RangeError", "oob", "b.js").await;
    ingest(addr, "SyntaxError", "bad", "c.js").await;

    let page: Value = c
        .get(format!(
            "http://{addr}/api/projects/1/issues?sort=event_count&order=desc&limit=2"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page["total"], 3, "total counts beyond the page");
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "limit respected");
    assert!(items[0]["title"].as_str().unwrap().starts_with("TypeError"));

    // Unknown sort column is a 400, not silent fallback.
    let resp = c
        .get(format!("http://{addr}/api/projects/1/issues?sort=title"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
}

#[tokio::test]
async fn project_events_feed_supports_full_text_search() {
    let addr = spawn_app().await;
    let c = admin(addr).await;

    ingest(addr, "TypeError", "boom", "checkout.js").await;
    ingest(addr, "RangeError", "oob", "billing.js").await;

    // Whole-project feed, newest first.
    let all: Value = c
        .get(format!("http://{addr}/api/projects/1/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(all["total"], 2);

    // FTS reaches into the raw payload — the filename only exists in a stack frame.
    let hit: Value = c
        .get(format!("http://{addr}/api/projects/1/events?q=checkout.js"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(hit["total"], 1);
    assert_eq!(hit["items"][0]["exception_type"], "TypeError");
    assert!(hit["items"][0]["issue_id"].is_i64());

    // FTS operator characters are treated as literals, never a 500.
    let resp = c
        .get(format!(
            "http://{addr}/api/projects/1/events?q=NEAR(%22unbalanced"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
}

#[tokio::test]
async fn bulk_patch_and_delete_issue() {
    let addr = spawn_app().await;
    let c = admin(addr).await;

    ingest(addr, "TypeError", "boom", "a.js").await;
    ingest(addr, "RangeError", "oob", "b.js").await;

    // Bulk resolve both (one id is bogus — it just doesn't count).
    let out: Value = c
        .patch(format!("http://{addr}/api/issues"))
        .json(&json!({"ids": [1, 2, 999], "status": "resolved"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(out["updated"], 2);

    let page: Value = c
        .get(format!(
            "http://{addr}/api/projects/1/issues?status=resolved"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page["total"], 2);

    // Delete issue 1 → gone, and its events with it.
    let resp = c
        .delete(format!("http://{addr}/api/issues/1"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 204);
    let resp = c
        .get(format!("http://{addr}/api/issues/1"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
    let feed: Value = c
        .get(format!("http://{addr}/api/projects/1/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(feed["total"], 1, "deleted issue's events are gone");

    // Empty ids is a 400.
    let resp = c
        .patch(format!("http://{addr}/api/issues"))
        .json(&json!({"ids": [], "status": "resolved"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
}

#[tokio::test]
async fn heartbeat_history_records_transitions() {
    let addr = spawn_app().await;
    let c = admin(addr).await;

    let monitor: Value = c
        .post(format!("http://{addr}/api/projects/1/heartbeats"))
        .json(&json!({"name": "cron", "period_seconds": 60}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = monitor["id"].as_i64().unwrap();
    let ping_key = monitor["ping_key"].as_str().unwrap().to_string();

    // pending → up (ping), up → paused (patch), paused → pending (resume).
    let r = reqwest::Client::new()
        .get(format!("http://{addr}/ping/{ping_key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    for status in ["paused", "pending"] {
        let r = c
            .patch(format!("http://{addr}/api/heartbeats/{id}"))
            .json(&json!({"status": status}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 200);
    }

    let history: Value = c
        .get(format!("http://{addr}/api/heartbeats/{id}/history"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(history["total"], 3);
    let items = history["items"].as_array().unwrap();
    // Newest first.
    assert_eq!(items[0]["from_status"], "paused");
    assert_eq!(items[0]["to_status"], "pending");
    assert_eq!(items[2]["from_status"], "pending");
    assert_eq!(items[2]["to_status"], "up");

    // Unknown monitor → 404.
    let resp = c
        .get(format!("http://{addr}/api/heartbeats/999/history"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
}
