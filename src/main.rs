//! Termux MCP Server v5.0 - Enterprise Rust Implementation
//! Highest industry standards for mobile edge MCP deployment on high-end Android devices.

use std::net::{IpAddr, SocketAddr};

use anyhow::{bail, Context};
use axum::{routing::get, Router};
use rmcp::{
    model::{ServerCapabilities, ServerInfo},
    tool,
    transport::{sse_server::SseServerConfig, SseServer},
    ServerHandler,
};
use termux_mcp_server::{
    config::AppConfig,
    tools::{FileSystemTools, ShellTools, SystemTools},
};
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct TermuxMcpService {
    filesystem: FileSystemTools,
    system: SystemTools,
    shell: ShellTools,
}

#[tool(tool_box)]
impl TermuxMcpService {
    #[tool(description = "List a safe-rooted directory with bounded traversal")]
    async fn list_directory(
        &self,
        #[tool(aggr)] params: termux_mcp_server::tools::filesystem::ListDirectoryParams,
    ) -> Result<String, String> {
        self.filesystem
            .mcp_list_directory(rmcp::handler::server::tool::Parameters(params))
            .await
    }

    #[tool(description = "Read a UTF-8 file from a configured safe root")]
    async fn read_file(
        &self,
        #[tool(aggr)] params: termux_mcp_server::tools::filesystem::ReadFileParams,
    ) -> Result<String, String> {
        self.filesystem
            .mcp_read_file(rmcp::handler::server::tool::Parameters(params))
            .await
    }

    #[tool(description = "Atomically write a UTF-8 file under a configured safe root")]
    async fn write_file(
        &self,
        #[tool(aggr)] params: termux_mcp_server::tools::filesystem::WriteFileParams,
    ) -> Result<String, String> {
        self.filesystem
            .mcp_write_file(rmcp::handler::server::tool::Parameters(params))
            .await
    }

    #[tool(description = "Read Android sensor data with structured output")]
    async fn read_sensor(&self, #[tool(param)] sensor: String) -> Result<String, String> {
        self.system
            .read_sensor(sensor)
            .await
            .and_then(|result| serde_json::to_string(&result).map_err(Into::into))
            .map_err(|error: termux_mcp_server::error::AppError| error.to_string())
    }

    #[tool(description = "Get recent Android logcat lines")]
    async fn get_logcat(&self, #[tool(param)] lines: Option<u32>) -> Result<String, String> {
        self.system
            .get_logcat(lines)
            .await
            .and_then(|result| serde_json::to_string(&result).map_err(Into::into))
            .map_err(|error: termux_mcp_server::error::AppError| error.to_string())
    }

    #[tool(description = "Execute a shell command through rish")]
    async fn rish_exec(&self, #[tool(param)] command: String) -> Result<String, String> {
        self.shell
            .rish_exec(command)
            .await
            .and_then(|result| serde_json::to_string(&result).map_err(Into::into))
            .map_err(|error: termux_mcp_server::error::AppError| error.to_string())
    }

    #[tool(description = "Dump and parse the Android UI hierarchy")]
    async fn dump_ui_hierarchy(&self) -> Result<String, String> {
        self.system
            .dump_ui_hierarchy()
            .await
            .and_then(|result| serde_json::to_string(&result).map_err(Into::into))
            .map_err(|error: termux_mcp_server::error::AppError| error.to_string())
    }
}

#[tool(tool_box)]
impl ServerHandler for TermuxMcpService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Termux MCP edge server exposing safe filesystem, Android system, and rish tools"
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

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

    validate_auth_posture(&config, &config.server.host)?;
    let bind_addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .context("server host/port must form a valid socket address")?;

    let filesystem = FileSystemTools::try_new(config.file.safe_roots.clone())
        .context("invalid filesystem safe-root configuration")?;
    let service = TermuxMcpService {
        filesystem,
        system: SystemTools,
        shell: ShellTools,
    };

    // rmcp 0.1 exposes the server-side HTTP transport as SSE endpoints.
    // Protected MCP endpoint set:
    //   GET  /mcp/sse
    //   POST /mcp/message?sessionId=<session>
    let ct = CancellationToken::new();
    let mcp_server = SseServer::serve_with_config(SseServerConfig {
        bind: bind_addr,
        sse_path: "/mcp/sse".to_string(),
        post_path: "/mcp/message".to_string(),
        ct: ct.clone(),
    })
    .await?;
    let mcp_ct = mcp_server.with_service(move || service.clone());

    // Health is intentionally unauthenticated and hosted on localhost+1 to avoid
    // conflicting with rmcp 0.1's self-contained SSE listener.
    let health_addr = SocketAddr::new(bind_addr.ip(), bind_addr.port().saturating_add(1));
    tokio::spawn(async move {
        if let Err(error) = serve_health(health_addr).await {
            error!(%error, "health endpoint stopped unexpectedly");
        }
    });

    info!(%bind_addr, sse = "/mcp/sse", message = "/mcp/message", "MCP SSE transport listening");
    shutdown_signal().await;
    mcp_ct.cancel();
    ct.cancel();
    info!("Server shutdown complete");
    Ok(())
}

async fn serve_health(addr: SocketAddr) -> anyhow::Result<()> {
    let app = Router::new().route("/health", get(health_check));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn validate_auth_posture(config: &AppConfig, host: &str) -> anyhow::Result<()> {
    if let Some(ref token) = config.auth.static_token {
        if token.trim().is_empty() {
            bail!("MCP__AUTH__STATIC_TOKEN is configured but empty; please provide a non-empty token or use localhost-only unauthenticated mode");
        }

        warn!("rmcp 0.1 SSE transport is active; place this listener behind an authenticating reverse proxy until first-party transport middleware is available");
        return Ok(());
    }

    if !config.auth.allow_unauthenticated_localhost_only {
        bail!(
            "MCP__AUTH__STATIC_TOKEN is required unless MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true is explicitly set for local-only development"
        );
    }

    if !is_loopback_host(host) {
        bail!(
            "Unauthenticated mode is only allowed on localhost; set MCP__AUTH__STATIC_TOKEN or bind MCP__SERVER__HOST to localhost, 127.0.0.1, or ::1"
        );
    }

    warn!(
        "Unauthenticated local-only development mode enabled; do not expose this listener remotely"
    );
    Ok(())
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
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
