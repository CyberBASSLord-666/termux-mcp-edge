//! Design-only command execution gate primitives.
//!
//! This module models the command-execution allowlist and audit decisions for a
//! future staged gate. It deliberately does not spawn processes, invoke a shell,
//! read environment values, inspect process state, expose command output, mutate
//! services, or add any MCP transport surface.

use crate::audit::{AuditDecision, AuditEvent, AuditMode};

/// Command execution remains disabled until a later transport/runtime gate.
pub const COMMAND_EXECUTION_ENABLED: bool = false;

const COMMAND_EXECUTION_TOOL_NAME: &str = "command_execution_policy";
const COMMAND_EXECUTION_GATE: &str = "command_execution";
const COMMAND_ORDINAL_METADATA: &str = "command_ordinal";
const ARGV_COUNT_METADATA: &str = "argv_count";
const TIMEOUT_SECONDS_METADATA: &str = "timeout_seconds";
const MAX_STDOUT_BYTES_METADATA: &str = "max_stdout_bytes";
const MAX_STDERR_BYTES_METADATA: &str = "max_stderr_bytes";
const ENV_NAME_COUNT_METADATA: &str = "env_name_count";

const POLICY_ALLOWED_REASON: &str = "policy_preview_allowed";
const EXECUTION_DISABLED_REASON: &str = "execution_disabled";
const COMMAND_NOT_ALLOWLISTED_REASON: &str = "command_not_allowlisted";
const ARGV_MISMATCH_REASON: &str = "argv_mismatch";
const TIMEOUT_EXCEEDS_LIMIT_REASON: &str = "timeout_exceeds_limit";
const STDOUT_CAP_EXCEEDS_LIMIT_REASON: &str = "stdout_cap_exceeds_limit";
const STDERR_CAP_EXCEEDS_LIMIT_REASON: &str = "stderr_cap_exceeds_limit";
const ENVIRONMENT_NOT_ALLOWLISTED_REASON: &str = "environment_not_allowlisted";
const SAFE_ROOT_REQUIRED_REASON: &str = "safe_root_required";

