#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end tests for personal API tokens: issue via session, authenticate via bearer,
//! revoke, and the token-cannot-manage-tokens rule.

use crashbox::app_state::AppState;
use crashbox::config::Config;
use crashbox::{db, http};
use reqwest::redirect::Policy;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn spawn_app() -> (SocketAddr, String) {
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
    let db_url = cfg.database_url.clone();
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
    (addr, db_url)
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

async fn mint(c: &reqwest::Client, addr: SocketAddr, body: Value) -> Value {
    let resp = c
        .post(format!("http://{addr}/api/tokens"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 201, "token create should succeed");
    resp.json().await.unwrap()
}

#[tokio::test]
async fn token_full_lifecycle() {
    let (addr, _db) = spawn_app().await;
    let c = admin(addr).await;

    let created = mint(&c, addr, json!({"name": "claude-code"})).await;
    let token = created["token"].as_str().unwrap();
    let id = created["id"].as_i64().unwrap();
    assert!(token.starts_with("cbx_"), "got {token}");
    assert!(created["expires_at"].is_null(), "non-expiring by default");
    assert_eq!(created["token_prefix"], token[..10].to_string());

    // Bearer works on the admin API — fresh client, no cookies at all.
    let bearer = client();
    let resp = bearer
        .get(format!("http://{addr}/api/projects"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let projects: Vec<Value> = resp.json().await.unwrap();
    assert_eq!(projects[0]["slug"], "main");

    // /api/auth/me identifies the token's user — handy for automation to self-check.
    let me = bearer
        .get(format!("http://{addr}/api/auth/me"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(me.status().as_u16(), 200);

    // The list never contains the secret, only the prefix.
    let list: Vec<Value> = c
        .get(format!("http://{addr}/api/tokens"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].get("token").is_none());
    assert!(list[0].get("token_hash").is_none());
    assert_eq!(list[0]["token_prefix"], token[..10].to_string());
    assert!(
        list[0]["last_used_at"].is_string(),
        "bearer use should stamp last_used_at"
    );

    // Revoke → the very next bearer request is refused.
    let resp = c
        .delete(format!("http://{addr}/api/tokens/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 204);
    let resp = bearer
        .get(format!("http://{addr}/api/projects"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401, "revocation must be instant");
}

#[tokio::test]
async fn token_cannot_manage_tokens() {
    let (addr, _db) = spawn_app().await;
    let c = admin(addr).await;
    let token = mint(&c, addr, json!({"name": "t"})).await["token"]
        .as_str()
        .unwrap()
        .to_string();

    let bearer = client();
    for req in [
        bearer
            .get(format!("http://{addr}/api/tokens"))
            .header("authorization", format!("Bearer {token}")),
        bearer
            .post(format!("http://{addr}/api/tokens"))
            .header("authorization", format!("Bearer {token}"))
            .json(&json!({"name": "escalated"})),
        bearer
            .delete(format!("http://{addr}/api/tokens/1"))
            .header("authorization", format!("Bearer {token}")),
    ] {
        let resp = req.send().await.unwrap();
        assert_eq!(
            resp.status().as_u16(),
            401,
            "token endpoints must be session-only"
        );
    }
}

#[tokio::test]
async fn expired_token_is_refused() {
    let (addr, db_url) = spawn_app().await;
    let c = admin(addr).await;

    // Valid bounds enforced.
    let resp = c
        .post(format!("http://{addr}/api/tokens"))
        .json(&json!({"name": "bad", "expires_in_days": 0}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);

    let created = mint(&c, addr, json!({"name": "short", "expires_in_days": 1})).await;
    let token = created["token"].as_str().unwrap().to_string();
    assert!(created["expires_at"].is_string());

    // Force-expire it directly in the DB, then the bearer must be refused.
    // (No sleeping: we edit expires_at to the past.)
    let pool = db::connect(&db_url).await.unwrap();
    sqlx::query("UPDATE api_tokens SET expires_at = '2000-01-01T00:00:00+00:00'")
        .execute(&pool)
        .await
        .unwrap();

    let bearer = client();
    let resp = bearer
        .get(format!("http://{addr}/api/projects"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401);
}

#[tokio::test]
async fn garbage_credentials_are_uniform_401() {
    let (addr, _db) = spawn_app().await;
    let anon = client();

    for header in [
        "Bearer cbx_00000000000000000000000000000000",
        "Bearer not-even-our-prefix",
        "Basic dXNlcjpwdw==",
        "Bearer",
        "cbx_raw-token-without-scheme",
    ] {
        let resp = anon
            .get(format!("http://{addr}/api/projects"))
            .header("authorization", header)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 401, "header {header:?}");
    }
}
