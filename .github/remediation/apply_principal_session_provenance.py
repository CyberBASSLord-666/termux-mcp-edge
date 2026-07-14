#!/usr/bin/env python3
from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}: {old[:140]!r}")
    path.write_text(text.replace(old, new, 1))


Path("src/auth.rs").write_text(r'''//! Request authentication for the staged MCP transport.
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
        McpAuthPolicy::UnauthenticatedLocalhostOnly => next.run(request).await,
        McpAuthPolicy::StaticBearer { token, principal } => {
            let authorized = extract_bearer_token(request.headers())
                .is_some_and(|provided| constant_time_eq(provided.as_bytes(), token.as_bytes()));

            if authorized {
                if let Some(principal) = principal {
                    request.extensions_mut().insert(principal.clone());
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
    if !stable_id.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
    }) {
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
        let policy = McpAuthPolicy::static_bearer_for_principal(
            "expected-value",
            "operator.primary:v1",
        )
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

    #[test]
    fn fixed_work_comparison_matches_only_equal_tokens() {
        assert!(constant_time_eq(b"same-value", b"same-value"));
        assert!(!constant_time_eq(b"same-value", b"different"));
        assert!(!constant_time_eq(b"short", b"shorter"));
        assert!(!constant_time_eq(b"shorter", b"short"));
    }

    #[test]
    fn policy_debug_output_redacts_static_token_and_principal() {
        let policy = McpAuthPolicy::static_bearer_for_principal(
            "secret-value",
            "operator.primary:v1",
        )
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
''')

