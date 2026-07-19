//! Fixed-profile policy for the opt-in command diagnostics gate.
//!
//! The public request surface selects only a reviewed profile identifier. The
//! executable, argv, working directory, environment, timeout, and output limits
//! are all project-owned. No caller-controlled command string reaches process
//! construction.

use std::{fmt, time::Duration};

use crate::audit::{AuditDecision, AuditEvent, AuditMode};

pub const RUN_COMMAND_PROFILE_TOOL: &str = "run_command_profile";
pub const COMMAND_EXECUTION_GATE: &str = "fixed_command_execution";
pub const COMMAND_PROFILE_ORDINAL_METADATA: &str = "command_profile_ordinal";

pub const COMMAND_PROFILE_ALLOWED_REASON: &str = "command_profile_execution_allowed";
pub const COMMAND_FEATURE_DISABLED_REASON: &str = "command_feature_not_compiled";
pub const COMMAND_RUNTIME_DISABLED_REASON: &str = "command_runtime_disabled";
pub const COMMAND_MISSING_ARGUMENTS_REASON: &str = "command_profile_missing_arguments";
pub const COMMAND_INVALID_ARGUMENTS_REASON: &str = "command_profile_invalid_arguments";
pub const COMMAND_PROFILE_NOT_ALLOWLISTED_REASON: &str = "command_profile_not_allowlisted";
pub const COMMAND_SAFE_ROOT_UNAVAILABLE_REASON: &str = "command_safe_root_unavailable";

pub const COMMAND_PROGRAM_UNAVAILABLE_REASON: &str = "command_program_unavailable";
pub const COMMAND_SPAWN_FAILED_REASON: &str = "command_spawn_failed";
pub const COMMAND_WAIT_FAILED_REASON: &str = "command_wait_failed";
pub const COMMAND_TIMEOUT_REASON: &str = "command_timeout";
pub const COMMAND_STDOUT_LIMIT_REASON: &str = "command_stdout_limit_exceeded";
pub const COMMAND_STDERR_LIMIT_REASON: &str = "command_stderr_limit_exceeded";
pub const COMMAND_PROGRAM_FAILED_REASON: &str = "command_program_failed";
pub const COMMAND_OUTPUT_INVALID_UTF8_REASON: &str = "command_output_invalid_utf8";
pub const COMMAND_CONCURRENCY_LIMIT_REASON: &str = "command_concurrency_limit_exceeded";

pub const MAX_COMMAND_PROFILE_ID_BYTES: usize = 64;

/// Conservative profiles that can introspect only this exact server binary.
///
/// Program identity is resolved from the running executable, not from PATH or
/// request data. All profiles are read-only and have no placeholders.
pub(crate) const COMMAND_PROFILES: &[CommandProfile] = &[
    CommandProfile {
        id: "server_version",
        ordinal: 1,
        argv: &["--version"],
        timeout: Duration::from_secs(5),
        max_stdout_bytes: 4 * 1024,
        max_stderr_bytes: 1024,
    },
    CommandProfile {
        id: "server_help",
        ordinal: 2,
        argv: &["--help"],
        timeout: Duration::from_secs(5),
        max_stdout_bytes: 16 * 1024,
        max_stderr_bytes: 1024,
    },
    CommandProfile {
        id: "execution_boundary",
        ordinal: 3,
        argv: &["--self-check-command-boundary"],
        timeout: Duration::from_secs(5),
        max_stdout_bytes: 1024,
        max_stderr_bytes: 1024,
    },
];

/// Opaque policy-owned command profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandProfile {
    id: &'static str,
    ordinal: u64,
    argv: &'static [&'static str],
    timeout: Duration,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
}

