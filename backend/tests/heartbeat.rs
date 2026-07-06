#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end tests for heartbeat monitors: admin CRUD + the public ping endpoint.

use crashbox::app_state::AppState;
use crashbox::config::Config;
use crashbox::{db, http};
use reqwest::redirect::Policy;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn spawn_app_with(extra_env: &[(&str, &str)]) -> SocketAddr {
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
        std::env::set_var("CRASHBOX_PUBLIC_URL", "http://crash.example.com");
        std::env::set_var("CRASHBOX_ADMIN_EMAIL", "admin@example.com");
        std::env::set_var("CRASHBOX_ADMIN_PASSWORD", "hunter2");
        std::env::set_var("CRASHBOX_PROJECT_NAME", "main");
        std::env::set_var("CRASHBOX_PROJECT_PUBLIC_KEY", "pkmain");
        for (k, v) in extra_env {
            std::env::set_var(k, v);
        }
        let cfg = Config::from_env().expect("cfg");
        // Restore so a custom limit doesn't leak into apps spawned by other tests.
        for (k, _) in extra_env {
            std::env::remove_var(k);
        }
        cfg
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

async fn spawn_app() -> SocketAddr {
    spawn_app_with(&[]).await
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
    assert_eq!(resp.status().as_u16(), 200, "login should succeed");
    c
}

async fn create_monitor(c: &reqwest::Client, addr: SocketAddr, body: Value) -> Value {
    let resp = c
        .post(format!("http://{addr}/api/projects/1/heartbeats"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 201, "create should succeed");
    resp.json().await.unwrap()
}

#[tokio::test]
async fn ping_lifecycle_pending_to_up() {
    let addr = spawn_app().await;
    let c = admin(addr).await;

    let monitor = create_monitor(
        &c,
        addr,
        json!({"name": "nightly backup", "period_seconds": 3600}),
    )
    .await;
    assert_eq!(monitor["status"], "pending");
    assert_eq!(monitor["grace_seconds"], 60, "default grace");
    assert!(monitor["last_ping_at"].is_null());
    let ping_key = monitor["ping_key"].as_str().unwrap();
    assert_eq!(
        monitor["ping_url"].as_str().unwrap(),
        format!("http://crash.example.com/ping/{ping_key}")
    );

    // Ping is public: no cookies, GET, trailing-slash variant too.
    let anon = client();
    for path in [format!("/ping/{ping_key}"), format!("/ping/{ping_key}/")] {
        let resp = anon
            .get(format!("http://{addr}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(resp.text().await.unwrap(), "OK");
    }
    // POST works as well.
    let resp = anon
        .post(format!("http://{addr}/ping/{ping_key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);

    let list: Vec<Value> = c
        .get(format!("http://{addr}/api/projects/1/heartbeats"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["status"], "up");
    assert!(list[0]["last_ping_at"].is_string());
}

#[tokio::test]
async fn unknown_key_is_404() {
    let addr = spawn_app().await;
    let anon = client();
    let resp = anon
        .get(format!("http://{addr}/ping/not-a-real-key"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
}

#[tokio::test]
async fn create_and_patch_validation() {
    let addr = spawn_app().await;
    let c = admin(addr).await;

    for bad in [
        json!({"name": "  ", "period_seconds": 60}),
        json!({"name": "x", "period_seconds": 5}),
        json!({"name": "x", "period_seconds": 99_999_999}),
        json!({"name": "x", "period_seconds": 60, "grace_seconds": -1}),
        json!({"name": "x", "period_seconds": 60, "grace_seconds": 999_999}),
    ] {
        let resp = c
            .post(format!("http://{addr}/api/projects/1/heartbeats"))
            .json(&bad)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 400, "should reject {bad}");
    }

    let monitor = create_monitor(&c, addr, json!({"name": "job", "period_seconds": 60})).await;
    let id = monitor["id"].as_i64().unwrap();

    // Status is not free-form: up/down are owned by pings and the sweep.
    let resp = c
        .patch(format!("http://{addr}/api/heartbeats/{id}"))
        .json(&json!({"status": "down"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);

    // Edit fields.
    let patched: Value = c
        .patch(format!("http://{addr}/api/heartbeats/{id}"))
        .json(&json!({"name": "job v2", "period_seconds": 120, "grace_seconds": 30}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(patched["name"], "job v2");
    assert_eq!(patched["period_seconds"], 120);
    assert_eq!(patched["grace_seconds"], 30);

    // Unknown monitor → 404.
    let resp = c
        .patch(format!("http://{addr}/api/heartbeats/424242"))
        .json(&json!({"name": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
}

#[tokio::test]
async fn description_set_keep_and_clear() {
    let addr = spawn_app().await;
    let c = admin(addr).await;

    // Created with a note; blank-only note is treated as absent.
    let m = create_monitor(
        &c,
        addr,
        json!({"name": "reconcile", "period_seconds": 900,
               "description": "  nightly payment reconciliation — pages finance if silent  "}),
    )
    .await;
    let id = m["id"].as_i64().unwrap();
    assert_eq!(
        m["description"], "nightly payment reconciliation — pages finance if silent",
        "stored trimmed"
    );

    // PATCH without the field keeps it.
    let kept: Value = c
        .patch(format!("http://{addr}/api/heartbeats/{id}"))
        .json(&json!({"period_seconds": 1800}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        kept["description"].is_string(),
        "absent field must keep note"
    );

    // PATCH with a blank clears it.
    let cleared: Value = c
        .patch(format!("http://{addr}/api/heartbeats/{id}"))
        .json(&json!({"description": "  "}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(cleared["description"].is_null(), "blank must clear note");

    // Over the cap → 400.
    let resp = c
        .post(format!("http://{addr}/api/projects/1/heartbeats"))
        .json(&json!({"name": "x", "period_seconds": 60, "description": "d".repeat(501)}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
}

#[tokio::test]
async fn pause_resume_and_ping_from_paused() {
    let addr = spawn_app().await;
    let c = admin(addr).await;

    let monitor = create_monitor(&c, addr, json!({"name": "job", "period_seconds": 60})).await;
    let id = monitor["id"].as_i64().unwrap();
    let ping_key = monitor["ping_key"].as_str().unwrap();

    let paused: Value = c
        .patch(format!("http://{addr}/api/heartbeats/{id}"))
        .json(&json!({"status": "paused"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(paused["status"], "paused");

    // Resume goes back to pending (not up): a stale last_ping_at must not be able to
    // trigger an instant down-alert.
    let resumed: Value = c
        .patch(format!("http://{addr}/api/heartbeats/{id}"))
        .json(&json!({"status": "pending"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resumed["status"], "pending");

    // Pause again; a ping from paused resumes to up.
    c.patch(format!("http://{addr}/api/heartbeats/{id}"))
        .json(&json!({"status": "paused"}))
        .send()
        .await
        .unwrap();
    let anon = client();
    let resp = anon
        .get(format!("http://{addr}/ping/{ping_key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let got: Value = c
        .get(format!("http://{addr}/api/projects/1/heartbeats"))
        .send()
        .await
        .unwrap()
        .json::<Vec<Value>>()
        .await
        .unwrap()
        .remove(0);
    assert_eq!(got["status"], "up");
}

#[tokio::test]
async fn delete_invalidates_ping_url() {
    let addr = spawn_app().await;
    let c = admin(addr).await;

    let monitor = create_monitor(&c, addr, json!({"name": "job", "period_seconds": 60})).await;
    let id = monitor["id"].as_i64().unwrap();
    let ping_key = monitor["ping_key"].as_str().unwrap();

    let anon = client();
    let resp = anon
        .get(format!("http://{addr}/ping/{ping_key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);

    let resp = c
        .delete(format!("http://{addr}/api/heartbeats/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 204);

    let resp = anon
        .get(format!("http://{addr}/ping/{ping_key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404, "deleted monitor URL is dead");
}

#[tokio::test]
async fn admin_api_requires_session() {
    let addr = spawn_app().await;
    let anon = client();

    let resp = anon
        .get(format!("http://{addr}/api/projects/1/heartbeats"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401);

    let resp = anon
        .post(format!("http://{addr}/api/projects/1/heartbeats"))
        .json(&json!({"name": "job", "period_seconds": 60}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401);
}

#[tokio::test]
async fn ping_rate_limit_returns_429() {
    let addr = spawn_app_with(&[("CRASHBOX_HEARTBEAT_MAX_PINGS_PER_MINUTE", "2")]).await;
    let c = admin(addr).await;

    let monitor = create_monitor(&c, addr, json!({"name": "job", "period_seconds": 60})).await;
    let ping_key = monitor["ping_key"].as_str().unwrap();

    let anon = client();
    for _ in 0..2 {
        let resp = anon
            .get(format!("http://{addr}/ping/{ping_key}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
    }
    let resp = anon
        .get(format!("http://{addr}/ping/{ping_key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 429);
    assert!(resp.headers().get("retry-after").is_some());
}
