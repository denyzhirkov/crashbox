#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for GET /api/admin/backup: the streamed snapshot is a valid SQLite
//! database containing the ingested data, and the temp file is cleaned up.

use crashbox::app_state::AppState;
use crashbox::config::Config;
use crashbox::{db, http};
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Mutex;

// Setup mutates process-global env vars. Serialize the setup phase across tests.
static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn spawn_app() -> (SocketAddr, PathBuf) {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let db_dir = tmp.path().to_path_buf();
    let db_path = db_dir.join("crashbox.db");

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
        std::env::set_var("CRASHBOX_ADMIN_EMAIL", "admin@example.com");
        std::env::set_var("CRASHBOX_ADMIN_PASSWORD", "hunter2");
        std::env::set_var("CRASHBOX_PROJECT_NAME", "test");
        std::env::set_var("CRASHBOX_PROJECT_PUBLIC_KEY", "testpublickey");
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
    Box::leak(Box::new(tmp));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (addr, db_dir)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .expect("client")
}

async fn login(c: &reqwest::Client, addr: SocketAddr) {
    let resp = c
        .post(format!("http://{addr}/api/auth/login"))
        .json(&json!({"email": "admin@example.com", "password": "hunter2"}))
        .send()
        .await
        .expect("login");
    assert_eq!(resp.status().as_u16(), 200);
}

#[tokio::test]
async fn backup_requires_auth() {
    let (addr, _dir) = spawn_app().await;
    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/api/admin/backup"))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 401);
}

#[tokio::test]
async fn backup_streams_a_valid_snapshot_and_cleans_up() {
    let (addr, db_dir) = spawn_app().await;
    let c = client();

    // Ingest one event so the snapshot has content to prove itself with.
    let payload = "{\"message\":\"backup me\",\"platform\":\"node\"}";
    let envelope = format!(
        "{{\"event_id\":\"e\"}}\n{{\"type\":\"event\",\"length\":{}}}\n{}\n",
        payload.len(),
        payload
    );
    let ingest = c
        .post(format!("http://{addr}/api/1/envelope/"))
        .header("x-sentry-auth", "Sentry sentry_key=testpublickey")
        .body(envelope)
        .send()
        .await
        .expect("ingest");
    assert_eq!(ingest.status().as_u16(), 200);

    login(&c, addr).await;
    let resp = c
        .get(format!("http://{addr}/api/admin/backup"))
        .send()
        .await
        .expect("backup");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/octet-stream")
    );
    let disposition = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .expect("content-disposition")
        .to_string();
    assert!(disposition.contains("crashbox-"), "{disposition}");
    let bytes = resp.bytes().await.expect("body");
    assert!(
        bytes.starts_with(b"SQLite format 3\0"),
        "snapshot must be a SQLite file"
    );

    // The snapshot is a fully usable database with the ingested event.
    let snap_path = db_dir.join("restored.db");
    std::fs::write(&snap_path, &bytes).expect("write snapshot");
    let snap_pool = db::connect(&format!("sqlite://{}", snap_path.display()))
        .await
        .expect("open snapshot");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE project_id = 1")
        .fetch_one(&snap_pool)
        .await
        .expect("count");
    assert_eq!(count, 1);

    // Temp snapshot files next to the live DB are removed once the body is consumed.
    let leftovers: Vec<_> = std::fs::read_dir(&db_dir)
        .expect("read dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".crashbox-backup-"))
        .collect();
    assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
}
