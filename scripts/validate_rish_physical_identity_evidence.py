#!/usr/bin/env python3
"""Validate development-only physical Shizuku/rish identity evidence.

The validator intentionally uses only Python's standard library. It validates
the committed closed policy, rejects duplicate JSON keys before constructing
objects, and reconciles every device/candidate identity supplied by the
protected controller. It never prints an identity value or source path.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import pathlib
import re
import stat
import sys
from typing import Any


EVIDENCE_FILE_NAME = "android-rish-physical-identity-evidence-v1.json"
POLICY_FILE_NAME = "android-rish-physical-identity-policy-v1.json"
REPOSITORY = "CyberBASSLord-666/termux-mcp-edge"
QUALIFICATION_CLASS = "physical_shizuku_rish_identity_development_v1"
SCOPE = "s2_5_uid_probe_only"
ARTIFACT_NAME = (
    "termux-mcp-server-aarch64-linux-android-android-rish-development"
)
MAX_EVIDENCE_BYTES = 65_536
MAX_POLICY_BYTES = 65_536
MAX_ARTIFACT_BYTES = 67_108_864
MAX_DEX_BYTES = 16_777_216
MAX_OBSERVATION_SECONDS = 1_800
MAX_U64 = 18_446_744_073_709_551_615

SHA1_RE = re.compile(r"^[0-9a-f]{40}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
VERSION_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
RUN_ID_RE = re.compile(r"^[1-9][0-9]{0,19}$")
SECURITY_PATCH_RE = re.compile(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$")
TIMESTAMP_RE = re.compile(
    r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"
)

SCENARIO_IDS = [
    "controller_offline_posture_pre_candidate",
    "trusted_direct_rish_probe_pre_candidate",
    "runtime_disabled_tool_absent",
    "candidate_mcp_status_uid_2000",
    "extra_arguments_rejected",
    "unknown_shell_rejected",
    "dex_tamper_rejected",
    "dex_mode_rejected",
    "dex_symlink_rejected",
    "all_mutation_gates_disabled",
    "trusted_direct_rish_probe_post_candidate",
    "controller_offline_posture_post_candidate",
    "bounded_test_fixture_cleanup",
    "device_slot_quarantined_after_candidate",
]

VALIDATION_CONSTANTS = {
    "disabledToolCount": 17,
    "enabledToolCount": 18,
    "exactToolOrder": True,
    "emptyArgumentsSchema": True,
    "extraArgumentsRejected": True,
    "unknownShellRejected": True,
    "allMutationGatesDisabled": True,
    "controllerOfflinePosturePreCandidate": True,
    "trustedDirectRishProbePreCandidate": True,
    "trustedDirectRishProbePostCandidate": True,
    "controllerOfflinePosturePostCandidate": True,
    "dexTamperRejected": True,
    "dexModeRejected": True,
    "dexSymlinkRejected": True,
}

CLAIM_BOUNDARY = {
    "s3Attestation": False,
    "typedReads": False,
    "grantV2": False,
    "deviceMutation": False,
    "productionControl": False,
    "sameUidPersistenceExcluded": False,
    "continuousNetworkIsolation": False,
    "adversarialNetworkIsolation": False,
}

CLEANUP = {
    "candidateProcessGroupStopped": True,
    "portReleased": True,
    "deviceFixtureStateRemoved": True,
    "controllerTransportRemoved": True,
    "deviceSlotQuarantinedAfterCandidate": True,
}

EXPECTED_POLICY = {
    "schemaVersion": 1,
    "policyVersion": "1",
    "gateVersion": "1",
    "qualificationClass": QUALIFICATION_CLASS,
    "scope": SCOPE,
    "releaseEligible": False,
    "productionControlQualified": False,
    "evidenceFileName": EVIDENCE_FILE_NAME,
    "maximumEvidenceBytes": MAX_EVIDENCE_BYTES,
    "maximumRawReportBytes": 16_777_216,
    "maximumObservationSeconds": MAX_OBSERVATION_SECONDS,
    "workflow": {
        "name": "Android Rish Physical Identity",
        "runAttempt": 1,
        "event": "workflow_dispatch",
        "protectedEnvironment": "android-rish-physical-development",
        "definitionAndCompanionRunsRequired": True,
    },
    "artifact": {
        "artifactName": ARTIFACT_NAME,
        "posture": "android-rish-development",
        "features": ["android-rish"],
        "target": "aarch64-linux-android",
        "maximumBytes": MAX_ARTIFACT_BYTES,
    },
    "environment": {
        "physicalDeviceObserved": True,
        "androidFrameworkObserved": True,
        "architecture": "aarch64",
        "minimumApiLevel": 30,
        "maximumApiLevel": 36,
        "adbShellUid": 2000,
        "shizukuStartMode": "adb",
        "termuxPackageName": "com.termux",
        "versionAndSignerCommitmentsRequired": True,
        "buildFingerprintCommitmentRequired": True,
    },
    "backend": {
        "name": "shizuku_rish",
        "principal": "android_shell",
        "uid": 2000,
        "state": "verified_shell_uid",
        "rootAccepted": False,
        "arbitraryShell": False,
        "mutationReady": False,
        "timeoutSeconds": 5,
        "stdoutBytes": 1024,
        "stderrBytes": 4096,
        "minimumDexBytes": 1,
        "maximumDexBytes": MAX_DEX_BYTES,
        "dexMode": "0400",
        "dexParentMode": "0700",
        "dexLinkCount": 1,
        "dexOwnerMatchesTermuxUid": True,
        "dexCanonicalPrivatePath": True,
    },
    "validation": {
        **VALIDATION_CONSTANTS,
        "scenarioIds": SCENARIO_IDS,
    },
    "claimBoundary": CLAIM_BOUNDARY,
    "cleanup": CLEANUP,
    "privacy": {
        "rawCommandOutputPublic": False,
        "rawPrivateReportPublic": False,
        "deviceIdentifiersPublic": False,
        "devicePathsPublic": False,
        "secretsPublic": False,
        "packageInventoryPublic": False,
        "settingsValuesPublic": False,
    },
}


class ValidationFailure(Exception):
    """A stable, non-reflective validation failure."""

    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


def fail(code: str) -> None:
    raise ValidationFailure(code)


def closed_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            fail("duplicate_json_key")
        value[key] = item
    return value


def reject_constant(_value: str) -> None:
    fail("non_finite_json_number")


def load_closed_json(
    raw_path: str,
    expected_name: str,
    maximum_bytes: int,
    invalid_code: str,
) -> tuple[dict[str, Any], bytes]:
    path = pathlib.Path(raw_path)
    try:
        if not path.is_absolute() or path.name != expected_name or path.is_symlink():
            fail(invalid_code)
        resolved = path.resolve(strict=True)
        if resolved != path or resolved.is_symlink():
            fail(invalid_code)
        metadata = resolved.stat()
        if not stat.S_ISREG(metadata.st_mode):
            fail(invalid_code)
        if metadata.st_size < 1 or metadata.st_size > maximum_bytes:
            fail(f"{invalid_code}_size")
        raw = resolved.read_bytes()
    except ValidationFailure:
        raise
    except OSError:
        fail(invalid_code)

    if raw.startswith(b"\xef\xbb\xbf"):
        fail(invalid_code)
    try:
        text = raw.decode("utf-8")
        value = json.loads(
            text,
            object_pairs_hook=closed_object,
            parse_constant=reject_constant,
        )
    except ValidationFailure:
        raise
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError):
        fail(invalid_code)
    if type(value) is not dict:
        fail(invalid_code)
    return value, raw


def exact_json(left: Any, right: Any) -> bool:
    if type(left) is not type(right):
        return False
    if type(left) is dict:
        return set(left) == set(right) and all(
            exact_json(left[key], right[key]) for key in left
        )
    if type(left) is list:
        return len(left) == len(right) and all(
            exact_json(left_item, right_item)
            for left_item, right_item in zip(left, right)
        )
    return bool(left == right)


def exact_keys(value: Any, keys: set[str], code: str) -> dict[str, Any]:
    if type(value) is not dict or set(value) != keys:
        fail(code)
    return value


def require_string(
    value: Any,
    pattern: re.Pattern[str],
    code: str,
) -> str:
    if type(value) is not str or pattern.fullmatch(value) is None:
        fail(code)
    return value


def require_integer(value: Any, minimum: int, maximum: int, code: str) -> int:
    if type(value) is not int or value < minimum or value > maximum:
        fail(code)
    return value


def parse_timestamp(value: Any, code: str) -> dt.datetime:
    require_string(value, TIMESTAMP_RE, code)
    try:
        parsed = dt.datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ")
    except ValueError:
        fail(code)
    return parsed.replace(tzinfo=dt.timezone.utc)


def parse_security_patch(value: Any) -> dt.date:
    require_string(value, SECURITY_PATCH_RE, "environment_identity_invalid")
    try:
        parsed = dt.datetime.strptime(value, "%Y-%m-%d").date()
    except ValueError:
        fail("environment_identity_invalid")
    return parsed


def validate_expected_arguments(args: argparse.Namespace) -> None:
    require_string(args.expected_commit, SHA1_RE, "expected_identity_invalid")
    require_string(args.expected_version, VERSION_RE, "expected_identity_invalid")
    require_string(
        args.expected_policy_sha256, SHA256_RE, "expected_identity_invalid"
    )
    require_string(
        args.expected_cargo_lock_sha256, SHA256_RE, "expected_identity_invalid"
    )
    require_string(
        args.expected_raw_report_sha256, SHA256_RE, "expected_identity_invalid"
    )
    run_id = require_string(
        args.expected_workflow_run_id, RUN_ID_RE, "expected_identity_invalid"
    )
    run_ids = (
        run_id,
        args.expected_ci_run_id,
        args.expected_security_run_id,
        args.expected_android_run_id,
    )
    for candidate_run_id in run_ids:
        require_string(candidate_run_id, RUN_ID_RE, "expected_identity_invalid")
        if int(candidate_run_id) > MAX_U64:
            fail("expected_identity_invalid")
    for value in (
        args.expected_workflow_definition_sha256,
        args.expected_controller_challenge_sha256,
        args.expected_artifact_sha256,
        args.expected_device_profile_commitment,
        args.expected_build_fingerprint_sha256,
        args.expected_termux_signer_sha256,
        args.expected_shizuku_signer_sha256,
        args.expected_dex_sha256,
    ):
        require_string(value, SHA256_RE, "expected_identity_invalid")
    require_string(
        args.expected_termux_version, VERSION_RE, "expected_identity_invalid"
    )
    require_string(
        args.expected_shizuku_version, VERSION_RE, "expected_identity_invalid"
    )
    require_integer(
        args.expected_artifact_bytes,
        1,
        MAX_ARTIFACT_BYTES,
        "expected_identity_invalid",
    )
    require_integer(
        args.expected_dex_bytes, 1, MAX_DEX_BYTES, "expected_identity_invalid"
    )
    require_integer(args.expected_api_level, 30, 36, "expected_identity_invalid")
    parse_security_patch(args.expected_security_patch)


def validate_evidence(
    evidence: dict[str, Any],
    policy_sha256: str,
    args: argparse.Namespace,
) -> None:
    exact_keys(
        evidence,
        {
            "schemaVersion",
            "gateVersion",
            "status",
            "failureCode",
            "releaseEligible",
            "productionControlQualified",
            "qualificationClass",
            "scope",
            "startedAt",
            "completedAt",
            "repository",
            "commit",
            "version",
            "policySha256",
            "cargoLockSha256",
            "rawReportSha256",
            "workflow",
            "artifact",
            "environment",
            "backend",
            "validation",
            "claimBoundary",
            "cleanup",
        },
        "evidence_contract_invalid",
    )
    constants = {
        "schemaVersion": 1,
        "gateVersion": "1",
        "status": "pass",
        "failureCode": None,
        "releaseEligible": False,
        "productionControlQualified": False,
        "qualificationClass": QUALIFICATION_CLASS,
        "scope": SCOPE,
        "repository": REPOSITORY,
    }
    if not all(exact_json(evidence[key], value) for key, value in constants.items()):
        fail("evidence_contract_invalid")

    started = parse_timestamp(evidence["startedAt"], "observation_time_invalid")
    completed = parse_timestamp(evidence["completedAt"], "observation_time_invalid")
    duration = (completed - started).total_seconds()
    if duration < 0 or duration > MAX_OBSERVATION_SECONDS:
        fail("observation_time_invalid")

    if (
        evidence["commit"] != args.expected_commit
        or evidence["version"] != args.expected_version
        or evidence["policySha256"] != policy_sha256
        or evidence["policySha256"] != args.expected_policy_sha256
        or evidence["cargoLockSha256"] != args.expected_cargo_lock_sha256
        or evidence["rawReportSha256"] != args.expected_raw_report_sha256
    ):
        fail("candidate_identity_mismatch")

    workflow = exact_keys(
        evidence["workflow"],
        {
            "name",
            "definitionSha256",
            "runId",
            "runAttempt",
            "event",
            "protectedEnvironment",
            "controllerChallengeSha256",
            "ciRunId",
            "securityRunId",
            "androidRunId",
        },
        "workflow_identity_invalid",
    )
    workflow_expected = {
        "name": "Android Rish Physical Identity",
        "definitionSha256": args.expected_workflow_definition_sha256,
        "runId": args.expected_workflow_run_id,
        "runAttempt": 1,
        "event": "workflow_dispatch",
        "protectedEnvironment": "android-rish-physical-development",
        "controllerChallengeSha256": args.expected_controller_challenge_sha256,
        "ciRunId": args.expected_ci_run_id,
        "securityRunId": args.expected_security_run_id,
        "androidRunId": args.expected_android_run_id,
    }
    if not exact_json(workflow, workflow_expected):
        fail("workflow_identity_invalid")

    artifact = exact_keys(
        evidence["artifact"],
        {"artifactName", "posture", "features", "target", "sha256", "bytes"},
        "artifact_identity_invalid",
    )
    artifact_expected = {
        "artifactName": ARTIFACT_NAME,
        "posture": "android-rish-development",
        "features": ["android-rish"],
        "target": "aarch64-linux-android",
        "sha256": args.expected_artifact_sha256,
        "bytes": args.expected_artifact_bytes,
    }
    if not exact_json(artifact, artifact_expected):
        fail("artifact_identity_invalid")

    environment = exact_keys(
        evidence["environment"],
        {
            "physicalDeviceObserved",
            "androidFrameworkObserved",
            "architecture",
            "apiLevel",
            "securityPatch",
            "deviceProfileCommitment",
            "buildFingerprintSha256",
            "adbShellUid",
            "shizukuStartMode",
            "termuxVersion",
            "termuxSignerSha256",
            "shizukuVersion",
            "shizukuSignerSha256",
        },
        "environment_identity_invalid",
    )
    environment_expected = {
        "physicalDeviceObserved": True,
        "androidFrameworkObserved": True,
        "architecture": "aarch64",
        "apiLevel": args.expected_api_level,
        "securityPatch": args.expected_security_patch,
        "deviceProfileCommitment": args.expected_device_profile_commitment,
        "buildFingerprintSha256": args.expected_build_fingerprint_sha256,
        "adbShellUid": 2000,
        "shizukuStartMode": "adb",
        "termuxVersion": args.expected_termux_version,
        "termuxSignerSha256": args.expected_termux_signer_sha256,
        "shizukuVersion": args.expected_shizuku_version,
        "shizukuSignerSha256": args.expected_shizuku_signer_sha256,
    }
    if not exact_json(environment, environment_expected):
        fail("environment_identity_invalid")
    patch = parse_security_patch(environment["securityPatch"])
    if patch > completed.date():
        fail("environment_identity_invalid")

    backend = exact_keys(
        evidence["backend"],
        {
            "name",
            "dexSha256",
            "dexBytes",
            "dexMode",
            "dexParentMode",
            "dexLinkCount",
            "dexOwnerMatchesTermuxUid",
            "dexCanonicalPrivatePath",
            "principal",
            "uid",
            "state",
            "rootAccepted",
            "arbitraryShell",
            "mutationReady",
        },
        "backend_identity_invalid",
    )
    backend_expected = {
        "name": "shizuku_rish",
        "dexSha256": args.expected_dex_sha256,
        "dexBytes": args.expected_dex_bytes,
        "dexMode": "0400",
        "dexParentMode": "0700",
        "dexLinkCount": 1,
        "dexOwnerMatchesTermuxUid": True,
        "dexCanonicalPrivatePath": True,
        "principal": "android_shell",
        "uid": 2000,
        "state": "verified_shell_uid",
        "rootAccepted": False,
        "arbitraryShell": False,
        "mutationReady": False,
    }
    if not exact_json(backend, backend_expected):
        fail("backend_identity_invalid")

    validation = exact_keys(
        evidence["validation"],
        set(VALIDATION_CONSTANTS) | {"scenarioResults"},
        "validation_contract_invalid",
    )
    if not all(
        exact_json(validation[key], value)
        for key, value in VALIDATION_CONSTANTS.items()
    ):
        fail("validation_contract_invalid")
    expected_scenarios = [
        {"id": scenario_id, "execution": "physical", "outcome": "pass"}
        for scenario_id in SCENARIO_IDS
    ]
    if not exact_json(validation["scenarioResults"], expected_scenarios):
        fail("validation_contract_invalid")
    if not exact_json(evidence["claimBoundary"], CLAIM_BOUNDARY):
        fail("claim_boundary_invalid")
    if not exact_json(evidence["cleanup"], CLEANUP):
        fail("cleanup_unconfirmed")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Validate one closed development-only physical Shizuku/rish "
            "identity evidence record."
        )
    )
    parser.add_argument("--evidence", required=True)
    parser.add_argument("--policy", required=True)
    parser.add_argument("--expected-commit", required=True)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--expected-policy-sha256", required=True)
    parser.add_argument("--expected-cargo-lock-sha256", required=True)
    parser.add_argument("--expected-raw-report-sha256", required=True)
    parser.add_argument("--expected-workflow-definition-sha256", required=True)
    parser.add_argument("--expected-workflow-run-id", required=True)
    parser.add_argument("--expected-ci-run-id", required=True)
    parser.add_argument("--expected-security-run-id", required=True)
    parser.add_argument("--expected-android-run-id", required=True)
    parser.add_argument("--expected-controller-challenge-sha256", required=True)
    parser.add_argument("--expected-artifact-sha256", required=True)
    parser.add_argument("--expected-artifact-bytes", required=True, type=int)
    parser.add_argument("--expected-api-level", required=True, type=int)
    parser.add_argument("--expected-security-patch", required=True)
    parser.add_argument("--expected-device-profile-commitment", required=True)
    parser.add_argument("--expected-build-fingerprint-sha256", required=True)
    parser.add_argument("--expected-termux-version", required=True)
    parser.add_argument("--expected-termux-signer-sha256", required=True)
    parser.add_argument("--expected-shizuku-version", required=True)
    parser.add_argument("--expected-shizuku-signer-sha256", required=True)
    parser.add_argument("--expected-dex-sha256", required=True)
    parser.add_argument("--expected-dex-bytes", required=True, type=int)
    return parser.parse_args()


def main() -> int:
    try:
        args = parse_args()
        validate_expected_arguments(args)
        policy, policy_raw = load_closed_json(
            args.policy,
            POLICY_FILE_NAME,
            MAX_POLICY_BYTES,
            "policy_input_invalid",
        )
        if not exact_json(policy, EXPECTED_POLICY):
            fail("policy_contract_invalid")
        policy_sha256 = hashlib.sha256(policy_raw).hexdigest()
        if policy_sha256 != args.expected_policy_sha256:
            fail("policy_identity_mismatch")
        evidence, _ = load_closed_json(
            args.evidence,
            EVIDENCE_FILE_NAME,
            MAX_EVIDENCE_BYTES,
            "evidence_input_invalid",
        )
        validate_evidence(evidence, policy_sha256, args)
    except ValidationFailure as error:
        print(
            f"RISH_PHYSICAL_IDENTITY_VALIDATION_RESULT=FAIL reason={error.code}",
            file=sys.stderr,
        )
        return 1
    except (OSError, ValueError, OverflowError):
        print(
            "RISH_PHYSICAL_IDENTITY_VALIDATION_RESULT=FAIL "
            "reason=validator_internal_error",
            file=sys.stderr,
        )
        return 1

    print("RISH_PHYSICAL_IDENTITY_VALIDATION_RESULT=PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