#[cfg(feature = "command-execution")]
impl CommandProfile {
    pub(crate) const fn id(&self) -> &'static str {
        self.id
    }

    pub(crate) const fn argv(&self) -> &'static [&'static str] {
        self.argv
    }

    pub(crate) const fn timeout(&self) -> Duration {
        self.timeout
    }

    pub(crate) const fn max_stdout_bytes(&self) -> usize {
        self.max_stdout_bytes
    }

    pub(crate) const fn max_stderr_bytes(&self) -> usize {
        self.max_stderr_bytes
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CommandPolicyDecision {
    pub allowed: bool,
    pub reason_code: &'static str,
    pub(crate) profile: Option<&'static CommandProfile>,
}

impl fmt::Debug for CommandPolicyDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandPolicyDecision")
            .field("allowed", &self.allowed)
            .field("reason_code", &self.reason_code)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommandExecutionPolicy;

impl CommandExecutionPolicy {
    pub const fn new() -> Self {
        Self
    }

    pub fn evaluate(
        self,
        profile_id: &str,
        runtime_enabled: bool,
        safe_root_available: bool,
    ) -> CommandPolicyDecision {
        // Do not disclose allowlist membership through a disabled gate.
        if !runtime_enabled {
            return denied(COMMAND_RUNTIME_DISABLED_REASON, None);
        }

        let profile = if profile_id.len() <= MAX_COMMAND_PROFILE_ID_BYTES {
            COMMAND_PROFILES
                .iter()
                .find(|profile| profile.id == profile_id)
        } else {
            None
        };
        let Some(profile) = profile else {
            return denied(COMMAND_PROFILE_NOT_ALLOWLISTED_REASON, None);
        };

        if !safe_root_available {
            return denied(COMMAND_SAFE_ROOT_UNAVAILABLE_REASON, Some(profile));
        }

        CommandPolicyDecision {
            allowed: true,
            reason_code: COMMAND_PROFILE_ALLOWED_REASON,
            profile: Some(profile),
        }
    }

    pub fn audit_decision(
        self,
        timestamp_unix_seconds: u64,
        decision: &CommandPolicyDecision,
    ) -> AuditEvent {
        let mut event = AuditEvent::new(
            timestamp_unix_seconds,
            RUN_COMMAND_PROFILE_TOOL,
            COMMAND_EXECUTION_GATE,
            AuditMode::ReadOnly,
            if decision.allowed {
                AuditDecision::Allowed
            } else {
                AuditDecision::Denied
            },
            decision.reason_code,
        );
        if let Some(profile) = decision.profile {
            event = event.with_metadata(COMMAND_PROFILE_ORDINAL_METADATA, profile.ordinal);
        }
        event
    }
}

#[cfg(all(test, feature = "command-execution"))]
pub(crate) fn test_command_profile(
    argv: &'static [&'static str],
    timeout: Duration,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
) -> &'static CommandProfile {
    Box::leak(Box::new(CommandProfile {
        id: "test_profile",
        ordinal: u64::MAX,
        argv,
        timeout,
        max_stdout_bytes,
        max_stderr_bytes,
    }))
}

#[cfg(test)]
pub(crate) fn command_profile(profile_id: &str) -> Option<&'static CommandProfile> {
    COMMAND_PROFILES
        .iter()
        .find(|profile| profile.id == profile_id)
}

pub fn command_profile_ids() -> impl Iterator<Item = &'static str> {
    COMMAND_PROFILES.iter().map(|profile| profile.id)
}

