//! End-to-end tests for the admin/auth/projects/issues HTTP API.

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
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    crashbox::bootstrap::run(&pool, &cfg).await.expect("bootstrap");

    let state = AppState::new(cfg, pool, crashbox::metrics_layer::MetricsHandle::dummy());
    let app = http::routes::build(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("listen");
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

async fn login(c: &reqwest::Client, addr: SocketAddr, email: &str, password: &str) -> Value {
    let resp = c
        .post(format!("http://{addr}/api/auth/login"))
        .json(&json!({"email": email, "password": password}))
        .send()
        .await
        .expect("login");
    assert_eq!(resp.status().as_u16(), 200, "login should succeed");
    resp.json().await.expect("json")
}

#[tokio::test]
async fn auth_login_me_logout_flow() {
    let addr = spawn_app().await;
    let c = client();

    // Anonymous /me → 401
    let anon = c.get(format!("http://{addr}/api/auth/me")).send().await.unwrap();
    assert_eq!(anon.status().as_u16(), 401);

    // Wrong password → 401
    let wrong = c
        .post(format!("http://{addr}/api/auth/login"))
        .json(&json!({"email": "admin@example.com", "password": "nope"}))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status().as_u16(), 401);

    // Login
    let body = login(&c, addr, "admin@example.com", "hunter2").await;
    assert_eq!(body["user"]["email"], "admin@example.com");
    assert_eq!(body["user"]["is_admin"], true);

    // /me succeeds with cookie
    let me = c.get(format!("http://{addr}/api/auth/me")).send().await.unwrap();
    assert_eq!(me.status().as_u16(), 200);
    let me_json: Value = me.json().await.unwrap();
    assert_eq!(me_json["email"], "admin@example.com");

    // Logout
    let lo = c.post(format!("http://{addr}/api/auth/logout")).send().await.unwrap();
    assert_eq!(lo.status().as_u16(), 200);

    // After logout, /me → 401 again
    let me2 = c.get(format!("http://{addr}/api/auth/me")).send().await.unwrap();
    assert_eq!(me2.status().as_u16(), 401);
}

#[tokio::test]
async fn projects_list_dsn_and_create() {
    let addr = spawn_app().await;
    let c = client();
    login(&c, addr, "admin@example.com", "hunter2").await;

    // Bootstrap created one project.
    let list: Vec<Value> = c
        .get(format!("http://{addr}/api/projects"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"], "main");

    // DSN is exposed and formatted correctly.
    let dsn: Value = c
        .get(format!("http://{addr}/api/projects/1/dsn"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(dsn["dsn"], "http://pkmain@localhost/1");

    // Create a second project.
    let created: Value = c
        .post(format!("http://{addr}/api/projects"))
        .json(&json!({"name": "Other Service", "platform": "python"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["name"], "Other Service");
    assert_eq!(created["slug"], "other-service");
    assert_eq!(created["platform"], "python");

    // Rotate-key returns a fresh DSN and invalidates the old one for ingestion.
    let rotated: Value = c
        .post(format!("http://{addr}/api/projects/1/rotate-key"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(rotated["public_key"].as_str().unwrap() != "pkmain");
    assert!(rotated["dsn"].as_str().unwrap().contains("@localhost/1"));
}

#[tokio::test]
async fn issues_list_includes_24h_sparkline_buckets() {
    let addr = spawn_app().await;
    let c = client();
    login(&c, addr, "admin@example.com", "hunter2").await;

    let payload = serde_json::json!({
        "platform": "node",
        "exception": {"values": [{"type": "T", "value": "x"}]}
    })
    .to_string();
    let env_body = format!(
        "{{}}\n{{\"type\":\"event\",\"length\":{}}}\n{}\n",
        payload.len(),
        payload
    );
    let ingest = reqwest::Client::new();
    for _ in 0..3 {
        ingest
            .post(format!("http://{addr}/api/1/envelope/"))
            .header("x-sentry-auth", "Sentry sentry_key=pkmain")
            .body(env_body.clone())
            .send()
            .await
            .unwrap();
    }

    let issues: Vec<serde_json::Value> = c
        .get(format!("http://{addr}/api/projects/1/issues"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(issues.len(), 1);
    let buckets = issues[0]["last_24h_buckets"].as_array().expect("sparkline array");
    assert_eq!(buckets.len(), 24, "must be exactly 24 hour buckets");
    let total: i64 = buckets.iter().filter_map(|v| v.as_i64()).sum();
    assert_eq!(total, 3);
    assert_eq!(buckets[23].as_i64().unwrap(), 3, "current hour holds all 3 events");
}

#[tokio::test]
async fn issues_filter_and_resolve_flow() {
    let addr = spawn_app().await;
    let c = client();
    login(&c, addr, "admin@example.com", "hunter2").await;

    // Push two grouped events + one different.
    let ingest = reqwest::Client::new();
    for value in ["row 11111111 missing", "row 22222222 missing"] {
        let payload = json!({
            "platform": "node",
            "exception": {"values": [{
                "type": "TypeError",
                "value": value,
                "stacktrace": {"frames": [
                    {"function": "load", "filename": "db.js", "lineno": 7, "in_app": true}
                ]}
            }]}
        })
        .to_string();
        let env_body = format!(
            "{{}}\n{{\"type\":\"event\",\"length\":{}}}\n{}\n",
            payload.len(),
            payload
        );
        let r = ingest
            .post(format!("http://{addr}/api/1/envelope/"))
            .header("x-sentry-auth", "Sentry sentry_key=pkmain")
            .body(env_body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 200);
    }
    let payload = json!({
        "platform": "node",
        "exception": {"values": [{"type": "RangeError", "value": "oob"}]}
    })
    .to_string();
    let env_body = format!(
        "{{}}\n{{\"type\":\"event\",\"length\":{}}}\n{}\n",
        payload.len(),
        payload
    );
    ingest
        .post(format!("http://{addr}/api/1/envelope/"))
        .header("x-sentry-auth", "Sentry sentry_key=pkmain")
        .body(env_body)
        .send()
        .await
        .unwrap();

    // List issues → 2 unresolved, one with event_count=2.
    let issues: Vec<Value> = c
        .get(format!("http://{addr}/api/projects/1/issues"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(issues.len(), 2);
    let tt = issues
        .iter()
        .find(|i| i["title"].as_str().unwrap().starts_with("TypeError"))
        .expect("typeerror issue");
    assert_eq!(tt["event_count"], 2);

    // Text query filter narrows down.
    let only_range: Vec<Value> = c
        .get(format!("http://{addr}/api/projects/1/issues?query=RangeError"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(only_range.len(), 1);

    // Resolve the TypeError issue.
    let issue_id = tt["id"].as_i64().unwrap();
    let resolved: Value = c
        .patch(format!("http://{addr}/api/issues/{issue_id}"))
        .json(&json!({"status": "resolved"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resolved["status"], "resolved");

    // status=unresolved (default) now shows only RangeError.
    let unresolved_list: Vec<Value> = c
        .get(format!(
            "http://{addr}/api/projects/1/issues?status=unresolved"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(unresolved_list.len(), 1);
    assert!(unresolved_list[0]["title"]
        .as_str()
        .unwrap()
        .contains("RangeError"));

    // Event detail includes parsed raw payload.
    let events: Vec<Value> = c
        .get(format!("http://{addr}/api/issues/{issue_id}/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let event_row_id = events[0]["id"].as_i64().unwrap();
    let event: Value = c
        .get(format!("http://{addr}/api/events/{event_row_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(event["data"]["exception"]["values"][0]["type"], "TypeError");
}
