use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::app_state::AppState;

pub async fn healthz() -> &'static str {
    "ok"
}

pub async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.db)
        .await
    {
        Ok(_) => (StatusCode::OK, "ready"),
        Err(e) => {
            tracing::warn!(error = %e, "readyz: db ping failed");
            (StatusCode::SERVICE_UNAVAILABLE, "not ready")
        }
    }
}
