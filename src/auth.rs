//! Request authentication for the staged MCP transport.
//!
//! Authentication is deliberately separate from tool authorization. This module
//! protects the complete MCP route before JSON-RPC parsing, discovery, or tool
//! dispatch. It never logs or serializes configured bearer-token or principal values.

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
#[cfg(feature = "mcp-runtime")]
use crate::directory_grant::{
    has_directory_grant_header, take_directory_grant_authorization, DirectoryGrantAuthorization,
};

/// Upper bound for both configured and presented bearer tokens.
///
/// The limit prevents an attacker-controlled Authorization header from causing
/// unbounded comparison work while remaining far above normal generated tokens.
pub const MAX_BEARER_TOKEN_BYTES: usize = 4_096;
/// Upper bound for a trusted, configured, non-secret principal identifier.
pub const MAX_PRINCIPAL_ID_BYTES: usize = 128;

/// Server-derived authenticated identity carried only in request extensions.
///
/// The value is intentionally opaque outside the authentication/session boundary,
/// never parsed from caller-controlled headers, and redacted from Debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    stable_id: Arc<str>,
}

impl AuthenticatedPrincipal {
    pub fn configured(stable_id: impl AsRef<str>) -> anyhow::Result<Self> {
        let stable_id = stable_id.as_ref();
        validate_principal_id(stable_id)?;
        Ok(Self {
            stable_id: Arc::from(stable_id),
        })
    }

    #[cfg(feature = "mcp-runtime")]
    pub(crate) fn stable_id(&self) -> &str {
        &self.stable_id
    }
}

impl fmt::Debug for AuthenticatedPrincipal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedPrincipal(<redacted>)")
    }
}

#[derive(Clone)]
pub enum McpAuthPolicy {
    StaticBearer {
        token: Arc<str>,
        principal: Option<AuthenticatedPrincipal>,
    },
    UnauthenticatedLocalhostOnly,
}

impl McpAuthPolicy {
    pub fn from_config(auth: &AuthConfig, posture: AuthPosture) -> anyhow::Result<Self> {
        match posture {
            AuthPosture::StaticTokenConfigured => {
                let token = auth.static_token.as_deref().ok_or_else(|| {
                    anyhow!("static-token auth posture requires a configured token")
                })?;
                match auth.static_principal_id.as_deref() {
                    Some(principal) => Self::static_bearer_for_principal(token, principal),
                    None => Self::static_bearer(token),
                }
            }
            AuthPosture::UnauthenticatedLocalhostOnly => Ok(Self::UnauthenticatedLocalhostOnly),
        }
    }

    pub fn static_bearer(token: impl AsRef<str>) -> anyhow::Result<Self> {
        let token = token.as_ref();
        validate_bearer_token(token)?;

        Ok(Self::StaticBearer {
            token: Arc::from(token),
            principal: None,
        })
    }

    pub fn static_bearer_for_principal(
        token: impl AsRef<str>,
        principal: impl AsRef<str>,
    ) -> anyhow::Result<Self> {
        let token = token.as_ref();
        let principal = principal.as_ref();
        validate_bearer_token(token)?;
        validate_principal_id(principal)?;
        if token == principal {
            bail!("configured principal identity must not equal the bearer credential");
        }

        Ok(Self::StaticBearer {
            token: Arc::from(token),
            principal: Some(AuthenticatedPrincipal::configured(principal)?),
        })
    }

    pub const fn unauthenticated_localhost_only() -> Self {
        Self::UnauthenticatedLocalhostOnly
    }

    pub fn has_stable_principal(&self) -> bool {
        matches!(
            self,
            Self::StaticBearer {
                principal: Some(_),
                ..
            }
        )
    }
}

