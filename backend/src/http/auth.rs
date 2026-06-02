use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::app_state::AppState;
use crate::db::users;
use crate::http::error::{AppError, AppResult};
use crate::security::sessions::{AuthUser, COOKIE_NAME};
use crate::security::{password, sessions};

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: i64,
    pub email: String,
    pub is_admin: bool,
    pub name: Option<String>,
    /// Global feature flag surfaced at auth-bootstrap so the UI can hide the Live Logs section
    /// when the server has it disabled. Not per-user.
    pub live_logs_enabled: bool,
}

/// POST /api/auth/login
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> AppResult<Response> {
    let user = users::find_by_email(&state.db, &req.email).await?;
    let Some(user) = user else {
        // Unify with wrong-password to avoid email enumeration.
        return Err(AppError::Unauthorized);
    };
    let ok = password::verify_password(&req.password, &user.password_hash)
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    if !ok {
        return Err(AppError::Unauthorized);
    }

    let (sid, _expires) = sessions::create(&state.db, user.id).await?;
    let cookie = sessions::build_set_cookie(
        &sid,
        state.config.security.cookie_secure,
        sessions::session_ttl(),
    );

    let body = Json(json!({
        "user": UserResponse {
            id: user.id,
            email: user.email,
            is_admin: user.is_admin,
            name: user.name,
            live_logs_enabled: state.config.livelog.enabled,
        }
    }));
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|e| AppError::Internal(anyhow::Error::new(e)))?,
    );
    Ok(resp)
}

/// POST /api/auth/logout
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    if let Some(sid) = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| cookie_value(s, COOKIE_NAME))
    {
        let _ = sessions::delete(&state.db, &sid).await;
    }
    let clear = sessions::build_clear_cookie(state.config.security.cookie_secure);
    let mut resp = (StatusCode::OK, Json(json!({"ok": true}))).into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&clear).map_err(|e| AppError::Internal(anyhow::Error::new(e)))?,
    );
    Ok(resp)
}

/// GET /api/auth/me
pub async fn me(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<UserResponse>> {
    Ok(Json(UserResponse {
        id: user.id,
        email: user.email,
        is_admin: user.is_admin,
        name: None,
        live_logs_enabled: state.config.livelog.enabled,
    }))
}

fn cookie_value(header: &str, name: &str) -> Option<String> {
    for raw in header.split(';') {
        let pair = raw.trim();
        if let Some(val) = pair.strip_prefix(&format!("{name}=")) {
            return Some(val.to_string());
        }
    }
    None
}
