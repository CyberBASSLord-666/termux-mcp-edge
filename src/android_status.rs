//! Read-only Android/Termux status primitives for the staged MCP runtime.
//!
//! This module is intentionally data-only. It does not call Android APIs, read
//! environment variables, inspect packages, enumerate processes, execute shell
//! commands, or perform device-control actions. The optional `mcp-runtime`
//! transport exposes this allowlisted data without expanding it into Android
//! inspection or control behavior.

use serde::Serialize;

/// Explicit allowlist for the read-only Android/Termux status gate.
///
/// These names match the serialized `AndroidStatus` shape and are deliberately
/// limited to coarse runtime posture and compile-time platform metadata.
pub const ANDROID_STATUS_ALLOWED_FIELDS: &[&str] = &[
    "status_mode",
    "target_os",
    "target_arch",
    "target_family",
    "package_version",
    "termux_runtime_hint",
    "android_api_access",
    "android_control_enabled",
    "shell_fallback_enabled",
    "command_execution_enabled",
    "high_impact_controls_enabled",
];

/// Explicit denylist for Android/Termux status expansion.
///
/// These fields must remain absent from the read-only status primitive and from
/// every MCP response unless a later, separately reviewed gate explicitly
/// authorizes them.
pub const ANDROID_STATUS_DENIED_FIELDS: &[&str] = &[
    "android_id",
    "advertising_id",
    "device_serial",
    "imei",
    "imsi",
    "subscriber_id",
    "phone_number",
    "accounts",
    "contacts",
    "sms",
    "notifications",
    "location",
    "camera",
    "microphone",
    "accessibility_state",
    "installed_packages",
    "package_inventory",
    "processes",
    "environment",
    "env",
    "secrets",
    "tokens",
    "shell",
    "command_output",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AndroidStatus {
    /// Gate posture for future MCP metadata.
    pub status_mode: &'static str,
    /// Rust compile target OS, not a device identifier.
    pub target_os: &'static str,
    /// Rust compile target architecture, not hardware inventory.
    pub target_arch: &'static str,
    /// Rust compile target family.
    pub target_family: &'static str,
    /// Package version from Cargo metadata.
    pub package_version: &'static str,
    /// Coarse Termux suitability hint derived only from the compile target.
    pub termux_runtime_hint: &'static str,
    /// Android API access posture for this primitive.
    pub android_api_access: &'static str,
    /// Android device-control actions remain disabled.
    pub android_control_enabled: bool,
    /// Shell fallback remains disabled.
    pub shell_fallback_enabled: bool,
    /// Command execution remains disabled.
    pub command_execution_enabled: bool,
    /// High-impact controls remain disabled.
    pub high_impact_controls_enabled: bool,
}

pub fn collect_android_status() -> AndroidStatus {
    AndroidStatus {
        status_mode: "read_only_allowlisted_status",
        target_os: std::env::consts::OS,
        target_arch: std::env::consts::ARCH,
        target_family: std::env::consts::FAMILY,
        package_version: env!("CARGO_PKG_VERSION"),
        termux_runtime_hint: if cfg!(target_os = "android") {
            "android_termux_candidate"
        } else {
            "non_android_build_or_test_host"
        },
        android_api_access: "not_used",
        android_control_enabled: false,
        shell_fallback_enabled: false,
        command_execution_enabled: false,
        high_impact_controls_enabled: false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;

    use super::*;

    #[test]
    fn android_status_contains_only_allowlisted_fields() {
        let value = serde_json::to_value(collect_android_status()).unwrap();
        let object = value.as_object().unwrap();
        let actual_keys = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
        let allowed_keys = ANDROID_STATUS_ALLOWED_FIELDS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        assert_eq!(actual_keys, allowed_keys);
    }

    #[test]
    fn android_status_distinguishes_read_only_status_from_control() {
        let status = collect_android_status();

        assert_eq!(status.status_mode, "read_only_allowlisted_status");
        assert_eq!(status.android_api_access, "not_used");
        assert!(!status.android_control_enabled);
        assert!(!status.shell_fallback_enabled);
        assert!(!status.command_execution_enabled);
        assert!(!status.high_impact_controls_enabled);
    }

    #[test]
    fn android_status_serialization_excludes_denied_fields() {
        let value = serde_json::to_value(collect_android_status()).unwrap();
        let object = value.as_object().unwrap();

        for denied_field in ANDROID_STATUS_DENIED_FIELDS {
            assert_eq!(
                object.get(*denied_field),
                None,
                "unexpected denied Android status field: {denied_field}"
            );
        }

        assert_no_sensitive_tokens(&value);
    }

    #[test]
    fn allowlist_and_denylist_do_not_overlap() {
        let allowed = ANDROID_STATUS_ALLOWED_FIELDS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        for denied in ANDROID_STATUS_DENIED_FIELDS {
            assert!(
                !allowed.contains(denied),
                "field cannot be both allowed and denied: {denied}"
            );
        }
    }

    fn assert_no_sensitive_tokens(value: &Value) {
        let text = value.to_string().to_ascii_lowercase();
        for token in [
            "android_id",
            "advertising_id",
            "serial",
            "imei",
            "imsi",
            "subscriber",
            "phone_number",
            "contacts",
            "sms",
            "notification",
            "latitude",
            "longitude",
            "camera",
            "microphone",
            "accessibility",
            "installed_packages",
            "package_inventory",
            "processes",
            "password",
            "secret",
            "token",
            "/data/",
            "/sdcard",
            "/storage/",
        ] {
            assert!(!text.contains(token), "unexpected sensitive token: {token}");
        }
    }
}