/// Fixed command allowlist for the design-only execution gate.
///
/// Entries are logical identifiers with fixed argv vectors. They are not an MCP
/// runtime surface and cannot be extended by callers.
pub const COMMAND_ALLOWLIST: &[AllowedCommand] = &[AllowedCommand {
    id: "cargo_test_readiness",
    ordinal: 1,
    argv: &["cargo", "test", "--all-targets"],
    timeout_seconds: 120,
    max_stdout_bytes: 65_536,
    max_stderr_bytes: 65_536,
    allowed_environment_names: &["CARGO_TERM_COLOR"],
}];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllowedCommand {
    pub id: &'static str,
    pub ordinal: u64,
    pub argv: &'static [&'static str],
    pub timeout_seconds: u64,
    pub max_stdout_bytes: u64,
    pub max_stderr_bytes: u64,
    /// Environment variable names that may be supplied by a later runtime gate.
    /// This model intentionally never stores or exposes environment values.
    pub allowed_environment_names: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPolicyRequest<'a> {
    pub command_id: &'a str,
    pub argv: &'a [&'a str],
    pub timeout_seconds: u64,
    pub max_stdout_bytes: u64,
    pub max_stderr_bytes: u64,
    pub environment_names: &'a [&'a str],
    pub working_directory_is_safe_rooted: bool,
    pub execution_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPolicyDecision {
    pub allowed: bool,
    pub reason_code: &'static str,
    pub command_ordinal: Option<u64>,
    pub argv_count: usize,
    pub timeout_seconds: u64,
    pub max_stdout_bytes: u64,
    pub max_stderr_bytes: u64,
    pub environment_name_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandExecutionPolicy;

impl CommandExecutionPolicy {
    pub const fn new() -> Self {
        Self
    }

    pub fn evaluate(self, request: &CommandPolicyRequest<'_>) -> CommandPolicyDecision {
        let Some(command) = find_allowed_command(request.command_id) else {
            return denied(request, None, COMMAND_NOT_ALLOWLISTED_REASON);
        };

        if request.execution_requested && !COMMAND_EXECUTION_ENABLED {
            return denied(request, Some(command), EXECUTION_DISABLED_REASON);
        }

        if request.argv != command.argv {
            return denied(request, Some(command), ARGV_MISMATCH_REASON);
        }

        if request.timeout_seconds > command.timeout_seconds {
            return denied(request, Some(command), TIMEOUT_EXCEEDS_LIMIT_REASON);
        }

        if request.max_stdout_bytes > command.max_stdout_bytes {
            return denied(request, Some(command), STDOUT_CAP_EXCEEDS_LIMIT_REASON);
        }

        if request.max_stderr_bytes > command.max_stderr_bytes {
            return denied(request, Some(command), STDERR_CAP_EXCEEDS_LIMIT_REASON);
        }

        if !request.working_directory_is_safe_rooted {
            return denied(request, Some(command), SAFE_ROOT_REQUIRED_REASON);
        }

        if !environment_names_are_allowlisted(
            request.environment_names,
            command.allowed_environment_names,
        ) {
            return denied(request, Some(command), ENVIRONMENT_NOT_ALLOWLISTED_REASON);
        }

        CommandPolicyDecision {
            allowed: true,
            reason_code: POLICY_ALLOWED_REASON,
            command_ordinal: Some(command.ordinal),
            argv_count: request.argv.len(),
            timeout_seconds: request.timeout_seconds,
            max_stdout_bytes: request.max_stdout_bytes,
            max_stderr_bytes: request.max_stderr_bytes,
            environment_name_count: request.environment_names.len(),
        }
    }

    pub fn audit_decision(
        self,
        timestamp_unix_seconds: u64,
        request: &CommandPolicyRequest<'_>,
    ) -> AuditEvent {
        let decision = self.evaluate(request);
        let audit_decision = if decision.allowed {
            AuditDecision::Allowed
        } else {
            AuditDecision::Denied
        };
        let audit_mode = if request.execution_requested {
            AuditMode::Mutating
        } else {
            AuditMode::ReadOnly
        };

        let mut event = AuditEvent::new(
            timestamp_unix_seconds,
            COMMAND_EXECUTION_TOOL_NAME,
            COMMAND_EXECUTION_GATE,
            audit_mode,
            audit_decision,
            decision.reason_code,
        )
        .with_metadata(ARGV_COUNT_METADATA, usize_to_u64(decision.argv_count))
        .with_metadata(TIMEOUT_SECONDS_METADATA, decision.timeout_seconds)
        .with_metadata(MAX_STDOUT_BYTES_METADATA, decision.max_stdout_bytes)
        .with_metadata(MAX_STDERR_BYTES_METADATA, decision.max_stderr_bytes)
        .with_metadata(
            ENV_NAME_COUNT_METADATA,
            usize_to_u64(decision.environment_name_count),
        );

        if let Some(command_ordinal) = decision.command_ordinal {
            event = event.with_metadata(COMMAND_ORDINAL_METADATA, command_ordinal);
        }

        event
    }
}

impl Default for CommandExecutionPolicy {
    fn default() -> Self {
        Self::new()
    }
}

fn denied(
    request: &CommandPolicyRequest<'_>,
    command: Option<&AllowedCommand>,
    reason_code: &'static str,
) -> CommandPolicyDecision {
    CommandPolicyDecision {
        allowed: false,
        reason_code,
        command_ordinal: command.map(|allowed| allowed.ordinal),
        argv_count: request.argv.len(),
        timeout_seconds: request.timeout_seconds,
        max_stdout_bytes: request.max_stdout_bytes,
        max_stderr_bytes: request.max_stderr_bytes,
        environment_name_count: request.environment_names.len(),
    }
}

fn find_allowed_command(command_id: &str) -> Option<&'static AllowedCommand> {
    COMMAND_ALLOWLIST.iter().find(|command| command.id == command_id)
}

