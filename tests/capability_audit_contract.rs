use serde_json::json;
use termux_mcp_server::audit::{AuditCounters, AuditDecision, AuditEvent, AuditMode};
use termux_mcp_server::capability_token::{
    evaluate_capability_grant, CapabilityClass, CapabilityDecision, CapabilityGrant,
    CapabilityReasonCode, CapabilityRequirement,
};

const NOW: u64 = 1_725_000_000;

#[test]
fn capability_evaluation_can_feed_non_sensitive_audit_counters() {
    let requirement = CapabilityRequirement::new(
        CapabilityClass::CommandExecution,
        "command-profile:diagnostics",
        true,
    )
    .unwrap();
    let grant = CapabilityGrant::new(
        "grant-diagnostics-001",
        CapabilityClass::CommandExecution,
        "command-profile:diagnostics",
        NOW + 60,
    )
    .unwrap()
    .with_confirmation_satisfied(true);

    let evaluation = evaluate_capability_grant(&requirement, Some(&grant), NOW);
    assert_eq!(evaluation.decision, CapabilityDecision::Allowed);

    let event = capability_audit_event(
        "command_diagnostics_preview",
        AuditMode::DryRun,
        evaluation.decision,
        evaluation.reason_code,
    );
    let mut counters = AuditCounters::default();
    counters.record_event(&event);

    let value = serde_json::to_value(counters).unwrap();
    assert_eq!(
        value,
        json!({
            "allowed_total": 1,
            "denied_total": 0,
            "by_tool": {
                "command_diagnostics_preview": {
                    "allowed": 1,
                    "denied": 0
                }
            },
            "by_reason_code": {
                "capability_grant_allowed": {
                    "allowed": 1,
                    "denied": 0
                }
            }
        })
    );
    assert_non_sensitive_counter_output(&value);
}

#[test]
fn denied_capability_evaluations_remain_low_cardinality_counter_labels() {
    let requirement = CapabilityRequirement::new(
        CapabilityClass::ProjectServiceMutation,
        "project-service:restart",
        true,
    )
    .unwrap();
    let grant = CapabilityGrant::new(
        "grant-needs-confirmation-001",
        CapabilityClass::ProjectServiceMutation,
        "project-service:restart",
        NOW + 60,
    )
    .unwrap();

    let evaluation = evaluate_capability_grant(&requirement, Some(&grant), NOW);
    assert_eq!(evaluation.decision, CapabilityDecision::Denied);
    assert_eq!(
        evaluation.reason_code,
        CapabilityReasonCode::CapabilityConfirmationRequired
    );

    let event = capability_audit_event(
        "project_service_restart_preview",
        AuditMode::DryRun,
        evaluation.decision,
        evaluation.reason_code,
    );
    let mut counters = AuditCounters::default();
    counters.record_event(&event);

    let value = serde_json::to_value(counters).unwrap();
    assert_eq!(value["allowed_total"], 0);
    assert_eq!(value["denied_total"], 1);
    assert_non_sensitive_counter_output(&value);
}

fn capability_audit_event(
    tool_name: &str,
    mode: AuditMode,
    decision: CapabilityDecision,
    reason_code: CapabilityReasonCode,
) -> AuditEvent {
    AuditEvent::new(
        NOW,
        tool_name,
        "capability_evaluation",
        mode,
        match decision {
            CapabilityDecision::Allowed => AuditDecision::Allowed,
            CapabilityDecision::Denied => AuditDecision::Denied,
        },
        capability_reason_code_label(reason_code),
    )
}

fn capability_reason_code_label(reason_code: CapabilityReasonCode) -> &'static str {
    match reason_code {
        CapabilityReasonCode::CapabilityGrantAllowed => "capability_grant_allowed",
        CapabilityReasonCode::CapabilityGrantMissing => "capability_grant_missing",
        CapabilityReasonCode::CapabilityGrantInactive => "capability_grant_inactive",
        CapabilityReasonCode::CapabilityGrantExpired => "capability_grant_expired",
        CapabilityReasonCode::CapabilityClassMismatch => "capability_class_mismatch",
        CapabilityReasonCode::CapabilityScopeMismatch => "capability_scope_mismatch",
        CapabilityReasonCode::CapabilityConfirmationRequired => "capability_confirmation_required",
    }
}

fn assert_non_sensitive_counter_output(value: &serde_json::Value) {
    let serialized = value.to_string().to_ascii_lowercase();
    for forbidden in [
        "grant-diagnostics-001",
        "grant-needs-confirmation-001",
        "bearer",
        "access_token",
        "refresh_token",
        "password",
        "secret",
        "/data/",
        "/home/",
        "stdout",
        "stderr",
        "environment",
        "android_id",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "counter output leaked sensitive or high-cardinality value: {forbidden}"
        );
    }
}