Path("src/mcp_session.rs").write_text(r'''//! Bounded in-memory lifecycle state for Streamable HTTP MCP sessions.
//!
//! Session records retain only a random identifier, lifecycle phase, last-activity
//! timestamp, and an optional server-derived authenticated-principal association.
//! Client-provided initialization metadata and raw credentials are never stored here.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use uuid::Uuid;

use crate::auth::AuthenticatedPrincipal;

pub(crate) const MAX_MCP_SESSIONS: usize = 64;
pub(crate) const MCP_SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionPhase {
    AwaitingInitialized,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionStoreError {
    CapacityExhausted,
    NotFound,
    PrincipalMismatch,
    Poisoned,
}

#[derive(Clone)]
pub(crate) struct McpSessionStore {
    inner: Arc<Mutex<SessionRegistry>>,
    max_sessions: usize,
    idle_timeout: Duration,
}

#[derive(Default)]
struct SessionRegistry {
    sessions: HashMap<String, SessionRecord>,
}

struct SessionRecord {
    phase: SessionPhase,
    principal: Option<AuthenticatedPrincipal>,
    last_activity: Instant,
}

impl McpSessionStore {
    pub(crate) fn new() -> Self {
        Self::with_limits(MAX_MCP_SESSIONS, MCP_SESSION_IDLE_TIMEOUT)
    }

    fn with_limits(max_sessions: usize, idle_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SessionRegistry::default())),
            max_sessions,
            idle_timeout,
        }
    }

    pub(crate) fn create(
        &self,
        principal: Option<&AuthenticatedPrincipal>,
    ) -> Result<String, SessionStoreError> {
        self.create_at(principal, Instant::now())
    }

    pub(crate) fn phase(
        &self,
        session_id: &str,
        principal: Option<&AuthenticatedPrincipal>,
    ) -> Result<SessionPhase, SessionStoreError> {
        self.phase_at(session_id, principal, Instant::now())
    }

    pub(crate) fn activate(
        &self,
        session_id: &str,
        principal: Option<&AuthenticatedPrincipal>,
    ) -> Result<(), SessionStoreError> {
        self.activate_at(session_id, principal, Instant::now())
    }

    pub(crate) fn terminate(
        &self,
        session_id: &str,
        principal: Option<&AuthenticatedPrincipal>,
    ) -> Result<(), SessionStoreError> {
        self.terminate_at(session_id, principal, Instant::now())
    }

    fn create_at(
        &self,
        principal: Option<&AuthenticatedPrincipal>,
        now: Instant,
    ) -> Result<String, SessionStoreError> {
        let mut registry = self.inner.lock().map_err(|_| SessionStoreError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);

        if registry.sessions.len() >= self.max_sessions {
            return Err(SessionStoreError::CapacityExhausted);
        }

        let session_id = loop {
            let candidate = Uuid::new_v4().to_string();
            if !registry.sessions.contains_key(&candidate) {
                break candidate;
            }
        };

        registry.sessions.insert(
            session_id.clone(),
            SessionRecord {
                phase: SessionPhase::AwaitingInitialized,
                principal: principal.cloned(),
                last_activity: now,
            },
        );
        Ok(session_id)
    }

    fn phase_at(
        &self,
        session_id: &str,
        principal: Option<&AuthenticatedPrincipal>,
        now: Instant,
    ) -> Result<SessionPhase, SessionStoreError> {
        let mut registry = self.inner.lock().map_err(|_| SessionStoreError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);
        let session = registry
            .sessions
            .get_mut(session_id)
            .ok_or(SessionStoreError::NotFound)?;
        validate_principal(session, principal)?;
        session.last_activity = now;
        Ok(session.phase)
    }

    fn activate_at(
        &self,
        session_id: &str,
        principal: Option<&AuthenticatedPrincipal>,
        now: Instant,
    ) -> Result<(), SessionStoreError> {
        let mut registry = self.inner.lock().map_err(|_| SessionStoreError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);
        let session = registry
            .sessions
            .get_mut(session_id)
            .ok_or(SessionStoreError::NotFound)?;
        validate_principal(session, principal)?;
        session.phase = SessionPhase::Active;
        session.last_activity = now;
        Ok(())
    }

    fn terminate_at(
        &self,
        session_id: &str,
        principal: Option<&AuthenticatedPrincipal>,
        now: Instant,
    ) -> Result<(), SessionStoreError> {
        let mut registry = self.inner.lock().map_err(|_| SessionStoreError::Poisoned)?;
        registry.prune_expired(now, self.idle_timeout);
        let session = registry
            .sessions
            .get(session_id)
            .ok_or(SessionStoreError::NotFound)?;
        validate_principal(session, principal)?;
        registry.sessions.remove(session_id);
        Ok(())
    }
}

fn validate_principal(
    session: &SessionRecord,
    presented: Option<&AuthenticatedPrincipal>,
) -> Result<(), SessionStoreError> {
    match (session.principal.as_ref(), presented) {
        (None, None) => Ok(()),
        (Some(expected), Some(presented)) if expected == presented => Ok(()),
        _ => Err(SessionStoreError::PrincipalMismatch),
    }
}

impl SessionRegistry {
    fn prune_expired(&mut self, now: Instant, idle_timeout: Duration) {
        self.sessions.retain(|_, session| {
            now.checked_duration_since(session.last_activity)
                .is_none_or(|idle| idle < idle_timeout)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_visible_ascii_uuid_sessions_in_pending_phase() {
        let store = McpSessionStore::new();
        let session_id = store.create(None).unwrap();

        assert_eq!(
            Uuid::parse_str(&session_id).unwrap().to_string(),
            session_id
        );
        assert!(session_id.bytes().all(|byte| (0x21..=0x7e).contains(&byte)));
        assert_eq!(
            store.phase(&session_id, None).unwrap(),
            SessionPhase::AwaitingInitialized
        );
    }

    #[test]
    fn activation_is_idempotent_and_scoped_to_one_session() {
        let store = McpSessionStore::new();
        let first = store.create(None).unwrap();
        let second = store.create(None).unwrap();

        store.activate(&first, None).unwrap();
        store.activate(&first, None).unwrap();

        assert_eq!(store.phase(&first, None).unwrap(), SessionPhase::Active);
        assert_eq!(
            store.phase(&second, None).unwrap(),
            SessionPhase::AwaitingInitialized
        );
    }

    #[test]
    fn principal_bound_sessions_reject_missing_and_cross_principal_access() {
        let store = McpSessionStore::new();
        let first = AuthenticatedPrincipal::configured("operator.primary:v1").unwrap();
        let second = AuthenticatedPrincipal::configured("operator.secondary:v1").unwrap();
        let session_id = store.create(Some(&first)).unwrap();

        assert_eq!(
            store.phase(&session_id, Some(&first)).unwrap(),
            SessionPhase::AwaitingInitialized
        );
        assert_eq!(
            store.phase(&session_id, None).unwrap_err(),
            SessionStoreError::PrincipalMismatch
        );
        assert_eq!(
            store.phase(&session_id, Some(&second)).unwrap_err(),
            SessionStoreError::PrincipalMismatch
        );
        assert_eq!(
            store.activate(&session_id, Some(&second)).unwrap_err(),
            SessionStoreError::PrincipalMismatch
        );
        assert_eq!(
            store.terminate(&session_id, None).unwrap_err(),
            SessionStoreError::PrincipalMismatch
        );

        store.activate(&session_id, Some(&first)).unwrap();
        assert_eq!(
            store.phase(&session_id, Some(&first)).unwrap(),
            SessionPhase::Active
        );
        store.terminate(&session_id, Some(&first)).unwrap();
    }

    #[test]
    fn unbound_sessions_reject_later_principal_injection() {
        let store = McpSessionStore::new();
        let principal = AuthenticatedPrincipal::configured("operator.primary:v1").unwrap();
        let session_id = store.create(None).unwrap();

        assert_eq!(
            store.phase(&session_id, Some(&principal)).unwrap_err(),
            SessionStoreError::PrincipalMismatch
        );
        assert_eq!(
            store.phase(&session_id, None).unwrap(),
            SessionPhase::AwaitingInitialized
        );
    }

    #[test]
    fn capacity_is_bounded_until_a_session_is_terminated() {
        let store = McpSessionStore::with_limits(2, Duration::from_secs(60));
        let first = store.create(None).unwrap();
        let _second = store.create(None).unwrap();

        assert_eq!(
            store.create(None).unwrap_err(),
            SessionStoreError::CapacityExhausted
        );

        store.terminate(&first, None).unwrap();
        assert!(store.create(None).is_ok());
    }

    #[test]
    fn idle_sessions_expire_and_release_capacity() {
        let start = Instant::now();
        let store = McpSessionStore::with_limits(1, Duration::from_secs(10));
        let expired = store.create_at(None, start).unwrap();

        assert_eq!(
            store
                .phase_at(&expired, None, start + Duration::from_secs(9))
                .unwrap(),
            SessionPhase::AwaitingInitialized
        );
        assert_eq!(
            store
                .create_at(None, start + Duration::from_secs(10))
                .unwrap_err(),
            SessionStoreError::CapacityExhausted
        );

        let replacement = store
            .create_at(None, start + Duration::from_secs(20))
            .unwrap();
        assert_ne!(replacement, expired);
        assert_eq!(
            store
                .phase_at(&expired, None, start + Duration::from_secs(20))
                .unwrap_err(),
            SessionStoreError::NotFound
        );
    }

    #[test]
    fn terminated_and_unknown_sessions_are_not_found() {
        let store = McpSessionStore::new();
        let session_id = store.create(None).unwrap();

        store.terminate(&session_id, None).unwrap();

        assert_eq!(
            store.phase(&session_id, None).unwrap_err(),
            SessionStoreError::NotFound
        );
        assert_eq!(
            store.terminate("not-a-session", None).unwrap_err(),
            SessionStoreError::NotFound
        );
    }
}
''')

