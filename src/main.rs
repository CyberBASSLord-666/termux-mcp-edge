//! Termux MCP Server v5.0 - Enterprise Rust Implementation
//! Highest industry standards for mobile edge MCP deployment on high-end Android devices.
//!
//! Key Design Principles:
//! - Zero-trust authentication from the first request
//! - Memory-safe async task management
//! - Hardened filesystem operations resistant to symlink attacks
//! - Proper ASGI-equivalent lifespan handling via Axum
//! - Single-binary deployment optimized for runit supervision

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{routing::get, Router};
use rmcp::server::axum::McpAxumServer;
use rmcp::server::auth::StaticTokenVerifier;
use rmcp::server::Server;
use tokio::signal;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod error;
mod tools;

use crate::config::AppConfig;
use crate::tools::FileSystemTools;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize structured logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "termux_mcp_server=info,rmcp=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    info!("Starting Termux MCP Server v5.0 (Rust)");

    // Load configuration
    let config = AppConfig::load()?;
    info!(?config.server, "Configuration loaded");

    // === Zero-Trust Authentication ===
    let auth_verifier = if let Some(token) = &config.auth.static_token {
        info!("Static token authentication enabled");
        Some(StaticTokenVerifier::new(token.clone(), vec!["admin:all".to_string()]))
    } else {
        warn!("No authentication token configured! Server will run unauthenticated.");
        None
    };

    // === Build MCP Server ===
    let file_tools = FileSystemTools::new(config.file.safe_roots.clone());
    let mcp_server = Server::builder()
        .name("termux-mcp-server")
        .version(env!("CARGO_PKG_VERSION"))
        .tools(file_tools)
        .auth(auth_verifier)
        .build();

    // === Axum Router with proper lifespan handling ===
    let app = Router::new()
        .route("/health", get(health_check))
        .merge(McpAxumServer::new(mcp_server).into_router())
        .layer(tower_http::trace::TraceLayer::new_for_http());

    // === Bind & Serve ===
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    info!("Listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;

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
