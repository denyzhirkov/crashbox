#![allow(clippy::unwrap_used, clippy::expect_used)]
//! CRASHBOX_HEARTBEAT_MONITORS: declarative, idempotent monitor provisioning at startup.
//! Own test binary — these tests mutate the process-global monitors env var.

use crashbox::config::Config;
use crashbox::db;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Build a Config with the given CRASHBOX_HEARTBEAT_MONITORS value against a fresh temp DB.
/// Returns the error message instead of a config when validation fails.
fn config_with_monitors(
    db_path: &std::path::Path,
    monitors: Option<&str>,
) -> Result<Config, String> {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    std::env::set_var(
        "CRASHBOX_DATABASE_URL",
        format!("sqlite://{}", db_path.display()),
    );
    std::env::set_var("CRASHBOX_PUBLIC_URL", "http://localhost");
    std::env::set_var("CRASHBOX_ADMIN_EMAIL", "a@b.c");
    std::env::set_var("CRASHBOX_ADMIN_PASSWORD", "x");
    std::env::set_var("CRASHBOX_PROJECT_NAME", "test");
    std::env::set_var("CRASHBOX_PROJECT_PUBLIC_KEY", "testpublickey");
    match monitors {
        Some(json) => std::env::set_var("CRASHBOX_HEARTBEAT_MONITORS", json),
        None => std::env::remove_var("CRASHBOX_HEARTBEAT_MONITORS"),
    }
    Config::from_env().map_err(|e| e.to_string())
}

async fn boot(cfg: &Config) -> sqlx::SqlitePool {
    let pool = db::connect(&cfg.database_url).await.expect("pool");
    db::migrate(&pool).await.expect("migrate");
    crashbox::bootstrap::run(&pool, cfg)
        .await
        .expect("bootstrap");
    pool
}

#[derive(sqlx::FromRow)]
struct Row {
    name: String,
    ping_key: String,
    period_seconds: i64,
    grace_seconds: i64,
    description: Option<String>,
    status: String,
}

async fn monitors(pool: &sqlx::SqlitePool) -> Vec<Row> {
    sqlx::query_as(
        "SELECT name, ping_key, period_seconds, grace_seconds, description, status \
         FROM heartbeat_monitors ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("rows")
}

#[tokio::test]
async fn provisions_declared_monitors_idempotently() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let db_path = tmp.path().join("crashbox.db");
    let json = r#"[
        {"name":"db-backup","ping_key":"backupkey12345678","period_seconds":86400,"grace_seconds":3600,"description":"nightly pg_dump"},
        {"name":"queue-worker","ping_key":"workerkey12345678","period_seconds":60}
    ]"#;

    let cfg = config_with_monitors(&db_path, Some(json)).expect("config");
    let pool = boot(&cfg).await;
    // Second startup with the same env must not duplicate or churn anything.
    crashbox::bootstrap::run(&pool, &cfg).await.expect("re-run");

    let rows = monitors(&pool).await;
    assert_eq!(rows.len(), 2);
    let backup = &rows[0];
    assert_eq!(backup.name, "db-backup");
    assert_eq!(backup.ping_key, "backupkey12345678");
    assert_eq!(backup.period_seconds, 86400);
    assert_eq!(backup.grace_seconds, 3600);
    assert_eq!(backup.description.as_deref(), Some("nightly pg_dump"));
    assert_eq!(backup.status, "pending");
    let worker = &rows[1];
    assert_eq!(worker.grace_seconds, 60, "default grace when not declared");
    assert_eq!(worker.description, None);
}

#[tokio::test]
async fn env_converges_declared_fields_but_preserves_user_edits() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let db_path = tmp.path().join("crashbox.db");
    let json_v1 = r#"[{"name":"cron","ping_key":"cronkey1234567890","period_seconds":300}]"#;

    let cfg = config_with_monitors(&db_path, Some(json_v1)).expect("config");
    let pool = boot(&cfg).await;

    // Simulate UI edits: user tweaks period + adds a description; a ping arrives.
    sqlx::query(
        "UPDATE heartbeat_monitors SET period_seconds = 999, description = 'my note', \
         status = 'up' WHERE name = 'cron'",
    )
    .execute(&pool)
    .await
    .expect("edit");

    // Restart with a rotated ping_key; period is declared (env wins), description is not
    // declared (user edit survives), status is never touched.
    let json_v2 = r#"[{"name":"cron","ping_key":"cronkeyROTATED890","period_seconds":300}]"#;
    let cfg2 = config_with_monitors(&db_path, Some(json_v2)).expect("config v2");
    crashbox::bootstrap::run(&pool, &cfg2)
        .await
        .expect("re-run");

    let rows = monitors(&pool).await;
    assert_eq!(rows.len(), 1);
    let m = &rows[0];
    assert_eq!(m.ping_key, "cronkeyROTATED890", "env wins for ping_key");
    assert_eq!(m.period_seconds, 300, "env wins for declared period");
    assert_eq!(
        m.description.as_deref(),
        Some("my note"),
        "undeclared description keeps the user edit"
    );
    assert_eq!(m.status, "up", "status is never altered by bootstrap");
}

#[tokio::test]
async fn malformed_specs_fail_loud_at_startup() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let db_path = tmp.path().join("crashbox.db");

    let err = config_with_monitors(&db_path, Some("not json")).expect_err("must reject");
    assert!(err.contains("CRASHBOX_HEARTBEAT_MONITORS"), "{err}");

    let short_key = r#"[{"name":"x","ping_key":"short","period_seconds":60}]"#;
    let err = config_with_monitors(&db_path, Some(short_key)).expect_err("must reject");
    assert!(err.contains("CRASHBOX_HEARTBEAT_MONITORS"), "{err}");

    let dup = r#"[
        {"name":"a","ping_key":"aaaaaaaaaaaaaaaa","period_seconds":60},
        {"name":"a","ping_key":"bbbbbbbbbbbbbbbb","period_seconds":60}
    ]"#;
    let err = config_with_monitors(&db_path, Some(dup)).expect_err("must reject");
    assert!(err.contains("CRASHBOX_HEARTBEAT_MONITORS"), "{err}");
}
