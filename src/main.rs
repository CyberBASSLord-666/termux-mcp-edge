//! Termux MCP Edge server entrypoint.
//!
//! Key design principles:
//! - Fail-closed startup authentication posture
//! - Request authentication before staged MCP discovery or tool dispatch
//! - Bounded MCP concurrency, request duration, and request-body size
//! - Memory-safe async task management
//! - Hardened filesystem operations resistant to traversal and symlink attacks
//! - Graceful shutdown under runit supervision
//! - Single-binary deployment optimized for Android Termux

#[cfg(feature = "mcp-runtime")]
use axum::{extract::DefaultBodyLimit, middleware};
use axum::{extract::State, routing::get, Json, Router};
#[cfg(feature = "mcp-runtime")]
use termux_mcp_server::{
    auth::{require_mcp_auth, McpAuthPolicy},
    request_limits::{enforce_mcp_request_limits, McpRequestLimits},
    transport_security::TransportSecurityPolicy,
};
use termux_mcp_server::{
    config::{validate_runtime_auth_posture, AppConfig, AuthPosture},
    health::{build_readiness_response, McpRequestLimitReadiness, ReadinessResponse},
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
                .unwrap_or_else(|_| "termux_mcp_server=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "Starting Termux MCP Edge"
    );

    let config = AppConfig::load()?;
    info!(?config.server, "Configuration loaded");

    let auth_posture = validate_runtime_auth_posture(&config)?;
    let auth_posture_label = match auth_posture {
        AuthPosture::StaticTokenConfigured => {
            info!("Static token authentication configured");
            "static_token"
        }
        AuthPosture::UnauthenticatedLocalhostOnly => {
            warn!(
                "Unauthenticated local-only development mode enabled; do not expose this listener remotely"
            );
            "unauthenticated_localhost_only"
        }
    };

    #[cfg(feature = "mcp-runtime")]
    let mcp_auth_policy = McpAuthPolicy::from_config(&config.auth, auth_posture)?;

    #[cfg(feature = "mcp-runtime")]
    let mcp_request_limits = McpRequestLimits::from_seconds(
        config.transport.max_concurrent_requests,
        config.transport.request_timeout_seconds,
        config.transport.max_body_bytes,
    )?;

    #[cfg(feature = "mcp-runtime")]
    info!(
        max_concurrent_requests = config.transport.max_concurrent_requests,
        request_timeout_seconds = config.transport.request_timeout_seconds,
        max_body_bytes = config.transport.max_body_bytes,
        "MCP request limits configured"
    );

    let display_addr = format!("{}:{}", config.server.host, config.server.port);
    let bind_addr = (config.server.host.as_str(), config.server.port);

    // Initialize filesystem tools once so startup validates configured safe roots.
    // The optional staged MCP runtime reuses this instance for bounded safe-rooted
    // listing, reads, dry-run previews, and explicitly requested writes.
    let file_tools = FileSystemTools::new(config.file.safe_roots.clone());

    #[cfg(feature = "mcp-runtime")]
    let readiness_limits = Some(McpRequestLimitReadiness {
        max_concurrent_requests: config.transport.max_concurrent_requests,
        request_timeout_seconds: config.transport.request_timeout_seconds,
        max_body_bytes: config.transport.max_body_bytes,
    });

    #[cfg(not(feature = "mcp-runtime"))]
    let readiness_limits = None;

    let readiness = build_readiness_response(
        config.file.safe_roots.len(),
        auth_posture_label,
        readiness_limits,
    );

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        .with_state(readiness)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    #[cfg(feature = "mcp-runtime")]
    let app = {
        let mcp_app = termux_mcp_server::mcp_transport::router(
            TransportSecurityPolicy::new(
                config.transport.allowed_hosts.clone(),
                config.transport.allowed_origins.clone(),
                config.transport.allow_missing_origin,
            ),
            file_tools,
        )
        .layer(DefaultBodyLimit::max(config.transport.max_body_bytes))
        .route_layer(middleware::from_fn_with_state(
            mcp_request_limits,
            enforce_mcp_request_limits,
        ))
        .route_layer(middleware::from_fn_with_state(
            mcp_auth_policy,
            require_mcp_auth,
        ));

        app.merge(mcp_app)
    };

    #[cfg(not(feature = "mcp-runtime"))]
    let _ = file_tools;

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

async fn readiness_check(State(readiness): State<ReadinessResponse>) -> Json<ReadinessResponse> {
    Json(readiness)
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
