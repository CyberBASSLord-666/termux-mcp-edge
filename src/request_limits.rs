//! Resource limits for the staged MCP transport.
//!
//! This module bounds authenticated MCP request concurrency, duration, and body
//! size without introducing a new tool or host capability. Limit failures use
//! stable, non-sensitive responses and do not include request contents.

use std::{fmt, sync::Arc, time::Duration};

use anyhow::bail;
use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use tokio::{sync::Semaphore, time::timeout};

pub const DEFAULT_MAX_CONCURRENT_REQUESTS: usize = 8;
pub const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 30;
pub const DEFAULT_MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

pub const MAX_CONFIGURED_CONCURRENT_REQUESTS: usize = 256;
pub const MAX_CONFIGURED_REQUEST_TIMEOUT_SECONDS: u64 = 300;
pub const MIN_CONFIGURED_BODY_BYTES: usize = 1_024;
pub const MAX_CONFIGURED_BODY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct McpRequestLimits {
    max_concurrent_requests: usize,
    request_timeout: Duration,
    max_body_bytes: usize,
    semaphore: Arc<Semaphore>,
}

impl McpRequestLimits {
    pub fn new(
        max_concurrent_requests: usize,
        request_timeout: Duration,
        max_body_bytes: usize,
    ) -> anyhow::Result<Self> {
        if max_concurrent_requests == 0 {
            bail!("MCP maximum concurrent requests must be greater than zero");
        }
        if request_timeout.is_zero() {
            bail!("MCP request timeout must be greater than zero");
        }
        if max_body_bytes == 0 {
            bail!("MCP maximum request-body bytes must be greater than zero");
        }

        Ok(Self {
            max_concurrent_requests,
            request_timeout,
            max_body_bytes,
            semaphore: Arc::new(Semaphore::new(max_concurrent_requests)),
        })
    }

    pub fn from_seconds(
        max_concurrent_requests: usize,
        request_timeout_seconds: u64,
        max_body_bytes: usize,
    ) -> anyhow::Result<Self> {
        Self::new(
            max_concurrent_requests,
            Duration::from_secs(request_timeout_seconds),
            max_body_bytes,
        )
    }

    pub const fn max_concurrent_requests(&self) -> usize {
        self.max_concurrent_requests
    }

    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    pub const fn max_body_bytes(&self) -> usize {
        self.max_body_bytes
    }
}

impl fmt::Debug for McpRequestLimits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpRequestLimits")
            .field("max_concurrent_requests", &self.max_concurrent_requests)
            .field("request_timeout", &self.request_timeout)
            .field("max_body_bytes", &self.max_body_bytes)
            .finish()
    }
}

/// Enforce fail-fast concurrency and a bounded total request duration.
///
/// This middleware is intended to run after bearer authentication. Axum's
/// `DefaultBodyLimit` provides the actual streaming body ceiling so the request
/// body is not buffered twice on memory-constrained Termux devices. This layer
/// performs an early `Content-Length` rejection when possible and normalizes
/// Axum's non-JSON body-limit response.
pub async fn enforce_mcp_request_limits(
    State(limits): State<McpRequestLimits>,
    request: Request,
    next: Next,
) -> Response {
    if content_length_exceeds_limit(&request, limits.max_body_bytes) {
        return body_limit_response();
    }

    let permit = match limits.semaphore.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return concurrency_limit_response(),
    };

    let response = match timeout(limits.request_timeout, next.run(request)).await {
        Ok(response) => normalize_body_limit_response(response),
        Err(_) => timeout_response(),
    };

    drop(permit);
    response
}

fn content_length_exceeds_limit(request: &Request, max_body_bytes: usize) -> bool {
    request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > max_body_bytes as u64)
}

fn normalize_body_limit_response(response: Response) -> Response {
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));

    if response.status() == StatusCode::PAYLOAD_TOO_LARGE && !is_json {
        body_limit_response()
    } else {
        response
    }
}

fn concurrency_limit_response() -> Response {
    let mut response = limit_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "mcp_concurrency_limit_reached",
        "The MCP transport is at its configured concurrent-request limit.",
    );
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

fn timeout_response() -> Response {
    limit_response(
        StatusCode::GATEWAY_TIMEOUT,
        "mcp_request_timeout",
        "The MCP request exceeded the configured execution timeout.",
    )
}

fn body_limit_response() -> Response {
    limit_response(
        StatusCode::PAYLOAD_TOO_LARGE,
        "mcp_request_body_too_large",
        "The MCP request body exceeds the configured byte limit.",
    )
}

