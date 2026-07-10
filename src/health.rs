//! Health and readiness response models.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadinessResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub mcp_runtime_enabled: bool,
    pub safe_root_count: usize,
    pub auth_posture: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_request_limits: Option<McpRequestLimitReadiness>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpRequestLimitReadiness {
    pub max_concurrent_requests: usize,
    pub request_timeout_seconds: u64,
    pub max_body_bytes: usize,
}

pub fn build_readiness_response(
    safe_root_count: usize,
    auth_posture: &'static str,
    mcp_request_limits: Option<McpRequestLimitReadiness>,
) -> ReadinessResponse {
    ReadinessResponse {
        status: "ready",
        version: env!("CARGO_PKG_VERSION"),
        mcp_runtime_enabled: cfg!(feature = "mcp-runtime"),
        safe_root_count,
        auth_posture,
        mcp_request_limits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_response_uses_package_version_feature_state_and_limits() {
        let limits = McpRequestLimitReadiness {
            max_concurrent_requests: 4,
            request_timeout_seconds: 30,
            max_body_bytes: 2 * 1024 * 1024,
        };
        let response = build_readiness_response(2, "static_token", Some(limits.clone()));

        assert_eq!(response.status, "ready");
        assert_eq!(response.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(response.mcp_runtime_enabled, cfg!(feature = "mcp-runtime"));
        assert_eq!(response.safe_root_count, 2);
        assert_eq!(response.auth_posture, "static_token");
        assert_eq!(response.mcp_request_limits, Some(limits));
    }

    #[test]
    fn readiness_response_omits_limits_when_runtime_is_not_configured() {
        let response = build_readiness_response(1, "static_token", None);
        let serialized = serde_json::to_value(response).unwrap();

        assert!(serialized.get("mcp_request_limits").is_none());
    }
}
