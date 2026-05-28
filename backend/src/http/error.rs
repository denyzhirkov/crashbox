use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("internal error")]
    Internal(#[source] anyhow::Error),
}

impl AppError {
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn public_message(&self) -> String {
        match self {
            Self::NotFound => "not found".into(),
            Self::BadRequest(m) => m.clone(),
            Self::Unauthorized => "unauthorized".into(),
            Self::Forbidden => "forbidden".into(),
            Self::Conflict(m) => m.clone(),
            Self::Internal(_) => "internal error".into(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        if let Self::Internal(err) = &self {
            tracing::error!(error = ?err, "internal error");
        }
        let body = Json(json!({ "error": self.public_message() }));
        (status, body).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => Self::NotFound,
            other => Self::Internal(anyhow::Error::new(other)),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e)
    }
}

pub type AppResult<T> = std::result::Result<T, AppError>;