fn limit_response(status: StatusCode, error: &'static str, message: &'static str) -> Response {
    let mut response = (
        status,
        Json(json!({
            "error": error,
            "message": message,
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
    use std::sync::Arc;

    use axum::{
        body::{to_bytes, Body, Bytes},
        extract::{DefaultBodyLimit, State as HandlerState},
        http::Request as HttpRequest,
        middleware,
        routing::post,
        Router,
    };
    use serde_json::Value;
    use tokio::{sync::Notify, time::sleep};
    use tower::ServiceExt;

    use super::*;

    fn request(body: impl Into<Body>) -> HttpRequest<Body> {
        HttpRequest::post("/mcp").body(body.into()).unwrap()
    }

    async fn json_body(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn body_handler(body: Bytes) -> String {
        format!("{}", body.len())
    }

    fn limited_router(limits: McpRequestLimits) -> Router {
        let max_body_bytes = limits.max_body_bytes();
        Router::new()
            .route("/mcp", post(body_handler))
            .layer(DefaultBodyLimit::max(max_body_bytes))
            .route_layer(middleware::from_fn_with_state(
                limits,
                enforce_mcp_request_limits,
            ))
    }

    #[tokio::test]
    async fn request_inside_limits_reaches_handler() {
        let limits = McpRequestLimits::new(1, Duration::from_secs(1), 64).unwrap();
        let response = limited_router(limits)
            .oneshot(request("small-body"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn oversized_chunked_body_is_rejected_with_non_sensitive_response() {
        let limits = McpRequestLimits::new(1, Duration::from_secs(1), 8).unwrap();
        let response = limited_router(limits)
            .oneshot(request("123456789"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store"))
        );
        let payload = json_body(response).await;
        assert_eq!(payload["error"], "mcp_request_body_too_large");
        assert!(!payload.to_string().contains("123456789"));
    }

    #[tokio::test]
    async fn oversized_content_length_is_rejected_before_handler() {
        let limits = McpRequestLimits::new(1, Duration::from_secs(1), 8).unwrap();
        let response = limited_router(limits)
            .oneshot(
                HttpRequest::post("/mcp")
                    .header(header::CONTENT_LENGTH, "9")
                    .body(Body::from("small"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let payload = json_body(response).await;
        assert_eq!(payload["error"], "mcp_request_body_too_large");
    }

    #[tokio::test]
    async fn request_timeout_returns_gateway_timeout() {
        let limits = McpRequestLimits::new(1, Duration::from_millis(5), 64).unwrap();
        let app = Router::new()
            .route(
                "/mcp",
                post(|| async {
                    sleep(Duration::from_millis(50)).await;
                    "late"
                }),
            )
            .route_layer(middleware::from_fn_with_state(
                limits,
                enforce_mcp_request_limits,
            ));

        let response = app.oneshot(request(Body::empty())).await.unwrap();
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        let payload = json_body(response).await;
        assert_eq!(payload["error"], "mcp_request_timeout");
    }

    #[derive(Clone)]
    struct TestGate {
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    async fn blocking_handler(HandlerState(gate): HandlerState<TestGate>) -> &'static str {
        gate.entered.notify_one();
        gate.release.notified().await;
        "released"
    }

    #[tokio::test]
    async fn saturated_concurrency_fails_fast() {
        let limits = McpRequestLimits::new(1, Duration::from_secs(1), 64).unwrap();
        let gate = TestGate {
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        };
        let app = Router::new()
            .route("/mcp", post(blocking_handler))
            .with_state(gate.clone())
            .route_layer(middleware::from_fn_with_state(
                limits,
                enforce_mcp_request_limits,
            ));

        let first_app = app.clone();
        let first = tokio::spawn(async move {
            first_app
                .oneshot(request(Body::empty()))
                .await
                .unwrap()
        });
        gate.entered.notified().await;

        let second = app.oneshot(request(Body::empty())).await.unwrap();
        assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            second.headers().get(header::RETRY_AFTER),
            Some(&HeaderValue::from_static("1"))
        );
        let payload = json_body(second).await;
        assert_eq!(payload["error"], "mcp_concurrency_limit_reached");

        gate.release.notify_one();
        assert_eq!(first.await.unwrap().status(), StatusCode::OK);
    }

    #[test]
    fn invalid_zero_limits_are_rejected() {
        assert!(McpRequestLimits::new(0, Duration::from_secs(1), 1).is_err());
        assert!(McpRequestLimits::new(1, Duration::ZERO, 1).is_err());
        assert!(McpRequestLimits::new(1, Duration::from_secs(1), 0).is_err());
    }

    #[test]
    fn debug_output_contains_only_non_sensitive_limit_values() {
        let limits = McpRequestLimits::new(8, Duration::from_secs(30), 1024).unwrap();
        let debug = format!("{limits:?}");

        assert!(debug.contains("max_concurrent_requests"));
        assert!(debug.contains("request_timeout"));
        assert!(debug.contains("max_body_bytes"));
    }
}
