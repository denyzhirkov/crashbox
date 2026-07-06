use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::app_state::AppState;
use crate::http::{assets, auth, health, heartbeats, ingest, issues, livelog, projects};
use crate::metrics_layer;

pub fn build(state: AppState) -> Router {
    let envelope_limit = state.config.ingest.max_envelope_bytes;
    let log_batch_limit = state.config.livelog.max_batch_bytes;
    let live_logs_enabled = state.config.livelog.enabled;

    let ingest_router = Router::new()
        .route("/api/:project_id/envelope", post(ingest::envelope_endpoint))
        .route(
            "/api/:project_id/envelope/",
            post(ingest::envelope_endpoint),
        )
        .layer(DefaultBodyLimit::max(envelope_limit));

    // Heartbeat pings are public (authenticated by the unguessable key alone). GET is
    // supported so a bare `curl <url>` at the end of a cron line works.
    let ping_router = Router::new()
        .route(
            "/ping/:ping_key",
            get(heartbeats::ping).post(heartbeats::ping),
        )
        .route(
            "/ping/:ping_key/",
            get(heartbeats::ping).post(heartbeats::ping),
        );

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
            "/api/projects/:project_id/heartbeats",
            get(heartbeats::list).post(heartbeats::create),
        )
        .route(
            "/api/heartbeats/:id",
            delete(heartbeats::remove).patch(heartbeats::patch),
        )
        .route("/api/issues/:id", get(issues::get).patch(issues::patch))
        .route("/api/issues/:id/events", get(issues::list_events))
        .route("/api/events/:id", get(issues::get_event));

    let mut router = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .route("/metrics", get(metrics_layer::render))
        .merge(ingest_router)
        .merge(ping_router)
        .merge(admin_api);

    // Live Logs routes are mounted only when the feature is enabled, so a disabled deploy returns
    // 404 rather than carrying dormant endpoints. Ingest carries its own (smaller) body limit.
    if live_logs_enabled {
        let logs_router = Router::new()
            .route("/api/:project_id/logs", post(livelog::logs_ingest))
            .route("/api/:project_id/logs/", post(livelog::logs_ingest))
            .layer(DefaultBodyLimit::max(log_batch_limit));
        router = router
            .merge(logs_router)
            .route("/api/projects/:id/logs/stream", get(livelog::logs_stream));
    }

    router
        .fallback(assets::fallback)
        .layer(axum::middleware::from_fn(metrics_layer::http_middleware))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