fn environment_names_are_allowlisted(requested: &[&str], allowed: &[&str]) -> bool {
    requested
        .iter()
        .all(|requested_name| allowed.contains(requested_name))
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn valid_request() -> CommandPolicyRequest<'static> {
        CommandPolicyRequest {
            command_id: "cargo_test_readiness",
            argv: &["cargo", "test", "--all-targets"],
            timeout_seconds: 120,
            max_stdout_bytes: 65_536,
            max_stderr_bytes: 65_536,
            environment_names: &["CARGO_TERM_COLOR"],
            working_directory_is_safe_rooted: true,
            execution_requested: false,
        }
    }

    #[test]
    fn command_execution_is_design_only_and_disabled() {
        let mut request = valid_request();
        request.execution_requested = true;

        let decision = CommandExecutionPolicy::new().evaluate(&request);
        assert!(!decision.allowed);
        assert_eq!(decision.reason_code, EXECUTION_DISABLED_REASON);
    }

    #[test]
    fn fixed_allowlist_policy_preview_can_be_allowed_without_execution() {
        let decision = CommandExecutionPolicy::new().evaluate(&valid_request());

        assert!(decision.allowed);
        assert_eq!(decision.reason_code, POLICY_ALLOWED_REASON);
        assert_eq!(decision.command_ordinal, Some(1));
        assert_eq!(decision.argv_count, 3);
    }

    #[test]
    fn disallowed_commands_are_denied() {
        let mut request = valid_request();
        request.command_id = "sh";

        let decision = CommandExecutionPolicy::new().evaluate(&request);
        assert!(!decision.allowed);
        assert_eq!(decision.reason_code, COMMAND_NOT_ALLOWLISTED_REASON);
        assert_eq!(decision.command_ordinal, None);
    }

    #[test]
    fn shell_injection_attempts_are_denied_by_fixed_argv() {
        for argv in [
            &["sh", "-c", "cargo test; id"][..],
            &["cargo", "test", "--all-targets", "&&", "id"][..],
            &["cargo", "test", "--all-targets;cat", "/etc/passwd"][..],
        ] {
            let mut request = valid_request();
            request.argv = argv;

            let decision = CommandExecutionPolicy::new().evaluate(&request);
            assert!(!decision.allowed);
            assert_eq!(decision.reason_code, ARGV_MISMATCH_REASON);
        }
    }

    #[test]
    fn timeout_above_allowlist_limit_is_denied() {
        let mut request = valid_request();
        request.timeout_seconds = 121;

        let decision = CommandExecutionPolicy::new().evaluate(&request);
        assert!(!decision.allowed);
        assert_eq!(decision.reason_code, TIMEOUT_EXCEEDS_LIMIT_REASON);
    }

    #[test]
    fn stdout_cap_above_allowlist_limit_is_denied() {
        let mut request = valid_request();
        request.max_stdout_bytes = 65_537;

        let decision = CommandExecutionPolicy::new().evaluate(&request);
        assert!(!decision.allowed);
        assert_eq!(decision.reason_code, STDOUT_CAP_EXCEEDS_LIMIT_REASON);
    }

    #[test]
    fn stderr_cap_above_allowlist_limit_is_denied() {
        let mut request = valid_request();
        request.max_stderr_bytes = 65_537;

        let decision = CommandExecutionPolicy::new().evaluate(&request);
        assert!(!decision.allowed);
        assert_eq!(decision.reason_code, STDERR_CAP_EXCEEDS_LIMIT_REASON);
    }

    #[test]
    fn environment_names_outside_allowlist_are_denied_without_reading_values() {
        let mut request = valid_request();
        request.environment_names = &["CARGO_TERM_COLOR", "TOKEN"];

        let decision = CommandExecutionPolicy::new().evaluate(&request);
        assert!(!decision.allowed);
        assert_eq!(decision.reason_code, ENVIRONMENT_NOT_ALLOWLISTED_REASON);
        assert_eq!(decision.environment_name_count, 2);
    }

    #[test]
    fn safe_root_violations_are_denied_without_serializing_paths() {
        let mut request = valid_request();
        request.working_directory_is_safe_rooted = false;

        let decision = CommandExecutionPolicy::new().evaluate(&request);
        assert!(!decision.allowed);
        assert_eq!(decision.reason_code, SAFE_ROOT_REQUIRED_REASON);
    }

    #[test]
    fn audit_event_is_emitted_per_policy_evaluation_without_sensitive_fields() {
        let mut request = valid_request();
        request.argv = &["cargo", "test", "--all-targets", "&&", "env"];
        request.environment_names = &["TOKEN"];

        let event = CommandExecutionPolicy::new().audit_decision(1_725_000_000, &request);
        let value = serde_json::to_value(event).unwrap();

        assert_eq!(value["timestamp_unix_seconds"], 1_725_000_000);
        assert_eq!(value["tool_name"], COMMAND_EXECUTION_TOOL_NAME);
        assert_eq!(value["gate_name"], COMMAND_EXECUTION_GATE);
        assert_eq!(value["mode"], "read_only");
        assert_eq!(value["decision"], "denied");
        assert_eq!(value["reason_code"], ARGV_MISMATCH_REASON);
        assert_eq!(value["metadata"][ARGV_COUNT_METADATA], 5);
        assert_eq!(value["metadata"][COMMAND_ORDINAL_METADATA], 1);
        assert_eq!(value["metadata"][MAX_STDOUT_BYTES_METADATA], 65_536);
        assert_eq!(value["metadata"][MAX_STDERR_BYTES_METADATA], 65_536);
        assert_eq!(value["metadata"][ENV_NAME_COUNT_METADATA], 1);

        assert_no_sensitive_command_tokens(&value);
    }

    #[test]
    fn audit_event_records_mutating_intent_for_execution_requests() {
        let mut request = valid_request();
        request.execution_requested = true;

        let event = CommandExecutionPolicy::new().audit_decision(2, &request);
        let value = serde_json::to_value(event).unwrap();

        assert_eq!(value["mode"], "mutating");
        assert_eq!(value["decision"], "denied");
        assert_eq!(value["reason_code"], EXECUTION_DISABLED_REASON);
    }

    fn assert_no_sensitive_command_tokens(value: &Value) {
        let text = value.to_string().to_ascii_lowercase();
        for token in [
            "token",
            "secret",
            "password",
            "bearer",
            "command_output",
            "cmdline",
            "/data/",
            "/home/",
            "/sdcard",
            "/storage/",
        ] {
            assert!(!text.contains(token), "unexpected sensitive token: {token}");
        }
    }
}
