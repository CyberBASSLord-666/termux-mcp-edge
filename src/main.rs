//! Termux MCP Server v5.0 - Enterprise Rust Implementation
//! Highest industry standards for mobile edge MCP deployment on high-end Android devices.

use std::{collections::HashMap, net::IpAddr, net::SocketAddr, sync::Arc};

use anyhow::{bail, Context};
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures::{Sink, SinkExt, Stream, StreamExt};
use rmcp::{
    handler::server::tool::Parameters,
    model::{ClientJsonRpcMessage, ServerCapabilities, ServerInfo},
    service::{RxJsonRpcMessage, ServiceExt, TxJsonRpcMessage},
    tool, RoleServer, ServerHandler,
};
use termux_mcp_server::{
    config::AppConfig,
    tools::{FileSystemTools, ShellTools, SystemTools},
};
use tokio::signal;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::{CancellationToken, PollSender};
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

type SessionId = Arc<str>;
type ClientMessageSender = tokio::sync::mpsc::Sender<ClientJsonRpcMessage>;
type SessionStore = Arc<tokio::sync::RwLock<HashMap<SessionId, ClientMessageSender>>>;

#[derive(Clone)]
enum AuthMode {
    BearerToken(Arc<str>),
    LocalhostUnauthenticated,
}

#[derive(Clone)]
struct McpHttpState {
    sessions: SessionStore,
    transport_tx: tokio::sync::mpsc::UnboundedSender<AuthenticatedSseTransport>,
    post_path: Arc<str>,
    auth: AuthMode,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostEventQuery {
    session_id: String,
}

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
        self.filesystem.mcp_list_directory(Parameters(params)).await
    }

    #[tool(description = "Read a UTF-8 file from a configured safe root")]
    async fn read_file(
        &self,
        #[tool(aggr)] params: termux_mcp_server::tools::filesystem::ReadFileParams,
    ) -> Result<String, String> {
        self.filesystem.mcp_read_file(Parameters(params)).await
    }

    #[tool(description = "Atomically write a UTF-8 file under a configured safe root")]
    async fn write_file(
        &self,
        #[tool(aggr)] params: termux_mcp_server::tools::filesystem::WriteFileParams,
    ) -> Result<String, String> {
        self.filesystem.mcp_write_file(Parameters(params)).await
    }

    #[tool(description = "Read Android sensor data with structured output")]
    async fn read_sensor(&self, #[tool(param)] sensor: String) -> Result<String, String> {
        serialize_tool_result(self.system.read_sensor(sensor).await)
    }

    #[tool(description = "Get recent Android logcat lines")]
    async fn get_logcat(&self, #[tool(param)] lines: Option<u32>) -> Result<String, String> {
        serialize_tool_result(self.system.get_logcat(lines).await)
    }

    #[tool(description = "Execute a shell command through rish")]
    async fn rish_exec(&self, #[tool(param)] command: String) -> Result<String, String> {
        serialize_tool_result(self.shell.rish_exec(command).await)
    }

    #[tool(description = "Dump and parse the Android UI hierarchy")]
    async fn dump_ui_hierarchy(&self) -> Result<String, String> {
        serialize_tool_result(self.system.dump_ui_hierarchy().await)
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

fn serialize_tool_result<T: serde::Serialize>(
    result: Result<T, termux_mcp_server::error::AppError>,
) -> Result<String, String> {
    result
        .and_then(|value| serde_json::to_string(&value).map_err(Into::into))
        .map_err(|error| error.to_string())
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

    let auth = validate_auth_posture(&config, &config.server.host)?;
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

    let cancellation = CancellationToken::new();
    serve_mcp_http(bind_addr, auth, service, cancellation.clone()).await?;

    info!(%bind_addr, sse = "/mcp/sse", message = "/mcp/message", health = "/health", "HTTP server listening");
    shutdown_signal().await;
    cancellation.cancel();
    info!("Server shutdown complete");
    Ok(())
}

async fn serve_mcp_http(
    bind_addr: SocketAddr,
    auth: AuthMode,
    service: TermuxMcpService,
    cancellation: CancellationToken,
) -> anyhow::Result<()> {
    let (transport_tx, mut transport_rx) = tokio::sync::mpsc::unbounded_channel();
    let state = McpHttpState {
        sessions: SessionStore::default(),
        transport_tx,
        post_path: Arc::from("/mcp/message"),
        auth,
    };

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/mcp/sse", get(sse_handler))
        .route("/mcp/message", post(post_event_handler))
        .with_state(state)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let service_cancellation = cancellation.clone();
    tokio::spawn(async move {
        while let Some(transport) = transport_rx.recv().await {
            let service = service.clone();
            let request_cancellation = service_cancellation.child_token();
            tokio::spawn(async move {
                match service.serve_with_ct(transport, request_cancellation).await {
                    Ok(running_service) => {
                        if let Err(error) = running_service.waiting().await {
                            warn!(%error, "MCP service session ended with an error");
                        }
                    }
                    Err(error) => warn!(%error, "failed to start MCP service session"),
                }
            });
        }
    });

    let shutdown = cancellation.clone();
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await
        {
            warn!(%error, "HTTP server stopped with an error");
        }
    });

    Ok(())
}

