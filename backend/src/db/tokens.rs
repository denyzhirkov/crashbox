//! API token repository. The plaintext token never reaches this module — callers pass the
//! SHA-256 (see `security::tokens`). `token_hash` never leaves it either.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::security::sessions::AuthUser;

/// How stale `last_used_at` may get before an authenticated request refreshes it. Keeps the
/// hot path from issuing a write per request.
const LAST_USED_REFRESH_SECONDS: i64 = 300;

pub const SCOPE_FULL: &str = "full";
pub const SCOPE_READ: &str = "read";

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ApiToken {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub token_prefix: String,
    pub scope: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
}

const COLUMNS: &str =
    "id, user_id, name, token_prefix, scope, created_at, expires_at, last_used_at";

pub async fn list_for_user(pool: &SqlitePool, user_id: i64) -> sqlx::Result<Vec<ApiToken>> {
    sqlx::query_as::<_, ApiToken>(&format!(
        "SELECT {COLUMNS} FROM api_tokens WHERE user_id = ? ORDER BY id ASC"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn insert(
    pool: &SqlitePool,
    user_id: i64,
    name: &str,
    token_hash: &str,
    token_prefix: &str,
    scope: &str,
    expires_at: Option<&str>,
) -> sqlx::Result<i64> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO api_tokens \
            (user_id, name, token_hash, token_prefix, scope, created_at, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(name)
    .bind(token_hash)
    .bind(token_prefix)
    .bind(scope)
    .bind(&now)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(row.last_insert_rowid())
}

pub async fn find_by_id(pool: &SqlitePool, id: i64) -> sqlx::Result<Option<ApiToken>> {
    sqlx::query_as::<_, ApiToken>(&format!("SELECT {COLUMNS} FROM api_tokens WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn delete(pool: &SqlitePool, id: i64, user_id: i64) -> sqlx::Result<u64> {
    let result = sqlx::query("DELETE FROM api_tokens WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Resolve a token hash to its user, mirroring `sessions::lookup`: `None` for unknown or
/// expired (uniform 401 at the edge). Refreshes `last_used_at` lazily — at most once per
/// `LAST_USED_REFRESH_SECONDS` — as a spawned fire-and-forget write off the hot path.
pub async fn lookup_by_hash(pool: &SqlitePool, token_hash: &str) -> sqlx::Result<Option<AuthUser>> {
    #[derive(sqlx::FromRow)]
    struct TokenAuthRow {
        token_id: i64,
        user_id: i64,
        email: String,
        is_admin: bool,
        scope: String,
        expires_at: Option<String>,
        last_used_at: Option<String>,
    }

    let row: Option<TokenAuthRow> = sqlx::query_as(
        "SELECT api_tokens.id AS token_id, users.id AS user_id, users.email, users.is_admin, \
                api_tokens.scope, api_tokens.expires_at, api_tokens.last_used_at \
         FROM api_tokens \
         JOIN users ON users.id = api_tokens.user_id \
         WHERE api_tokens.token_hash = ?",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;

    let Some(TokenAuthRow {
        token_id,
        user_id,
        email,
        is_admin,
        scope,
        expires_at,
        last_used_at,
    }) = row
    else {
        return Ok(None);
    };

    let now = Utc::now();
    if let Some(raw) = expires_at {
        let expired = DateTime::parse_from_rfc3339(&raw)
            .map(|t| t.with_timezone(&Utc) < now)
            .unwrap_or(true);
        if expired {
            return Ok(None);
        }
    }

    let stale = match last_used_at.as_deref().map(DateTime::parse_from_rfc3339) {
        Some(Ok(t)) => (now - t.with_timezone(&Utc)).num_seconds() >= LAST_USED_REFRESH_SECONDS,
        _ => true,
    };
    if stale {
        let pool = pool.clone();
        let now_iso = now.to_rfc3339();
        tokio::spawn(async move {
            let _ = sqlx::query("UPDATE api_tokens SET last_used_at = ? WHERE id = ?")
                .bind(&now_iso)
                .bind(token_id)
                .execute(&pool)
                .await;
        });
    }

    Ok(Some(AuthUser {
        id: user_id,
        email,
        is_admin,
        read_only: scope == SCOPE_READ,
    }))
}
