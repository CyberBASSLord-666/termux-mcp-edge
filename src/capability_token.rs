//! Inert high-impact capability-token policy primitives.
//!
//! This module models future high-impact authorization decisions without
//! enabling any high-impact runtime surface. It intentionally does not accept,
//! generate, persist, serialize, or validate raw bearer tokens or secrets.
//! Callers provide only bounded stable metadata.

use serde::Serialize;
use std::{error::Error, fmt};

pub const MAX_CAPABILITY_SCOPE_BYTES: usize = 96;
pub const MAX_CAPABILITY_GRANT_ID_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityClass {
    FilesystemMutation,
    AndroidPlatformControl,
    PackageManagement,
    ProjectServiceMutation,
    NetworkMutation,
    DeviceControl,
    CommandExecution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityMetadataField {
    GrantId,
    Scope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityMetadataErrorKind {
    Empty,
    TooLong,
    InvalidCharacter,
    InvalidSeparator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityMetadataError {
    pub field: CapabilityMetadataField,
    pub kind: CapabilityMetadataErrorKind,
    pub max_bytes: usize,
}

impl fmt::Display for CapabilityMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let field = match self.field {
            CapabilityMetadataField::GrantId => "grant id",
            CapabilityMetadataField::Scope => "scope",
        };
        let reason = match self.kind {
            CapabilityMetadataErrorKind::Empty => "must not be empty",
            CapabilityMetadataErrorKind::TooLong => "exceeds its byte limit",
            CapabilityMetadataErrorKind::InvalidCharacter => "contains an invalid character",
            CapabilityMetadataErrorKind::InvalidSeparator => "has an invalid separator layout",
        };
        write!(formatter, "{field} {reason}")
    }
}

impl Error for CapabilityMetadataError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityRequirement {
    pub capability_class: CapabilityClass,
    pub scope: String,
    pub confirmation_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityGrant {
    pub grant_id: String,
    pub capability_class: CapabilityClass,
    pub scope: String,
    pub expires_unix_seconds: u64,
    pub active: bool,
    pub confirmation_satisfied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityEvaluation {
    pub decision: CapabilityDecision,
    pub reason_code: CapabilityReasonCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    pub capability_class: CapabilityClass,
    pub scope: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDecision {
    Allowed,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityReasonCode {
    CapabilityGrantAllowed,
    CapabilityGrantMissing,
    CapabilityGrantInactive,
    CapabilityGrantExpired,
    CapabilityClassMismatch,
    CapabilityScopeMismatch,
    CapabilityConfirmationRequired,
}

impl CapabilityRequirement {
    pub fn new(
        capability_class: CapabilityClass,
        scope: impl Into<String>,
        confirmation_required: bool,
    ) -> Result<Self, CapabilityMetadataError> {
        let scope = scope.into();
        validate_label(
            &scope,
            CapabilityMetadataField::Scope,
            MAX_CAPABILITY_SCOPE_BYTES,
        )?;
        Ok(Self {
            capability_class,
            scope,
            confirmation_required,
        })
    }
}

impl CapabilityGrant {
    pub fn new(
        grant_id: impl Into<String>,
        capability_class: CapabilityClass,
        scope: impl Into<String>,
        expires_unix_seconds: u64,
    ) -> Result<Self, CapabilityMetadataError> {
        let grant_id = grant_id.into();
        let scope = scope.into();
        validate_label(
            &grant_id,
            CapabilityMetadataField::GrantId,
            MAX_CAPABILITY_GRANT_ID_BYTES,
        )?;
        validate_label(
            &scope,
            CapabilityMetadataField::Scope,
            MAX_CAPABILITY_SCOPE_BYTES,
        )?;
        Ok(Self {
            grant_id,
            capability_class,
            scope,
            expires_unix_seconds,
            active: true,
            confirmation_satisfied: false,
        })
    }

    pub fn inactive(mut self) -> Self {
        self.active = false;
        self
    }

    pub fn with_confirmation_satisfied(mut self, confirmation_satisfied: bool) -> Self {
        self.confirmation_satisfied = confirmation_satisfied;
        self
    }
}

fn validate_label(
    value: &str,
    field: CapabilityMetadataField,
    max_bytes: usize,
) -> Result<(), CapabilityMetadataError> {
    if value.is_empty() {
        return Err(CapabilityMetadataError {
            field,
            kind: CapabilityMetadataErrorKind::Empty,
            max_bytes,
        });
    }
    if value.len() > max_bytes {
        return Err(CapabilityMetadataError {
            field,
            kind: CapabilityMetadataErrorKind::TooLong,
            max_bytes,
        });
    }

    let mut previous_was_separator = false;
    for (index, byte) in value.bytes().enumerate() {
        let is_separator = matches!(byte, b'-' | b'_' | b':');
        let valid = byte.is_ascii_lowercase() || byte.is_ascii_digit() || is_separator;
        if !valid {
            return Err(CapabilityMetadataError {
                field,
                kind: CapabilityMetadataErrorKind::InvalidCharacter,
                max_bytes,
            });
        }
        if is_separator && (index == 0 || index + 1 == value.len() || previous_was_separator) {
            return Err(CapabilityMetadataError {
                field,
                kind: CapabilityMetadataErrorKind::InvalidSeparator,
                max_bytes,
            });
        }
        previous_was_separator = is_separator;
    }
    Ok(())
}

#[rustfmt::skip]
pub fn evaluate_capability_grant(
    requirement: &CapabilityRequirement,
    grant: Option<&CapabilityGrant>,
    now_unix_seconds: u64,
) -> CapabilityEvaluation {
    let Some(grant) = grant else {
        return denied(requirement, None, CapabilityReasonCode::CapabilityGrantMissing);
    };
    if !grant.active {
        return denied(requirement, Some(grant), CapabilityReasonCode::CapabilityGrantInactive);
    }
    if now_unix_seconds >= grant.expires_unix_seconds {
        return denied(requirement, Some(grant), CapabilityReasonCode::CapabilityGrantExpired);
    }
    if grant.capability_class != requirement.capability_class {
        return denied(requirement, Some(grant), CapabilityReasonCode::CapabilityClassMismatch);
    }
    if grant.scope != requirement.scope {
        return denied(requirement, Some(grant), CapabilityReasonCode::CapabilityScopeMismatch);
    }
    if requirement.confirmation_required && !grant.confirmation_satisfied {
        return denied(requirement, Some(grant), CapabilityReasonCode::CapabilityConfirmationRequired);
    }

    CapabilityEvaluation {
        decision: CapabilityDecision::Allowed,
        reason_code: CapabilityReasonCode::CapabilityGrantAllowed,
        grant_id: Some(grant.grant_id.clone()),
        capability_class: requirement.capability_class,
        scope: requirement.scope.clone(),
    }
}

fn denied(
    requirement: &CapabilityRequirement,
    grant: Option<&CapabilityGrant>,
    reason_code: CapabilityReasonCode,
) -> CapabilityEvaluation {
    CapabilityEvaluation {
        decision: CapabilityDecision::Denied,
        reason_code,
        grant_id: grant.map(|grant| grant.grant_id.clone()),
        capability_class: requirement.capability_class,
        scope: requirement.scope.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_725_000_000;

    fn requirement(
        class: CapabilityClass,
        scope: &str,
        confirmation: bool,
    ) -> CapabilityRequirement {
        CapabilityRequirement::new(class, scope, confirmation).unwrap()
    }

    fn grant(id: &str, class: CapabilityClass, scope: &str, expiry: u64) -> CapabilityGrant {
        CapabilityGrant::new(id, class, scope, expiry).unwrap()
    }

    #[test]
    fn allows_matching_active_unexpired_confirmed_grant() {
        let requirement = requirement(
            CapabilityClass::ProjectServiceMutation,
            "project-service:restart",
            true,
        );
        let grant = grant(
            "grant-status-restart-001",
            CapabilityClass::ProjectServiceMutation,
            "project-service:restart",
            NOW + 60,
        )
        .with_confirmation_satisfied(true);
        let evaluation = evaluate_capability_grant(&requirement, Some(&grant), NOW);
        assert_eq!(evaluation.decision, CapabilityDecision::Allowed);
        assert_eq!(
            evaluation.reason_code,
            CapabilityReasonCode::CapabilityGrantAllowed
        );
    }

    #[test]
    fn filesystem_mutation_is_an_explicit_capability_class() {
        let requirement = requirement(
            CapabilityClass::FilesystemMutation,
            "filesystem:write-file",
            true,
        );
        let grant = grant(
            "grant-write-file-001",
            CapabilityClass::FilesystemMutation,
            "filesystem:write-file",
            NOW + 60,
        )
        .with_confirmation_satisfied(true);

        let evaluation = evaluate_capability_grant(&requirement, Some(&grant), NOW);
        assert_eq!(evaluation.decision, CapabilityDecision::Allowed);
        assert_eq!(
            evaluation.reason_code,
            CapabilityReasonCode::CapabilityGrantAllowed
        );
    }

    #[test]
    fn preserves_stable_serialized_shape() {
        let requirement = requirement(
            CapabilityClass::CommandExecution,
            "command-profile:diagnostics",
            false,
        );
        let grant = grant(
            "grant-diagnostics-001",
            CapabilityClass::CommandExecution,
            "command-profile:diagnostics",
            NOW + 60,
        );
        assert_eq!(
            serde_json::to_value(evaluate_capability_grant(&requirement, Some(&grant), NOW))
                .unwrap(),
            json!({
                "decision": "allowed",
                "reason_code": "capability_grant_allowed",
                "grant_id": "grant-diagnostics-001",
                "capability_class": "command_execution",
                "scope": "command-profile:diagnostics",
            })
        );
    }

    #[test]
    fn rejects_empty_oversized_and_malformed_metadata() {
        assert_eq!(
            CapabilityRequirement::new(CapabilityClass::DeviceControl, "", false)
                .unwrap_err()
                .kind,
            CapabilityMetadataErrorKind::Empty
        );
        assert_eq!(
            CapabilityRequirement::new(
                CapabilityClass::DeviceControl,
                "a".repeat(MAX_CAPABILITY_SCOPE_BYTES + 1),
                false,
            )
            .unwrap_err()
            .kind,
            CapabilityMetadataErrorKind::TooLong
        );
        for value in [
            "UPPER",
            "has space",
            "/data/path",
            ":leading",
            "trailing:",
            "double::separator",
        ] {
            assert!(
                CapabilityRequirement::new(CapabilityClass::DeviceControl, value, false).is_err(),
                "{value}"
            );
        }
        assert!(CapabilityGrant::new(
            "grant__bad",
            CapabilityClass::DeviceControl,
            "device:wake-lock",
            NOW + 1,
        )
        .is_err());
    }

    #[test]
    fn accepts_exact_byte_limits() {
        let scope = "a".repeat(MAX_CAPABILITY_SCOPE_BYTES);
        let grant_id = "g".repeat(MAX_CAPABILITY_GRANT_ID_BYTES);
        assert!(
            CapabilityRequirement::new(CapabilityClass::DeviceControl, scope.clone(), false)
                .is_ok()
        );
        assert!(
            CapabilityGrant::new(grant_id, CapabilityClass::DeviceControl, scope, NOW + 1).is_ok()
        );
    }

    #[test]
    fn denial_precedence_remains_stable() {
        let requirement = requirement(
            CapabilityClass::ProjectServiceMutation,
            "project-service:restart",
            true,
        );
        let inactive = grant(
            "grant-inactive-001",
            CapabilityClass::ProjectServiceMutation,
            "project-service:restart",
            NOW + 60,
        )
        .inactive();
        assert_eq!(
            evaluate_capability_grant(&requirement, Some(&inactive), NOW).reason_code,
            CapabilityReasonCode::CapabilityGrantInactive
        );
        let expired = grant(
            "grant-expired-001",
            CapabilityClass::ProjectServiceMutation,
            "project-service:restart",
            NOW,
        );
        assert_eq!(
            evaluate_capability_grant(&requirement, Some(&expired), NOW).reason_code,
            CapabilityReasonCode::CapabilityGrantExpired
        );
    }
}
