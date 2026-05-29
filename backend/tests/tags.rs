//! Integration tests for B3 tag filtering.

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
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var(
            "CRASHBOX_DATABASE_URL",
            format!("sqlite://{}", db_path.display()),
        );
        std::env::set_var("CRASHBOX_PORT", "0");
        std::env::set_var("CRASHBOX_PUBLIC_URL", "http://localhost");
        std::env::set_var("CRASHBOX_ADMIN_EMAIL", "a@b.c");
        std::env::set_var("CRASHBOX_ADMIN_PASSWORD", "x");
        std::env::set_var("CRASHBOX_PROJECT_NAME", "tags");
        std::env::set_var("CRASHBOX_PROJECT_PUBLIC_KEY", "tagkey");
        std::env::remove_var("CRASHBOX_TELEGRAM_BOT_TOKEN");
        std::env::remove_var("CRASHBOX_DISCORD_WEBHOOK_URL");
        std::env::remove_var("CRASHBOX_GENERIC_WEBHOOK_URL");
        Config::from_env().unwrap()
    };
    let pool = db::connect(&cfg.database_url).await.unwrap();
    db::migrate(&pool).await.unwrap();
    crashbox::bootstrap::run(&pool, &cfg).await.unwrap();
    let state = AppState::new(cfg, pool);
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

async fn ingest(addr: SocketAddr, ty: &str, tags: Value) {
    let payload = json!({
        "platform": "node",
        "exception": {"values": [{"type": ty, "value": "v"}]},
        "tags": tags,
    })
    .to_string();
    let envelope = format!(
        "{{}}\n{{\"type\":\"event\",\"length\":{}}}\n{}\n",
        payload.len(),
        payload
    );
    reqwest::Client::new()
        .post(format!("http://{addr}/api/1/envelope/"))
        .header("x-sentry-auth", "Sentry sentry_key=tagkey")
        .body(envelope)
        .send()
        .await
        .unwrap();
}

#[tokio::test]
async fn tag_filter_narrows_results() {
    let addr = spawn_app().await;
    let c = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(Policy::none())
        .build()
        .unwrap();
    c.post(format!("http://{addr}/api/auth/login"))
        .json(&json!({"email": "a@b.c", "password": "x"}))
        .send()
        .await
        .unwrap();

    ingest(addr, "A", json!({"env": "production", "shard": "us-east"})).await;
    ingest(addr, "B", json!({"env": "production"})).await;
    ingest(addr, "C", json!({"env": "staging"})).await;

    // No filter → all 3
    let all: Vec<Value> = c
        .get(format!("http://{addr}/api/projects/1/issues?status=all"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(all.len(), 3);

    // tag=env=production → 2 (A and B)
    let prod: Vec<Value> = c
        .get(format!(
            "http://{addr}/api/projects/1/issues?status=all&tag=env=production"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(prod.len(), 2);

    // tag=env=production AND tag=shard=us-east → 1 (A only)
    let east: Vec<Value> = c
        .get(format!(
            "http://{addr}/api/projects/1/issues?status=all&tag=env=production&tag=shard=us-east"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(east.len(), 1);
    assert!(east[0]["title"].as_str().unwrap().starts_with("A:"));
}