async fn post_event_handler(
    State(state): State<McpHttpState>,
    headers: HeaderMap,
    Query(PostEventQuery { session_id }): Query<PostEventQuery>,
    Json(message): Json<ClientJsonRpcMessage>,
) -> Result<StatusCode, Response> {
    authorize(&headers, &state.auth).map_err(|_| unauthorized_response())?;
    let tx = {
        let sessions = state.sessions.read().await;
        sessions
            .get(session_id.as_str())
            .cloned()
            .ok_or_else(|| (StatusCode::NOT_FOUND, "unknown MCP session").into_response())?
    };

    tx.send(message)
        .await
        .map_err(|_| (StatusCode::GONE, "MCP session is closed").into_response())?;
    Ok(StatusCode::ACCEPTED)
}

async fn sse_handler(
    State(state): State<McpHttpState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, std::io::Error>>>, Response> {
    authorize(&headers, &state.auth).map_err(|_| unauthorized_response())?;

    let session_id: SessionId = Arc::from(uuid::Uuid::new_v4().simple().to_string());
    let (from_client_tx, from_client_rx) = tokio::sync::mpsc::channel(64);
    let (to_client_tx, to_client_rx) = tokio::sync::mpsc::channel(64);

    state
        .sessions
        .write()
        .await
        .insert(session_id.clone(), from_client_tx);

    let transport = AuthenticatedSseTransport {
        stream: ReceiverStream::new(from_client_rx),
        sink: PollSender::new(to_client_tx),
        session_id: session_id.clone(),
        sessions: state.sessions.clone(),
    };

    if state.transport_tx.send(transport).is_err() {
        state.sessions.write().await.remove(&session_id);
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "MCP server is closed").into_response());
    }

    let endpoint_event = Event::default()
        .event("endpoint")
        .data(format!("{}?sessionId={session_id}", state.post_path));
    let stream = futures::stream::once(async move { Ok(endpoint_event) }).chain(
        ReceiverStream::new(to_client_rx).map(|message| {
            serde_json::to_string(&message)
                .map(|payload| Event::default().event("message").data(payload))
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        }),
    );

    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

struct AuthenticatedSseTransport {
    stream: ReceiverStream<RxJsonRpcMessage<RoleServer>>,
    sink: PollSender<TxJsonRpcMessage<RoleServer>>,
    session_id: SessionId,
    sessions: SessionStore,
}

impl Sink<TxJsonRpcMessage<RoleServer>> for AuthenticatedSseTransport {
    type Error = std::io::Error;

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.sink
            .poll_ready_unpin(cx)
            .map_err(std::io::Error::other)
    }

    fn start_send(
        mut self: std::pin::Pin<&mut Self>,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> Result<(), Self::Error> {
        self.sink
            .start_send_unpin(item)
            .map_err(std::io::Error::other)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.sink
            .poll_flush_unpin(cx)
            .map_err(std::io::Error::other)
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let result = self
            .sink
            .poll_close_unpin(cx)
            .map_err(std::io::Error::other);
        if result.is_ready() {
            let session_id = self.session_id.clone();
            let sessions = self.sessions.clone();
            tokio::spawn(async move {
                sessions.write().await.remove(&session_id);
            });
        }
        result
    }
}

impl Stream for AuthenticatedSseTransport {
    type Item = RxJsonRpcMessage<RoleServer>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.stream.poll_next_unpin(cx)
    }
}

fn authorize(headers: &HeaderMap, auth: &AuthMode) -> Result<(), ()> {
    match auth {
        AuthMode::LocalhostUnauthenticated => Ok(()),
        AuthMode::BearerToken(expected) => {
            let Some(value) = headers.get(header::AUTHORIZATION) else {
                return Err(());
            };
            let Ok(value) = value.to_str() else {
                return Err(());
            };
            let Some(token) = value.strip_prefix("Bearer ") else {
                return Err(());
            };
            if constant_time_eq(token.as_bytes(), expected.as_bytes()) {
                Ok(())
            } else {
                Err(())
            }
        }
    }
}

fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        "missing or invalid bearer token",
    )
        .into_response()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = (left.len() ^ right.len()) as u8;
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= left_byte ^ right_byte;
    }
    diff == 0
}

fn validate_auth_posture(config: &AppConfig, host: &str) -> anyhow::Result<AuthMode> {
    if let Some(ref token) = config.auth.static_token {
        if token.trim().is_empty() {
            bail!("MCP__AUTH__STATIC_TOKEN is configured but empty; please provide a non-empty token or use localhost-only unauthenticated mode");
        }

        info!("Static bearer-token authentication configured for MCP routes");
        return Ok(AuthMode::BearerToken(Arc::from(token.clone())));
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
    Ok(AuthMode::LocalhostUnauthenticated)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_auth_accepts_only_exact_valid_token() {
        let auth = AuthMode::BearerToken(Arc::from("secret-token"));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer secret-token".parse().unwrap(),
        );
        assert!(authorize(&headers, &auth).is_ok());

        headers.insert(header::AUTHORIZATION, "Bearer wrong-token".parse().unwrap());
        assert!(authorize(&headers, &auth).is_err());

        headers.insert(header::AUTHORIZATION, "Basic secret-token".parse().unwrap());
        assert!(authorize(&headers, &auth).is_err());
    }

    #[test]
    fn bearer_auth_rejects_missing_authorization_header() {
        let auth = AuthMode::BearerToken(Arc::from("secret-token"));
        assert!(authorize(&HeaderMap::new(), &auth).is_err());
    }

    #[test]
    fn localhost_unauthenticated_mode_skips_header_validation() {
        assert!(authorize(&HeaderMap::new(), &AuthMode::LocalhostUnauthenticated).is_ok());
    }

    #[test]
    fn constant_time_eq_requires_identical_bytes() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"same", b"same-longer"));
    }
}