impl fmt::Debug for McpAuthPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaticBearer { principal, .. } => formatter
                .debug_struct("McpAuthPolicy::StaticBearer")
                .field("token", &"<redacted>")
                .field("principal_configured", &principal.is_some())
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
    mut request: Request,
    next: Next,
) -> Response {
    match &policy {
        McpAuthPolicy::UnauthenticatedLocalhostOnly => {
            #[cfg(feature = "mcp-runtime")]
            if has_directory_grant_header(request.headers()) {
                request
                    .headers_mut()
                    .remove(crate::directory_grant::MCP_DIRECTORY_GRANT_HEADER);
                return authorization_context_response(
                    StatusCode::FORBIDDEN,
                    "authorization_context_requires_authentication",
                    "Capability-grant authorization context requires authenticated transport.",
                );
            }
            next.run(request).await
        }
        McpAuthPolicy::StaticBearer { token, principal } => {
            let authorized = extract_bearer_token(request.headers())
                .is_some_and(|provided| constant_time_eq(provided.as_bytes(), token.as_bytes()));

            if authorized {
                #[cfg(feature = "mcp-runtime")]
                let grant: Option<DirectoryGrantAuthorization> =
                    match take_directory_grant_authorization(request.headers_mut()) {
                        Ok(grant) => grant,
                        Err(_) => {
                            return authorization_context_response(
                                StatusCode::BAD_REQUEST,
                                "invalid_authorization_context",
                                "Capability-grant authorization context is malformed.",
                            );
                        }
                    };
                if let Some(principal) = principal {
                    request.extensions_mut().insert(principal.clone());
                }
                #[cfg(feature = "mcp-runtime")]
                if let Some(grant) = grant {
                    request.extensions_mut().insert(grant);
                }
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
        bail!("configured bearer token exceeds the {MAX_BEARER_TOKEN_BYTES}-byte safety limit");
    }
    if token.chars().any(char::is_whitespace) {
        bail!("configured bearer token must not contain whitespace");
    }

    Ok(())
}

fn validate_principal_id(stable_id: &str) -> anyhow::Result<()> {
    if stable_id.is_empty() {
        bail!("configured principal identity must not be empty");
    }
    if stable_id.len() > MAX_PRINCIPAL_ID_BYTES {
        bail!(
            "configured principal identity exceeds the {MAX_PRINCIPAL_ID_BYTES}-byte safety limit"
        );
    }
    if !stable_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        bail!(
            "configured principal identity may contain only ASCII letters, digits, '.', '_', '-', and ':'"
        );
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

#[cfg(feature = "mcp-runtime")]
fn authorization_context_response(
    status: StatusCode,
    error: &'static str,
    message: &'static str,
) -> Response {
    let mut response = (status, Json(json!({"error": error, "message": message}))).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
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

#[cfg(test)]
mod tests {
    use axum::{
        body::{to_bytes, Body},
        extract::Extension,
        http::Request as HttpRequest,
        middleware,
        routing::post,
        Router,
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    async fn principal_status(
        principal: Option<Extension<AuthenticatedPrincipal>>,
    ) -> &'static str {
        if principal.is_some() {
            "principal-bound"
        } else {
            "principal-unbound"
        }
    }

    async fn call(
        policy: McpAuthPolicy,
        authorization: Option<&str>,
        spoofed_principal_header: Option<&str>,
    ) -> Response {
        let app = Router::new()
            .route("/mcp", post(principal_status))
            .route_layer(middleware::from_fn_with_state(policy, require_mcp_auth));

        let mut request = HttpRequest::post("/mcp");
        if let Some(authorization) = authorization {
            request = request.header(header::AUTHORIZATION, authorization);
        }
        if let Some(spoofed_principal_header) = spoofed_principal_header {
            request = request.header("x-mcp-principal-id", spoofed_principal_header);
        }

        app.oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn json_body(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn text_body(response: Response) -> String {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn missing_authorization_is_rejected_without_sensitive_details() {
        let response = call(
            McpAuthPolicy::static_bearer("secret-value").unwrap(),
            None,
            None,
        )
        .await;

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
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn correct_bearer_token_is_allowed_without_implicit_shared_principal() {
        let response = call(
            McpAuthPolicy::static_bearer("expected-value").unwrap(),
            Some("Bearer expected-value"),
            None,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(text_body(response).await, "principal-unbound");
    }

    #[tokio::test]
    async fn configured_principal_is_inserted_only_after_successful_authentication() {
        let policy =
            McpAuthPolicy::static_bearer_for_principal("expected-value", "operator.primary:v1")
                .unwrap();
        assert!(policy.has_stable_principal());

        let response = call(
            policy.clone(),
            Some("Bearer expected-value"),
            Some("caller-selected-principal"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(text_body(response).await, "principal-bound");

        let rejected = call(
            policy,
            Some("Bearer wrong-value"),
            Some("operator.primary:v1"),
        )
        .await;
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn caller_principal_header_is_never_an_identity_source() {
        let response = call(
            McpAuthPolicy::static_bearer("expected-value").unwrap(),
            Some("Bearer expected-value"),
            Some("operator.primary:v1"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(text_body(response).await, "principal-unbound");
    }

    #[tokio::test]
    async fn bearer_scheme_is_case_insensitive() {
        let response = call(
            McpAuthPolicy::static_bearer("expected-value").unwrap(),
            Some("bearer expected-value"),
            None,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn localhost_development_policy_allows_request_without_header() {
        let response = call(McpAuthPolicy::unauthenticated_localhost_only(), None, None).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(text_body(response).await, "principal-unbound");
    }

    #[cfg(feature = "mcp-runtime")]
    #[tokio::test]
    async fn grant_context_requires_authentication_and_is_removed_before_dispatch() {
        use crate::directory_grant::{DirectoryGrantAuthorization, MCP_DIRECTORY_GRANT_HEADER};

        async fn grant_status(
            grant: Option<Extension<DirectoryGrantAuthorization>>,
            headers: HeaderMap,
        ) -> &'static str {
            match (
                grant.is_some(),
                headers.contains_key(MCP_DIRECTORY_GRANT_HEADER),
            ) {
                (true, false) => "opaque-grant-only",
                _ => "grant-boundary-failed",
            }
        }

        let app = Router::new().route("/mcp", post(grant_status)).route_layer(
            middleware::from_fn_with_state(
                McpAuthPolicy::static_bearer_for_principal("expected-value", "operator.primary:v1")
                    .unwrap(),
                require_mcp_auth,
            ),
        );
        let response = app
            .oneshot(
                HttpRequest::post("/mcp")
                    .header(header::AUTHORIZATION, "Bearer expected-value")
                    .header(MCP_DIRECTORY_GRANT_HEADER, "canonical.grant.value")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(text_body(response).await, "opaque-grant-only");

        let unauthenticated = Router::new().route("/mcp", post(grant_status)).route_layer(
            middleware::from_fn_with_state(
                McpAuthPolicy::unauthenticated_localhost_only(),
                require_mcp_auth,
            ),
        );
        let response = unauthenticated
            .oneshot(
                HttpRequest::post("/mcp")
                    .header(MCP_DIRECTORY_GRANT_HEADER, "canonical.grant.value")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[cfg(feature = "mcp-runtime")]
    #[tokio::test]
    async fn bearer_authentication_precedes_grant_context_validation() {
        use crate::directory_grant::MCP_DIRECTORY_GRANT_HEADER;

        let response = call(
            McpAuthPolicy::static_bearer("expected-value").unwrap(),
            Some("Bearer wrong-value"),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let app = Router::new()
            .route("/mcp", post(principal_status))
            .route_layer(middleware::from_fn_with_state(
                McpAuthPolicy::static_bearer("expected-value").unwrap(),
                require_mcp_auth,
            ));
        let response = app
            .oneshot(
                HttpRequest::post("/mcp")
                    .header(header::AUTHORIZATION, "Bearer wrong-value")
                    .header(MCP_DIRECTORY_GRANT_HEADER, "contains whitespace")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn fixed_work_comparison_matches_only_equal_tokens() {
        assert!(constant_time_eq(b"same-value", b"same-value"));
        assert!(!constant_time_eq(b"same-value", b"different"));
        assert!(!constant_time_eq(b"short", b"shorter"));
        assert!(!constant_time_eq(b"shorter", b"short"));
    }

    #[test]
    fn policy_debug_output_redacts_static_token_and_principal() {
        let policy =
            McpAuthPolicy::static_bearer_for_principal("secret-value", "operator.primary:v1")
                .unwrap();
        let debug = format!("{policy:?}");

        assert!(debug.contains("<redacted>"));
        assert!(debug.contains("principal_configured: true"));
        assert!(!debug.contains("secret-value"));
        assert!(!debug.contains("operator.primary:v1"));
    }

    #[test]
    fn invalid_configured_tokens_and_principals_are_rejected() {
        assert!(McpAuthPolicy::static_bearer("").is_err());
        assert!(McpAuthPolicy::static_bearer("contains whitespace").is_err());
        assert!(McpAuthPolicy::static_bearer("x".repeat(MAX_BEARER_TOKEN_BYTES + 1)).is_err());

        for principal in ["", "contains whitespace", "slash/not-allowed", "é"] {
            assert!(McpAuthPolicy::static_bearer_for_principal("token", principal).is_err());
        }
        assert!(McpAuthPolicy::static_bearer_for_principal(
            "token",
            "x".repeat(MAX_PRINCIPAL_ID_BYTES + 1)
        )
        .is_err());
        assert!(McpAuthPolicy::static_bearer_for_principal("same", "same").is_err());
    }
}
