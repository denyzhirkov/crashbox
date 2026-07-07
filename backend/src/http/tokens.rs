//! API token management. Session-authed ONLY (`SessionUser`): a bearer token must never be
//! able to mint or revoke tokens. The plaintext token appears exactly once — in the 201
//! response of `create` — and is never logged (prefix only).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::db::tokens::{self, ApiToken};
use crate::http::error::{AppError, AppResult};
use crate::security::sessions::SessionUser;
use crate::security::tokens as token_gen;

const NAME_MAX_CHARS: usize = 200;
const EXPIRES_MAX_DAYS: i64 = 3650;

#[derive(Debug, Deserialize)]
pub struct CreateToken {
    pub name: String,
    /// Omitted or null = the token never expires (deliberate default for a single-admin box).
    #[serde(default)]
    pub expires_in_days: Option<i64>,
    /// "full" (default) or "read". Read tokens authenticate GET/HEAD only.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreatedToken {
    #[serde(flatten)]
    pub meta: ApiToken,
    /// The plaintext — shown once, never retrievable again.
    pub token: String,
}

/// GET /api/tokens (session + admin)
pub async fn list(
    SessionUser(user): SessionUser,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<ApiToken>>> {
    if !user.is_admin {
        return Err(AppError::Forbidden);
    }
    Ok(Json(tokens::list_for_user(&state.db, user.id).await?))
}

/// POST /api/tokens (session + admin)
pub async fn create(
    SessionUser(user): SessionUser,
    State(state): State<AppState>,
    Json(body): Json<CreateToken>,
) -> AppResult<impl IntoResponse> {
    if !user.is_admin {
        return Err(AppError::Forbidden);
    }
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    if name.chars().count() > NAME_MAX_CHARS {
        return Err(AppError::BadRequest(format!(
            "name must be at most {NAME_MAX_CHARS} characters"
        )));
    }
    let expires_at = match body.expires_in_days {
        None => None,
        Some(days) if (1..=EXPIRES_MAX_DAYS).contains(&days) => {
            Some((Utc::now() + Duration::days(days)).to_rfc3339())
        }
        Some(_) => {
            return Err(AppError::BadRequest(format!(
                "expires_in_days must be between 1 and {EXPIRES_MAX_DAYS}"
            )))
        }
    };

    let scope = match body.scope.as_deref() {
        None => tokens::SCOPE_FULL,
        Some(s) if s == tokens::SCOPE_FULL || s == tokens::SCOPE_READ => s,
        Some(_) => {
            return Err(AppError::BadRequest(
                "scope must be 'full' or 'read'".into(),
            ))
        }
    };

    let token = token_gen::generate();
    let prefix = token_gen::display_prefix(&token);
    let id = tokens::insert(
        &state.db,
        user.id,
        name,
        &token_gen::hash(&token),
        &prefix,
        scope,
        expires_at.as_deref(),
    )
    .await?;
    let meta = tokens::find_by_id(&state.db, id)
        .await?
        .ok_or(AppError::NotFound)?;
    tracing::info!(token_id = id, prefix = %prefix, name = %name, "api token created");
    Ok((StatusCode::CREATED, Json(CreatedToken { meta, token })))
}

/// DELETE /api/tokens/:id (session + admin) — instant revocation.
pub async fn remove(
    SessionUser(user): SessionUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    if !user.is_admin {
        return Err(AppError::Forbidden);
    }
    let affected = tokens::delete(&state.db, id, user.id).await?;
    if affected == 0 {
        return Err(AppError::NotFound);
    }
    tracing::info!(token_id = id, "api token revoked");
    Ok(StatusCode::NO_CONTENT)
}
