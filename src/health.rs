//! Health and readiness response models.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadinessResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub mcp_runtime_enabled: bool,
    pub safe_root_count: usize,
    pub auth_posture: &'static str,
}

pub fn build_readiness_response(
    safe_root_count: usize,
    auth_posture: &'static str,
) -> ReadinessResponse {
    ReadinessResponse {
        status: "ready",
        version: env!("CARGO_PKG_VERSION"),
        mcp_runtime_enabled: cfg!(feature = "mcp-runtime"),
        safe_root_count,
        auth_posture,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_response_uses_package_version_and_feature_state() {
        let response = build_readiness_response(2, "static_token");

        assert_eq!(response.status, "ready");
        assert_eq!(response.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(response.mcp_runtime_enabled, cfg!(feature = "mcp-runtime"));
        assert_eq!(response.safe_root_count, 2);
        assert_eq!(response.auth_posture, "static_token");
    }
}
