use chrono::Utc;
use sqlx::{QueryBuilder, Sqlite, SqliteConnection, SqlitePool};

#[derive(Debug, sqlx::FromRow, serde::Serialize, Clone)]
pub struct Issue {
    pub id: i64,
    pub project_id: i64,
    pub fingerprint: String,
    pub title: String,
    pub status: String,
    pub level: Option<String>,
    pub platform: Option<String>,
    pub first_seen: String,
    pub last_seen: String,
    pub event_count: i64,
    pub last_event_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    #[sqlx(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spike_alerted_at: Option<String>,
    #[sqlx(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<String>,
}

/// HTTP-layer representation of an issue with sparkline data attached. We keep the DB type
/// (`Issue`) decode-clean and only add computed fields at the serialization boundary via
/// `serde(flatten)`.
#[derive(Debug, serde::Serialize, Clone)]
pub struct IssueWithSparkline {
    #[serde(flatten)]
    pub issue: Issue,
    /// 24-element array: counts of events per hour over the last 24h, ordered oldest→newest
    /// (index 0 = 23-24h ago, index 23 = current hour).
    pub last_24h_buckets: Vec<i64>,
}

#[derive(Debug, Default, Clone)]
pub struct IssueFilters {
    pub status: Option<String>,
    pub level: Option<String>,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub query: Option<String>,
    /// Each entry is a `(key, value)` pair from `?tag=key=value`. Multiple tags are ANDed:
    /// an issue must have *some* event matching every requested (key, value).
    pub tags: Vec<(String, String)>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn find_by_id(pool: &SqlitePool, id: i64) -> sqlx::Result<Option<Issue>> {
    sqlx::query_as::<_, Issue>("SELECT * FROM issues WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn list(
    pool: &SqlitePool,
    project_id: i64,
    f: &IssueFilters,
) -> sqlx::Result<Vec<Issue>> {
    let now_iso = Utc::now().to_rfc3339();
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT DISTINCT issues.* FROM issues ",
    );
    let needs_event_join = f.environment.is_some() || f.release.is_some();
    if needs_event_join {
        qb.push("JOIN events ON events.issue_id = issues.id ");
    }
    qb.push("WHERE issues.project_id = ");
    qb.push_bind(project_id);

    // Status semantics:
    //   unresolved (default): status='unresolved' AND NOT currently snoozed
    //   resolved:             status='resolved'
    //   snoozed:              currently snoozed (forever OR future timestamp)
    //   all:                  no status / snooze filter at all
    match f.status.as_deref() {
        Some("all") => {}
        Some("snoozed") => {
            qb.push(" AND (issues.snoozed_until = 'forever' OR issues.snoozed_until > ");
            qb.push_bind(now_iso.clone());
            qb.push(")");
        }
        Some(s) => {
            qb.push(" AND issues.status = ");
            qb.push_bind(s.to_string());
            // Hide currently-snoozed when looking at "unresolved" view.
            if s == "unresolved" {
                qb.push(" AND (issues.snoozed_until IS NULL OR (issues.snoozed_until != 'forever' AND issues.snoozed_until <= ");
                qb.push_bind(now_iso.clone());
                qb.push("))");
            }
        }
        None => {
            qb.push(" AND issues.status = 'unresolved'");
            qb.push(" AND (issues.snoozed_until IS NULL OR (issues.snoozed_until != 'forever' AND issues.snoozed_until <= ");
            qb.push_bind(now_iso.clone());
            qb.push("))");
        }
    }
    if let Some(level) = &f.level {
        qb.push(" AND issues.level = ");
        qb.push_bind(level.clone());
    }
    if let Some(env) = &f.environment {
        qb.push(" AND events.environment = ");
        qb.push_bind(env.clone());
    }
    if let Some(rel) = &f.release {
        qb.push(" AND events.release = ");
        qb.push_bind(rel.clone());
    }
    if let Some(q) = &f.query {
        // Case-insensitive substring match on the issue title.
        qb.push(" AND issues.title LIKE ");
        qb.push_bind(format!("%{q}%"));
    }
    // Tag filters: AND-of-EXISTS. Each tag must match some event of the issue. The subquery
    // hits idx_event_tags_key_value, then joins events on event_id for the issue_id link.
    for (k, v) in &f.tags {
        qb.push(
            " AND EXISTS (\
                SELECT 1 FROM event_tags et \
                JOIN events e ON e.id = et.event_id \
                WHERE e.issue_id = issues.id AND et.key = ",
        );
        qb.push_bind(k.clone());
        qb.push(" AND et.value = ");
        qb.push_bind(v.clone());
        qb.push(")");
    }
    qb.push(" ORDER BY issues.last_seen DESC LIMIT ");
    qb.push_bind(f.limit.clamp(1, 500));
    qb.push(" OFFSET ");
    qb.push_bind(f.offset.max(0));

    qb.build_query_as::<Issue>().fetch_all(pool).await
}

/// Wrap a batch of `Issue` rows with a 24h sparkline each. Single SQL query for the whole
/// batch — O(events in last 24h for these issues), with the existing
/// `idx_events_issue_received` index doing the heavy lifting per issue.
pub async fn with_sparklines(
    pool: &SqlitePool,
    issues: Vec<Issue>,
) -> sqlx::Result<Vec<IssueWithSparkline>> {
    if issues.is_empty() {
        return Ok(Vec::new());
    }
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT issue_id, \
                CAST((julianday('now') - julianday(received_at)) * 24 AS INTEGER) AS hour_ago, \
                COUNT(*) AS cnt \
         FROM events \
         WHERE received_at >= datetime('now', '-24 hours') AND issue_id IN (",
    );
    {
        let mut sep = qb.separated(", ");
        for i in &issues {
            sep.push_bind(i.id);
        }
    }
    qb.push(") GROUP BY issue_id, hour_ago");

    let rows: Vec<(i64, i64, i64)> = qb.build_query_as().fetch_all(pool).await?;

    let mut by_id: std::collections::HashMap<i64, [i64; 24]> = std::collections::HashMap::new();
    for (id, hour_ago, cnt) in rows {
        if !(0..=23).contains(&hour_ago) {
            continue;
        }
        let buckets = by_id.entry(id).or_insert([0; 24]);
        // hour_ago=0 means current hour → rightmost bar (index 23).
        let idx = 23 - hour_ago as usize;
        buckets[idx] += cnt;
    }
    Ok(issues
        .into_iter()
        .map(|issue| {
            let arr = by_id.remove(&issue.id).unwrap_or([0; 24]);
            IssueWithSparkline {
                issue,
                last_24h_buckets: arr.to_vec(),
            }
        })
        .collect())
}

pub async fn set_status(pool: &SqlitePool, id: i64, status: &str) -> sqlx::Result<u64> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query("UPDATE issues SET status = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// `snoozed_until` accepts:
/// - `None` to clear the snooze (i.e. wake the issue)
/// - `Some("forever")` to snooze until the next ingested event
/// - `Some("<rfc3339>")` for a time-bound snooze
pub async fn set_snooze(
    pool: &SqlitePool,
    id: i64,
    snoozed_until: Option<&str>,
) -> sqlx::Result<u64> {
    let now = Utc::now().to_rfc3339();
    let result =
        sqlx::query("UPDATE issues SET snoozed_until = ?, updated_at = ? WHERE id = ?")
            .bind(snoozed_until)
            .bind(&now)
            .bind(id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected())
}

/// What happened during [`upsert`]. The notify hub keys notifications off this:
/// - `Created` → fire `NewIssue` alert
/// - `Reopened` → fire `Reopened` alert (we auto-flipped status back to `unresolved`)
/// - `Existing` → no notification (handled by spike detection separately)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    Created,
    Reopened,
    Existing,
}

/// Find an existing issue's id for (project_id, fingerprint), or insert a new one. Always returns
/// the issue id and whether the issue was newly created, reopened from `resolved`, or already
/// existed and was unresolved. The caller bumps `event_count` / `last_seen` via
/// [`bump_after_event`] once the event row exists.
pub async fn upsert(
    conn: &mut SqliteConnection,
    project_id: i64,
    fingerprint: &str,
    title: &str,
    level: Option<&str>,
    platform: Option<&str>,
    timestamp_iso: &str,
) -> sqlx::Result<(i64, UpsertOutcome)> {
    let existing: Option<(i64, String, Option<String>)> = sqlx::query_as(
        "SELECT id, status, snoozed_until FROM issues \
         WHERE project_id = ? AND fingerprint = ?",
    )
    .bind(project_id)
    .bind(fingerprint)
    .fetch_optional(&mut *conn)
    .await?;

    if let Some((id, status, snoozed_until)) = existing {
        let now = Utc::now().to_rfc3339();
        // Auto-wake forever-snooze: the user's intent was "shut up until the next crash".
        // Time-bounded snoozes stay in place; the list query will surface them naturally
        // once their timestamp passes.
        if snoozed_until.as_deref() == Some("forever") {
            sqlx::query("UPDATE issues SET snoozed_until = NULL, updated_at = ? WHERE id = ?")
                .bind(&now)
                .bind(id)
                .execute(&mut *conn)
                .await?;
        }

        if status == "resolved" {
            sqlx::query(
                "UPDATE issues SET status = 'unresolved', updated_at = ? WHERE id = ?",
            )
            .bind(&now)
            .bind(id)
            .execute(&mut *conn)
            .await?;
            return Ok((id, UpsertOutcome::Reopened));
        }
        return Ok((id, UpsertOutcome::Existing));
    }

    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO issues \
            (project_id, fingerprint, title, status, level, platform, \
             first_seen, last_seen, event_count, created_at, updated_at) \
         VALUES (?, ?, ?, 'unresolved', ?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(project_id)
    .bind(fingerprint)
    .bind(title)
    .bind(level)
    .bind(platform)
    .bind(timestamp_iso)
    .bind(timestamp_iso)
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;
    Ok((row.last_insert_rowid(), UpsertOutcome::Created))
}

/// After the event row is inserted, update the issue's counters and pointers.
pub async fn bump_after_event(
    conn: &mut SqliteConnection,
    issue_id: i64,
    event_id_row: i64,
    timestamp_iso: &str,
) -> sqlx::Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE issues SET \
            event_count = event_count + 1, \
            last_seen = ?, \
            last_event_id = ?, \
            updated_at = ? \
         WHERE id = ?",
    )
    .bind(timestamp_iso)
    .bind(event_id_row)
    .bind(&now)
    .bind(issue_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
