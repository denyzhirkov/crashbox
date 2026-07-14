use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, patch, post};
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::app_state::AppState;
use crate::http::{
    assets, auth, backup, health, heartbeats, ingest, issues, livelog, projects, tokens,
};
use crate::metrics_layer;

pub fn build(state: AppState) -> Router {
    let envelope_limit = state.config.ingest.max_envelope_bytes;
    let log_batch_limit = state.config.livelog.max_batch_bytes;
    let live_logs_enabled = state.config.livelog.enabled;

    let mut ingest_router = Router::new()
        .route("/api/:project_id/envelope", post(ingest::envelope_endpoint))
        .route(
            "/api/:project_id/envelope/",
            post(ingest::envelope_endpoint),
        );
    // Mounted only when opted in — absent routes fall through to the SPA fallback,
    // mirroring how the live-logs routes behave when disabled.
    if state.config.ingest.enable_legacy_store_endpoint {
        ingest_router = ingest_router
            .route("/api/:project_id/store", post(ingest::store_endpoint))
            .route("/api/:project_id/store/", post(ingest::store_endpoint));
    }
    let ingest_router = ingest_router.layer(DefaultBodyLimit::max(envelope_limit));

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
        .route("/api/admin/backup", get(backup::download))
        .route("/api/tokens", get(tokens::list).post(tokens::create))
        .route("/api/tokens/:id", delete(tokens::remove))
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
            "/api/projects/:project_id/events",
            get(issues::project_events),
        )
        .route(
            "/api/projects/:project_id/heartbeats",
            get(heartbeats::list).post(heartbeats::create),
        )
        .route(
            "/api/heartbeats/:id",
            delete(heartbeats::remove).patch(heartbeats::patch),
        )
        .route("/api/heartbeats/:id/history", get(heartbeats::history))
        .route("/api/issues", patch(issues::bulk_patch))
        .route(
            "/api/issues/:id",
            get(issues::get).patch(issues::patch).delete(issues::remove),
        )
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
            .route("/api/projects/:id/logs/stream", get(livelog::logs_stream))
            .route("/api/projects/:id/logs/recent", get(livelog::logs_recent));
    }

    router
        .fallback(assets::fallback)
        .layer(axum::middleware::from_fn(metrics_layer::http_middleware))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
