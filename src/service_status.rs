//! Project-owned service status primitives for the staged MCP runtime.
//!
//! This module intentionally models service posture from an explicit in-repo
//! allowlist only. It does not enumerate processes, inspect arbitrary PIDs,
//! read command lines, read process environments, execute shell commands, call
//! Android APIs, or perform service-control actions. Transport exposure is left
//! to a later gate once this data-only surface has been reviewed independently.

use serde::Serialize;
use thiserror::Error;

/// Explicit allowlist for project-owned service-state lookups.
///
/// These names are repository-owned logical service identifiers. They are not
/// process names, package names, unit names, PIDs, command lines, or filesystem
/// paths, and callers cannot extend this list at runtime.
pub const PROJECT_SERVICE_ALLOWLIST: &[&str] = &["mcp_runtime"];

/// Fields that may appear in serialized project service status responses.
pub const PROJECT_SERVICE_STATUS_ALLOWED_FIELDS: &[&str] = &[
    "service_name",
    "ownership",
    "status_mode",
    "lifecycle_state",
    "health",
    "pid_inspection_enabled",
    "process_listing_enabled",
    "command_line_exposed",
    "environment_exposed",
    "command_execution_enabled",
    "mutation_enabled",
];

/// Fields and expansion targets that must remain absent from this gate.
pub const PROJECT_SERVICE_STATUS_DENIED_FIELDS: &[&str] = &[
    "pid",
    "ppid",
    "uid",
    "gid",
    "processes",
    "process_list",
    "process_inventory",
    "command",
    "cmd",
    "cmdline",
    "command_line",
    "argv",
    "args",
    "environment",
    "env",
    "cwd",
    "exe",
    "open_files",
    "sockets",
    "ports",
    "package_name",
    "installed_packages",
    "android_services",
    "system_services",
    "unit_name",
    "service_file",
    "logs",
    "stdout",
    "stderr",
    "restart",
    "stop",
    "start",
    "kill",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectServiceStatus {
    /// Allowlisted logical project service name.
    pub service_name: &'static str,
    /// Ownership posture for this gate.
    pub ownership: &'static str,
    /// Read-only status mode; this is not a control surface.
    pub status_mode: &'static str,
    /// Coarse lifecycle state derived from in-process runtime posture only.
    pub lifecycle_state: &'static str,
    /// Coarse health signal for the project-owned runtime.
    pub health: &'static str,
    /// Arbitrary PID inspection remains disabled.
    pub pid_inspection_enabled: bool,
    /// Global process listing remains disabled.
    pub process_listing_enabled: bool,
    /// Command lines remain unavailable.
    pub command_line_exposed: bool,
    /// Process environments remain unavailable.
    pub environment_exposed: bool,
    /// Command execution remains disabled.
    pub command_execution_enabled: bool,
    /// Service mutation/control remains disabled.
    pub mutation_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnsupportedProjectService {
    pub error: &'static str,
    pub requested_service: String,
    pub allowed_services: &'static [&'static str],
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProjectServiceStatusError {
    #[error("unsupported project service: {requested_service}")]
    UnsupportedService { requested_service: String },
}

pub fn collect_project_service_status(
    service_name: &str,
) -> Result<ProjectServiceStatus, ProjectServiceStatusError> {
    match service_name {
        "mcp_runtime" => Ok(ProjectServiceStatus {
            service_name: "mcp_runtime",
            ownership: "project_owned_allowlisted",
            status_mode: "read_only_project_service_status",
            lifecycle_state: "available_in_process",
            health: "transport_runtime_available",
            pid_inspection_enabled: false,
            process_listing_enabled: false,
            command_line_exposed: false,
            environment_exposed: false,
            command_execution_enabled: false,
            mutation_enabled: false,
        }),
        unsupported => Err(ProjectServiceStatusError::UnsupportedService {
            requested_service: unsupported.to_string(),
        }),
    }
}

pub fn unsupported_project_service_error(service_name: &str) -> UnsupportedProjectService {
    UnsupportedProjectService {
        error: "unsupported_project_service",
        requested_service: service_name.to_string(),
        allowed_services: PROJECT_SERVICE_ALLOWLIST,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;

    use super::*;

    #[test]
    fn project_service_status_returns_allowlisted_runtime_only() {
        let status = collect_project_service_status("mcp_runtime").unwrap();

        assert_eq!(status.service_name, "mcp_runtime");
        assert_eq!(status.ownership, "project_owned_allowlisted");
        assert_eq!(status.status_mode, "read_only_project_service_status");
        assert_eq!(status.lifecycle_state, "available_in_process");
        assert_eq!(status.health, "transport_runtime_available");
        assert!(!status.pid_inspection_enabled);
        assert!(!status.process_listing_enabled);
        assert!(!status.command_line_exposed);
        assert!(!status.environment_exposed);
        assert!(!status.command_execution_enabled);
        assert!(!status.mutation_enabled);
    }

    #[test]
    fn unsupported_services_return_structured_errors() {
        let error = collect_project_service_status("ssh").unwrap_err();
        assert_eq!(
            error,
            ProjectServiceStatusError::UnsupportedService {
                requested_service: "ssh".to_string(),
            }
        );

        let structured = unsupported_project_service_error("ssh");
        assert_eq!(structured.error, "unsupported_project_service");
        assert_eq!(structured.requested_service, "ssh");
        assert_eq!(structured.allowed_services, PROJECT_SERVICE_ALLOWLIST);
    }

    #[test]
    fn service_status_contains_only_allowlisted_fields() {
        let value =
            serde_json::to_value(collect_project_service_status("mcp_runtime").unwrap()).unwrap();
        let object = value.as_object().unwrap();
        let actual_keys = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
        let allowed_keys = PROJECT_SERVICE_STATUS_ALLOWED_FIELDS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        assert_eq!(actual_keys, allowed_keys);
    }

    #[test]
    fn service_status_serialization_excludes_process_and_control_fields() {
        let value =
            serde_json::to_value(collect_project_service_status("mcp_runtime").unwrap()).unwrap();
        let object = value.as_object().unwrap();

        for denied_field in PROJECT_SERVICE_STATUS_DENIED_FIELDS {
            assert_eq!(
                object.get(*denied_field),
                None,
                "unexpected denied service-status field: {denied_field}"
            );
        }

        assert_no_process_or_control_values(&value);
    }

    #[test]
    fn allowlist_and_denylist_do_not_overlap() {
        let allowed = PROJECT_SERVICE_STATUS_ALLOWED_FIELDS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        for denied in PROJECT_SERVICE_STATUS_DENIED_FIELDS {
            assert!(
                !allowed.contains(denied),
                "field cannot be both allowed and denied: {denied}"
            );
        }
    }

    fn assert_no_process_or_control_values(value: &Value) {
        let text = value.to_string().to_ascii_lowercase();
        for token in [
            "cmdline",
            "stdout",
            "stderr",
            "installed_packages",
            "android_services",
            "system_services",
            "restart",
            "kill",
            "/proc/",
            "/data/",
            "/sdcard",
            "/storage/",
        ] {
            assert!(!text.contains(token), "unexpected sensitive token: {token}");
        }
    }
}
