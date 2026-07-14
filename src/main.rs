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

use std::{ffi::OsStr, path::PathBuf};

#[cfg(feature = "mcp-runtime")]
use axum::{extract::DefaultBodyLimit, middleware};
use axum::{extract::State, routing::get, Json, Router};
#[cfg(feature = "mcp-runtime")]
use termux_mcp_server::health::McpRequestLimitReadiness;
#[cfg(feature = "mcp-runtime")]
use termux_mcp_server::{
    auth::{require_mcp_auth, McpAuthPolicy},
    directory_grant::DirectoryGrantVerifier,
    request_limits::{enforce_mcp_request_limits, McpRequestLimits},
    transport_security::TransportSecurityPolicy,
};
use termux_mcp_server::{
    config::{validate_runtime_auth_posture, AppConfig, AuthPosture},
    health::{build_readiness_response, ReadinessResponse},
    tools::FileSystemTools,
};
use tokio::signal;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const CLI_HELP: &str = "Termux MCP Edge\n\nUsage:\n  termux-mcp-server\n  termux-mcp-server --version\n  termux-mcp-server --help\n";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if handle_cli()? {
        return Ok(());
    }

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
    let _directory_grant_verifier = DirectoryGrantVerifier::load_optional(
        &config.directory_grant,
        config.auth.static_token.as_deref(),
    )?;

    #[cfg(feature = "mcp-runtime")]
    info!(
        verification_configured = _directory_grant_verifier.is_some(),
        mutation_enabled = false,
        "Directory capability-grant verification posture loaded"
    );

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

    // Anchor every configured jail root to an existing directory before any
    // listener is opened. Termux storage permissions and mount availability can
    // change independently of configuration; retaining an unresolved lexical
    // path would make startup appear healthy without a trustworthy jail anchor.
    let safe_roots = anchor_safe_roots(config.file.safe_roots.clone())?;
    let safe_root_count = safe_roots.len();

    // Initialize filesystem tools once so the optional staged MCP runtime reuses
    // the exact anchored roots for bounded listing, reads, dry-run previews, and
    // explicitly requested writes.
    let file_tools = FileSystemTools::new(safe_roots);

    #[cfg(feature = "mcp-runtime")]
    let readiness_limits = Some(McpRequestLimitReadiness {
        max_concurrent_requests: config.transport.max_concurrent_requests,
        request_timeout_seconds: config.transport.request_timeout_seconds,
        max_body_bytes: config.transport.max_body_bytes,
    });

    #[cfg(not(feature = "mcp-runtime"))]
    let readiness_limits = None;

    let readiness = build_readiness_response(safe_root_count, auth_posture_label, readiness_limits);
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
            )?,
            file_tools,
            config.android.battery_status_enabled,
            config.android.volume_status_enabled,
            config.command.enabled,
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

fn anchor_safe_roots(safe_roots: Vec<PathBuf>) -> anyhow::Result<Vec<PathBuf>> {
    if safe_roots.is_empty() {
        anyhow::bail!("at least one filesystem safe root must be configured");
    }

    let mut anchored = Vec::with_capacity(safe_roots.len());
    for (index, root) in safe_roots.into_iter().enumerate() {
        let position = index + 1;
        let canonical = root.canonicalize().map_err(|_| {
            anyhow::anyhow!("configured filesystem safe root {position} cannot be resolved")
        })?;
        let metadata = std::fs::metadata(&canonical).map_err(|_| {
            anyhow::anyhow!("configured filesystem safe root {position} cannot be inspected")
        })?;

        if !metadata.is_dir() {
            anyhow::bail!("configured filesystem safe root {position} is not a directory");
        }

        anchored.push(canonical);
    }

    anchored.sort_unstable();
    anchored.dedup();
    Ok(anchored)
}

fn handle_cli() -> anyhow::Result<bool> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let first = arguments.next();
    let second = arguments.next();

    match (first.as_deref(), second.as_deref()) {
        (None, None) => Ok(false),
        (Some(argument), None) if argument == OsStr::new("--version") => {
            println!("termux-mcp-server {}", env!("CARGO_PKG_VERSION"));
            Ok(true)
        }
        (Some(argument), None)
            if argument == OsStr::new("--help") || argument == OsStr::new("-h") =>
        {
            print!("{CLI_HELP}");
            Ok(true)
        }
        (Some(argument), None) if argument == OsStr::new("--self-check-command-boundary") => {
            verify_command_execution_boundary()?;
            println!("termux-mcp-command-boundary ok");
            Ok(true)
        }
        _ => anyhow::bail!("unsupported command-line arguments; use --help"),
    }
}

fn verify_command_execution_boundary() -> anyhow::Result<()> {
    if std::env::vars_os().next().is_some() {
        anyhow::bail!("command execution boundary check failed");
    }

    let working_directory = std::env::current_dir()
        .map_err(|_| anyhow::anyhow!("command execution boundary check failed"))?;
    if !working_directory.is_absolute() || working_directory == std::path::Path::new("/") {
        anyhow::bail!("command execution boundary check failed");
    }

    let stdin_target = std::fs::read_link("/proc/self/fd/0")
        .map_err(|_| anyhow::anyhow!("command execution boundary check failed"))?;
    if stdin_target != std::path::Path::new("/dev/null") {
        anyhow::bail!("command execution boundary check failed");
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_roots_anchor_existing_directories_and_deduplicate_aliases() {
        let root = tempfile::tempdir().unwrap();
        let expected = root.path().canonicalize().unwrap();

        let anchored = anchor_safe_roots(vec![
            root.path().to_path_buf(),
            root.path().join("."),
            root.path().to_path_buf(),
        ])
        .unwrap();

        assert_eq!(anchored, vec![expected]);
    }

    #[test]
    fn safe_roots_keep_distinct_directories_in_deterministic_order() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let mut expected = vec![
            first.path().canonicalize().unwrap(),
            second.path().canonicalize().unwrap(),
        ];
        expected.sort_unstable();

        let anchored = anchor_safe_roots(vec![
            second.path().to_path_buf(),
            first.path().to_path_buf(),
        ])
        .unwrap();

        assert_eq!(anchored, expected);
    }

    #[test]
    fn safe_roots_reject_missing_paths_without_disclosing_them() {
        let parent = tempfile::tempdir().unwrap();
        let missing = parent.path().join("private-missing-root");

        let error = anchor_safe_roots(vec![missing.clone()]).unwrap_err();
        let message = error.to_string();

        assert!(message.contains("safe root 1 cannot be resolved"));
        assert!(!message.contains(missing.to_string_lossy().as_ref()));
    }

    #[test]
    fn safe_roots_reject_regular_files_without_disclosing_them() {
        let parent = tempfile::tempdir().unwrap();
        let file = parent.path().join("not-a-root.txt");
        std::fs::write(&file, "not a directory").unwrap();

        let error = anchor_safe_roots(vec![file.clone()]).unwrap_err();
        let message = error.to_string();

        assert!(message.contains("safe root 1 is not a directory"));
        assert!(!message.contains(file.to_string_lossy().as_ref()));
    }

    #[test]
    fn safe_roots_reject_empty_configuration() {
        let error = anchor_safe_roots(Vec::new()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "at least one filesystem safe root must be configured"
        );
    }

    #[test]
    fn command_boundary_check_fails_under_the_ambient_test_process() {
        let error = verify_command_execution_boundary().unwrap_err();
        assert_eq!(error.to_string(), "command execution boundary check failed");
    }
}
