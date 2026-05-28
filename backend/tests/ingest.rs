//! Integration test: real Sentry-style envelope is accepted, stored, and looked up by event_id.

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
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        Config::from_env().expect("config")
    };
    let pool = db::connect(&cfg.database_url).await.expect("pool");
    db::migrate(&pool).await.expect("migrate");
    crashbox::bootstrap::run(&pool, &cfg)
        .await
        .expect("bootstrap");

    let state = AppState::new(cfg, pool.clone());
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
    // Give the server a moment to come up.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (addr, pool)
}

fn make_envelope(event_id: &str, payload: &str) -> String {
    format!(
        "{{\"event_id\":\"{event_id}\",\"sent_at\":\"2026-01-01T00:00:00Z\"}}\n\
         {{\"type\":\"event\",\"length\":{}}}\n\
         {}\n",
        payload.len(),
        payload
    )
}

#[tokio::test]
async fn happy_path_accepts_envelope_and_stores_event() {
    let (addr, pool) = spawn_app().await;
    let event_id = "abcdef1234567890abcdef1234567890";
    let payload = format!(
        "{{\"event_id\":\"{event_id}\",\"message\":\"hello from test\",\"platform\":\"node\"}}"
    );
    let envelope = make_envelope(event_id, &payload);

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/envelope/"))
        .header(
            "x-sentry-auth",
            "Sentry sentry_version=7, sentry_key=testpublickey, sentry_client=test/0",
        )
        .header("content-type", "application/x-sentry-envelope")
        .body(envelope)
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 200, "body: {:?}", resp.text().await);

    let stored: (String, String) =
        sqlx::query_as("SELECT event_id, raw_json FROM events WHERE project_id = 1")
            .fetch_one(&pool)
            .await
            .expect("fetch event");
    assert_eq!(stored.0, event_id);
    assert!(stored.1.contains("hello from test"));
}

#[tokio::test]
async fn rejects_unknown_sentry_key() {
    let (addr, _pool) = spawn_app().await;
    let payload = "{\"message\":\"x\"}";
    let envelope = make_envelope("0", payload);

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/envelope/"))
        .header("x-sentry-auth", "Sentry sentry_key=WRONG")
        .body(envelope)
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 401);
}

#[tokio::test]
async fn groups_same_exception_into_one_issue() {
    let (addr, pool) = spawn_app().await;
    let client = reqwest::Client::new();

    let send = |payload: String| {
        let client = client.clone();
        let addr = addr;
        async move {
            client
                .post(format!("http://{addr}/api/1/envelope/"))
                .header("x-sentry-auth", "Sentry sentry_key=testpublickey")
                .body(make_envelope("evt", &payload))
                .send()
                .await
                .expect("send")
        }
    };

    // Same TypeError, only the variable id differs — should group via normalize_message.
    let payload_a = serde_json::json!({
        "platform": "node",
        "exception": {"values": [{
            "type": "TypeError",
            "value": "row 11111111 missing",
            "stacktrace": {"frames": [
                {"function": "load", "filename": "/app/db.js", "lineno": 7, "in_app": true}
            ]}
        }]}
    })
    .to_string();
    let payload_b = serde_json::json!({
        "platform": "node",
        "exception": {"values": [{
            "type": "TypeError",
            "value": "row 22222222 missing",
            "stacktrace": {"frames": [
                {"function": "load", "filename": "/app/db.js", "lineno": 7, "in_app": true}
            ]}
        }]}
    })
    .to_string();
    let payload_c = serde_json::json!({
        "platform": "node",
        "exception": {"values": [{
            "type": "RangeError",
            "value": "i out of bounds",
            "stacktrace": {"frames": [
                {"function": "iter", "filename": "/app/it.js", "lineno": 12, "in_app": true}
            ]}
        }]}
    })
    .to_string();

    assert_eq!(send(payload_a).await.status().as_u16(), 200);
    assert_eq!(send(payload_b).await.status().as_u16(), 200);
    assert_eq!(send(payload_c).await.status().as_u16(), 200);

    let issue_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM issues WHERE project_id = 1")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(issue_count, 2, "TypeError variants merge, RangeError is its own");

    let event_counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT title, event_count FROM issues WHERE project_id = 1 ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("rows");
    let tt = event_counts
        .iter()
        .find(|(t, _)| t.starts_with("TypeError"))
        .expect("type-error issue");
    assert_eq!(tt.1, 2, "two type errors grouped into one issue");
}

