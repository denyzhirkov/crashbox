use std::sync::Arc;

use sqlx::SqlitePool;

use crate::config::Config;
use crate::ingest::rate_limit::RateLimiter;
use crate::metrics_layer::MetricsHandle;
use crate::notify::NotifyHub;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: SqlitePool,
    pub rate_limiter: Arc<RateLimiter>,
    pub notify: Arc<NotifyHub>,
    pub metrics: MetricsHandle,
}

impl AppState {
    pub fn new(config: Config, db: SqlitePool, metrics: MetricsHandle) -> Self {
        let rate_limiter = Arc::new(RateLimiter::new(
            config.ingest.max_events_per_minute_per_project,
        ));
        let notify = Arc::new(NotifyHub::from_config(&config));
        Self {
            config: Arc::new(config),
            db,
            rate_limiter,
            notify,
            metrics,
        }
    }
}
