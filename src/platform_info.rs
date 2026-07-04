//! Non-sensitive platform metadata for staged MCP runtime reporting.
//!
//! This module intentionally avoids environment variables, filesystem paths,
//! process listings, package inventories, shell access, Android API calls, and
//! other host-specific details that would expand the runtime's security surface.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformInfo {
    pub os: &'static str,
    pub arch: &'static str,
    pub family: &'static str,
    pub available_parallelism: usize,
    pub package_version: &'static str,
}

pub fn collect_platform_info() -> PlatformInfo {
    PlatformInfo {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        family: std::env::consts::FAMILY,
        available_parallelism: std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1),
        package_version: env!("CARGO_PKG_VERSION"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn platform_info_contains_expected_non_sensitive_metadata() {
        let info = collect_platform_info();

        assert_eq!(info.os, std::env::consts::OS);
        assert_eq!(info.arch, std::env::consts::ARCH);
        assert_eq!(info.family, std::env::consts::FAMILY);
        assert!(info.available_parallelism >= 1);
        assert_eq!(info.package_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn platform_info_serialization_does_not_include_sensitive_host_details() {
        let value = serde_json::to_value(collect_platform_info()).unwrap();
        let object = value.as_object().unwrap();

        assert!(object.contains_key("os"));
        assert!(object.contains_key("arch"));
        assert!(object.contains_key("family"));
        assert!(object.contains_key("available_parallelism"));
        assert!(object.contains_key("package_version"));

        let forbidden_keys = [
            "env",
            "environment",
            "home",
            "path",
            "processes",
            "shell",
            "username",
            "hostname",
            "android_id",
        ];

        for key in forbidden_keys {
            assert_eq!(object.get(key), None, "unexpected sensitive key: {key}");
        }

        assert_no_nested_sensitive_strings(&value);
    }

    fn assert_no_nested_sensitive_strings(value: &Value) {
        let text = value.to_string().to_ascii_lowercase();
        for token in ["/home/", "/data/", "password", "secret", "token"] {
            assert!(!text.contains(token), "unexpected sensitive token: {token}");
        }
    }
}
