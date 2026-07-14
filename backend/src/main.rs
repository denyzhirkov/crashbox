// unwrap/expect are forbidden in production code but fine in unit tests.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::Arc;

use clap::Parser;
use crashbox::app_state::AppState;
use crashbox::cli::Cli;
use crashbox::config::Config;
use crashbox::{bootstrap, db, http, jobs};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cfg = Config::from_env()?;

    // Run CLI subcommand if one was given; tracing is initialized lightly for them so they
    // don't drown the user in INFO spans.
    if crashbox::cli::run_if_present(cli, &cfg).await? {
        return Ok(());
    }

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
    jobs::cleanup::spawn(
        pool.clone(),
        Arc::new(cfg.retention.clone()),
        cancel.clone(),
    );

    let metrics_handle = crashbox::metrics_layer::init();
    let metrics = crashbox::metrics_layer::MetricsHandle {
        handle: Arc::new(metrics_handle),
    };
    let state = AppState::new(cfg.clone(), pool.clone(), metrics);
    jobs::spike::spawn(
        pool.clone(),
        Arc::new(cfg.spike.clone()),
        state.notify.clone(),
        cancel.clone(),
    );
    jobs::heartbeat::spawn(
        pool.clone(),
        Arc::new(cfg.heartbeat.clone()),
        state.notify.clone(),
        cancel.clone(),
    );
    jobs::digest::spawn(
        pool,
        Arc::new(cfg.digest.clone()),
        state.notify.clone(),
        cancel.clone(),
    );
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
        () = ctrl_c => {},
        () = terminate => {},
    }
}
