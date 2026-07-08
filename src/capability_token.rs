//! Inert high-impact capability-token policy primitives.
//!
//! This module models future high-impact authorization decisions without
//! enabling any high-impact runtime surface. It intentionally does not accept,
//! generate, persist, serialize, or validate raw bearer tokens or secrets.
//! Callers provide only stable grant metadata such as an opaque identifier,
//! capability class, bounded scope label, expiry, active state, and whether a
//! separate operator confirmation has already been satisfied.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityClass {
    AndroidPlatformControl,
    PackageManagement,
    ProjectServiceMutation,
    NetworkMutation,
    DeviceControl,
    CommandExecution,
}

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
    ) -> Self {
        Self {
            capability_class,
            scope: scope.into(),
            confirmation_required,
        }
    }
}

impl CapabilityGrant {
    pub fn new(
        grant_id: impl Into<String>,
        capability_class: CapabilityClass,
        scope: impl Into<String>,
        expires_unix_seconds: u64,
    ) -> Self {
        Self {
            grant_id: grant_id.into(),
            capability_class,
            scope: scope.into(),
            expires_unix_seconds,
            active: true,
            confirmation_satisfied: false,
        }
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

pub fn evaluate_capability_grant(
    requirement: &CapabilityRequirement,
    grant: Option<&CapabilityGrant>,
    now_unix_seconds: u64,
) -> CapabilityEvaluation {
    let Some(grant) = grant else {
        return denied(
            requirement,
            None,
            CapabilityReasonCode::CapabilityGrantMissing,
        );
    };

    if !grant.active {
        return denied(
            requirement,
            Some(grant),
            CapabilityReasonCode::CapabilityGrantInactive,
        );
    }

    if now_unix_seconds >= grant.expires_unix_seconds {
        return denied(
            requirement,
            Some(grant),
            CapabilityReasonCode::CapabilityGrantExpired,
        );
    }

    if grant.capability_class != requirement.capability_class {
        return denied(
            requirement,
            Some(grant),
            CapabilityReasonCode::CapabilityClassMismatch,
        );
    }

    if grant.scope != requirement.scope {
        return denied(
            requirement,
            Some(grant),
            CapabilityReasonCode::CapabilityScopeMismatch,
        );
    }

    if requirement.confirmation_required && !grant.confirmation_satisfied {
        return denied(
            requirement,
            Some(grant),
            CapabilityReasonCode::CapabilityConfirmationRequired,
        );
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
    use serde_json::{json, Value};

    const NOW: u64 = 1_725_000_000;

    #[test]
    fn allows_matching_active_unexpired_confirmed_grant() {
        let requirement = CapabilityRequirement::new(
            CapabilityClass::ProjectServiceMutation,
            "project-service:restart",
            true,
        );
        let grant = CapabilityGrant::new(
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
        assert_eq!(evaluation.grant_id.as_deref(), Some("grant-status-restart-001"));
        assert_non_sensitive_json(&serde_json::to_value(evaluation).unwrap());
    }

    #[test]
    fn denies_missing_grant() {
        let requirement = CapabilityRequirement::new(
            CapabilityClass::AndroidPlatformControl,
            "android:read-settings-preview",
            false,
        );

        let evaluation = evaluate_capability_grant(&requirement, None, NOW);

        assert_eq!(evaluation.decision, CapabilityDecision::Denied);
        assert_eq!(
            evaluation.reason_code,
            CapabilityReasonCode::CapabilityGrantMissing
        );
        assert_eq!(evaluation.grant_id, None);
    }

    #[test]
    fn denies_inactive_grant() {
        let requirement = CapabilityRequirement::new(
            CapabilityClass::NetworkMutation,
            "network:tunnel-reload",
            true,
        );
        let grant = CapabilityGrant::new(
            "grant-network-001",
            CapabilityClass::NetworkMutation,
            "network:tunnel-reload",
            NOW + 60,
        )
        .inactive()
        .with_confirmation_satisfied(true);

        let evaluation = evaluate_capability_grant(&requirement, Some(&grant), NOW);

        assert_eq!(evaluation.decision, CapabilityDecision::Denied);
        assert_eq!(
            evaluation.reason_code,
            CapabilityReasonCode::CapabilityGrantInactive
        );
    }

    #[test]
    fn denies_expired_grant_at_boundary() {
        let requirement = CapabilityRequirement::new(
            CapabilityClass::CommandExecution,
            "command-profile:status-only",
            true,
        );
        let grant = CapabilityGrant::new(
            "grant-command-001",
            CapabilityClass::CommandExecution,
            "command-profile:status-only",
            NOW,
        )
        .with_confirmation_satisfied(true);

        let evaluation = evaluate_capability_grant(&requirement, Some(&grant), NOW);

        assert_eq!(evaluation.decision, CapabilityDecision::Denied);
        assert_eq!(
            evaluation.reason_code,
            CapabilityReasonCode::CapabilityGrantExpired
        );
    }

    #[test]
    fn denies_capability_class_mismatch() {
        let requirement = CapabilityRequirement::new(
            CapabilityClass::PackageManagement,
            "package:install-preview",
            true,
        );
        let grant = CapabilityGrant::new(
            "grant-wrong-class-001",
            CapabilityClass::NetworkMutation,
            "package:install-preview",
            NOW + 60,
        )
        .with_confirmation_satisfied(true);

        let evaluation = evaluate_capability_grant(&requirement, Some(&grant), NOW);

        assert_eq!(evaluation.decision, CapabilityDecision::Denied);
        assert_eq!(
            evaluation.reason_code,
            CapabilityReasonCode::CapabilityClassMismatch
        );
    }

    #[test]
    fn denies_scope_mismatch() {
        let requirement = CapabilityRequirement::new(
            CapabilityClass::DeviceControl,
            "device:wake-lock-preview",
            true,
        );
        let grant = CapabilityGrant::new(
            "grant-wrong-scope-001",
            CapabilityClass::DeviceControl,
            "device:notification-preview",
            NOW + 60,
        )
        .with_confirmation_satisfied(true);

        let evaluation = evaluate_capability_grant(&requirement, Some(&grant), NOW);

        assert_eq!(evaluation.decision, CapabilityDecision::Denied);
        assert_eq!(
            evaluation.reason_code,
            CapabilityReasonCode::CapabilityScopeMismatch
        );
    }

    #[test]
    fn denies_missing_required_confirmation() {
        let requirement = CapabilityRequirement::new(
            CapabilityClass::ProjectServiceMutation,
            "project-service:restart",
            true,
        );
        let grant = CapabilityGrant::new(
            "grant-needs-confirmation-001",
            CapabilityClass::ProjectServiceMutation,
            "project-service:restart",
            NOW + 60,
        );

        let evaluation = evaluate_capability_grant(&requirement, Some(&grant), NOW);

        assert_eq!(evaluation.decision, CapabilityDecision::Denied);
        assert_eq!(
            evaluation.reason_code,
            CapabilityReasonCode::CapabilityConfirmationRequired
        );
    }

    #[test]
    fn serialized_evaluation_has_stable_non_secret_shape() {
        let requirement = CapabilityRequirement::new(
            CapabilityClass::CommandExecution,
            "command-profile:diagnostics",
            false,
        );
        let grant = CapabilityGrant::new(
            "grant-diagnostics-001",
            CapabilityClass::CommandExecution,
            "command-profile:diagnostics",
            NOW + 60,
        );

        let value = serde_json::to_value(evaluate_capability_grant(
            &requirement,
            Some(&grant),
            NOW,
        ))
        .unwrap();

        assert_eq!(
            value,
            json!({
                "decision": "allowed",
                "reason_code": "capability_grant_allowed",
                "grant_id": "grant-diagnostics-001",
                "capability_class": "command_execution",
                "scope": "command-profile:diagnostics",
            })
        );
        assert_non_sensitive_json(&value);
    }

    fn assert_non_sensitive_json(value: &Value) {
        let serialized = value.to_string().to_ascii_lowercase();
        for forbidden in [
            "password",
            "secret",
            "bearer",
            "access_token",
            "refresh_token",
            "/data/",
            "/home/",
            "stdout",
            "stderr",
            "command_output",
            "environment",
            "android_id",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "unexpected sensitive token: {forbidden}"
            );
        }
    }
}
