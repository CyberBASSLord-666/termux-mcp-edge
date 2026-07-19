//! Request authentication for the staged MCP transport.
//!
//! Authentication is deliberately separate from tool authorization. This module
//! protects the complete MCP route before JSON-RPC parsing, discovery, or tool
//! dispatch. Local development mode requires request-time
//! `ConnectInfo<McpConnectionInfo>` produced from the actual accepted TCP stream
//! and fails closed unless both the peer and local listener match the validated
//! loopback boundary. This module never logs or serializes configured bearer-token
//! values or rejected socket addresses.

use std::{fmt, net::SocketAddr, sync::Arc};

use anyhow::{anyhow, bail};
use axum::{extract::connect_info::Connected, serve::IncomingStream};
#[cfg(feature = "mcp-runtime")]
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
#[cfg(feature = "mcp-runtime")]
use serde_json::json;

use crate::config::{AuthConfig, AuthPosture};

/// Upper bound for both configured and presented bearer tokens.
///
/// The limit prevents an attacker-controlled Authorization header from causing
/// unbounded comparison work while remaining far above normal generated tokens.
pub const MAX_BEARER_TOKEN_BYTES: usize = 4_096;

/// Opaque connection metadata required by unauthenticated loopback mode.
///
/// Axum derives this value directly from each accepted TCP stream. Its fields
/// intentionally remain private so downstream embeddings cannot synthesize a
/// trusted peer/local-address pair. Serve an MCP router with
/// `into_make_service_with_connect_info::<McpConnectionInfo>()`.
///
/// ```compile_fail
/// use std::net::SocketAddr;
/// use termux_mcp_server::auth::McpConnectionInfo;
///
/// let peer_address = SocketAddr::from(([127, 0, 0, 1], 40_000));
/// let _forged = McpConnectionInfo {
///     peer_address,
///     local_address: Some(peer_address),
/// };
/// ```
#[derive(Clone)]
#[cfg_attr(not(feature = "mcp-runtime"), allow(dead_code))]
pub struct McpConnectionInfo {
    peer_address: SocketAddr,
    local_address: Option<SocketAddr>,
}

impl Connected<IncomingStream<'_, tokio::net::TcpListener>> for McpConnectionInfo {
    fn connect_info(stream: IncomingStream<'_, tokio::net::TcpListener>) -> Self {
        Self {
            peer_address: *stream.remote_addr(),
            local_address: stream.io().local_addr().ok(),
        }
    }
}

impl fmt::Debug for McpConnectionInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpConnectionInfo")
            .field("peer_address", &"<redacted>")
            .field("local_address", &"<redacted>")
            .finish()
    }
}

#[derive(Clone)]
pub struct McpAuthPolicy {
    kind: McpAuthPolicyKind,
}

#[derive(Clone)]
enum McpAuthPolicyKind {
    StaticBearer { token: Arc<str> },
    UnauthenticatedLocalhostOnly,
}

impl McpAuthPolicy {
    pub fn from_config(auth: &AuthConfig, posture: AuthPosture) -> anyhow::Result<Self> {
        match posture {
            AuthPosture::StaticTokenConfigured => {
                let token = auth.static_token.as_deref().ok_or_else(|| {
                    anyhow!("static-token auth posture requires a configured token")
                })?;
                Self::static_bearer(token)
            }
            AuthPosture::UnauthenticatedLocalhostOnly => Ok(Self::unauthenticated_localhost_only()),
        }
    }

    pub fn static_bearer(token: impl AsRef<str>) -> anyhow::Result<Self> {
        let token = token.as_ref();
        validate_bearer_token(token)?;

        Ok(Self {
            kind: McpAuthPolicyKind::StaticBearer {
                token: Arc::from(token),
            },
        })
    }

    pub const fn unauthenticated_localhost_only() -> Self {
        Self {
            kind: McpAuthPolicyKind::UnauthenticatedLocalhostOnly,
        }
    }

    #[cfg(feature = "mcp-runtime")]
    pub(crate) const fn is_unauthenticated_localhost_only(&self) -> bool {
        matches!(&self.kind, McpAuthPolicyKind::UnauthenticatedLocalhostOnly)
    }

