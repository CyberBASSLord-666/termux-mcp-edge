//! Non-sensitive audit event primitives for staged MCP capability gates.
//!
//! This module models audit decisions without selecting a persistence backend.
//! It intentionally avoids raw file contents, command output, environment
//! values, secrets, and private host metadata.

use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditEvent {
    pub timestamp_unix_seconds: u64,
    pub tool_name: String,
    pub gate_name: String,
    pub mode: AuditMode,
    pub decision: AuditDecision,
    pub reason_code: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditMode {
    ReadOnly,
    DryRun,
    Mutating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditDecision {
    Allowed,
    Denied,
}

impl AuditEvent {
    pub fn new(
        timestamp_unix_seconds: u64,
        tool_name: impl Into<String>,
        gate_name: impl Into<String>,
        mode: AuditMode,
        decision: AuditDecision,
        reason_code: impl Into<String>,
    ) -> Self {
        Self {
            timestamp_unix_seconds,
            tool_name: tool_name.into(),
            gate_name: gate_name.into(),
            mode,
            decision,
            reason_code: reason_code.into(),
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: u64) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

/// Build a backend-neutral audit event for an allowed read-only staged tool call.
///
/// Callers supply the stable tool name, gate name, and non-sensitive reason code.
/// The event intentionally does not capture caller arguments, raw result values,
/// filesystem paths, environment values, command output, or host identifiers.
pub fn read_only_allowed_event(
    timestamp_unix_seconds: u64,
    tool_name: impl Into<String>,
    gate_name: impl Into<String>,
    reason_code: impl Into<String>,
) -> AuditEvent {
    AuditEvent::new(
        timestamp_unix_seconds,
        tool_name,
        gate_name,
        AuditMode::ReadOnly,
        AuditDecision::Allowed,
        reason_code,
    )
}

/// Build a backend-neutral audit event for a denied read-only staged tool call.
///
/// This helper standardizes denial shape while keeping the record non-sensitive.
/// Use metadata only for bounded counts or limit values, never for caller-supplied
/// raw strings, paths, command output, environment values, or secrets.
pub fn read_only_denied_event(
    timestamp_unix_seconds: u64,
    tool_name: impl Into<String>,
    gate_name: impl Into<String>,
    reason_code: impl Into<String>,
) -> AuditEvent {
    AuditEvent::new(
        timestamp_unix_seconds,
        tool_name,
        gate_name,
        AuditMode::ReadOnly,
        AuditDecision::Denied,
        reason_code,
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AuditCounters {
    pub allowed_total: u64,
    pub denied_total: u64,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub by_tool: BTreeMap<String, AuditDecisionCounters>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub by_reason_code: BTreeMap<String, AuditDecisionCounters>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AuditDecisionCounters {
    pub allowed: u64,
    pub denied: u64,
}

impl AuditCounters {
    /// Record a non-sensitive audit event into deterministic in-memory counters.
    ///
    /// The counter stores only stable tool names, reason codes, and aggregate
    /// totals. It deliberately ignores metadata to avoid accidentally turning
    /// caller-supplied values, paths, command output, environment values, or
    /// private host details into an observability backend.
    pub fn record_event(&mut self, event: &AuditEvent) {
        match event.decision {
            AuditDecision::Allowed => self.allowed_total += 1,
            AuditDecision::Denied => self.denied_total += 1,
        }

        self.by_tool
            .entry(event.tool_name.clone())
            .or_default()
            .record(event.decision);
        self.by_reason_code
            .entry(event.reason_code.clone())
            .or_default()
            .record(event.decision);
    }

    pub fn total(&self) -> u64 {
        self.allowed_total + self.denied_total
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

impl AuditDecisionCounters {
    fn record(&mut self, decision: AuditDecision) {
        match decision {
            AuditDecision::Allowed => self.allowed += 1,
            AuditDecision::Denied => self.denied += 1,
        }
    }

    pub fn total(&self) -> u64 {
        self.allowed + self.denied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn audit_event_serializes_stable_non_sensitive_shape() {
        let event = AuditEvent::new(
            1_725_000_000,
            "write_file",
            "filesystem_write",
            AuditMode::DryRun,
            AuditDecision::Allowed,
            "dry_run_preview",
        )
        .with_metadata("content_bytes", 42)
        .with_metadata("max_bytes", 1_048_576);

        let value = serde_json::to_value(event).unwrap();

        assert_eq!(value["timestamp_unix_seconds"], 1_725_000_000);
        assert_eq!(value["tool_name"], "write_file");
        assert_eq!(value["gate_name"], "filesystem_write");
        assert_eq!(value["mode"], "dry_run");
        assert_eq!(value["decision"], "allowed");
        assert_eq!(value["reason_code"], "dry_run_preview");
        assert_eq!(
            value["metadata"],
            json!({
                "content_bytes": 42,
                "max_bytes": 1_048_576,
            })
        );
    }

    #[test]
    fn empty_metadata_is_omitted() {
        let event = AuditEvent::new(
            1,
            "platform_info",
            "platform_metadata",
            AuditMode::ReadOnly,
            AuditDecision::Allowed,
            "read_only_metadata",
        );

        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value.get("metadata"), None);
    }

    #[test]
    fn read_only_allowed_helper_uses_standard_shape_without_metadata() {
        let event = read_only_allowed_event(
            1_725_000_100,
            "android_status",
            "android_read_only_status",
            "allowlisted_status_metadata",
        );

        assert_eq!(event.timestamp_unix_seconds, 1_725_000_100);
        assert_eq!(event.tool_name, "android_status");
        assert_eq!(event.gate_name, "android_read_only_status");
        assert_eq!(event.mode, AuditMode::ReadOnly);
        assert_eq!(event.decision, AuditDecision::Allowed);
        assert_eq!(event.reason_code, "allowlisted_status_metadata");
        assert!(event.metadata.is_empty());

        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["mode"], "read_only");
        assert_eq!(value["decision"], "allowed");
        assert_eq!(value.get("metadata"), None);
        assert_no_sensitive_tokens(&value);
    }

    #[test]
    fn read_only_denied_helper_uses_standard_shape_without_sensitive_argument_capture() {
        let event = read_only_denied_event(
            1_725_000_200,
            "project_service_status",
            "project_service_state",
            "unsupported_service",
        )
        .with_metadata("provided_argument_count", 1);

        assert_eq!(event.timestamp_unix_seconds, 1_725_000_200);
        assert_eq!(event.tool_name, "project_service_status");
        assert_eq!(event.gate_name, "project_service_state");
        assert_eq!(event.mode, AuditMode::ReadOnly);
        assert_eq!(event.decision, AuditDecision::Denied);
        assert_eq!(event.reason_code, "unsupported_service");
        assert_eq!(event.metadata.get("provided_argument_count"), Some(&1));

        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["mode"], "read_only");
        assert_eq!(value["decision"], "denied");
        assert_eq!(value["metadata"], json!({ "provided_argument_count": 1 }));
        assert_no_sensitive_tokens(&value);
    }

    #[test]
    fn audit_counters_record_allowed_and_denied_totals_by_stable_labels() {
        let mut counters = AuditCounters::default();

        counters.record_event(&read_only_allowed_event(
            1,
            "android_status",
            "android_read_only_status",
            "allowlisted_status_metadata",
        ));
        counters.record_event(&read_only_allowed_event(
            2,
            "project_service_status",
            "project_service_state",
            "allowlisted_project_service",
        ));
        counters.record_event(&read_only_denied_event(
            3,
            "project_service_status",
            "project_service_state",
            "unsupported_service",
        ));

        assert_eq!(counters.allowed_total, 2);
        assert_eq!(counters.denied_total, 1);
        assert_eq!(counters.total(), 3);
        assert!(!counters.is_empty());

        assert_eq!(counters.by_tool["android_status"].allowed, 1);
        assert_eq!(counters.by_tool["android_status"].denied, 0);
        assert_eq!(counters.by_tool["project_service_status"].allowed, 1);
        assert_eq!(counters.by_tool["project_service_status"].denied, 1);
        assert_eq!(
            counters.by_reason_code["unsupported_service"],
            AuditDecisionCounters {
                allowed: 0,
                denied: 1,
            }
        );
    }

    #[test]
    fn audit_counters_serialize_deterministically_without_event_metadata() {
        let mut counters = AuditCounters::default();

        let event = read_only_denied_event(
            1,
            "project_service_status",
            "project_service_state",
            "unsupported_service",
        )
        .with_metadata("provided_argument_count", 1);

        counters.record_event(&event);

        let value = serde_json::to_value(counters).unwrap();
        assert_eq!(value["allowed_total"], 0);
        assert_eq!(value["denied_total"], 1);
        assert_eq!(
            value["by_tool"],
            json!({
                "project_service_status": {
                    "allowed": 0,
                    "denied": 1,
                },
            })
        );
        assert_eq!(
            value["by_reason_code"],
            json!({
                "unsupported_service": {
                    "allowed": 0,
                    "denied": 1,
                },
            })
        );
        assert!(
            !value.to_string().contains("provided_argument_count"),
            "counter output must not copy event metadata"
        );
        assert_no_sensitive_tokens(&value);
    }

    #[test]
    fn empty_audit_counters_omit_sparse_maps() {
        let counters = AuditCounters::default();

        assert!(counters.is_empty());
        let value = serde_json::to_value(counters).unwrap();

        assert_eq!(value["allowed_total"], 0);
        assert_eq!(value["denied_total"], 0);
        assert_eq!(value.get("by_tool"), None);
        assert_eq!(value.get("by_reason_code"), None);
    }

    #[test]
    fn audit_event_shape_does_not_include_sensitive_fields() {
        let event = AuditEvent::new(
            1,
            "command_probe",
            "command_execution",
            AuditMode::Mutating,
            AuditDecision::Denied,
            "command_not_allowlisted",
        )
        .with_metadata("attempted_args", 3);

        let value = serde_json::to_value(event).unwrap();
        let object = value.as_object().unwrap();

        for forbidden_key in [
            "secret",
            "token",
            "password",
            "env",
            "environment",
            "file_content",
            "command_output",
            "stdout",
            "stderr",
            "hostname",
            "username",
            "android_id",
        ] {
            assert_eq!(
                object.get(forbidden_key),
                None,
                "unexpected sensitive key: {forbidden_key}"
            );
        }

        assert_no_sensitive_tokens(&value);
    }

    fn assert_no_sensitive_tokens(value: &Value) {
        let serialized = value.to_string().to_ascii_lowercase();
        for token in ["password", "secret", "token", "/data/", "/home/", "bearer"] {
            assert!(
                !serialized.contains(token),
                "unexpected sensitive token: {token}"
            );
        }
    }
}