Path("tests/mcp_auth.rs").write_text(r'''#![cfg(feature = "mcp-runtime")]

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    middleware,
    response::Response,
    Router,
};
use serde_json::{json, Value};
use termux_mcp_server::{
    auth::{require_mcp_auth, McpAuthPolicy},
    mcp_transport::{
        self, MCP_POST_ACCEPT, MCP_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION_HEADER,
        MCP_SESSION_ID_HEADER,
    },
    tools::FileSystemTools,
    transport_security::TransportSecurityPolicy,
};
use tower::ServiceExt;

fn protected_router(policy: McpAuthPolicy, file_tools: FileSystemTools) -> Router {
    mcp_transport::router(
        TransportSecurityPolicy::localhost(8000, false)
            .expect("test localhost policy must be valid"),
        file_tools,
        false,
        false,
        false,
    )
    .route_layer(middleware::from_fn_with_state(policy, require_mcp_auth))
}

async fn post_tools_list(policy: McpAuthPolicy, authorization: Option<&str>) -> Response {
    let root = tempfile::tempdir().unwrap();
    let file_tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let app = protected_router(policy, file_tools);

    let initialize = authenticated_request(
        json!({
            "jsonrpc": "2.0",
            "id": "auth-initialize",
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "auth-tests", "version": "1.0.0"}
            }
        }),
        authorization,
        None,
        None,
    );
    let initialize_response = app.clone().oneshot(initialize).await.unwrap();
    if initialize_response.status() != StatusCode::OK {
        return initialize_response;
    }
    let session_id = initialize_response
        .headers()
        .get(MCP_SESSION_ID_HEADER)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();

    let initialized = authenticated_request(
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
        authorization,
        Some(&session_id),
        None,
    );
    let initialized_response = app.clone().oneshot(initialized).await.unwrap();
    assert_eq!(initialized_response.status(), StatusCode::ACCEPTED);

    app.oneshot(authenticated_request(
        json!({
            "jsonrpc": "2.0",
            "id": "auth-test",
            "method": "tools/list"
        }),
        authorization,
        Some(&session_id),
        None,
    ))
    .await
    .unwrap()
}

fn authenticated_request(
    body: Value,
    authorization: Option<&str>,
    session_id: Option<&str>,
    spoofed_principal: Option<&str>,
) -> Request<Body> {
    let mut request = Request::post("/mcp")
        .header(header::HOST, "localhost:8000")
        .header(header::ORIGIN, "http://localhost:8000")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, MCP_POST_ACCEPT);
    if let Some(authorization) = authorization {
        request = request.header(header::AUTHORIZATION, authorization);
    }
    if let Some(session_id) = session_id {
        request = request
            .header(MCP_PROTOCOL_VERSION_HEADER, MCP_PROTOCOL_VERSION)
            .header(MCP_SESSION_ID_HEADER, session_id);
    }
    if let Some(spoofed_principal) = spoofed_principal {
        request = request.header("x-mcp-principal-id", spoofed_principal);
    }
    request.body(Body::from(body.to_string())).unwrap()
}

async fn response_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn unauthorized_client_cannot_reach_tool_discovery() {
    let response = post_tools_list(
        McpAuthPolicy::static_bearer("expected-token").unwrap(),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers().get(header::WWW_AUTHENTICATE),
        Some(&header::HeaderValue::from_static("Bearer"))
    );
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "unauthorized");
    assert!(payload.get("result").is_none());
    assert!(!payload.to_string().contains("runtime_status"));
}

#[tokio::test]
async fn correct_bearer_token_reaches_tool_discovery() {
    let response = post_tools_list(
        McpAuthPolicy::static_bearer_for_principal(
            "expected-token",
            "operator.primary:v1",
        )
        .unwrap(),
        Some("Bearer expected-token"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert_eq!(payload["id"], "auth-test");
    let tools = payload["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["name"] == "runtime_status"));
}

#[tokio::test]
async fn caller_selected_principal_header_does_not_create_identity_provenance() {
    let root = tempfile::tempdir().unwrap();
    let file_tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let app = protected_router(
        McpAuthPolicy::static_bearer("expected-token").unwrap(),
        file_tools,
    );
    let initialize = authenticated_request(
        json!({
            "jsonrpc": "2.0",
            "id": "spoofed-principal",
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "auth-tests", "version": "1.0.0"}
            }
        }),
        Some("Bearer expected-token"),
        None,
        Some("operator.primary:v1"),
    );

    let response = app.oneshot(initialize).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn explicit_loopback_development_policy_reaches_discovery_without_header() {
    let response = post_tools_list(McpAuthPolicy::unauthenticated_localhost_only(), None).await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn authentication_rejects_before_transport_validation_or_body_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let file_tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let app = protected_router(
        McpAuthPolicy::static_bearer_for_principal(
            "expected-token",
            "operator.primary:v1",
        )
        .unwrap(),
        file_tools,
    );

    let response = app
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "https://example.invalid")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-mcp-principal-id", "operator.primary:v1")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let payload = response_json(response).await;
    assert_eq!(payload["error"], "unauthorized");
}
''')