fn denied(
    reason_code: &'static str,
    profile: Option<&'static CommandProfile>,
) -> CommandPolicyDecision {
    CommandPolicyDecision {
        allowed: false,
        reason_code,
        profile,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;

    use super::*;

    #[test]
    fn disabled_gate_does_not_disclose_profile_membership() {
        for profile in ["server_version", "sh", "server_version;id"] {
            let decision = CommandExecutionPolicy::new().evaluate(profile, false, true);
            assert!(!decision.allowed);
            assert_eq!(decision.reason_code, COMMAND_RUNTIME_DISABLED_REASON);
            assert_eq!(decision.profile, None);
        }
    }

    #[test]
    fn enabled_gate_resolves_only_fixed_allowlisted_profiles() {
        for profile in COMMAND_PROFILES {
            let decision = CommandExecutionPolicy::new().evaluate(profile.id, true, true);
            assert!(decision.allowed);
            assert_eq!(decision.reason_code, COMMAND_PROFILE_ALLOWED_REASON);
            assert_eq!(decision.profile, Some(profile));
        }
    }

    #[test]
    fn raw_commands_and_injection_shapes_are_not_profiles() {
        for profile in [
            "sh",
            "bash -c id",
            "server_version;id",
            "server_help&&env",
            "../server_version",
            "/data/data/com.termux/files/usr/bin/sh",
            "x\0y",
        ] {
            let decision = CommandExecutionPolicy::new().evaluate(profile, true, true);
            assert!(!decision.allowed);
            assert_eq!(decision.reason_code, COMMAND_PROFILE_NOT_ALLOWLISTED_REASON);
            assert_eq!(decision.profile, None);
        }
    }

    #[test]
    fn oversized_profile_identifier_is_rejected_before_comparison() {
        let profile = "x".repeat(MAX_COMMAND_PROFILE_ID_BYTES + 1);
        let decision = CommandExecutionPolicy::new().evaluate(&profile, true, true);
        assert!(!decision.allowed);
        assert_eq!(decision.reason_code, COMMAND_PROFILE_NOT_ALLOWLISTED_REASON);
    }

    #[test]
    fn safe_root_is_required_after_allowlist_resolution() {
        let decision = CommandExecutionPolicy::new().evaluate("server_version", true, false);
        assert!(!decision.allowed);
        assert_eq!(decision.reason_code, COMMAND_SAFE_ROOT_UNAVAILABLE_REASON);
        assert_eq!(decision.profile.map(|profile| profile.ordinal), Some(1));
    }

    #[test]
    fn profile_registry_is_unique_fixed_and_bounded() {
        let mut ids = BTreeSet::new();
        let mut ordinals = BTreeSet::new();

        for profile in COMMAND_PROFILES {
            assert!(ids.insert(profile.id));
            assert!(ordinals.insert(profile.ordinal));
            assert!(!profile.id.is_empty());
            assert!(profile.id.len() <= MAX_COMMAND_PROFILE_ID_BYTES);
            assert!(!profile.argv.is_empty());
            assert!(profile.timeout >= Duration::from_millis(4));
            assert!(profile.max_stdout_bytes > 0);
            assert!(profile.max_stderr_bytes > 0);

            for argument in profile.argv {
                assert!(!argument.is_empty());
                assert!(!argument.contains('\0'));
                for shell_token in [";", "&&", "||", "|", ">", "<", "$(", "`"] {
                    assert!(!argument.contains(shell_token));
                }
            }
        }
    }

    #[test]
    fn profile_helpers_preserve_canonical_registry_order() {
        assert_eq!(
            command_profile_ids().collect::<Vec<_>>(),
            vec!["server_version", "server_help", "execution_boundary"]
        );
        assert_eq!(
            command_profile("server_version").unwrap().argv,
            ["--version"]
        );
        assert_eq!(command_profile("missing"), None);
    }

    #[test]
    fn public_decision_debug_never_exposes_the_resolved_profile() {
        let decision = CommandExecutionPolicy::new().evaluate("server_version", true, true);
        let debug = format!("{decision:?}");
        assert!(debug.contains("allowed: true"));
        assert!(debug.contains(COMMAND_PROFILE_ALLOWED_REASON));
        for private in [
            "server_version",
            "--version",
            "timeout",
            "max_stdout",
            "ordinal",
        ] {
            assert!(!debug.contains(private), "debug leaked {private}");
        }
    }

    #[test]
    fn audit_events_use_only_stable_profile_ordinal_and_reason() {
        let policy = CommandExecutionPolicy::new();
        let allowed = policy.evaluate("server_version", true, true);
        let value = serde_json::to_value(policy.audit_decision(1_725_000_000, &allowed)).unwrap();

        assert_eq!(value["tool_name"], RUN_COMMAND_PROFILE_TOOL);
        assert_eq!(value["gate_name"], COMMAND_EXECUTION_GATE);
        assert_eq!(value["mode"], "read_only");
        assert_eq!(value["decision"], "allowed");
        assert_eq!(value["reason_code"], COMMAND_PROFILE_ALLOWED_REASON);
        assert_eq!(value["metadata"][COMMAND_PROFILE_ORDINAL_METADATA], 1);
        assert_no_sensitive_command_tokens(&value);

        let denied = policy.evaluate("private raw command", true, true);
        let value = serde_json::to_value(policy.audit_decision(1_725_000_001, &denied)).unwrap();
        assert_eq!(value["decision"], "denied");
        assert_eq!(value.get("metadata"), None);
        assert_no_sensitive_command_tokens(&value);
    }

    fn assert_no_sensitive_command_tokens(value: &Value) {
        let text = value.to_string().to_ascii_lowercase();
        for token in [
            "password",
            "secret",
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
