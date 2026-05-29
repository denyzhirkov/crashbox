//! Retention sweep correctness:
//! - Events older than `retention_days` are deleted...
//! - ...EXCEPT the most recent `max_events_per_issue` per issue, which always survive.
//! - Issue summary rows are never deleted by retention.
//! - Cascade FK removes related tags/breadcrumbs.

use chrono::{Duration as ChronoDuration, Utc};
use crashbox::config::Retention;
use crashbox::db;
use crashbox::jobs::cleanup;
use sqlx::SqlitePool;

async fn fresh_pool() -> SqlitePool {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("ret.db");
    Box::leak(Box::new(tmp));
    let url = format!("sqlite://{}", path.display());
    let pool = db::connect(&url).await.expect("pool");
    db::migrate(&pool).await.expect("migrate");
    pool
}

async fn insert_project(pool: &SqlitePool) -> i64 {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO projects (name, slug, public_key, created_at, updated_at) \
         VALUES ('p', 'p', 'pk', ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .expect("project");
    1
}

async fn insert_issue(pool: &SqlitePool, project_id: i64, fp: &str) -> i64 {
    let now = Utc::now().to_rfc3339();
    let r = sqlx::query(
        "INSERT INTO issues \
            (project_id, fingerprint, title, status, first_seen, last_seen, \
             event_count, created_at, updated_at) \
         VALUES (?, ?, 't', 'unresolved', ?, ?, 0, ?, ?)",
    )
    .bind(project_id)
    .bind(fp)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .expect("issue");
    r.last_insert_rowid()
}

/// Insert an event whose `received_at` is `days_ago` in the past.
async fn insert_event_aged(
    pool: &SqlitePool,
    project_id: i64,
    issue_id: Option<i64>,
    days_ago: i64,
) -> i64 {
    let when = (Utc::now() - ChronoDuration::days(days_ago)).to_rfc3339();
    let r = sqlx::query(
        "INSERT INTO events (event_id, project_id, issue_id, received_at, raw_json) \
         VALUES (NULL, ?, ?, ?, '{}')",
    )
    .bind(project_id)
    .bind(issue_id)
    .bind(&when)
    .execute(pool)
    .await
    .expect("event");
    let id = r.last_insert_rowid();
    // Add one tag per event so we can verify FK cascade.
    sqlx::query("INSERT INTO event_tags (event_id, key, value) VALUES (?, 'k', 'v')")
        .bind(id)
        .execute(pool)
        .await
        .expect("tag");
    id
}

async fn count(pool: &SqlitePool, table: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .expect("count")
}

#[tokio::test]
async fn keeps_last_n_per_issue_even_if_old() {
    let pool = fresh_pool().await;
    let project_id = insert_project(&pool).await;
    let issue_id = insert_issue(&pool, project_id, "fp1").await;

    // Five events, all older than the retention cutoff (30d default).
    // ages: 100d, 95d, 90d, 85d, 80d  — newest is 80d, oldest is 100d.
    for ago in [100, 95, 90, 85, 80] {
        insert_event_aged(&pool, project_id, Some(issue_id), ago).await;
    }
    assert_eq!(count(&pool, "events").await, 5);

    let retention = Retention {
        retention_days: 30,
        max_events_per_issue: 3,
        cleanup_interval_seconds: 0,
        auto_resolve_days: 0,
    };
    let deleted = cleanup::run_once(&pool, &retention).await.expect("sweep");
    assert_eq!(deleted, 2, "should delete 2 oldest, keep 3 newest");
    assert_eq!(count(&pool, "events").await, 3);

    // Issue summary is untouched.
    assert_eq!(count(&pool, "issues").await, 1);

    // Tags of deleted events also gone via cascade.
    assert_eq!(count(&pool, "event_tags").await, 3);
}

#[tokio::test]
async fn max_per_issue_is_a_floor_protects_active_issues() {
    // Contract: `max_events_per_issue` is a FLOOR, not a cap. It protects the most recent N
    // events per issue from age-based deletion. With only 3 events and max=10, all survive
    // even if one is past retention — otherwise an active issue would lose history every cycle.
    let pool = fresh_pool().await;
    let project_id = insert_project(&pool).await;
    let issue_id = insert_issue(&pool, project_id, "fp1").await;

    insert_event_aged(&pool, project_id, Some(issue_id), 1).await;
    insert_event_aged(&pool, project_id, Some(issue_id), 5).await;
    insert_event_aged(&pool, project_id, Some(issue_id), 90).await;

    let retention = Retention {
        retention_days: 30,
        max_events_per_issue: 10,
        cleanup_interval_seconds: 0,
        auto_resolve_days: 0,
    };
    let deleted = cleanup::run_once(&pool, &retention).await.expect("sweep");
    assert_eq!(deleted, 0);
    assert_eq!(count(&pool, "events").await, 3);

    // Lower the floor below the number of events — now the old one is no longer protected.
    let tighter = Retention {
        retention_days: 30,
        max_events_per_issue: 2,
        cleanup_interval_seconds: 0,
        auto_resolve_days: 0,
    };
    let deleted2 = cleanup::run_once(&pool, &tighter).await.expect("sweep");
    assert_eq!(deleted2, 1);
    assert_eq!(count(&pool, "events").await, 2);
}

#[tokio::test]
async fn orphan_events_without_issue_age_out_purely_by_age() {
    let pool = fresh_pool().await;
    let project_id = insert_project(&pool).await;

    // Two orphans, one old one fresh.
    insert_event_aged(&pool, project_id, None, 60).await;
    insert_event_aged(&pool, project_id, None, 1).await;

    let retention = Retention {
        retention_days: 30,
        max_events_per_issue: 100,
        cleanup_interval_seconds: 0,
        auto_resolve_days: 0,
    };
    let deleted = cleanup::run_once(&pool, &retention).await.expect("sweep");
    assert_eq!(deleted, 1);
    assert_eq!(count(&pool, "events").await, 1);
}

#[tokio::test]
async fn auto_resolves_stale_unresolved_issues() {
    let pool = fresh_pool().await;
    let project_id = insert_project(&pool).await;

    // Issue A: last_seen 30 days ago, status=unresolved → should auto-resolve.
    let stale_id = insert_issue(&pool, project_id, "stale").await;
    let long_ago = (Utc::now() - ChronoDuration::days(30)).to_rfc3339();
    sqlx::query("UPDATE issues SET last_seen = ? WHERE id = ?")
        .bind(&long_ago)
        .bind(stale_id)
        .execute(&pool)
        .await
        .expect("update");

    // Issue B: last_seen 1 day ago, status=unresolved → must stay unresolved.
    let fresh_id = insert_issue(&pool, project_id, "fresh").await;
    let yesterday = (Utc::now() - ChronoDuration::days(1)).to_rfc3339();
    sqlx::query("UPDATE issues SET last_seen = ? WHERE id = ?")
        .bind(&yesterday)
        .bind(fresh_id)
        .execute(&pool)
        .await
        .expect("update");

    let retention = Retention {
        retention_days: 365, // irrelevant — we only care about auto-resolve here
        max_events_per_issue: 100,
        cleanup_interval_seconds: 0,
        auto_resolve_days: 14,
    };
    cleanup::run_once(&pool, &retention).await.expect("sweep");

    let stale_status: String =
        sqlx::query_scalar("SELECT status FROM issues WHERE id = ?")
            .bind(stale_id)
            .fetch_one(&pool)
            .await
            .expect("stale status");
    assert_eq!(stale_status, "resolved");

    let fresh_status: String =
        sqlx::query_scalar("SELECT status FROM issues WHERE id = ?")
            .bind(fresh_id)
            .fetch_one(&pool)
            .await
            .expect("fresh status");
    assert_eq!(fresh_status, "unresolved");
}

#[tokio::test]
async fn auto_resolve_disabled_when_days_is_zero() {
    let pool = fresh_pool().await;
    let project_id = insert_project(&pool).await;
    let issue_id = insert_issue(&pool, project_id, "old").await;
    let long_ago = (Utc::now() - ChronoDuration::days(99)).to_rfc3339();
    sqlx::query("UPDATE issues SET last_seen = ? WHERE id = ?")
        .bind(&long_ago)
        .bind(issue_id)
        .execute(&pool)
        .await
        .expect("update");

    let retention = Retention {
        retention_days: 365,
        max_events_per_issue: 100,
        cleanup_interval_seconds: 0,
        auto_resolve_days: 0, // off
    };
    cleanup::run_once(&pool, &retention).await.expect("sweep");

    let status: String = sqlx::query_scalar("SELECT status FROM issues WHERE id = ?")
        .bind(issue_id)
        .fetch_one(&pool)
        .await
        .expect("status");
    assert_eq!(status, "unresolved");
}

#[tokio::test]
async fn fresh_events_inside_retention_never_deleted() {
    let pool = fresh_pool().await;
    let project_id = insert_project(&pool).await;
    let issue_id = insert_issue(&pool, project_id, "fp1").await;

    for ago in [0, 1, 2, 3, 4] {
        insert_event_aged(&pool, project_id, Some(issue_id), ago).await;
    }

    let retention = Retention {
        retention_days: 30,
        max_events_per_issue: 1,
        cleanup_interval_seconds: 0,
        auto_resolve_days: 0,
    };
    let deleted = cleanup::run_once(&pool, &retention).await.expect("sweep");
    assert_eq!(deleted, 0, "events within retention window must not be deleted");
    assert_eq!(count(&pool, "events").await, 5);
}
