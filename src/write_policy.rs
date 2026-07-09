//! Default-deny write policy helpers for staged MCP filesystem write support.
//!
//! The MCP transport must remain dry-run-first. These helpers keep that intent
//! centralized before the write-capable transport surface is exposed.

use std::fmt;

use crate::audit::{AuditDecision, AuditEvent, AuditMode};

pub const DEFAULT_MAX_WRITE_BYTES: usize = 1_048_576;

const WRITE_FILE_TOOL_NAME: &str = "write_file";
const FILESYSTEM_WRITE_GATE: &str = "filesystem_write";
const CONTENT_BYTES_METADATA: &str = "content_bytes";
const MAX_BYTES_METADATA: &str = "max_bytes";
const DRY_RUN_REASON: &str = "dry_run_preview";
const MUTATING_REASON: &str = "explicit_mutation";
const PAYLOAD_TOO_LARGE_REASON: &str = "payload_too_large";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    DryRun,
    Mutating,
}

impl WriteMode {
    const fn audit_mode(self) -> AuditMode {
        match self {
            Self::DryRun => AuditMode::DryRun,
            Self::Mutating => AuditMode::Mutating,
        }
    }

    const fn allowed_reason_code(self) -> &'static str {
        match self {
            Self::DryRun => DRY_RUN_REASON,
            Self::Mutating => MUTATING_REASON,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WritePolicy {
    max_write_bytes: usize,
}

impl Default for WritePolicy {
    fn default() -> Self {
        Self {
            max_write_bytes: DEFAULT_MAX_WRITE_BYTES,
        }
    }
}

impl WritePolicy {
    pub const fn new(max_write_bytes: usize) -> Self {
        Self { max_write_bytes }
    }

    pub const fn max_write_bytes(self) -> usize {
        self.max_write_bytes
    }

    pub fn resolve_mode(self, dry_run: Option<bool>) -> WriteMode {
        if dry_run.unwrap_or(true) {
            WriteMode::DryRun
        } else {
            WriteMode::Mutating
        }
    }

    pub fn validate_payload_size(self, bytes: usize) -> Result<(), WritePolicyError> {
        if bytes > self.max_write_bytes {
            Err(WritePolicyError::PayloadTooLarge {
                bytes,
                max_bytes: self.max_write_bytes,
            })
        } else {
            Ok(())
        }
    }

    pub fn audit_payload_decision(
        self,
        timestamp_unix_seconds: u64,
        bytes: usize,
        dry_run: Option<bool>,
    ) -> AuditEvent {
        let mode = self.resolve_mode(dry_run);

        match self.validate_payload_size(bytes) {
            Ok(()) => write_audit_event(
                timestamp_unix_seconds,
                mode.audit_mode(),
                AuditDecision::Allowed,
                mode.allowed_reason_code(),
                bytes,
                self.max_write_bytes,
            ),
            Err(WritePolicyError::PayloadTooLarge { bytes, max_bytes }) => write_audit_event(
                timestamp_unix_seconds,
                mode.audit_mode(),
                AuditDecision::Denied,
                PAYLOAD_TOO_LARGE_REASON,
                bytes,
                max_bytes,
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePolicyError {
    PayloadTooLarge { bytes: usize, max_bytes: usize },
}

impl fmt::Display for WritePolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge { bytes, max_bytes } => {
                let unit = if *bytes == 1 { "byte" } else { "bytes" };

                write!(
                    formatter,
                    "write payload is {bytes} {unit}, which exceeds the {max_bytes}-byte limit"
                )
            }
        }
    }
}

impl std::error::Error for WritePolicyError {}

fn write_audit_event(
    timestamp_unix_seconds: u64,
    mode: AuditMode,
    decision: AuditDecision,
    reason_code: &'static str,
    bytes: usize,
    max_bytes: usize,
) -> AuditEvent {
    AuditEvent::new(
        timestamp_unix_seconds,
        WRITE_FILE_TOOL_NAME,
        FILESYSTEM_WRITE_GATE,
        mode,
        decision,
        reason_code,
    )
    .with_metadata(CONTENT_BYTES_METADATA, usize_to_u64(bytes))
    .with_metadata(MAX_BYTES_METADATA, usize_to_u64(max_bytes))
}

fn usize_to_u64(value: usize) -> u64 {
    value as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_mode_defaults_to_dry_run_when_omitted() {
        let policy = WritePolicy::default();

        assert_eq!(policy.resolve_mode(None), WriteMode::DryRun);
        assert_eq!(policy.resolve_mode(Some(true)), WriteMode::DryRun);
    }

    #[test]
    fn write_mode_requires_explicit_false_for_mutation() {
        let policy = WritePolicy::default();

        assert_eq!(policy.resolve_mode(Some(false)), WriteMode::Mutating);
    }

    #[test]
    fn write_payload_size_allows_exact_limit() {
        let policy = WritePolicy::new(16);

        assert_eq!(policy.validate_payload_size(16), Ok(()));
    }

    #[test]
    fn write_payload_size_rejects_above_limit() {
        let policy = WritePolicy::new(16);

        assert_eq!(
            policy.validate_payload_size(17),
            Err(WritePolicyError::PayloadTooLarge {
                bytes: 17,
                max_bytes: 16,
            })
        );
    }

    #[test]
    fn write_payload_size_zero_limit_accepts_zero_bytes() {
        let policy = WritePolicy::new(0);

        assert_eq!(policy.validate_payload_size(0), Ok(()));
    }

    #[test]
    fn write_payload_size_zero_limit_rejects_nonzero_bytes() {
        let policy = WritePolicy::new(0);

        assert_eq!(
            policy.validate_payload_size(1),
            Err(WritePolicyError::PayloadTooLarge {
                bytes: 1,
                max_bytes: 0,
            })
        );
    }

    #[test]
    fn write_policy_error_formats_payload_limit() {
        let error = WritePolicyError::PayloadTooLarge {
            bytes: 17,
            max_bytes: 16,
        };

        assert_eq!(
            error.to_string(),
            "write payload is 17 bytes, which exceeds the 16-byte limit"
        );
    }

    #[test]
    fn write_policy_error_formats_singular_payload_size() {
        let error = WritePolicyError::PayloadTooLarge {
            bytes: 1,
            max_bytes: 0,
        };

        assert_eq!(
            error.to_string(),
            "write payload is 1 byte, which exceeds the 0-byte limit"
        );
    }

    #[test]
    fn write_policy_audits_default_dry_run_preview() {
        let event = WritePolicy::new(16).audit_payload_decision(1_725_000_000, 7, None);

        assert_eq!(event.timestamp_unix_seconds, 1_725_000_000);
        assert_eq!(event.tool_name, WRITE_FILE_TOOL_NAME);
        assert_eq!(event.gate_name, FILESYSTEM_WRITE_GATE);
        assert_eq!(event.mode, AuditMode::DryRun);
        assert_eq!(event.decision, AuditDecision::Allowed);
        assert_eq!(event.reason_code, DRY_RUN_REASON);
        assert_eq!(event.metadata[CONTENT_BYTES_METADATA], 7);
        assert_eq!(event.metadata[MAX_BYTES_METADATA], 16);
    }

    #[test]
    fn write_policy_audits_explicit_mutation() {
        let event = WritePolicy::new(16).audit_payload_decision(2, 16, Some(false));

        assert_eq!(event.mode, AuditMode::Mutating);
        assert_eq!(event.decision, AuditDecision::Allowed);
        assert_eq!(event.reason_code, MUTATING_REASON);
        assert_eq!(event.metadata[CONTENT_BYTES_METADATA], 16);
        assert_eq!(event.metadata[MAX_BYTES_METADATA], 16);
    }

    #[test]
    fn write_policy_audits_oversized_payload_denial_without_content() {
        let event = WritePolicy::new(16).audit_payload_decision(3, 17, Some(false));

        assert_eq!(event.mode, AuditMode::Mutating);
        assert_eq!(event.decision, AuditDecision::Denied);
        assert_eq!(event.reason_code, PAYLOAD_TOO_LARGE_REASON);
        assert_eq!(event.metadata[CONTENT_BYTES_METADATA], 17);
        assert_eq!(event.metadata[MAX_BYTES_METADATA], 16);

        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value.get("path"), None);
        assert_eq!(value.get("content"), None);
        assert_eq!(value.get("file_content"), None);
    }
}
