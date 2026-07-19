#[cfg(feature = "mcp-runtime")]
use std::path::PathBuf;

#[cfg(feature = "mcp-runtime")]
use termux_mcp_server::{
    auth::{McpAuthPolicy, McpConnectionInfo},
    mcp_transport::McpRouterBuilder,
    request_limits::McpRequestLimits,
    transport_security::TransportSecurityPolicy,
};

#[cfg(feature = "mcp-runtime")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let token = std::env::var("MCP_EXAMPLE_STATIC_TOKEN")?;
    let safe_root = PathBuf::from(std::env::var_os("MCP_EXAMPLE_SAFE_ROOT").ok_or_else(|| {
        anyhow::anyhow!("MCP_EXAMPLE_SAFE_ROOT must name an existing absolute private directory")
    })?);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8000").await?;
    let auth = McpAuthPolicy::static_bearer(token)?;
    let limits = McpRequestLimits::from_seconds(4, 30, 2 * 1024 * 1024)?;
    let transport = TransportSecurityPolicy::localhost(8000, false)?;

    let app = McpRouterBuilder::try_new(&listener, auth, limits, transport, vec![safe_root])?
        .with_sse_enabled(false)
        .build()?;

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<McpConnectionInfo>(),
    )
    .await?;
    Ok(())
}

#[cfg(not(feature = "mcp-runtime"))]
fn main() {
    eprintln!("secure_embedding requires --features mcp-runtime");
}
