#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Ingest rate limiting speaks the Sentry backoff protocol: 429 + Retry-After +
//! X-Sentry-Rate-Limits. Lives in its own test binary because it lowers
//! CRASHBOX_MAX_EVENTS_PER_MINUTE_PER_PROJECT process-wide.

use crashbox::app_state::AppState;
use crashbox::config::Config;
use crashbox::{db, http};
use std::net::SocketAddr;

async fn spawn_app() -> SocketAddr {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let db_path = tmp.path().join("crashbox.db");

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
    std::env::set_var("CRASHBOX_MAX_EVENTS_PER_MINUTE_PER_PROJECT", "2");
    std::env::set_var("CRASHBOX_MAX_LOGS_PER_MINUTE_PER_PROJECT", "2");
    std::env::set_var("CRASHBOX_ENABLE_LEGACY_STORE_ENDPOINT", "true");
    let cfg = Config::from_env().expect("config");

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
    addr
}

fn envelope(msg: &str) -> String {
    let payload = format!("{{\"message\":\"{msg}\"}}");
    format!(
        "{{\"event_id\":\"e\"}}\n{{\"type\":\"event\",\"length\":{}}}\n{}\n",
        payload.len(),
        payload
    )
}

#[tokio::test]
async fn rate_limited_ingest_sends_sentry_backoff_headers() {
    let addr = spawn_app().await;
    let client = reqwest::Client::new();

    let send_envelope = |i: usize| {
        let client = client.clone();
        async move {
            client
                .post(format!("http://{addr}/api/1/envelope/"))
                .header("x-sentry-auth", "Sentry sentry_key=testpublickey")
                .body(envelope(&format!("msg {i}")))
                .send()
                .await
                .expect("send")
        }
    };

    assert_eq!(send_envelope(1).await.status().as_u16(), 200);
    assert_eq!(send_envelope(2).await.status().as_u16(), 200);

    let limited = send_envelope(3).await;
    assert_eq!(limited.status().as_u16(), 429);
    let retry_after = limited
        .headers()
        .get("retry-after")
        .expect("retry-after present")
        .to_str()
        .expect("ascii");
    assert!(retry_after.parse::<u32>().expect("numeric") >= 1);
    let sentry_limits = limited
        .headers()
        .get("x-sentry-rate-limits")
        .expect("x-sentry-rate-limits present")
        .to_str()
        .expect("ascii");
    assert_eq!(sentry_limits, format!("{retry_after}:error:project"));

    // /store/ shares the same limiter and headers.
    let store = client
        .post(format!("http://{addr}/api/1/store/"))
        .header("x-sentry-auth", "Sentry sentry_key=testpublickey")
        .body("{\"message\":\"store\"}")
        .send()
        .await
        .expect("send");
    assert_eq!(store.status().as_u16(), 429);
    assert!(store.headers().get("x-sentry-rate-limits").is_some());

    // The logs endpoint uses its own limiter but the same response shape, with the
    // log_item category.
    let logs_url = format!("http://{addr}/api/1/logs?sentry_key=testpublickey");
    for _ in 0..2 {
        let ok = client
            .post(&logs_url)
            .body("{\"message\":\"log line\"}")
            .send()
            .await
            .expect("send");
        assert_eq!(ok.status().as_u16(), 202);
    }
    let limited_logs = client
        .post(&logs_url)
        .body("{\"message\":\"log line\"}")
        .send()
        .await
        .expect("send");
    assert_eq!(limited_logs.status().as_u16(), 429);
    assert!(limited_logs.headers().get("retry-after").is_some());
    let log_limits = limited_logs
        .headers()
        .get("x-sentry-rate-limits")
        .expect("x-sentry-rate-limits present")
        .to_str()
        .expect("ascii");
    assert!(log_limits.ends_with(":log_item:project"), "{log_limits}");
}