    /// Return the exact authenticated principal for startup-only authority
    /// compatibility checks. The value is never serialized or logged.
    #[cfg(feature = "mcp-runtime")]
    pub(crate) fn static_principal(&self) -> Option<&str> {
        match &self.kind {
            McpAuthPolicyKind::StaticBearer { token } => Some(token),
            McpAuthPolicyKind::UnauthenticatedLocalhostOnly => None,
        }
    }
}

impl fmt::Debug for McpAuthPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            McpAuthPolicyKind::StaticBearer { token: _token } => formatter
                .debug_struct("McpAuthPolicy::StaticBearer")
                .field("token", &"<redacted>")
                .finish(),
            McpAuthPolicyKind::UnauthenticatedLocalhostOnly => {
                formatter.write_str("McpAuthPolicy::UnauthenticatedLocalhostOnly")
            }
        }
    }
}

#[cfg(feature = "mcp-runtime")]
#[derive(Clone)]
pub(crate) struct McpAuthBoundary {
    policy: McpAuthPolicy,
    expected_listener_address: SocketAddr,
}

#[cfg(feature = "mcp-runtime")]
impl McpAuthBoundary {
    pub(crate) const fn new(policy: McpAuthPolicy, expected_listener_address: SocketAddr) -> Self {
        Self {
            policy,
            expected_listener_address,
        }
    }
}

#[cfg(feature = "mcp-runtime")]
impl fmt::Debug for McpAuthBoundary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpAuthBoundary")
            .field("policy", &self.policy)
            .field("expected_listener_address", &"<redacted>")
            .finish()
    }
}

/// Enforce MCP request authentication before transport parsing or dispatch.
///
/// `UnauthenticatedLocalhostOnly` is not a blanket bypass: it accepts only a
/// request carrying opaque Axum connection metadata produced from the actual
/// accepted TCP stream. The peer must be loopback and the stream's local address
/// must exactly equal the listener validated by `McpRouterBuilder`.
#[cfg(feature = "mcp-runtime")]
pub(crate) async fn require_mcp_auth(
    State(boundary): State<McpAuthBoundary>,
    request: Request,
    next: Next,
) -> Response {
    match &boundary.policy.kind {
        McpAuthPolicyKind::UnauthenticatedLocalhostOnly => {
            let connection_matches_boundary = request
                .extensions()
                .get::<ConnectInfo<McpConnectionInfo>>()
                .is_some_and(|ConnectInfo(connection)| {
                    connection.peer_address.ip().is_loopback()
                        && connection.local_address == Some(boundary.expected_listener_address)
                });

            if connection_matches_boundary {
                next.run(request).await
            } else {
                localhost_peer_required_response()
            }
        }
        McpAuthPolicyKind::StaticBearer { token } => {
            let authorized = extract_bearer_token(request.headers())
                .is_some_and(|provided| constant_time_eq(provided.as_bytes(), token.as_bytes()));

            if authorized {
                next.run(request).await
            } else {
                unauthorized_response()
            }
        }
    }
}

pub(crate) fn validate_bearer_token(token: &str) -> anyhow::Result<()> {
    if token.is_empty() {
        bail!("configured bearer token must not be empty");
    }
    if token.len() > MAX_BEARER_TOKEN_BYTES {
        bail!("configured bearer token exceeds the {MAX_BEARER_TOKEN_BYTES}-byte safety limit");
    }
    if !token.bytes().all(|byte| byte.is_ascii_graphic()) {
        bail!("configured bearer token must contain only ASCII graphic bytes");
    }

    Ok(())
}

#[cfg(feature = "mcp-runtime")]
fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }

    let value = value.to_str().ok()?;
    if value.len() > "Bearer ".len() + MAX_BEARER_TOKEN_BYTES {
        return None;
    }

    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer") || validate_bearer_token(token).is_err() {
        return None;
    }

    Some(token)
}

