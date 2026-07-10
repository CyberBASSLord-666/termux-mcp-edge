//! Request authentication for the staged MCP transport.
//!
//! Authentication is deliberately separate from tool authorization. This module
//! protects the complete MCP route before JSON-RPC parsing, discovery, or tool
//! dispatch. It never logs or serializes configured bearer-token values.

use std::{fmt, sync::Arc};

use anyhow::{anyhow, bail};
use axum::{
    extract::{Request, State},
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
pub enum McpAuthPolicy {
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
            AuthPosture::UnauthenticatedLocalhostOnly => {
                Ok(Self::UnauthenticatedLocalhostOnly)
            }
        }
    }

    pub fn static_bearer(token: impl AsRef<str>) -> anyhow::Result<Self> {
        let token = token.as_ref();
        validate_bearer_token(token)?;

        Ok(Self::StaticBearer {
            token: Arc::from(token),
        })
    }

    pub const fn unauthenticated_localhost_only() -> Self {
        Self::UnauthenticatedLocalhostOnly
    }
}

impl fmt::Debug for McpAuthPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaticBearer { .. } => formatter
                .debug_struct("McpAuthPolicy::StaticBearer")
                .field("token", &"<redacted>")
                .finish(),
            Self::UnauthenticatedLocalhostOnly => {
                formatter.write_str("McpAuthPolicy::UnauthenticatedLocalhostOnly")
            }
        }
    }
}

/// Enforce MCP request authentication before transport parsing or dispatch.
pub async fn require_mcp_auth(
    State(policy): State<McpAuthPolicy>,
    request: Request,
    next: Next,
) -> Response {
    match &policy {
        McpAuthPolicy::UnauthenticatedLocalhostOnly => next.run(request).await,
        McpAuthPolicy::StaticBearer { token } => {
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

fn validate_bearer_token(token: &str) -> anyhow::Result<()> {
    if token.is_empty() {
        bail!("configured bearer token must not be empty");
    }
    if token.len() > MAX_BEARER_TOKEN_BYTES {
        bail!(
            "configured bearer token exceeds the {MAX_BEARER_TOKEN_BYTES}-byte safety limit"
        );
    }
    if token.chars().any(char::is_whitespace) {
        bail!("configured bearer token must not contain whitespace");
    }

    Ok(())
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    if value.len() > "Bearer ".len() + MAX_BEARER_TOKEN_BYTES {
        return None;
    }

    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer")
        || token.is_empty()
        || token.len() > MAX_BEARER_TOKEN_BYTES
        || token.chars().any(char::is_whitespace)
    {
        return None;
    }

    Some(token)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut difference = left.len() ^ right.len();

    for index in 0..max_len {
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

    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Bearer"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{to_bytes, Body},
        http::Request as HttpRequest,
        middleware,
        routing::post,
        Router,
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    async fn call(policy: McpAuthPolicy, authorization: Option<&str>) -> Response {
        let app = Router::new()
            .route("/mcp", post(|| async { "authorized" }))
            .route_layer(middleware::from_fn_with_state(policy, require_mcp_auth));

        let mut request = HttpRequest::post("/mcp");
        if let Some(authorization) = authorization {
            request = request.header(header::AUTHORIZATION, authorization);
        }

        app.oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
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
    async fn bearer_scheme_is_case_insensitive() {
        let response = call(
            McpAuthPolicy::static_bearer("expected-value").unwrap(),
            Some("bearer expected-value"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn localhost_development_policy_allows_request_without_header() {
        let response = call(McpAuthPolicy::unauthenticated_localhost_only(), None).await;

        assert_eq!(response.status(), StatusCode::OK);
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
        assert!(McpAuthPolicy::static_bearer("x".repeat(MAX_BEARER_TOKEN_BYTES + 1)).is_err());
    }
}
