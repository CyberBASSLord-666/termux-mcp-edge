#![cfg(feature = "mcp-runtime")]
#![allow(dead_code)]

use axum::{
    body::{to_bytes, Body},
    http::{header, Request},
    response::Response,
    Router,
};
use serde_json::Value;
use tempfile::TempDir;
use termux_mcp_server::{
    mcp_transport::router, tools::FileSystemTools, transport_security::TransportSecurityPolicy,
};
use tower::ServiceExt;

pub(super) fn test_file_tools() -> (TempDir, FileSystemTools) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("visible.txt"), "safe content").unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    (root, tools)
}

pub(super) fn empty_test_file_tools() -> (TempDir, FileSystemTools) {
    let root = tempfile::tempdir().unwrap();
    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    (root, tools)
}

pub(super) fn test_router(file_tools: FileSystemTools) -> Router {
    router(TransportSecurityPolicy::localhost(8000, false), file_tools)
}

pub(super) async fn post_raw(body: impl Into<Body>) -> Response {
    let (_root, file_tools) = test_file_tools();
    test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .body(body.into())
                .unwrap(),
        )
        .await
        .unwrap()
}

pub(super) async fn post_json(request_body: Value) -> Response {
    post_raw(request_body.to_string()).await
}

pub(super) async fn post_json_with_empty_root(request_body: Value) -> Response {
    let (_root, file_tools) = empty_test_file_tools();
    test_router(file_tools)
        .oneshot(
            Request::post("/mcp")
                .header(header::HOST, "localhost:8000")
                .header(header::ORIGIN, "http://localhost:8000")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(request_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

pub(super) async fn response_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}