/// Compare two already-bounded bearer-token byte slices using fixed work.
///
/// Every accepted input is at most `MAX_BEARER_TOKEN_BYTES`, so iterating the
/// full bound prevents the comparison loop count from revealing either token
/// length. Length equality is folded into the accumulated difference.
#[cfg(feature = "mcp-runtime")]
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();

    for index in 0..MAX_BEARER_TOKEN_BYTES {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }

    difference == 0
}

#[cfg(feature = "mcp-runtime")]
fn unauthorized_response() -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "unauthorized",
            "message": "Valid bearer authentication is required for the MCP transport."
        })),
    )
        .into_response();

    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(feature = "mcp-runtime")]
fn localhost_peer_required_response() -> Response {
    let mut response = (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "localhost_peer_required",
            "message": "Unauthenticated development access requires the validated loopback listener and a loopback network peer."
        })),
    )
        .into_response();

    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(all(test, feature = "mcp-runtime"))]
mod tests {
    use std::net::SocketAddr;

    use axum::{
        body::{to_bytes, Body},
        extract::ConnectInfo,
        http::Request as HttpRequest,
        middleware,
        routing::post,
        Router,
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    fn expected_listener() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 8_000))
    }

    async fn call(policy: McpAuthPolicy, authorization: Option<&str>) -> Response {
        call_with_authorizations(policy, authorization.into_iter()).await
    }

    async fn call_with_authorizations<'a>(
        policy: McpAuthPolicy,
        authorizations: impl IntoIterator<Item = &'a str>,
    ) -> Response {
        let app = Router::new()
            .route("/mcp", post(|| async { "authorized" }))
            .route_layer(middleware::from_fn_with_state(
                McpAuthBoundary::new(policy, expected_listener()),
                require_mcp_auth,
            ));

        let mut request = HttpRequest::post("/mcp").body(Body::empty()).unwrap();
        for authorization in authorizations {
            request.headers_mut().append(
                header::AUTHORIZATION,
                HeaderValue::try_from(authorization).unwrap(),
            );
        }

        app.oneshot(request).await.unwrap()
    }

    async fn call_local_development_connection(
        expected_listener_address: SocketAddr,
        connection: Option<McpConnectionInfo>,
    ) -> Response {
        let app = Router::new()
            .route("/mcp", post(|| async { "authorized" }))
            .route_layer(middleware::from_fn_with_state(
                McpAuthBoundary::new(
                    McpAuthPolicy::unauthenticated_localhost_only(),
                    expected_listener_address,
                ),
                require_mcp_auth,
            ));
        let mut request = HttpRequest::post("/mcp").body(Body::empty()).unwrap();
        if let Some(connection) = connection {
            request.extensions_mut().insert(ConnectInfo(connection));
        }

        app.oneshot(request).await.unwrap()
    }

    async fn json_body(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn missing_authorization_is_rejected_without_sensitive_details() {
        let response = call(McpAuthPolicy::static_bearer("secret-value").unwrap(), None).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers().get(header::WWW_AUTHENTICATE),
            Some(&HeaderValue::from_static("Bearer"))
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store"))
        );
        let payload = json_body(response).await;
        assert_eq!(payload["error"], "unauthorized");
        assert!(!payload.to_string().contains("secret-value"));
    }

    #[tokio::test]
    async fn malformed_and_incorrect_authorization_are_rejected() {
        for authorization in [
            "Basic abc",
            "Bearer",
            "Bearer ",
            "Bearer wrong-value",
            "Bearer wrong value",
        ] {
            let response = call(
                McpAuthPolicy::static_bearer("expected-value").unwrap(),
                Some(authorization),
            )
            .await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn correct_bearer_token_is_allowed() {
        let response = call(
            McpAuthPolicy::static_bearer("expected-value").unwrap(),
            Some("Bearer expected-value"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn duplicate_authorization_headers_are_rejected_even_when_each_token_is_correct() {
        let response = call_with_authorizations(
            McpAuthPolicy::static_bearer("expected-value").unwrap(),
            ["Bearer expected-value", "Bearer expected-value"],
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn exact_maximum_length_bearer_token_is_allowed() {
        let token = "x".repeat(MAX_BEARER_TOKEN_BYTES);
        let authorization = format!("Bearer {token}");
        let response = call(
            McpAuthPolicy::static_bearer(&token).unwrap(),
            Some(&authorization),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn bearer_scheme_is_case_insensitive() {
        let response = call(
            McpAuthPolicy::static_bearer("expected-value").unwrap(),
            Some("bearer expected-value"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn localhost_development_policy_allows_exact_loopback_connections_without_header() {
        for (expected_listener_address, peer_address) in [
            (
                SocketAddr::from(([127, 0, 0, 1], 8_000)),
                SocketAddr::from(([127, 0, 0, 1], 40_000)),
            ),
            (
                SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], 8_001)),
                SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], 40_001)),
            ),
        ] {
            let response = call_local_development_connection(
                expected_listener_address,
                Some(McpConnectionInfo {
                    peer_address,
                    local_address: Some(expected_listener_address),
                }),
            )
            .await;

            assert_eq!(response.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn localhost_development_policy_fails_closed_without_connection_info() {
        let response = call_local_development_connection(expected_listener(), None).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store"))
        );
        let payload = json_body(response).await;
        assert_eq!(payload["error"], "localhost_peer_required");
    }

    #[tokio::test]
    async fn localhost_development_policy_rejects_non_loopback_peer() {
        let expected_listener_address = expected_listener();
        let response = call_local_development_connection(
            expected_listener_address,
            Some(McpConnectionInfo {
                peer_address: SocketAddr::from(([192, 0, 2, 10], 40_002)),
                local_address: Some(expected_listener_address),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let payload = json_body(response).await;
        assert_eq!(payload["error"], "localhost_peer_required");
        assert!(!payload.to_string().contains("192.0.2.10"));
    }

    #[tokio::test]
    async fn localhost_development_policy_rejects_missing_or_substituted_local_listener() {
        let expected_listener_address = expected_listener();
        for local_address in [
            None,
            Some(SocketAddr::from(([127, 0, 0, 1], 8_001))),
            Some(SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], 8_000))),
        ] {
            let response = call_local_development_connection(
                expected_listener_address,
                Some(McpConnectionInfo {
                    peer_address: SocketAddr::from(([127, 0, 0, 1], 40_003)),
                    local_address,
                }),
            )
            .await;

            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            let payload = json_body(response).await;
            assert_eq!(payload["error"], "localhost_peer_required");
            for sensitive in ["127.0.0.1", "::1", "8001"] {
                assert!(!payload.to_string().contains(sensitive));
            }
        }
    }

    #[test]
    fn fixed_work_comparison_matches_only_equal_tokens() {
        assert!(constant_time_eq(b"same-value", b"same-value"));
        assert!(!constant_time_eq(b"same-value", b"different"));
        assert!(!constant_time_eq(b"short", b"shorter"));
        assert!(!constant_time_eq(b"shorter", b"short"));
    }

    #[test]
    fn authentication_debug_output_redacts_tokens_and_socket_addresses() {
        let policy = McpAuthPolicy::static_bearer("secret-value").unwrap();
        let boundary =
            McpAuthBoundary::new(policy.clone(), SocketAddr::from(([127, 0, 0, 1], 54_321)));
        let connection = McpConnectionInfo {
            peer_address: SocketAddr::from(([127, 0, 0, 1], 45_678)),
            local_address: Some(SocketAddr::from(([127, 0, 0, 1], 54_321))),
        };
        let diagnostics = format!("{policy:?} {boundary:?} {connection:?}");

        assert!(diagnostics.contains("<redacted>"));
        for sensitive in ["secret-value", "127.0.0.1", "45678", "54321"] {
            assert!(!diagnostics.contains(sensitive));
        }
    }

    #[test]
    fn invalid_configured_tokens_are_rejected() {
        assert!(McpAuthPolicy::static_bearer("").is_err());
        assert!(McpAuthPolicy::static_bearer("contains whitespace").is_err());
        assert!(McpAuthPolicy::static_bearer("contains\u{7f}control").is_err());
        assert!(McpAuthPolicy::static_bearer("contains-non-ascii-é").is_err());
        assert!(McpAuthPolicy::static_bearer("x".repeat(MAX_BEARER_TOKEN_BYTES + 1)).is_err());
    }
}