config = Path("src/config.rs")
replace_once(
    config,
    '''pub struct AuthConfig {
    /// Static bearer token for simple deployments.
    /// For production, consider integrating with external IdP.
    pub static_token: Option<String>,
    /// Explicit unsafe/local-only opt-in for development without a bearer token.
''',
    '''pub struct AuthConfig {
    /// Static bearer token for simple deployments.
    /// For production, consider integrating with external IdP.
    pub static_token: Option<String>,
    /// Optional trusted, non-secret stable identity for the configured static credential.
    /// Caller-controlled headers and tool arguments are never principal sources.
    pub static_principal_id: Option<String>,
    /// Explicit unsafe/local-only opt-in for development without a bearer token.
''',
)
replace_once(
    config,
    '''            .field(
                "allow_unauthenticated_localhost_only",
                &self.allow_unauthenticated_localhost_only,
            )
''',
    '''            .field(
                "static_principal_id_configured",
                &self.static_principal_id.is_some(),
            )
            .field(
                "allow_unauthenticated_localhost_only",
                &self.allow_unauthenticated_localhost_only,
            )
''',
)
replace_once(
    config,
    '''            auth: AuthConfig {
                static_token: optional_env_string(&read_variable, "MCP__AUTH__STATIC_TOKEN")?,
                allow_unauthenticated_localhost_only: env_bool(
''',
    '''            auth: AuthConfig {
                static_token: optional_env_string(&read_variable, "MCP__AUTH__STATIC_TOKEN")?,
                static_principal_id: optional_env_string(
                    &read_variable,
                    "MCP__AUTH__STATIC_PRINCIPAL_ID",
                )?,
                allow_unauthenticated_localhost_only: env_bool(
''',
)
replace_once(
    config,
    '''pub fn validate_runtime_auth_posture(config: &AppConfig) -> anyhow::Result<AuthPosture> {
    if let Some(ref token) = config.auth.static_token {
''',
    '''pub fn validate_runtime_auth_posture(config: &AppConfig) -> anyhow::Result<AuthPosture> {
    validate_static_principal_identity(&config.auth)?;

    if let Some(ref token) = config.auth.static_token {
''',
)
replace_once(
    config,
    '''#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPosture {
''',
    '''fn validate_static_principal_identity(auth: &AuthConfig) -> anyhow::Result<()> {
    const MAX_PRINCIPAL_ID_BYTES: usize = 128;

    let Some(principal) = auth.static_principal_id.as_deref() else {
        return Ok(());
    };
    let Some(token) = auth.static_token.as_deref() else {
        bail!("MCP__AUTH__STATIC_PRINCIPAL_ID requires MCP__AUTH__STATIC_TOKEN");
    };
    if principal.is_empty() {
        bail!("MCP__AUTH__STATIC_PRINCIPAL_ID must not be empty");
    }
    if principal.len() > MAX_PRINCIPAL_ID_BYTES {
        bail!(
            "MCP__AUTH__STATIC_PRINCIPAL_ID exceeds the {MAX_PRINCIPAL_ID_BYTES}-byte safety limit"
        );
    }
    if !principal.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
    }) {
        bail!(
            "MCP__AUTH__STATIC_PRINCIPAL_ID may contain only ASCII letters, digits, '.', '_', '-', and ':'"
        );
    }
    if principal == token {
        bail!("MCP__AUTH__STATIC_PRINCIPAL_ID must not equal MCP__AUTH__STATIC_TOKEN");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPosture {
''',
)
replace_once(
    config,
    '''            auth: AuthConfig {
                static_token: static_token.map(str::to_owned),
                allow_unauthenticated_localhost_only: allow_localhost_only,
            },
''',
    '''            auth: AuthConfig {
                static_token: static_token.map(str::to_owned),
                static_principal_id: None,
                allow_unauthenticated_localhost_only: allow_localhost_only,
            },
''',
)
replace_once(
    config,
    '''        let auth = AuthConfig {
            static_token: Some("secret-value".to_owned()),
            allow_unauthenticated_localhost_only: false,
        };
''',
    '''        let auth = AuthConfig {
            static_token: Some("secret-value".to_owned()),
            static_principal_id: Some("operator.primary:v1".to_owned()),
            allow_unauthenticated_localhost_only: false,
        };
''',
)
replace_once(
    config,
    '''        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret-value"));
''',
    '''        assert!(debug.contains("<redacted>"));
        assert!(debug.contains("static_principal_id_configured: true"));
        assert!(!debug.contains("secret-value"));
        assert!(!debug.contains("operator.primary:v1"));
''',
)
replace_once(
    config,
    '''        assert_eq!(config.auth.static_token, None);
        assert!(!config.android.battery_status_enabled);
''',
    '''        assert_eq!(config.auth.static_token, None);
        assert_eq!(config.auth.static_principal_id, None);
        assert!(!config.android.battery_status_enabled);
''',
)
replace_once(
    config,
    '''            "MCP__AUTH__STATIC_TOKEN",
            "MCP__SERVER__HOST",
''',
    '''            "MCP__AUTH__STATIC_TOKEN",
            "MCP__AUTH__STATIC_PRINCIPAL_ID",
            "MCP__SERVER__HOST",
''',
)
replace_once(
    config,
    '''    #[test]
    fn static_token_auth_posture_is_accepted_for_non_loopback_hosts() {
        let config = app_config("0.0.0.0", Some("configured-token"), false);

        let posture = validate_runtime_auth_posture(&config).expect("token auth should validate");

        assert_eq!(posture, AuthPosture::StaticTokenConfigured);
    }
''',
    '''    #[test]
    fn static_token_auth_posture_is_accepted_for_non_loopback_hosts() {
        let config = app_config("0.0.0.0", Some("configured-token"), false);

        let posture = validate_runtime_auth_posture(&config).expect("token auth should validate");

        assert_eq!(posture, AuthPosture::StaticTokenConfigured);
    }

    #[test]
    fn configured_static_principal_identity_is_validated_without_disclosure() {
        let mut config = app_config("0.0.0.0", Some("configured-token"), false);
        config.auth.static_principal_id = Some("operator.primary:v1".to_owned());
        assert_eq!(
            validate_runtime_auth_posture(&config).unwrap(),
            AuthPosture::StaticTokenConfigured
        );

        for invalid in ["", "contains whitespace", "slash/not-allowed", "é"] {
            config.auth.static_principal_id = Some(invalid.to_owned());
            let error = validate_runtime_auth_posture(&config).unwrap_err();
            assert!(!error.to_string().contains(invalid));
        }

        config.auth.static_principal_id = Some("configured-token".to_owned());
        let error = validate_runtime_auth_posture(&config).unwrap_err();
        assert!(error.to_string().contains("must not equal"));
        assert!(!error.to_string().contains("configured-token"));
    }

    #[test]
    fn static_principal_identity_requires_static_token_authentication() {
        let mut config = app_config("127.0.0.1", None, true);
        config.auth.static_principal_id = Some("operator.primary:v1".to_owned());

        let error = validate_runtime_auth_posture(&config).unwrap_err();
        assert_eq!(
            error.to_string(),
            "MCP__AUTH__STATIC_PRINCIPAL_ID requires MCP__AUTH__STATIC_TOKEN"
        );
    }
''',
)

