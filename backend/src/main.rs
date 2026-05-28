use std::sync::Arc;

use crashbox::app_state::AppState;
use crashbox::config::Config;
use crashbox::{bootstrap, db, http, jobs};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    init_tracing(&cfg.log_level);

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        bind = %cfg.bind_addr(),
        "crashbox starting"
    );

    let pool = db::connect(&cfg.database_url).await?;
    db::migrate(&pool).await?;
    bootstrap::run(&pool, &cfg).await?;

    let cancel = CancellationToken::new();
    jobs::cleanup::spawn(pool.clone(), Arc::new(cfg.retention.clone()), cancel.clone());

    let state = AppState::new(cfg.clone(), pool);
    let app = http::routes::build(state);

    let listener = tokio::net::TcpListener::bind(cfg.bind_addr()).await?;
    tracing::info!(addr = %listener.local_addr()?, "crashbox listening");

    let shutdown = {
        let cancel = cancel.clone();
        async move {
            shutdown_signal().await;
            cancel.cancel();
        }
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;

    tracing::info!("crashbox shutting down");
    Ok(())
}

fn init_tracing(default_level: &str) {
    let filter = EnvFilter::try_from_env("CRASHBOX_LOG_FILTER")
        .or_else(|_| EnvFilter::try_new(default_level))
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut s) = signal(SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
