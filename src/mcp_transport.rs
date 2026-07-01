use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::transport_security::TransportSecurityPolicy;

#[derive(Clone)]
struct McpTransportState {
    security_policy: TransportSecurityPolicy,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: Option<String>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: &'static str,
}

/// Build the staged MCP transport shell.
///
/// This route intentionally exposes transport liveness and an empty discovery
/// contract only. Capability invocation, filesystem access, Android platform
/// access, and high-impact actions remain unavailable until later validated stages.
pub fn router(security_policy: TransportSecurityPolicy) -> Router {
    Router::new()
        .route("/mcp", post(handle_mcp_request))
        .with_state(McpTransportState { security_policy })
}

async fn handle_mcp_request(
    State(state): State<McpTransportState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let host = header_value(&headers, header::HOST);
    let origin = header_value(&headers, header::ORIGIN);

    if let Err(error) = state.security_policy.validate_request(host, origin) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "transport_security_rejected",
                "message": error.to_string(),
            })),
        )
            .into_response();
    }

    let Ok(request) = serde_json::from_slice::<JsonRpcRequest>(&body) else {
        return Json(JsonRpcResponse {
            jsonrpc: "2.0",
            id: None,
            result: None,
            error: Some(JsonRpcError {
                code: -32700,
                message: "Parse error",
            }),
        })
        .into_response();
    };

    if request.jsonrpc.as_deref() != Some("2.0") {
        return Json(JsonRpcResponse {
            jsonrpc: "2.0",
            id: request.id,
            result: None,
            error: Some(JsonRpcError {
                code: -32600,
                message: "Invalid Request",
            }),
        })
        .into_response();
    }

    match request.method.as_deref() {
        Some("tools/list") => Json(JsonRpcResponse {
            jsonrpc: "2.0",
            id: request.id,
            result: Some(json!({ "tools": [] })),
            error: None,
        })
        .into_response(),
        Some("tools/call") => Json(JsonRpcResponse {
            jsonrpc: "2.0",
            id: request.id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: "Capability invocation is not enabled in this stage",
            }),
        })
        .into_response(),
        _ => Json(JsonRpcResponse {
            jsonrpc: "2.0",
            id: request.id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: "Method not found",
            }),
        })
        .into_response(),
    }
}

fn header_value(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    fn valid_request(body: &'static str) -> Request<Body> {
        Request::post("/mcp")
            .header(header::HOST, "localhost:8000")
            .header(header::ORIGIN, "http://localhost:8000")
            .body(Body::from(body))
            .unwrap()
    }

    async fn json_response(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn transport_shell_lists_empty_capability_registry() {
        let app = router(TransportSecurityPolicy::localhost(8000, false));

        let response = app
            .oneshot(valid_request(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = json_response(response).await;
        assert_eq!(body["jsonrpc"], "2.0");
        assert_eq!(body["id"], 1);
        assert_eq!(body["result"]["tools"], json!([]));
    }

    #[tokio::test]
    async fn transport_shell_rejects_capability_invocation() {
        let app = router(TransportSecurityPolicy::localhost(8000, false));

        let response = app
            .oneshot(valid_request(
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = json_response(response).await;
        assert_eq!(body["error"]["code"], -32601);
        assert_eq!(
            body["error"]["message"],
            "Capability invocation is not enabled in this stage"
        );
    }

    #[tokio::test]
    async fn transport_shell_rejects_untrusted_host_before_transport_handling() {
        let app = router(TransportSecurityPolicy::localhost(8000, false));

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "example.com:8000")
                    .header(header::ORIGIN, "http://localhost:8000")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn transport_shell_rejects_malformed_origin_before_transport_handling() {
        let app = router(TransportSecurityPolicy::localhost(8000, false));

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::HOST, "localhost:8000")
                    .header(header::ORIGIN, "http://localhost:8000/path")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
