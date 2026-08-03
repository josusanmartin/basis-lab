use std::{env, net::SocketAddr};

use basis_lab::{AppState, router};
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "basis_lab=info,tower_http=info".into()),
        )
        .compact()
        .init();

    let port = env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8080);
    let concurrency = env::var("MAX_UPSTREAM_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(64);
    let address = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(address)
        .await
        .expect("failed to bind server port");
    info!(%address, "Basis Lab listening");

    let state = AppState::new(concurrency);
    let catalog = state.service.clone();
    tokio::spawn(async move {
        match catalog.tickers().await {
            Ok(tickers) => info!(tickers = tickers.len(), "Ticker cache warmed"),
            Err(error) => warn!(%error, "Ticker cache warm-up was incomplete"),
        }
    });

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server failed");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler")
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
}
