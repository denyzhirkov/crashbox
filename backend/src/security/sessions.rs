//! Server-side session table backed by SQLite.
//!
//! - Session id is a ULID. Cookie value is the id directly (the secret is the id itself, drawn
//!   from a cryptographic PRNG via ULID's randomness component).
//! - Expiration: 30 days; refreshed on every successful auth check.
//! - The `AuthUser` extractor reads the cookie, looks up the session, and returns the user — or
//!   `AppError::Unauthorized` if missing/expired.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use ulid::Ulid;

use crate::app_state::AppState;
use crate::http::error::AppError;

pub const COOKIE_NAME: &str = "crashbox_session";
const SESSION_TTL: Duration = Duration::from_secs(60 * 60 * 24 * 30);

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i64,
    pub email: String,
    pub is_admin: bool,
}

pub async fn create(pool: &SqlitePool, user_id: i64) -> sqlx::Result<(String, DateTime<Utc>)> {
    let id = Ulid::new().to_string();
    let now = Utc::now();
    let expires = now + chrono::Duration::from_std(SESSION_TTL).unwrap_or_default();
    sqlx::query("INSERT INTO sessions (id, user_id, expires_at, created_at) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(user_id)
        .bind(expires.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;
    Ok((id, expires))
}

pub async fn delete(pool: &SqlitePool, session_id: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn lookup(pool: &SqlitePool, session_id: &str) -> sqlx::Result<Option<AuthUser>> {
    let row: Option<(i64, String, bool, String)> = sqlx::query_as(
        "SELECT users.id, users.email, users.is_admin, sessions.expires_at \
         FROM sessions \
         JOIN users ON users.id = sessions.user_id \
         WHERE sessions.id = ?",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;

    let Some((id, email, is_admin, expires_at)) = row else {
        return Ok(None);
    };

    let expired = DateTime::parse_from_rfc3339(&expires_at)
        .map(|t| t.with_timezone(&Utc) < Utc::now())
        .unwrap_or(true);
    if expired {
        // Best-effort cleanup; ignore errors here.
        let _ = sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id)
            .execute(pool)
            .await;
        return Ok(None);
    }

    Ok(Some(AuthUser {
        id,
        email,
        is_admin,
    }))
}

pub fn build_set_cookie(session_id: &str, secure: bool, max_age: Duration) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={age}{secure_attr}",
        name = COOKIE_NAME,
        value = session_id,
        age = max_age.as_secs(),
    )
}

pub fn build_clear_cookie(secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure_attr}")
}

fn cookie_from_header(header: &str, name: &str) -> Option<String> {
    for raw in header.split(';') {
        let pair = raw.trim();
        if let Some(val) = pair.strip_prefix(&format!("{name}=")) {
            return Some(val.to_string());
        }
    }
    None
}

fn session_id_from_parts(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| cookie_from_header(s, COOKIE_NAME))
}

async fn auth_via_session(state: &AppState, parts: &Parts) -> Result<Option<AuthUser>, AppError> {
    let Some(raw) = session_id_from_parts(parts) else {
        return Ok(None);
    };
    lookup(&state.db, &raw)
        .await
        .map_err(|e| AppError::Internal(anyhow::Error::new(e)))
}

async fn auth_via_bearer(state: &AppState, parts: &Parts) -> Result<Option<AuthUser>, AppError> {
    let Some(token) = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(crate::security::tokens::from_authorization_header)
    else {
        return Ok(None);
    };
    let hash = crate::security::tokens::hash(token);
    crate::db::tokens::lookup_by_hash(&state.db, &hash)
        .await
        .map_err(|e| AppError::Internal(anyhow::Error::new(e)))
}

/// Accepts either credential: session cookie first, then `Authorization: Bearer cbx_…`.
/// Every admin endpoint that takes `AuthUser` is automatically usable with API tokens.
#[axum::async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(user) = auth_via_session(state, parts).await? {
            return Ok(user);
        }
        if let Some(user) = auth_via_bearer(state, parts).await? {
            return Ok(user);
        }
        Err(AppError::Unauthorized)
    }
}

/// Session-cookie-only variant. Used by the token-management endpoints so an API token can
/// never mint or revoke tokens — a leaked token must not be able to grant itself successors.
#[derive(Debug, Clone)]
pub struct SessionUser(pub AuthUser);

#[axum::async_trait]
impl FromRequestParts<AppState> for SessionUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match auth_via_session(state, parts).await? {
            Some(user) => Ok(Self(user)),
            None => Err(AppError::Unauthorized),
        }
    }
}

// Keep an Arc<AppState> wrapper-compatible extractor not needed — Axum extracts AppState by Clone
// (it's already an Arc inside). This module just leaves the From impl on AppState.

#[allow(dead_code)]
pub fn session_ttl() -> Duration {
    SESSION_TTL
}

#[allow(dead_code)]
pub fn _arc_marker(_: Arc<()>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cookie_value() {
        let h = "foo=bar; crashbox_session=ABC123; baz=qux";
        assert_eq!(
            cookie_from_header(h, COOKIE_NAME).as_deref(),
            Some("ABC123")
        );
    }

    #[test]
    fn cookie_attrs_set_and_clear() {
        let set = build_set_cookie("xyz", true, Duration::from_secs(60));
        assert!(set.contains("crashbox_session=xyz"));
        assert!(set.contains("HttpOnly"));
        assert!(set.contains("Secure"));
        assert!(set.contains("Max-Age=60"));

        let clear = build_clear_cookie(false);
        assert!(clear.contains("Max-Age=0"));
        assert!(!clear.contains("Secure"));
    }
}