transport = Path("src/mcp_transport.rs")
replace_once(
    transport,
    '''use axum::{
    body::Bytes,
    extract::State,
''',
    '''use axum::{
    body::Bytes,
    extract::{Extension, State},
''',
)
replace_once(
    transport,
    '''    android_status::collect_android_status,
    audit::{
''',
    '''    android_status::collect_android_status,
    auth::AuthenticatedPrincipal,
    audit::{
''',
)
replace_once(
    transport,
    '''async fn handle_mcp_request(
    State(state): State<McpTransportState>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
''',
    '''async fn handle_mcp_request(
    State(state): State<McpTransportState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
''',
)
replace_once(
    transport,
    '''    let host = header_value(&headers, header::HOST);
    let origin = header_value(&headers, header::ORIGIN);

    let mut response = if let Err(error) = state.security_policy.validate_request(host, origin) {
''',
    '''    let host = header_value(&headers, header::HOST);
    let origin = header_value(&headers, header::ORIGIN);
    let principal = principal.as_ref().map(|value| &value.0);

    let mut response = if let Err(error) = state.security_policy.validate_request(host, origin) {
''',
)
replace_once(
    transport,
    '''        match method {
            Method::POST => handle_mcp_post(&state, &headers, body).await,
            Method::GET => handle_mcp_get(&state, &headers),
            Method::DELETE => handle_mcp_delete(&state, &headers),
''',
    '''        match method {
            Method::POST => handle_mcp_post(&state, &headers, body, principal).await,
            Method::GET => handle_mcp_get(&state, &headers, principal),
            Method::DELETE => handle_mcp_delete(&state, &headers, principal),
''',
)
replace_once(
    transport,
    '''async fn handle_mcp_post(state: &McpTransportState, headers: &HeaderMap, body: Bytes) -> Response {
''',
    '''async fn handle_mcp_post(
    state: &McpTransportState,
    headers: &HeaderMap,
    body: Bytes,
    principal: Option<&AuthenticatedPrincipal>,
) -> Response {
''',
)
replace_once(
    transport,
    '''            return initialize_response(Some(id.clone()), params.clone(), state);
''',
    '''            return initialize_response(Some(id.clone()), params.clone(), state, principal);
''',
)
replace_once(
    transport,
    '''    let (session_id, phase) = match validate_session_request(headers, &state.sessions) {
''',
    '''    let (session_id, phase) = match validate_session_request(headers, &state.sessions, principal) {
''',
)
replace_once(
    transport,
    '''                return match state.sessions.activate(&session_id) {
''',
    '''                return match state.sessions.activate(&session_id, principal) {
''',
)
replace_once(
    transport,
    '''fn handle_mcp_get(state: &McpTransportState, headers: &HeaderMap) -> Response {
''',
    '''fn handle_mcp_get(
    state: &McpTransportState,
    headers: &HeaderMap,
    principal: Option<&AuthenticatedPrincipal>,
) -> Response {
''',
)
replace_once(
    transport,
    '''    let (_, phase) = match validate_session_request(headers, &state.sessions) {
''',
    '''    let (_, phase) = match validate_session_request(headers, &state.sessions, principal) {
''',
)
replace_once(
    transport,
    '''fn handle_mcp_delete(state: &McpTransportState, headers: &HeaderMap) -> Response {
    let (session_id, _) = match validate_session_request(headers, &state.sessions) {
''',
    '''fn handle_mcp_delete(
    state: &McpTransportState,
    headers: &HeaderMap,
    principal: Option<&AuthenticatedPrincipal>,
) -> Response {
    let (session_id, _) = match validate_session_request(headers, &state.sessions, principal) {
''',
)
replace_once(
    transport,
    '''    match state.sessions.terminate(&session_id) {
''',
    '''    match state.sessions.terminate(&session_id, principal) {
''',
)
replace_once(
    transport,
    '''fn initialize_response(
    id: Option<Value>,
    params: Option<Value>,
    state: &McpTransportState,
) -> Response {
''',
    '''fn initialize_response(
    id: Option<Value>,
    params: Option<Value>,
    state: &McpTransportState,
    principal: Option<&AuthenticatedPrincipal>,
) -> Response {
''',
)
replace_once(
    transport,
    '''    let session_id = match state.sessions.create() {
''',
    '''    let session_id = match state.sessions.create(principal) {
''',
)
replace_once(
    transport,
    '''fn validate_session_request(
    headers: &HeaderMap,
    sessions: &McpSessionStore,
) -> Result<(String, SessionPhase), SessionRequestError> {
''',
    '''fn validate_session_request(
    headers: &HeaderMap,
    sessions: &McpSessionStore,
    principal: Option<&AuthenticatedPrincipal>,
) -> Result<(String, SessionPhase), SessionRequestError> {
''',
)
replace_once(
    transport,
    '''    sessions
        .phase(&session_id)
''',
    '''    sessions
        .phase(&session_id, principal)
''',
)
replace_once(
    transport,
    '''        SessionStoreError::NotFound => transport_error(
            StatusCode::NOT_FOUND,
            "session_not_found",
            "The MCP session does not exist or has expired.",
        ),
''',
    '''        SessionStoreError::NotFound | SessionStoreError::PrincipalMismatch => transport_error(
            StatusCode::NOT_FOUND,
            "session_not_found",
            "The MCP session does not exist, has expired, or is not associated with this authenticated principal.",
        ),
''',
)

# The source must not retain any old unbound session calls after the guarded replacements.
remaining = [
    "state.sessions.create()",
    "state.sessions.activate(&session_id)",
    "state.sessions.terminate(&session_id)",
    ".phase(&session_id)\n",
]
transport_text = transport.read_text()
for obsolete in remaining:
    if obsolete in transport_text:
        raise SystemExit(f"src/mcp_transport.rs: obsolete unbound session call remains: {obsolete}")
