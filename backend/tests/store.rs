#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for the legacy store API: POST /api/:project_id/store[/] with a bare
//! event JSON body (no envelope framing).

use crashbox::app_state::AppState;
use crashbox::config::Config;
use crashbox::{db, http};
use std::net::SocketAddr;
use std::sync::Mutex;

// Setup mutates process-global env vars. Serialize the setup phase across tests.
static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn spawn_app() -> (SocketAddr, sqlx::SqlitePool) {
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
        std::env::set_var("CRASHBOX_PROJECT_PUBLIC_KEY", "testpublickey");
        std::env::set_var("CRASHBOX_ENABLE_LEGACY_STORE_ENDPOINT", "true");
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
    let app = http::routes::build(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("addr");

    // Leak the tmpdir guard so the DB lives as long as the test process.
    Box::leak(Box::new(tmp));

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (addr, pool)
}

#[tokio::test]
async fn happy_path_stores_bare_event_and_returns_id() {
    let (addr, pool) = spawn_app().await;
    let event_id = "11112222333344445555666677778888";
    let payload = format!(
        "{{\"event_id\":\"{event_id}\",\"message\":\"legacy store hello\",\"platform\":\"python\"}}"
    );

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/store/"))
        .header(
            "x-sentry-auth",
            "Sentry sentry_version=7, sentry_key=testpublickey, sentry_client=raven/legacy",
        )
        .header("content-type", "application/json")
        .body(payload)
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["id"], event_id);

    let stored: (String, String) =
        sqlx::query_as("SELECT event_id, raw_json FROM events WHERE project_id = 1")
            .fetch_one(&pool)
            .await
            .expect("fetch event");
    assert_eq!(stored.0, event_id);
    assert!(stored.1.contains("legacy store hello"));

    let issues: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM issues WHERE project_id = 1")
        .fetch_one(&pool)
        .await
        .expect("issues");
    assert_eq!(issues, 1, "bare event goes through grouping too");
}

#[tokio::test]
async fn both_slash_variants_work() {
    let (addr, pool) = spawn_app().await;
    let client = reqwest::Client::new();
    for (i, path) in ["store", "store/"].iter().enumerate() {
        let resp = client
            .post(format!("http://{addr}/api/1/{path}"))
            .header("x-sentry-auth", "Sentry sentry_key=testpublickey")
            .body(format!("{{\"message\":\"variant {i}\"}}"))
            .send()
            .await
            .expect("send");
        assert_eq!(resp.status().as_u16(), 200, "path {path}");
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE project_id = 1")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 2);
}

#[tokio::test]
async fn accepts_real_sdk_event_fixture() {
    let (addr, pool) = spawn_app().await;
    let payload = include_str!("fixtures/envelopes/sentry-node-typeerror.event.json");

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/store/"))
        .header("x-sentry-auth", "Sentry sentry_key=testpublickey")
        .body(payload)
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 200);

    let title: String = sqlx::query_scalar("SELECT title FROM issues WHERE project_id = 1")
        .fetch_one(&pool)
        .await
        .expect("title");
    assert!(title.starts_with("TypeError"), "title: {title}");
}

#[tokio::test]
async fn accepts_gzip_compressed_store_body() {
    use std::io::Write;
    let (addr, pool) = spawn_app().await;
    let payload = b"{\"message\":\"compressed legacy\",\"platform\":\"python\"}";
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(payload).expect("gzip write");
    let compressed = enc.finish().expect("gzip finish");

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/store/"))
        .header("x-sentry-auth", "Sentry sentry_key=testpublickey")
        .header("content-encoding", "gzip")
        .body(compressed)
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 200);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE project_id = 1")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn rejects_unknown_key_with_401() {
    let (addr, _pool) = spawn_app().await;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/store/"))
        .header("x-sentry-auth", "Sentry sentry_key=WRONG")
        .body("{\"message\":\"x\"}")
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 401);
}

#[tokio::test]
async fn rejects_non_json_body_with_400() {
    let (addr, pool) = spawn_app().await;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/store/"))
        .header("x-sentry-auth", "Sentry sentry_key=testpublickey")
        .body("not json at all")
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 400);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE project_id = 1")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 0);
}
