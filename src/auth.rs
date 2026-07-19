//! Request authentication for the staged MCP transport.
//!
//! Authentication is deliberately separate from tool authorization. This module
//! protects the complete MCP route before JSON-RPC parsing, discovery, or tool
//! dispatch. Local development mode requires request-time
//! `ConnectInfo<SocketAddr>` for an actual loopback peer and fails closed when
//! peer metadata is absent. This module never logs or serializes configured
//! bearer-token values or rejected peer addresses.

use std::{fmt, net::SocketAddr, sync::Arc};

use anyhow::{anyhow, bail};
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::config::{AuthConfig, AuthPosture};

/// Upper bound for both configured and presented bearer tokens.
///
/// The limit prevents an attacker-controlled Authorization header from causing
/// unbounded comparison work while remaining far above normal generated tokens.
pub const MAX_BEARER_TOKEN_BYTES: usize = 4_096;

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
            McpAuthPolicyKind::StaticBearer { .. } => formatter
                .debug_struct("McpAuthPolicy::StaticBearer")
                .field("token", &"<redacted>")
                .finish(),
            McpAuthPolicyKind::UnauthenticatedLocalhostOnly => {
                formatter.write_str("McpAuthPolicy::UnauthenticatedLocalhostOnly")
            }
        }
    }
}

/// Enforce MCP request authentication before transport parsing or dispatch.
///
/// `UnauthenticatedLocalhostOnly` is not a blanket bypass: it accepts only a
/// request carrying Axum connection metadata for an actual IPv4 or IPv6
/// loopback peer. Applications serving this middleware must use a make-service
/// that inserts `ConnectInfo<SocketAddr>`.
pub async fn require_mcp_auth(
    State(policy): State<McpAuthPolicy>,
    request: Request,
    next: Next,
) -> Response {
    match &policy.kind {
        McpAuthPolicyKind::UnauthenticatedLocalhostOnly => {
            let peer_is_loopback = request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .is_some_and(|ConnectInfo(peer)| peer.ip().is_loopback());

            if peer_is_loopback {
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
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();

    for index in 0..MAX_BEARER_TOKEN_BYTES {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }

    difference == 0
}

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

fn localhost_peer_required_response() -> Response {
    let mut response = (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "localhost_peer_required",
            "message": "Unauthenticated development access requires a loopback network peer."
        })),
    )
        .into_response();

    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
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

    async fn call(policy: McpAuthPolicy, authorization: Option<&str>) -> Response {
        call_with_authorizations(policy, authorization.into_iter()).await
    }

    async fn call_with_authorizations<'a>(
        policy: McpAuthPolicy,
        authorizations: impl IntoIterator<Item = &'a str>,
    ) -> Response {
        let app = Router::new()
            .route("/mcp", post(|| async { "authorized" }))
            .route_layer(middleware::from_fn_with_state(policy, require_mcp_auth));

        let mut request = HttpRequest::post("/mcp").body(Body::empty()).unwrap();
        for authorization in authorizations {
            request.headers_mut().append(
                header::AUTHORIZATION,
                HeaderValue::try_from(authorization).unwrap(),
            );
        }

        app.oneshot(request).await.unwrap()
    }

    async fn call_local_development_peer(peer: Option<SocketAddr>) -> Response {
        let app = Router::new()
            .route("/mcp", post(|| async { "authorized" }))
            .route_layer(middleware::from_fn_with_state(
                McpAuthPolicy::unauthenticated_localhost_only(),
                require_mcp_auth,
            ));
        let mut request = HttpRequest::post("/mcp").body(Body::empty()).unwrap();
        if let Some(peer) = peer {
            request.extensions_mut().insert(ConnectInfo(peer));
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
    async fn localhost_development_policy_allows_loopback_peer_without_header() {
        for peer in [
            SocketAddr::from(([127, 0, 0, 1], 40_000)),
            SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], 40_001)),
        ] {
            let response = call_local_development_peer(Some(peer)).await;

            assert_eq!(response.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn localhost_development_policy_fails_closed_without_peer_connect_info() {
        let response = call_local_development_peer(None).await;

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
        let response =
            call_local_development_peer(Some(SocketAddr::from(([192, 0, 2, 10], 40_002)))).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let payload = json_body(response).await;
        assert_eq!(payload["error"], "localhost_peer_required");
        assert!(!payload.to_string().contains("192.0.2.10"));
    }

    #[test]
    fn fixed_work_comparison_matches_only_equal_tokens() {
        assert!(constant_time_eq(b"same-value", b"same-value"));
        assert!(!constant_time_eq(b"same-value", b"different"));
        assert!(!constant_time_eq(b"short", b"shorter"));
        assert!(!constant_time_eq(b"shorter", b"short"));
    }

    #[test]
    fn policy_debug_output_redacts_static_token() {
        let policy = McpAuthPolicy::static_bearer("secret-value").unwrap();
        let debug = format!("{policy:?}");

        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret-value"));
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