#[tokio::test]
async fn persists_tags_and_breadcrumbs() {
    let (addr, pool) = spawn_app().await;
    let payload = serde_json::json!({
        "platform": "node",
        "message": "broke",
        "tags": {"env": "prod", "shard": "us-east"},
        "breadcrumbs": {"values": [
            {"category": "ui", "message": "click", "level": "info"},
            {"category": "http", "message": "GET /x 500", "level": "error"},
        ]}
    })
    .to_string();

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/envelope/"))
        .header("x-sentry-auth", "Sentry sentry_key=testpublickey")
        .body(make_envelope("e1", &payload))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 200);

    let tag_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM event_tags")
        .fetch_one(&pool)
        .await
        .expect("tags");
    let crumb_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM event_breadcrumbs")
        .fetch_one(&pool)
        .await
        .expect("crumbs");
    assert_eq!(tag_count, 2);
    assert_eq!(crumb_count, 2);
}

#[tokio::test]
async fn concurrent_burst_does_not_deadlock_on_sqlite() {
    // Regression for SQLITE_BUSY under burst writes. Pre-fix: ~half of these would land
    // because `pool.begin()` (BEGIN DEFERRED) caused upgrade races between SHARED→RESERVED.
    // The fix: `db::begin_write()` uses `BEGIN IMMEDIATE`, which serializes cleanly via
    // busy_timeout.
    let (addr, pool) = spawn_app().await;
    let client = reqwest::Client::new();
    const N: usize = 50;

    // All identical so they group into one issue with event_count=N.
    let payload = serde_json::json!({
        "platform": "node",
        "exception": {"values": [{
            "type": "BurstError",
            "value": "concurrent ingest",
            "stacktrace": {"frames": [
                {"function": "f", "filename": "b.js", "lineno": 1, "in_app": true}
            ]}
        }]}
    })
    .to_string();
    let envelope = make_envelope("burst", &payload);

    let mut tasks = Vec::with_capacity(N);
    for _ in 0..N {
        let c = client.clone();
        let body = envelope.clone();
        let url = format!("http://{addr}/api/1/envelope/");
        tasks.push(tokio::spawn(async move {
            c.post(url)
                .header("x-sentry-auth", "Sentry sentry_key=testpublickey")
                .body(body)
                .send()
                .await
                .map(|r| r.status().as_u16())
        }));
    }

    let mut ok = 0;
    let mut failures = Vec::new();
    for t in tasks {
        match t.await.expect("join") {
            Ok(200) => ok += 1,
            Ok(other) => failures.push(format!("status {other}")),
            Err(e) => failures.push(format!("send error: {e}")),
        }
    }
    assert_eq!(
        ok, N,
        "every concurrent envelope must succeed; failures: {failures:?}"
    );

    // All N events should have landed under one fingerprint.
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE project_id = 1")
        .fetch_one(&pool)
        .await
        .expect("count events");
    assert_eq!(total as usize, N);

    let issues: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM issues WHERE project_id = 1")
        .fetch_one(&pool)
        .await
        .expect("count issues");
    assert_eq!(issues, 1, "all identical events must group into one issue");

    let event_count: i64 =
        sqlx::query_scalar("SELECT event_count FROM issues WHERE project_id = 1")
            .fetch_one(&pool)
            .await
            .expect("event_count");
    assert_eq!(event_count as usize, N, "issue.event_count must equal total");
}

#[tokio::test]
async fn rejects_garbage_envelope() {
    let (addr, _pool) = spawn_app().await;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/api/1/envelope"))
        .header("x-sentry-auth", "Sentry sentry_key=testpublickey")
        .body("this is not an envelope at all".to_string())
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status().as_u16(), 400);
}
