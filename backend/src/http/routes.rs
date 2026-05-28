use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::app_state::AppState;
use crate::http::{assets, auth, health, ingest, issues, projects};

pub fn build(state: AppState) -> Router {
    let envelope_limit = state.config.ingest.max_envelope_bytes;

    let ingest_router = Router::new()
        .route("/api/:project_id/envelope", post(ingest::envelope_endpoint))
        .route("/api/:project_id/envelope/", post(ingest::envelope_endpoint))
        .layer(DefaultBodyLimit::max(envelope_limit));

    let admin_api = Router::new()
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/me", get(auth::me))
        .route("/api/projects", get(projects::list).post(projects::create))
        .route("/api/projects/overview", get(projects::overview))
        .route(
            "/api/projects/:id",
            get(projects::get).patch(projects::patch),
        )
        .route("/api/projects/:id/dsn", get(projects::dsn))
        .route("/api/projects/:id/rotate-key", post(projects::rotate_key))
        .route("/api/projects/:project_id/issues", get(issues::list))
        .route(
            "/api/issues/:id",
            get(issues::get).patch(issues::patch),
        )
        .route("/api/issues/:id/events", get(issues::list_events))
        .route("/api/events/:id", get(issues::get_event));

    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .merge(ingest_router)
        .merge(admin_api)
        .fallback(assets::fallback)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
