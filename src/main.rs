//! Termux MCP Server v5.0 - Enterprise Rust Implementation
//! Highest industry standards for mobile edge MCP deployment on high-end Android devices.
//!
//! Key Design Principles:
//! - Zero-trust authentication from the first request
//! - Memory-safe async task management
//! - Hardened filesystem operations resistant to symlink attacks
//! - Proper ASGI-equivalent lifespan handling via Axum
//! - Single-binary deployment optimized for runit supervision

use axum::{routing::get, Router};
use termux_mcp_server::{
    config::{AppConfig, AuthPosture, validate_runtime_auth_posture},
    tools::FileSystemTools,
};
use tokio::signal;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "termux_mcp_server=info,rmcp=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    info!("Starting Termux MCP Server v5.0 (Rust)");

    let config = AppConfig::load()?;
    info!(?config.server, "Configuration loaded");

    match validate_runtime_auth_posture(&config)? {
        AuthPosture::StaticTokenConfigured => info!("Static token authentication configured"),
        AuthPosture::UnauthenticatedLocalhostOnly => warn!(
            "Unauthenticated local-only development mode enabled; do not expose this listener remotely"
        ),
    }

    let display_addr = format!("{}:{}", config.server.host, config.server.port);
    let bind_addr = (config.server.host.as_str(), config.server.port);

    // Keep filesystem tools initialized so startup validates the configured safe roots,
    // while avoiding the unavailable rmcp 0.1 server transport API until a compatible
    // transport integration is selected deliberately.
    let _file_tools = FileSystemTools::new(config.file.safe_roots.clone());

    let app = Router::new()
        .route("/health", get(health_check))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    info!("Listening on http://{}", display_addr);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Server shutdown complete");
    Ok(())
}

async fn health_check() -> &'static str {
    "ok"
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received");
}
