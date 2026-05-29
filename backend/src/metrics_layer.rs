//! Prometheus metrics endpoint and HTTP middleware.
//!
//! Metric naming follows `crashbox_*` convention. All labels are low-cardinality (project_slug,
//! level, status_class, reason). Per-event-id labels are NEVER emitted — Prometheus would
//! explode.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::IntoResponse;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use sqlx::SqlitePool;

use crate::app_state::AppState;

/// Initializes the global recorder and returns a handle that the /metrics endpoint can use to
/// render. Safe to call exactly once at process startup.
pub fn init() -> PrometheusHandle {
    PrometheusBuilder::new()
        .install_recorder()
        .expect("install prometheus recorder")
}

#[derive(Clone)]
pub struct MetricsHandle {
    pub handle: Arc<PrometheusHandle>,
}

impl MetricsHandle {
    /// Build a metrics handle without installing the recorder globally. Useful for tests so
    /// multiple test cases sharing one process don't collide on `install_recorder()`.
    /// `counter!()` calls without an installed recorder are silent no-ops.
    pub fn dummy() -> Self {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        Self {
            handle: Arc::new(handle),
        }
    }
}

/// GET /metrics — renders the current snapshot. Refreshes pool gauges first so they reflect
/// live state at the moment of the scrape (no separate ticker).
pub async fn render(State(state): State<AppState>) -> impl IntoResponse {
    update_pool_gauges(&state.db);
    let handle = state.metrics.handle.clone();
    let body = handle.render();
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
}

fn update_pool_gauges(pool: &SqlitePool) {
    metrics::gauge!("crashbox_db_pool_size").set(pool.size() as f64);
    metrics::gauge!("crashbox_db_pool_idle").set(pool.num_idle() as f64);
}

/// Wraps every HTTP request to record method, status_class, and duration. Mounted at the very
/// top of the router so it sees all responses (including 404s).
pub async fn http_middleware(req: Request, next: Next) -> axum::response::Response {
    let method = req.method().clone();
    let start = Instant::now();
    let resp = next.run(req).await;
    let status = resp.status();
    // Use status_class instead of raw status to keep cardinality bounded.
    let status_class = format!("{}xx", status.as_u16() / 100);
    let duration = start.elapsed().as_secs_f64();
    metrics::counter!(
        "crashbox_http_requests_total",
        "method" => method.to_string(),
        "status_class" => status_class.clone(),
    )
    .increment(1);
    metrics::histogram!(
        "crashbox_http_request_duration_seconds",
        "method" => method.to_string(),
    )
    .record(duration);
    resp
}
