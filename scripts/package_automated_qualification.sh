#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077
set +x

usage() {
  cat <<'EOF'
Usage: package_automated_qualification.sh \
  --policy release-qualification-policy-v1.json \
  --scenario-set automated-native-deployment-scenarios-v1.json \
  --aggregate-evidence termux-native-aggregate-evidence-v4.json \
  --deployment-evidence automated-native-deployment-v1.json \
  --classifier-evidence termux-observation-requirement-v3.json \
  --battery-evidence termux-battery-emulated-evidence.json \
  --volume-evidence termux-volume-emulated-evidence.json \
  --volume-control-evidence termux-volume-control-emulated-evidence.json \
  --command-evidence termux-command-emulated-evidence.json \
  --runtime-archive termux-qualified-runtime-image-v1.tar.gz \
  --runtime-package-lock termux-runtime-package-lock-v1.json \
  --runtime-snapshot termux-runtime-snapshot-v1.json \
  --runtime-replay termux-runtime-snapshot-replay-v1.json \
  --default-dir DIR --mcp-runtime-dir DIR --battery-dir DIR \
  --volume-dir DIR --volume-control-dir DIR --command-dir DIR \
  --full-suite-dir DIR \
  --qualification-run-id ID \
  --output /private/parent/automated-qualification-v1.json

Validates one exact first-attempt main candidate and atomically writes the
single closed automated-qualification-v1.json envelope. The envelope grants
only official_termux_native_automated_v1 qualification and explicitly records
that no physical device, Android framework, sustained physical soak, or
physical certification was observed.
EOF
}

if (($# == 1)) && [[ "$1" == -h || "$1" == --help ]]; then
  usage
  exit 0
fi

command -v python3 >/dev/null 2>&1 || {
  printf 'AUTOMATED_QUALIFICATION_PACKAGE_RESULT=FAIL reason=required_command_missing\n' >&2
  exit 1
}
command -v file >/dev/null 2>&1 || {
  printf 'AUTOMATED_QUALIFICATION_PACKAGE_RESULT=FAIL reason=required_command_missing\n' >&2
  exit 1
}

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
COMMIT_HELPER="$SCRIPT_DIR/commit_verified_file.py"
[[ -f "$COMMIT_HELPER" && ! -L "$COMMIT_HELPER" ]] || {
  printf 'AUTOMATED_QUALIFICATION_PACKAGE_RESULT=FAIL reason=commit_helper_invalid\n' >&2
  exit 1
}

COMMIT_HELPER="$COMMIT_HELPER" python3 - "$@" <<'PY'
import argparse
import datetime
import hashlib
import json
import math
import os
import pathlib
import re
import signal
import stat
import subprocess
import sys
import tempfile


CLASS = "official_termux_native_automated_v1"
REPOSITORY = "CyberBASSLord-666/termux-mcp-edge"
BASE_IMAGE = "termux/termux-docker:aarch64"
POLICY_SHA256 = "920a3334c5409e13d7cea062e1dfee5575f79b040faa2bcbf95765708045e823"
SCENARIO_SHA256 = "dd31d4f89f9f25dba1a1bb1c492fd796f5a2619b215e2d57f3b0e60f9f24b3bb"
RUNTIME_ARCHIVE_NAME = "termux-qualified-runtime-image-v1.tar.gz"
RUNTIME_LOCK_NAME = "termux-runtime-package-lock-v1.json"
RUNTIME_SNAPSHOT_NAME = "termux-runtime-snapshot-v1.json"
RUNTIME_REPLAY_NAME = "termux-runtime-snapshot-replay-v1.json"
REQUESTED_PACKAGES = ["file", "jq", "python", "termux-services"]
MAX_RUNTIME_ARCHIVE_BYTES = 1_610_612_736
CLAIM_BOUNDARY = {
    "physicalDeviceObserved": False,
    "androidFrameworkObserved": False,
    "sustainedPhysicalSoak": False,
    "physicalCertification": "not_run",
}
POSTURES = [
    "default",
    "mcp-runtime",
    "android-battery-status",
    "android-volume-status",
    "android-volume-control",
    "command-execution",
    "full-suite",
]
FEATURES = [
    [],
    ["mcp-runtime"],
    ["android-battery-status"],
    ["android-volume-status"],
    ["android-volume-control"],
    ["command-execution"],
    ["full-suite"],
]
ARTIFACT_NAMES = [
    "termux-mcp-server-aarch64-linux-android-default",
    "termux-mcp-server-aarch64-linux-android-mcp-runtime",
    "termux-mcp-server-aarch64-linux-android-android-battery-status",
    "termux-mcp-server-aarch64-linux-android-android-volume-status",
    "termux-mcp-server-aarch64-linux-android-android-volume-control",
    "termux-mcp-server-aarch64-linux-android-command-execution",
    "termux-mcp-server-aarch64-linux-android-full-suite",
]
SCENARIO_IDS = [
    "isolated_fresh_deploy",
    "failed_upgrade_recovery",
    "supervised_restart",
    "rollback_recovery",
    "uninstall",
    "bounded_cleanup",
]
AGGREGATE_COVERED = [
    "exact_android_artifacts",
    "official_termux_userland_native_arm64",
    "android_bionic_linker",
    "deterministic_provider_simulation",
    "runtime_gate_composition",
    "bounded_native_stress",
]
AGGREGATE_NOT_COVERED = [
    "physical_device",
    "android_framework",
    "oem_policy",
    "battery_aging",
    "thermal_soak",
    "radio",
    "doze",
]
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
SHA_RE = re.compile(r"^[0-9a-f]{64}$")
OCI_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
RUN_RE = re.compile(r"^[1-9][0-9]*$")
VERSION_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
TIME_RE = re.compile(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$")
PACKAGE_RE = re.compile(r"^[a-z0-9][a-z0-9+.-]{0,127}$")
ARCH_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,31}$")
PACKAGE_VERSION_RE = re.compile(r"^[!-~]{1,128}$")
FILE_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9+._%~-]{0,255}$")
DEB_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9+._~-]{0,255}\.deb$")
FORBIDDEN_FIELD_TOKENS = ("duration", "elapsed", "equivalent", "minute", "time_dilation")
RUNTIME_VERIFICATION = {
    "archiveDigestVerified": True,
    "singleImageArchive": True,
    "loadedImageIdVerified": True,
    "platformVerified": True,
    "rootfsLayersVerified": True,
    "packageLockVerified": True,
    "packageInputBytesVerified": True,
    "repositoryIndexBytesVerified": True,
    "installedPackageInventoryVerified": True,
    "requiredRuntimeCommandsVerified": True,
    "androidLinkerVerified": True,
    "runtimeNetworkAccess": False,
}


class QualificationError(Exception):
    def __init__(self, code):
        super().__init__(code)
        self.code = code


def fail(code):
    raise QualificationError(code)


def closed_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            fail("duplicate_json_key")
        value[key] = item
    return value


def reject_constant(_value):
    fail("non_finite_json_number")


def parse_json_bytes(raw, code):
    try:
        text = raw.decode("utf-8")
        value = json.loads(
            text,
            object_pairs_hook=closed_object,
            parse_constant=reject_constant,
        )
    except QualificationError:
        raise
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError):
        fail(code)
    if not isinstance(value, dict):
        fail(code)
    return value


def exact_keys(value, keys, code):
    if not isinstance(value, dict) or set(value) != set(keys):
        fail(code)


def is_int(value):
    return isinstance(value, int) and not isinstance(value, bool)


def strict_equal(left, right):
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return (
            set(left) == set(right)
            and all(strict_equal(left[key], right[key]) for key in left)
        )
    if isinstance(left, list):
        return (
            len(left) == len(right)
            and all(strict_equal(a, b) for a, b in zip(left, right))
        )
    return left == right


def expect_int(value, minimum, maximum, code):
    if not is_int(value) or value < minimum or value > maximum:
        fail(code)


def expect_pattern(value, pattern, code):
    if not isinstance(value, str) or pattern.fullmatch(value) is None:
        fail(code)


def parse_timestamp(value, code):
    if not isinstance(value, str) or TIME_RE.fullmatch(value) is None:
        fail(code)
    try:
        parsed = datetime.datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ")
    except ValueError:
        fail(code)
    return parsed.replace(tzinfo=datetime.timezone.utc)


def expect_timestamp_order(started_at, completed_at, code):
    started = parse_timestamp(started_at, code)
    completed = parse_timestamp(completed_at, code)
    if completed < started:
        fail(code)


def digest_bytes(raw):
    return hashlib.sha256(raw).hexdigest()


def require_absolute_canonical(path_text, code):
    path = pathlib.Path(path_text)
    if not path.is_absolute():
        fail(code)
    try:
        resolved = path.resolve(strict=True)
    except (OSError, RuntimeError):
        fail(code)
    if resolved != path:
        fail(code)
    return path


def read_regular(path_text, maximum, code, private=False, basenames=None):
    path = require_absolute_canonical(path_text, code)
    if basenames is not None and path.name not in basenames:
        fail(code)
    try:
        before = path.stat(follow_symlinks=False)
    except OSError:
        fail(code)
    if not stat.S_ISREG(before.st_mode) or stat.S_ISLNK(before.st_mode):
        fail(code)
    if private and stat.S_IMODE(before.st_mode) != 0o600:
        fail(code)
    if before.st_size < 1 or before.st_size > maximum:
        fail(code)
    try:
        with path.open("rb") as source:
            raw = source.read(maximum + 1)
            after = os.fstat(source.fileno())
    except OSError:
        fail(code)
    if len(raw) < 1 or len(raw) > maximum:
        fail(code)
    identity_before = (
        before.st_dev,
        before.st_ino,
        before.st_size,
        before.st_mtime_ns,
        before.st_ctime_ns,
    )
    identity_after = (
        after.st_dev,
        after.st_ino,
        after.st_size,
        after.st_mtime_ns,
        after.st_ctime_ns,
    )
    if identity_before != identity_after or len(raw) != after.st_size:
        fail(code)
    return path, raw


def file_record(path, raw, file_name):
    return {"fileName": file_name, "sha256": digest_bytes(raw), "bytes": len(raw)}


def inspect_runtime_input(path_text, expected_name, maximum, code, capture):
    path = pathlib.Path(path_text)
    if (
        not path.is_absolute()
        or pathlib.Path(os.path.normpath(path_text)) != path
        or path.name != expected_name
    ):
        fail(code)
    descriptor = -1
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
        before = os.fstat(descriptor)
        if (
            not stat.S_ISREG(before.st_mode)
            or before.st_uid != os.getuid()
            or stat.S_IMODE(before.st_mode) != 0o600
            or before.st_size < 1
            or before.st_size > maximum
        ):
            fail(code)
        digest = hashlib.sha256()
        chunks = [] if capture else None
        observed = 0
        while True:
            chunk = os.read(descriptor, 1024 * 1024)
            if not chunk:
                break
            observed += len(chunk)
            if observed > maximum:
                fail(code)
            digest.update(chunk)
            if chunks is not None:
                chunks.append(chunk)
        after = os.fstat(descriptor)
    except OSError:
        fail(code)
    finally:
        if descriptor >= 0:
            os.close(descriptor)
    before_identity = (
        before.st_dev,
        before.st_ino,
        before.st_size,
        before.st_mtime_ns,
        before.st_ctime_ns,
    )
    after_identity = (
        after.st_dev,
        after.st_ino,
        after.st_size,
        after.st_mtime_ns,
        after.st_ctime_ns,
    )
    if before_identity != after_identity or observed != before.st_size:
        fail(code)
    record = {
        "fileName": expected_name,
        "sha256": digest.hexdigest(),
        "bytes": observed,
    }
    return path, b"".join(chunks) if chunks is not None else None, record


def ensure_no_forbidden_fields(value, code):
    if isinstance(value, dict):
        for key, item in value.items():
            normalized = key.lower()
            if any(token in normalized for token in FORBIDDEN_FIELD_TOKENS):
                fail(code)
            ensure_no_forbidden_fields(item, code)
    elif isinstance(value, list):
        for item in value:
            ensure_no_forbidden_fields(item, code)
    elif isinstance(value, float) and not math.isfinite(value):
        fail(code)


def validate_policy(policy, raw):
    if digest_bytes(raw) != POLICY_SHA256:
        fail("policy_stale_or_modified")
    exact_keys(
        policy,
        {
            "schemaVersion",
            "policyVersion",
            "qualificationClass",
            "authorization",
            "repository",
            "branch",
            "workflowRequirements",
            "claimBoundary",
            "environmentRequirements",
            "deploymentScenarioSet",
            "artifactPostures",
            "requiredEvidence",
            "requiredGates",
            "prohibitedClaimFieldTokens",
        },
        "policy_contract_invalid",
    )
    expected_workflows = {
        "requiredWorkflows": ["ci", "security", "android"],
        "event": "push",
        "ref": "refs/heads/main",
        "attempt": 1,
        "conclusion": "success",
    }
    expected_environment = {
        "executionMode": "official-termux-docker-native-arm64",
        "architecture": "aarch64",
        "rootfsImage": "termux/termux-docker:aarch64",
        "rootfsImageDigestRequired": True,
        "rootfsImageIdRequired": True,
        "derivedRuntimeImageDigestRequired": True,
        "runtimeImageMustDifferFromRootfsImageId": True,
        "runtimeImageExecutionByDigest": True,
        "runtimePackages": ["file", "jq", "python", "termux-services"],
        "userlandPrefix": "/data/data/com.termux/files/usr",
        "androidAbi": "android-bionic",
        "dynamicLinkerPath": "/system/bin/linker64",
    }
    expected_scenario = {
        "fileName": "automated-native-deployment-scenarios-v1.json",
        "schemaVersion": 1,
        "scenarioSetVersion": "1",
        "scenarioCount": 6,
        "scenarioIds": SCENARIO_IDS,
        "sha256": SCENARIO_SHA256,
    }
    if (
        policy["schemaVersion"] != 1
        or policy["policyVersion"] != "1"
        or policy["qualificationClass"] != CLASS
        or policy["authorization"] != "release_qualification"
        or policy["repository"] != REPOSITORY
        or policy["branch"] != "main"
        or policy["workflowRequirements"] != expected_workflows
        or policy["claimBoundary"] != CLAIM_BOUNDARY
        or policy["environmentRequirements"] != expected_environment
        or policy["deploymentScenarioSet"] != expected_scenario
        or policy["artifactPostures"] != POSTURES
        or policy["requiredEvidence"]
        != [
            "aggregate",
            "deployment",
            "classifier",
            "android-battery-status",
            "android-volume-status",
            "android-volume-control",
            "command-execution",
        ]
        or policy["requiredGates"]
        != [
            "first_attempt_main_workflows",
            "artifact_lineage",
            "official_termux_native_runtime",
            "aggregate_composition",
            "specialized_provider_boundaries",
            "isolated_deployment_recovery",
            "automated_release_classification",
        ]
        or policy["prohibitedClaimFieldTokens"] != list(FORBIDDEN_FIELD_TOKENS)
    ):
        fail("policy_contract_invalid")


def validate_scenario_set(value, raw):
    if digest_bytes(raw) != SCENARIO_SHA256:
        fail("scenario_set_stale_or_modified")
    exact_keys(
        value,
        {"schemaVersion", "scenarioSetVersion", "qualificationClass", "scenarios"},
        "scenario_set_contract_invalid",
    )
    if (
        value["schemaVersion"] != 1
        or value["scenarioSetVersion"] != "1"
        or value["qualificationClass"] != CLASS
        or not isinstance(value["scenarios"], list)
        or len(value["scenarios"]) != 6
    ):
        fail("scenario_set_contract_invalid")
    expected_outcomes = ["pass", "recovered", "restarted", "recovered", "removed", "clean"]
    expected_faults = [
        "none",
        "target_scoped_readiness_probe_failure",
        "supervised_process_termination",
        "target_scoped_readiness_probe_failure",
        "none",
        "none",
    ]
    for index, scenario in enumerate(value["scenarios"]):
        exact_keys(
            scenario,
            {"id", "execution", "faultInjection", "expectedOutcome"},
            "scenario_set_contract_invalid",
        )
        if scenario != {
            "id": SCENARIO_IDS[index],
            "execution": "native",
            "faultInjection": expected_faults[index],
            "expectedOutcome": expected_outcomes[index],
        }:
            fail("scenario_set_contract_invalid")


def validate_artifact_bundle(directory_text, index, commit, version, android_run_id):
    root = require_absolute_canonical(directory_text, "artifact_bundle_invalid")
    try:
        mode = root.stat(follow_symlinks=False).st_mode
        members = sorted(item.name for item in root.iterdir())
    except OSError:
        fail("artifact_bundle_invalid")
    if not stat.S_ISDIR(mode) or stat.S_ISLNK(mode):
        fail("artifact_bundle_invalid")
    if members != ["SHA256SUMS", "artifact-manifest.json", "termux-mcp-server"]:
        fail("artifact_bundle_members_invalid")
    binary_path, binary_raw = read_regular(
        str(root / "termux-mcp-server"),
        67108864,
        "artifact_binary_invalid",
    )
    if binary_path.stat(follow_symlinks=False).st_mode & 0o111 == 0:
        fail("artifact_binary_not_executable")
    _checksum_path, checksum_raw = read_regular(
        str(root / "SHA256SUMS"),
        256,
        "artifact_checksum_invalid",
    )
    _manifest_path, manifest_raw = read_regular(
        str(root / "artifact-manifest.json"),
        65536,
        "artifact_manifest_invalid",
    )
    digest = digest_bytes(binary_raw)
    expected_checksum = f"{digest}  termux-mcp-server\n".encode("ascii")
    if checksum_raw != expected_checksum:
        fail("artifact_checksum_mismatch")
    manifest = parse_json_bytes(manifest_raw, "artifact_manifest_invalid")
    exact_keys(
        manifest,
        {
            "artifactName",
            "bytes",
            "commit",
            "createdAt",
            "elf",
            "features",
            "fileName",
            "posture",
            "repository",
            "schemaVersion",
            "sha256",
            "target",
            "version",
            "workflowRunId",
        },
        "artifact_manifest_invalid",
    )
    if (
        not is_int(manifest["schemaVersion"])
        or manifest["schemaVersion"] != 1
        or manifest["repository"] != REPOSITORY
        or manifest["commit"] != commit
        or manifest["workflowRunId"] != android_run_id
        or manifest["artifactName"] != ARTIFACT_NAMES[index]
        or manifest["posture"] != POSTURES[index]
        or manifest["features"] != FEATURES[index]
        or manifest["target"] != "aarch64-linux-android"
        or manifest["fileName"] != "termux-mcp-server"
        or manifest["version"] != version
        or manifest["sha256"] != digest
        or not is_int(manifest["bytes"])
        or manifest["bytes"] != len(binary_raw)
        or manifest["elf"] != "aarch64-android-elf"
        or not isinstance(manifest["createdAt"], str)
        or TIME_RE.fullmatch(manifest["createdAt"]) is None
    ):
        fail("artifact_manifest_mismatch")
    identity_snapshot = None
    try:
        fd, snapshot_name = tempfile.mkstemp(prefix=".qualification-elf.")
        identity_snapshot = pathlib.Path(snapshot_name)
        with os.fdopen(fd, "wb") as snapshot:
            snapshot.write(binary_raw)
            snapshot.flush()
            os.fsync(snapshot.fileno())
        identity = subprocess.run(
            ["file", "-b", "--", str(identity_snapshot)],
            check=True,
            capture_output=True,
            text=True,
            timeout=5,
            env={"LC_ALL": "C", "PATH": os.environ.get("PATH", "")},
        ).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        fail("artifact_identity_failed")
    finally:
        if identity_snapshot is not None:
            try:
                identity_snapshot.unlink()
            except OSError:
                pass
    if (
        "ELF" not in identity
        or "ARM aarch64" not in identity
        or ("Android" not in identity and "/system/bin/linker64" not in identity)
    ):
        fail("artifact_android_bionic_identity_invalid")
    return {
        "posture": POSTURES[index],
        "features": FEATURES[index],
        "workflowArtifactName": ARTIFACT_NAMES[index],
        "sha256": digest,
        "bytes": len(binary_raw),
        "manifestSha256": digest_bytes(manifest_raw),
    }


def validate_claim_boundary(value, code):
    exact_keys(value, CLAIM_BOUNDARY.keys(), code)
    if not strict_equal(value, CLAIM_BOUNDARY):
        fail(code)


def validate_aggregate(value, record, artifacts):
    exact_keys(
        value,
        {
            "schemaVersion",
            "gateVersion",
            "status",
            "failureCode",
            "releaseQualificationEligible",
            "startedAt",
            "completedAt",
            "candidate",
            "environment",
            "claimBoundary",
            "coverage",
            "runtimeValidation",
            "aggregateValidation",
            "stress",
        },
        "aggregate_evidence_invalid",
    )
    if (
        not is_int(value["schemaVersion"])
        or value["schemaVersion"] != 4
        or value["gateVersion"] != "4"
        or value["status"] != "pass"
        or value["failureCode"] is not None
        or value["releaseQualificationEligible"] is not False
        or not isinstance(value["startedAt"], str)
        or TIME_RE.fullmatch(value["startedAt"]) is None
        or not isinstance(value["completedAt"], str)
        or TIME_RE.fullmatch(value["completedAt"]) is None
    ):
        fail("aggregate_evidence_invalid")
    expect_timestamp_order(
        value["startedAt"],
        value["completedAt"],
        "aggregate_timestamp_invalid",
    )
    validate_claim_boundary(value["claimBoundary"], "aggregate_claim_boundary_invalid")
    if value["coverage"] != {
        "covered": AGGREGATE_COVERED,
        "notCovered": AGGREGATE_NOT_COVERED,
    }:
        fail("aggregate_coverage_invalid")
    candidate = value["candidate"]
    exact_keys(
        candidate,
        {
            "commit",
            "version",
            "ciRunId",
            "securityRunId",
            "androidRunId",
            "defaultArtifact",
            "mcpRuntimeArtifact",
            "androidVolumeControlArtifact",
            "fullSuiteArtifact",
        },
        "aggregate_candidate_invalid",
    )
    expect_pattern(candidate["commit"], COMMIT_RE, "aggregate_candidate_invalid")
    expect_pattern(candidate["version"], VERSION_RE, "aggregate_candidate_invalid")
    for key in ("ciRunId", "securityRunId", "androidRunId"):
        expect_pattern(candidate[key], RUN_RE, "aggregate_candidate_invalid")
    simple_expected = [
        ("defaultArtifact", 0),
        ("mcpRuntimeArtifact", 1),
        ("androidVolumeControlArtifact", 4),
    ]
    for key, index in simple_expected:
        if not strict_equal(candidate[key], {
            "sha256": artifacts[index]["sha256"],
            "bytes": artifacts[index]["bytes"],
        }):
            fail("aggregate_artifact_mismatch")
    full_suite = candidate["fullSuiteArtifact"]
    if not strict_equal(full_suite, {
        "sha256": artifacts[6]["sha256"],
        "bytes": artifacts[6]["bytes"],
        "manifestSha256": artifacts[6]["manifestSha256"],
        "artifactName": ARTIFACT_NAMES[6],
        "posture": "full-suite",
        "features": ["full-suite"],
        "fileName": "termux-mcp-server",
    }):
        fail("aggregate_artifact_mismatch")
    environment = value["environment"]
    exact_keys(
        environment,
        {
            "executionMode",
            "architecture",
            "image",
            "imageDigest",
            "rootfsImageId",
            "runtimeImageDigest",
            "androidLinker",
        },
        "aggregate_environment_invalid",
    )
    if (
        environment["executionMode"] != "official-termux-docker-native-arm64"
        or environment["architecture"] not in ("aarch64", "arm64")
        or environment["image"] != "termux/termux-docker:aarch64"
        or not isinstance(environment["imageDigest"], str)
        or OCI_RE.fullmatch(environment["imageDigest"]) is None
        or not isinstance(environment["rootfsImageId"], str)
        or OCI_RE.fullmatch(environment["rootfsImageId"]) is None
        or not isinstance(environment["runtimeImageDigest"], str)
        or OCI_RE.fullmatch(environment["runtimeImageDigest"]) is None
        or environment["runtimeImageDigest"] == environment["rootfsImageId"]
        or environment["androidLinker"] is not True
    ):
        fail("aggregate_environment_invalid")
    runtime = value["runtimeValidation"]
    exact_keys(runtime, {"status", "reportSha256", "resultCount", "phases"}, "aggregate_gate_missing")
    expect_pattern(runtime["reportSha256"], SHA_RE, "aggregate_gate_missing")
    expect_int(runtime["resultCount"], 1, 256, "aggregate_gate_missing")
    if runtime["status"] != "pass" or runtime["phases"] != {
        "preflight": "pass",
        "runtime": "pass",
        "deployment": "not_run",
    }:
        fail("aggregate_gate_missing")
    aggregate = value["aggregateValidation"]
    exact_keys(
        aggregate,
        {
            "status",
            "requests",
            "defaultDisabled",
            "fullyEnabled",
            "independentRuntimeGates",
            "filesystemMutationsDisabled",
            "boundedCleanup",
            "automatedQualificationComponent",
        },
        "aggregate_gate_missing",
    )
    expect_int(aggregate["requests"], 14, 128, "aggregate_gate_missing")
    if (
        aggregate["status"] != "pass"
        or not strict_equal(aggregate["defaultDisabled"], {
            "toolCount": 17,
            "exactToolOrder": True,
            "optionalFeaturesCompiled": True,
            "optionalToolsHidden": True,
            "runtimeFlagsOmitted": True,
        })
        or not strict_equal(aggregate["fullyEnabled"], {
            "toolCount": 21,
            "exactToolOrder": True,
            "allOptionalToolsExposed": True,
            "providerSuccesses": True,
            "volumePreviewNoMutation": True,
            "volumeGrantIsolation": True,
            "commandExecutableIdentityPinned": True,
        })
        or aggregate["independentRuntimeGates"] is not True
        or aggregate["filesystemMutationsDisabled"] is not True
        or aggregate["boundedCleanup"] is not True
        or aggregate["automatedQualificationComponent"] is not True
    ):
        fail("aggregate_gate_missing")
    stress = value["stress"]
    exact_keys(
        stress,
        {
            "status",
            "samples",
            "requests",
            "servicePidStable",
            "healthReadyStable",
            "sessionLifecycle",
            "exactToolAllowlist",
            "safeRootIdentityPinned",
            "safeRootAncestorIdentityPinned",
            "copyFileMutationDisabled",
            "highImpactDisabled",
            "longObservationRequired",
        },
        "aggregate_stress_invalid",
    )
    expect_int(stress["samples"], 32, 4096, "aggregate_stress_invalid")
    expect_int(stress["requests"], 100, 1000000, "aggregate_stress_invalid")
    if stress["requests"] < stress["samples"] * 3:
        fail("aggregate_stress_invalid")
    for key in (
        "servicePidStable",
        "healthReadyStable",
        "sessionLifecycle",
        "exactToolAllowlist",
        "safeRootIdentityPinned",
        "safeRootAncestorIdentityPinned",
        "copyFileMutationDisabled",
        "highImpactDisabled",
    ):
        if stress[key] is not True:
            fail("aggregate_stress_invalid")
    if stress["status"] != "pass" or stress["longObservationRequired"] is not False:
        fail("aggregate_stress_invalid")
    record["_candidate"] = candidate
    record["_environment"] = environment
    record["_samples"] = stress["samples"]


def specialized_validation_keys(kind):
    if kind == "battery":
        return {
            "status", "requests", "exactArtifact", "compileGate",
            "runtimeDefaultDisabled", "disabledDiscovery", "fixedProgram",
            "fixedWorkingDirectory", "noArguments", "inheritedEnvironmentCleared",
            "normalizedAllowlist", "sensitiveFieldsRedacted", "boundedOutput",
            "immediateOverflowTermination", "processGroupIsolation",
            "pipeHoldingDescendantCleanup", "callerCancellationCleanup",
            "boundedSupervisorCleanup", "stableErrors",
            "androidDeviceControlDisabled", "commandExecutionDisabled",
            "highImpactToolsDisabled",
        }
    if kind == "volume":
        return {
            "status", "requests", "exactArtifact", "compileGate",
            "runtimeDefaultDisabled", "disabledDiscovery", "fixedProgram",
            "fixedWorkingDirectory", "noArguments", "inheritedEnvironmentCleared",
            "normalizedAllowlist", "canonicalStreamOrdering",
            "unrecognizedFieldsRejected", "boundedOutput",
            "immediateOverflowTermination", "processGroupIsolation",
            "pipeHoldingDescendantCleanup", "callerCancellationCleanup",
            "boundedSupervisorCleanup", "stableErrors",
            "androidDeviceControlDisabled", "commandExecutionDisabled",
            "highImpactToolsDisabled",
        }
    if kind == "volume-control":
        return {
            "status", "requests", "exactArtifact", "compileGate",
            "runtimeDefaultDisabled", "disabledDiscovery", "staticTokenRequired",
            "capabilityKeyRequired", "closedInputSchema", "previewNoMutation",
            "previewDoesNotConsumeGrant", "headerContextEnforced",
            "exactGrantBinding", "singleUseReplay", "freshMaximum", "fixedProgram",
            "exactTwoArguments", "fixedWorkingDirectory",
            "inheritedEnvironmentCleared", "nullStdin", "nonQueueingConcurrency",
            "mutationVerified", "rollbackConfirmed", "rollbackUnconfirmed",
            "cancellationIndependentRecovery", "boundedSupervisor",
            "auditCounters", "redactedResponses",
            "arbitraryCommandExecutionDisabled", "broaderAndroidControlDisabled",
            "longObservationRequired",
        }
    return {
        "status", "requests", "exactArtifact", "compileGate",
        "runtimeDefaultDisabled", "disabledDiscovery", "fixedCurrentExecutable",
        "wrongExecutableNameFailsClosed",
        "wrongExecutableNameRejectedBeforeServing", "runningInodePinned",
        "workingDirectoryDescriptorPinned", "fixedArgvProfiles",
        "closedInputSchema", "overrideFieldsRejected", "unknownProfileRejected",
        "fixedWorkingDirectory", "inheritedEnvironmentCleared", "nullStdin",
        "boundedOutput", "utf8Output", "versionProfile", "helpProfile",
        "boundaryProfile", "auditCounters", "stableErrors",
        "arbitraryCommandExecutionDisabled", "androidDeviceControlDisabled",
        "highImpactToolsDisabled", "longObservationRequired",
    }


def validate_specialized(value, kind, candidate, aggregate_environment, artifacts):
    versions = {
        "battery": (3, "3", 2, None),
        "volume": (2, "2", 3, None),
        "volume-control": (2, "2", 4, 3),
        "command": (3, "3", 5, 0),
    }
    schema_version, gate_version, artifact_index, companion_index = versions[kind]
    exact_keys(
        value,
        {
            "schemaVersion",
            "gateVersion",
            "status",
            "failureCode",
            "releaseQualificationEligible",
            "startedAt",
            "completedAt",
            "candidate",
            "environment",
            "validation",
        },
        "specialized_evidence_invalid",
    )
    if (
        not is_int(value["schemaVersion"])
        or value["schemaVersion"] != schema_version
        or value["gateVersion"] != gate_version
        or value["status"] != "pass"
        or value["failureCode"] is not None
        or value["releaseQualificationEligible"] is not False
        or not isinstance(value["startedAt"], str)
        or TIME_RE.fullmatch(value["startedAt"]) is None
        or not isinstance(value["completedAt"], str)
        or TIME_RE.fullmatch(value["completedAt"]) is None
    ):
        fail("specialized_evidence_invalid")
    expect_timestamp_order(
        value["startedAt"],
        value["completedAt"],
        "specialized_timestamp_invalid",
    )
    specialized_candidate = value["candidate"]
    candidate_keys = {
        "commit", "version", "ciRunId", "securityRunId", "androidRunId", "artifact"
    }
    if kind == "volume-control":
        candidate_keys.add("incompatibleArtifact")
    if kind == "command":
        candidate_keys.add("defaultArtifact")
    exact_keys(specialized_candidate, candidate_keys, "specialized_candidate_invalid")
    for key in ("commit", "version", "ciRunId", "securityRunId", "androidRunId"):
        if specialized_candidate[key] != candidate[key]:
            fail("specialized_candidate_mismatch")
    expected_artifact = {
        "sha256": artifacts[artifact_index]["sha256"],
        "bytes": artifacts[artifact_index]["bytes"],
    }
    if not strict_equal(specialized_candidate["artifact"], expected_artifact):
        fail("specialized_artifact_mismatch")
    if kind == "volume-control" and not strict_equal(
        specialized_candidate["incompatibleArtifact"],
        {
            "sha256": artifacts[companion_index]["sha256"],
            "bytes": artifacts[companion_index]["bytes"],
        },
    ):
        fail("specialized_artifact_mismatch")
    if kind == "command" and not strict_equal(
        specialized_candidate["defaultArtifact"],
        {
            "sha256": artifacts[companion_index]["sha256"],
            "bytes": artifacts[companion_index]["bytes"],
        },
    ):
        fail("specialized_artifact_mismatch")
    environment = value["environment"]
    exact_keys(
        environment,
        {
            "architecture",
            "executionMode",
            "image",
            "imageDigest",
            "rootfsImageId",
            "runtimeImageDigest",
            "androidLinker",
        },
        "specialized_environment_invalid",
    )
    if not strict_equal(environment, aggregate_environment):
        fail("specialized_environment_invalid")
    validation = value["validation"]
    exact_keys(validation, specialized_validation_keys(kind), "specialized_gate_missing")
    minimums = {"battery": 18, "volume": 19, "volume-control": 20, "command": 29}
    expect_int(validation["requests"], minimums[kind], 10000, "specialized_gate_missing")
    if kind == "command" and validation["requests"] != 29:
        fail("specialized_gate_missing")
    if validation["status"] != "pass":
        fail("specialized_gate_missing")
    for key, item in validation.items():
        if key in ("status", "requests"):
            continue
        expected = False if key == "longObservationRequired" else True
        if item is not expected:
            fail("specialized_gate_missing")


def validate_classifier(value, record, candidate, aggregate_record, artifacts):
    exact_keys(
        value,
        {
            "schemaVersion", "classifierVersion", "status", "failureCode",
            "releaseQualificationEligible", "createdAt", "evidenceMode",
            "reasonCode", "inheritanceCandidate", "source", "candidate",
            "emulation", "claimBoundary", "protectedInputComparison",
            "changedInputClasses", "nextGate",
        },
        "classifier_evidence_invalid",
    )
    if (
        not is_int(value["schemaVersion"])
        or value["schemaVersion"] != 3
        or value["classifierVersion"] != "3"
        or value["status"] != "pass"
        or value["failureCode"] is not None
        or value["releaseQualificationEligible"] is not False
        or not isinstance(value["createdAt"], str)
        or TIME_RE.fullmatch(value["createdAt"]) is None
        or value["evidenceMode"] != "automated_release_qualification"
        or value["reasonCode"] != "automated_native_termux_evidence_required"
        or value["inheritanceCandidate"] is not False
        or value["nextGate"] != "assemble_automated_release_qualification"
    ):
        fail("classifier_evidence_invalid")
    parse_timestamp(value["createdAt"], "classifier_timestamp_invalid")
    validate_claim_boundary(value["claimBoundary"], "classifier_claim_boundary_invalid")
    exact_keys(value["source"], {"commit"}, "classifier_evidence_invalid")
    expect_pattern(value["source"]["commit"], COMMIT_RE, "classifier_evidence_invalid")
    classifier_candidate = value["candidate"]
    exact_keys(
        classifier_candidate,
        {
            "commit", "version", "ciRunId", "securityRunId", "androidRunId",
            "fullSuiteArtifactSha256", "fullSuiteManifestSha256",
        },
        "classifier_candidate_invalid",
    )
    expected_candidate = {
        "commit": candidate["commit"],
        "version": candidate["version"],
        "ciRunId": candidate["ciRunId"],
        "securityRunId": candidate["securityRunId"],
        "androidRunId": candidate["androidRunId"],
        "fullSuiteArtifactSha256": artifacts[6]["sha256"],
        "fullSuiteManifestSha256": artifacts[6]["manifestSha256"],
    }
    if not strict_equal(classifier_candidate, expected_candidate):
        fail("classifier_candidate_mismatch")
    emulation = value["emulation"]
    exact_keys(
        emulation,
        {"reportSha256", "executionMode", "imageDigest", "status", "samples"},
        "classifier_emulation_invalid",
    )
    if not strict_equal(emulation, {
        "reportSha256": aggregate_record["sha256"],
        "executionMode": "official-termux-docker-native-arm64",
        "imageDigest": aggregate_record["_environment"]["imageDigest"],
        "status": "pass",
        "samples": aggregate_record["_samples"],
    }):
        fail("classifier_emulation_mismatch")
    exact_keys(
        value["protectedInputComparison"],
        {
            "runtimeAndDeploymentInputsUnchanged",
            "cargoAndDependencyInputsUnchangedExceptRootVersion",
        },
        "classifier_evidence_invalid",
    )
    comparison = value["protectedInputComparison"]
    if not all(isinstance(item, bool) for item in comparison.values()):
        fail("classifier_evidence_invalid")
    changed = value["changedInputClasses"]
    expected_changed = []
    if not comparison["runtimeAndDeploymentInputsUnchanged"]:
        expected_changed.append("runtime_or_deployment")
    if not comparison["cargoAndDependencyInputsUnchangedExceptRootVersion"]:
        expected_changed.append("cargo_or_dependency")
    if changed != expected_changed:
        fail("classifier_evidence_invalid")


def validate_runtime_file_record(value, expected_name, maximum, code):
    exact_keys(value, {"fileName", "sha256", "bytes"}, code)
    if (
        value["fileName"] != expected_name
        or not isinstance(value["sha256"], str)
        or SHA_RE.fullmatch(value["sha256"]) is None
    ):
        fail(code)
    expect_int(value["bytes"], 1, maximum, code)


def validate_runtime_package_lock(value, actual_record, candidate, environment):
    code = "runtime_package_lock_contract_invalid"
    exact_keys(
        value,
        {
            "schemaVersion",
            "lockVersion",
            "repository",
            "commit",
            "androidRunId",
            "base",
            "requestedPackages",
            "resolution",
            "repositoryIndexes",
            "packages",
        },
        code,
    )
    if (
        not is_int(value["schemaVersion"])
        or value["schemaVersion"] != 1
        or value["lockVersion"] != "1"
        or value["repository"] != REPOSITORY
        or value["commit"] != candidate["commit"]
        or value["androidRunId"] != candidate["androidRunId"]
        or value["requestedPackages"] != REQUESTED_PACKAGES
        or value["resolution"]
        != {
            "resolver": "termux-apt-download-only",
            "repositoryMetadataAuthenticated": True,
            "packageBytesFrozenBeforeBuild": True,
            "finalImageBuildNetwork": "none",
        }
    ):
        fail(code)
    exact_keys(value["base"], {"image", "digest", "imageId"}, code)
    expected_base = {
        "image": BASE_IMAGE,
        "digest": environment["imageDigest"],
        "imageId": environment["rootfsImageId"],
    }
    if not strict_equal(value["base"], expected_base):
        fail("runtime_package_lock_environment_mismatch")

    indexes = value["repositoryIndexes"]
    if not isinstance(indexes, list) or not 1 <= len(indexes) <= 16:
        fail(code)
    index_names = []
    for record in indexes:
        if not isinstance(record, dict):
            fail(code)
        name = record.get("fileName")
        validate_runtime_file_record(record, name, 16_777_216, code)
        if (
            not isinstance(name, str)
            or FILE_RE.fullmatch(name) is None
            or name in index_names
        ):
            fail(code)
        index_names.append(name)
    if index_names != sorted(index_names):
        fail(code)

    packages = value["packages"]
    if not isinstance(packages, list) or not 1 <= len(packages) <= 512:
        fail(code)
    identities = set()
    names = set()
    observed_order = []
    for record in packages:
        exact_keys(
            record,
            {"package", "version", "architecture", "fileName", "sha256", "bytes"},
            code,
        )
        if (
            not isinstance(record["package"], str)
            or PACKAGE_RE.fullmatch(record["package"]) is None
            or not isinstance(record["version"], str)
            or PACKAGE_VERSION_RE.fullmatch(record["version"]) is None
            or not isinstance(record["architecture"], str)
            or ARCH_RE.fullmatch(record["architecture"]) is None
            or not isinstance(record["fileName"], str)
            or DEB_RE.fullmatch(record["fileName"]) is None
            or not isinstance(record["sha256"], str)
            or SHA_RE.fullmatch(record["sha256"]) is None
        ):
            fail(code)
        expect_int(record["bytes"], 1, 268_435_456, code)
        identity = (
            record["package"],
            record["version"],
            record["architecture"],
        )
        ordering = (*identity, record["fileName"])
        if identity in identities or record["fileName"] in names:
            fail(code)
        identities.add(identity)
        names.add(record["fileName"])
        observed_order.append(ordering)
    if observed_order != sorted(observed_order):
        fail(code)
    if not set(REQUESTED_PACKAGES).issubset({item[0] for item in identities}):
        fail(code)
    validate_runtime_file_record(
        actual_record,
        RUNTIME_LOCK_NAME,
        16_777_216,
        code,
    )


def validate_runtime_snapshot(value, actual_record, archive_record, lock_record):
    code = "runtime_snapshot_contract_invalid"
    exact_keys(
        value,
        {
            "schemaVersion",
            "snapshotVersion",
            "status",
            "failureCode",
            "releaseQualificationEligible",
            "repository",
            "commit",
            "androidRunId",
            "base",
            "runtimeImageId",
            "platform",
            "rootfsLayers",
            "packageLock",
            "installedPackages",
            "archive",
            "claimBoundary",
            "rebuildReproducibilityClaim",
        },
        code,
    )
    if (
        not is_int(value["schemaVersion"])
        or value["schemaVersion"] != 1
        or value["snapshotVersion"] != "1"
        or value["status"] != "pass"
        or value["failureCode"] is not None
        or value["releaseQualificationEligible"] is not False
        or value["repository"] != REPOSITORY
        or not isinstance(value["commit"], str)
        or COMMIT_RE.fullmatch(value["commit"]) is None
        or not isinstance(value["androidRunId"], str)
        or RUN_RE.fullmatch(value["androidRunId"]) is None
        or not isinstance(value["runtimeImageId"], str)
        or OCI_RE.fullmatch(value["runtimeImageId"]) is None
        or value["platform"] != {"os": "linux", "architecture": "arm64"}
        or value["rebuildReproducibilityClaim"] is not False
    ):
        fail(code)
    exact_keys(value["base"], {"image", "digest", "imageId"}, code)
    if (
        value["base"]["image"] != BASE_IMAGE
        or not isinstance(value["base"]["digest"], str)
        or OCI_RE.fullmatch(value["base"]["digest"]) is None
        or not isinstance(value["base"]["imageId"], str)
        or OCI_RE.fullmatch(value["base"]["imageId"]) is None
        or value["runtimeImageId"] == value["base"]["imageId"]
    ):
        fail(code)
    layers = value["rootfsLayers"]
    if (
        not isinstance(layers, list)
        or not 1 <= len(layers) <= 256
        or len(set(layers)) != len(layers)
        or any(
            not isinstance(layer, str) or OCI_RE.fullmatch(layer) is None
            for layer in layers
        )
    ):
        fail(code)
    validate_runtime_file_record(
        value["packageLock"],
        RUNTIME_LOCK_NAME,
        16_777_216,
        code,
    )
    if not strict_equal(value["packageLock"], lock_record):
        fail("runtime_snapshot_package_lock_mismatch")

    inventory = value["installedPackages"]
    exact_keys(inventory, {"sha256", "count", "packages"}, code)
    if (
        not isinstance(inventory["sha256"], str)
        or SHA_RE.fullmatch(inventory["sha256"]) is None
        or not isinstance(inventory["packages"], list)
    ):
        fail(code)
    expect_int(inventory["count"], 1, 4096, code)
    if len(inventory["packages"]) != inventory["count"]:
        fail(code)
    observed = []
    for package in inventory["packages"]:
        exact_keys(package, {"package", "version", "architecture"}, code)
        if (
            not isinstance(package["package"], str)
            or PACKAGE_RE.fullmatch(package["package"]) is None
            or not isinstance(package["version"], str)
            or PACKAGE_VERSION_RE.fullmatch(package["version"]) is None
            or not isinstance(package["architecture"], str)
            or ARCH_RE.fullmatch(package["architecture"]) is None
        ):
            fail(code)
        observed.append(
            (package["package"], package["version"], package["architecture"])
        )
    if observed != sorted(set(observed)):
        fail(code)
    inventory_raw = "".join(
        f"{package}\t{version}\t{architecture}\n"
        for package, version, architecture in observed
    ).encode()
    if digest_bytes(inventory_raw) != inventory["sha256"]:
        fail("runtime_snapshot_inventory_mismatch")

    archive = value["archive"]
    exact_keys(
        archive,
        {"fileName", "format", "compression", "sha256", "bytes"},
        code,
    )
    if (
        archive["fileName"] != RUNTIME_ARCHIVE_NAME
        or archive["format"] != "docker-image-archive-v1"
        or archive["compression"] != "gzip-no-name"
        or not isinstance(archive["sha256"], str)
        or SHA_RE.fullmatch(archive["sha256"]) is None
    ):
        fail(code)
    expect_int(archive["bytes"], 1, MAX_RUNTIME_ARCHIVE_BYTES, code)
    expected_archive = {
        "fileName": archive_record["fileName"],
        "format": "docker-image-archive-v1",
        "compression": "gzip-no-name",
        "sha256": archive_record["sha256"],
        "bytes": archive_record["bytes"],
    }
    if not strict_equal(archive, expected_archive):
        fail("runtime_snapshot_archive_mismatch")
    validate_claim_boundary(value["claimBoundary"], code)
    validate_runtime_file_record(
        actual_record,
        RUNTIME_SNAPSHOT_NAME,
        16_777_216,
        code,
    )


def validate_runtime_replay(value, actual_record, snapshot_record, snapshot, lock_record):
    code = "runtime_replay_contract_invalid"
    exact_keys(
        value,
        {
            "schemaVersion",
            "replayVersion",
            "status",
            "failureCode",
            "releaseQualificationEligible",
            "repository",
            "commit",
            "runtimeImageId",
            "snapshot",
            "packageLock",
            "installedPackages",
            "androidLinker",
            "verification",
            "claimBoundary",
            "rebuildReproducibilityClaim",
        },
        code,
    )
    if (
        not is_int(value["schemaVersion"])
        or value["schemaVersion"] != 1
        or value["replayVersion"] != "1"
        or value["status"] != "pass"
        or value["failureCode"] is not None
        or value["releaseQualificationEligible"] is not False
        or value["repository"] != REPOSITORY
        or value["commit"] != snapshot["commit"]
        or value["runtimeImageId"] != snapshot["runtimeImageId"]
        or value["rebuildReproducibilityClaim"] is not False
    ):
        fail(code)
    exact_keys(value["snapshot"], {"manifest", "archive"}, code)
    validate_runtime_file_record(
        value["snapshot"]["manifest"],
        RUNTIME_SNAPSHOT_NAME,
        16_777_216,
        code,
    )
    if (
        not strict_equal(value["snapshot"]["manifest"], snapshot_record)
        or not strict_equal(value["snapshot"]["archive"], snapshot["archive"])
    ):
        fail("runtime_replay_snapshot_mismatch")
    validate_runtime_file_record(
        value["packageLock"],
        RUNTIME_LOCK_NAME,
        16_777_216,
        code,
    )
    if not strict_equal(value["packageLock"], lock_record):
        fail("runtime_replay_package_lock_mismatch")
    expected_inventory = {
        "sha256": snapshot["installedPackages"]["sha256"],
        "count": snapshot["installedPackages"]["count"],
    }
    exact_keys(value["installedPackages"], {"sha256", "count"}, code)
    if not strict_equal(value["installedPackages"], expected_inventory):
        fail("runtime_replay_inventory_mismatch")
    exact_keys(value["androidLinker"], {"path", "sha256", "bytes"}, code)
    if (
        value["androidLinker"]["path"] != "/system/bin/linker64"
        or not isinstance(value["androidLinker"]["sha256"], str)
        or SHA_RE.fullmatch(value["androidLinker"]["sha256"]) is None
    ):
        fail(code)
    expect_int(value["androidLinker"]["bytes"], 1, 16_777_216, code)
    if not strict_equal(value["verification"], RUNTIME_VERIFICATION):
        fail("runtime_replay_verification_invalid")
    validate_claim_boundary(value["claimBoundary"], code)
    validate_runtime_file_record(
        actual_record,
        RUNTIME_REPLAY_NAME,
        16_777_216,
        code,
    )


def validate_retained_runtime(
    runtime_values,
    runtime_records,
    candidate,
    aggregate_environment,
    deployment_environment,
):
    package_lock = runtime_values["packageLock"]
    snapshot = runtime_values["snapshot"]
    replay = runtime_values["replay"]
    archive_record = runtime_records["archive"]
    lock_record = runtime_records["packageLock"]
    snapshot_record = runtime_records["snapshot"]
    replay_record = runtime_records["replay"]
    validate_runtime_package_lock(
        package_lock,
        lock_record,
        candidate,
        aggregate_environment,
    )
    validate_runtime_snapshot(
        snapshot,
        snapshot_record,
        archive_record,
        lock_record,
    )
    validate_runtime_replay(
        replay,
        replay_record,
        snapshot_record,
        snapshot,
        lock_record,
    )
    expected_base = {
        "image": BASE_IMAGE,
        "digest": aggregate_environment["imageDigest"],
        "imageId": aggregate_environment["rootfsImageId"],
    }
    if (
        snapshot["commit"] != candidate["commit"]
        or snapshot["androidRunId"] != candidate["androidRunId"]
        or not strict_equal(snapshot["base"], expected_base)
        or not strict_equal(package_lock["base"], expected_base)
        or snapshot["runtimeImageId"]
        != aggregate_environment["runtimeImageDigest"]
        or snapshot["runtimeImageId"]
        != deployment_environment["runtimeImageDigest"]
        or deployment_environment["rootfsImage"] != expected_base["image"]
        or deployment_environment["rootfsDigest"] != expected_base["digest"]
        or deployment_environment["rootfsImageId"] != expected_base["imageId"]
        or not strict_equal(
            replay["androidLinker"],
            {
                "path": deployment_environment["androidLinker"]["path"],
                "sha256": deployment_environment["androidLinker"]["sha256"],
                "bytes": deployment_environment["androidLinker"]["bytes"],
            },
        )
    ):
        fail("retained_runtime_cross_document_mismatch")
    return {
        "runtimeImageId": snapshot["runtimeImageId"],
        "base": snapshot["base"],
        "archive": archive_record,
        "packageLock": lock_record,
        "snapshot": snapshot_record,
        "replay": replay_record,
        "installedPackages": replay["installedPackages"],
        "androidLinker": replay["androidLinker"],
        "verification": replay["verification"],
        "claimBoundary": CLAIM_BOUNDARY,
        "rebuildReproducibilityClaim": False,
    }


def main(argv):
    parser = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    parser.add_argument("--policy", required=True)
    parser.add_argument("--scenario-set", required=True)
    parser.add_argument("--aggregate-evidence", required=True)
    parser.add_argument("--deployment-evidence", required=True)
    parser.add_argument("--classifier-evidence", required=True)
    parser.add_argument("--battery-evidence", required=True)
    parser.add_argument("--volume-evidence", required=True)
    parser.add_argument("--volume-control-evidence", required=True)
    parser.add_argument("--command-evidence", required=True)
    parser.add_argument("--runtime-archive", required=True)
    parser.add_argument("--runtime-package-lock", required=True)
    parser.add_argument("--runtime-snapshot", required=True)
    parser.add_argument("--runtime-replay", required=True)
    parser.add_argument("--default-dir", required=True)
    parser.add_argument("--mcp-runtime-dir", required=True)
    parser.add_argument("--battery-dir", required=True)
    parser.add_argument("--volume-dir", required=True)
    parser.add_argument("--volume-control-dir", required=True)
    parser.add_argument("--command-dir", required=True)
    parser.add_argument("--full-suite-dir", required=True)
    parser.add_argument("--qualification-run-id", required=True)
    parser.add_argument("--output", required=True)
    try:
        args = parser.parse_args(argv)
    except SystemExit:
        fail("arguments_invalid")
    if RUN_RE.fullmatch(args.qualification_run_id) is None:
        fail("qualification_run_id_invalid")

    policy_path, policy_raw = read_regular(
        args.policy,
        65536,
        "policy_file_invalid",
        basenames={"release-qualification-policy-v1.json"},
    )
    scenario_path, scenario_raw = read_regular(
        args.scenario_set,
        65536,
        "scenario_set_file_invalid",
        basenames={"automated-native-deployment-scenarios-v1.json"},
    )
    policy = parse_json_bytes(policy_raw, "policy_json_invalid")
    scenario_set = parse_json_bytes(scenario_raw, "scenario_set_json_invalid")
    validate_policy(policy, policy_raw)
    validate_scenario_set(scenario_set, scenario_raw)

    evidence_specs = [
        ("aggregate", args.aggregate_evidence, "termux-native-aggregate-evidence-v4.json"),
        ("deployment", args.deployment_evidence, "automated-native-deployment-v1.json"),
        ("classifier", args.classifier_evidence, "termux-observation-requirement-v3.json"),
        ("battery", args.battery_evidence, "termux-battery-emulated-evidence.json"),
        ("volume", args.volume_evidence, "termux-volume-emulated-evidence.json"),
        (
            "volume-control",
            args.volume_control_evidence,
            "termux-volume-control-emulated-evidence.json",
        ),
        ("command", args.command_evidence, "termux-command-emulated-evidence.json"),
    ]
    evidence_values = {}
    evidence_records = {}
    for key, path_text, name in evidence_specs:
        _path, raw = read_regular(
            path_text,
            16777216,
            f"{key}_evidence_file_invalid",
            private=True,
            basenames={name},
        )
        evidence_values[key] = parse_json_bytes(raw, f"{key}_evidence_json_invalid")
        evidence_records[key] = file_record(_path, raw, name)

    runtime_specs = [
        (
            "archive",
            args.runtime_archive,
            RUNTIME_ARCHIVE_NAME,
            MAX_RUNTIME_ARCHIVE_BYTES,
            False,
        ),
        (
            "packageLock",
            args.runtime_package_lock,
            RUNTIME_LOCK_NAME,
            16_777_216,
            True,
        ),
        (
            "snapshot",
            args.runtime_snapshot,
            RUNTIME_SNAPSHOT_NAME,
            16_777_216,
            True,
        ),
        (
            "replay",
            args.runtime_replay,
            RUNTIME_REPLAY_NAME,
            16_777_216,
            True,
        ),
    ]
    runtime_values = {}
    runtime_records = {}
    for key, path_text, name, maximum, capture in runtime_specs:
        _path, raw, record = inspect_runtime_input(
            path_text,
            name,
            maximum,
            f"runtime_{key}_file_invalid",
            capture,
        )
        runtime_records[key] = record
        if capture:
            runtime_values[key] = parse_json_bytes(
                raw,
                f"runtime_{key}_json_invalid",
            )

    aggregate_record = evidence_records["aggregate"]
    # Aggregate supplies the immutable candidate identity needed to verify all
    # seven exact bundles. Parse its closed candidate header before bundle work.
    aggregate = evidence_values["aggregate"]
    if not isinstance(aggregate.get("candidate"), dict):
        fail("aggregate_candidate_invalid")
    candidate_header = aggregate["candidate"]
    for key, pattern in (
        ("commit", COMMIT_RE),
        ("version", VERSION_RE),
        ("ciRunId", RUN_RE),
        ("securityRunId", RUN_RE),
        ("androidRunId", RUN_RE),
    ):
        if key not in candidate_header:
            fail("aggregate_candidate_invalid")
        expect_pattern(candidate_header[key], pattern, "aggregate_candidate_invalid")

    artifact_directories = [
        args.default_dir,
        args.mcp_runtime_dir,
        args.battery_dir,
        args.volume_dir,
        args.volume_control_dir,
        args.command_dir,
        args.full_suite_dir,
    ]
    artifacts = [
        validate_artifact_bundle(
            directory,
            index,
            candidate_header["commit"],
            candidate_header["version"],
            candidate_header["androidRunId"],
        )
        for index, directory in enumerate(artifact_directories)
    ]
    if len({artifact["sha256"] for artifact in artifacts}) != 7:
        fail("artifact_posture_digests_not_distinct")
    if len({artifact["manifestSha256"] for artifact in artifacts}) != 7:
        fail("artifact_manifest_digests_not_distinct")

    validate_aggregate(aggregate, aggregate_record, artifacts)
    candidate = aggregate_record["_candidate"]
    for kind in ("battery", "volume", "volume-control", "command"):
        validate_specialized(
            evidence_values[kind],
            kind,
            candidate,
            aggregate_record["_environment"],
            artifacts,
        )
    validate_classifier(
        evidence_values["classifier"],
        evidence_records["classifier"],
        candidate,
        aggregate_record,
        artifacts,
    )

    # Deployment evidence is validated after all exact artifact/evidence
    # identities are available. Its closed v1 contract is intentionally
    # consumed here rather than treated as a boolean.
    deployment = evidence_values["deployment"]
    exact_keys(
        deployment,
        {
            "schemaVersion", "gateVersion", "status", "failureCode",
            "releaseQualificationEligible", "qualificationClass", "startedAt",
            "completedAt", "candidate", "scenarioSet", "environment", "validation",
        },
        "deployment_evidence_invalid",
    )
    if (
        not is_int(deployment["schemaVersion"])
        or deployment["schemaVersion"] != 1
        or deployment["gateVersion"] != "1"
        or deployment["status"] != "pass"
        or deployment["failureCode"] is not None
        or deployment["releaseQualificationEligible"] is not False
        or deployment["qualificationClass"] != CLASS
        or not isinstance(deployment["startedAt"], str)
        or TIME_RE.fullmatch(deployment["startedAt"]) is None
        or not isinstance(deployment["completedAt"], str)
        or TIME_RE.fullmatch(deployment["completedAt"]) is None
    ):
        fail("deployment_evidence_invalid")
    expect_timestamp_order(
        deployment["startedAt"],
        deployment["completedAt"],
        "deployment_timestamp_invalid",
    )
    deployment_scenario = deployment["scenarioSet"]
    expected_scenario_record = {
        "fileName": scenario_path.name,
        "schemaVersion": 1,
        "scenarioSetVersion": "1",
        "sha256": SCENARIO_SHA256,
        "scenarioCount": 6,
        "scenarioIds": SCENARIO_IDS,
    }
    if not strict_equal(deployment_scenario, expected_scenario_record):
        fail("deployment_scenario_mismatch")
    deployment_candidate = deployment["candidate"]
    exact_keys(
        deployment_candidate,
        {
            "repository", "commit", "version", "ciRunId", "securityRunId",
            "nativeRunId", "artifact",
        },
        "deployment_candidate_invalid",
    )
    if (
        deployment_candidate["repository"] != REPOSITORY
        or deployment_candidate["commit"] != candidate["commit"]
        or deployment_candidate["version"] != candidate["version"]
        or deployment_candidate["ciRunId"] != candidate["ciRunId"]
        or deployment_candidate["securityRunId"] != candidate["securityRunId"]
        or deployment_candidate["nativeRunId"] != candidate["androidRunId"]
    ):
        fail("deployment_candidate_mismatch")
    if not strict_equal(deployment_candidate["artifact"], {
        "artifactName": ARTIFACT_NAMES[6],
        "posture": "full-suite",
        "features": ["full-suite"],
        "sha256": artifacts[6]["sha256"],
        "manifestSha256": artifacts[6]["manifestSha256"],
        "bytes": artifacts[6]["bytes"],
        "target": "aarch64-linux-android",
        "elf": "aarch64-android-elf",
    }):
        fail("deployment_artifact_mismatch")

    deployment_environment = deployment["environment"]
    # Exact keys are finalized by the native deployment gate and all values
    # below are mandatory release-envelope inputs.
    required_environment_keys = {
        "executionMode", "architecture", "rootfsImage", "rootfsDigest",
        "rootfsImageId",
        "runtimeImageDigest",
        "termuxPrefix", "androidLinker", "supervisor", "runitSupervisorObserved",
        "androidFrameworkObserved", "physicalHardwareObserved",
        "physicalDeviceObserved", "sustainedPhysicalSoak",
    }
    exact_keys(deployment_environment, required_environment_keys, "deployment_environment_invalid")
    if (
        deployment_environment["executionMode"] != "official-termux-docker-native-arm64"
        or deployment_environment["architecture"] != "aarch64"
        or deployment_environment["rootfsImage"] != "termux/termux-docker:aarch64"
        or not isinstance(deployment_environment["rootfsDigest"], str)
        or OCI_RE.fullmatch(deployment_environment["rootfsDigest"]) is None
        or deployment_environment["rootfsDigest"]
        != aggregate_record["_environment"]["imageDigest"]
        or not isinstance(deployment_environment["rootfsImageId"], str)
        or OCI_RE.fullmatch(deployment_environment["rootfsImageId"]) is None
        or deployment_environment["rootfsImageId"]
        != aggregate_record["_environment"]["rootfsImageId"]
        or not isinstance(deployment_environment["runtimeImageDigest"], str)
        or OCI_RE.fullmatch(deployment_environment["runtimeImageDigest"]) is None
        or deployment_environment["runtimeImageDigest"]
        != aggregate_record["_environment"]["runtimeImageDigest"]
        or deployment_environment["runtimeImageDigest"]
        == deployment_environment["rootfsImageId"]
        or deployment_environment["termuxPrefix"] != "/data/data/com.termux/files/usr"
        or deployment_environment["supervisor"] != "runit"
        or deployment_environment["runitSupervisorObserved"] is not True
        or deployment_environment["androidFrameworkObserved"] is not False
        or deployment_environment["physicalHardwareObserved"] is not False
        or deployment_environment["physicalDeviceObserved"] is not False
        or deployment_environment["sustainedPhysicalSoak"] is not False
    ):
        fail("deployment_environment_invalid")
    android_linker = deployment_environment["androidLinker"]
    exact_keys(
        android_linker,
        {"observed", "path", "sha256", "bytes"},
        "deployment_environment_invalid",
    )
    if (
        android_linker["observed"] is not True
        or android_linker["path"] != "/system/bin/linker64"
        or not isinstance(android_linker["sha256"], str)
        or SHA_RE.fullmatch(android_linker["sha256"]) is None
    ):
        fail("deployment_environment_invalid")
    expect_int(
        android_linker["bytes"],
        1,
        16777216,
        "deployment_environment_invalid",
    )
    deployment_validation = deployment["validation"]
    exact_keys(
        deployment_validation,
        {
            "status", "scenarioResults", "artifactManifestStrict",
            "scenarioSetStrict", "nativeArtifactExecuted",
            "isolatedFreshDeploy", "failedUpgradeRecovery", "supervisedRestart",
            "rollbackRecovery", "uninstall", "boundedCleanup", "exactArtifact",
            "isolatedServiceRoot", "runitSupervisorObserved",
            "realLoopbackProbes", "probeFaultInjectionBounded",
            "outputNoClobber", "workspaceRemoved", "serviceRemoved",
            "runsvdirTerminated", "physicalCertification",
        },
        "deployment_gate_missing",
    )
    if (
        deployment_validation["status"] != "pass"
        or deployment_validation["artifactManifestStrict"] is not True
        or deployment_validation["scenarioSetStrict"] is not True
        or deployment_validation["nativeArtifactExecuted"] is not True
        or deployment_validation["isolatedFreshDeploy"] is not True
        or deployment_validation["failedUpgradeRecovery"] is not True
        or deployment_validation["supervisedRestart"] is not True
        or deployment_validation["rollbackRecovery"] is not True
        or deployment_validation["uninstall"] is not True
        or deployment_validation["boundedCleanup"] is not True
        or deployment_validation["exactArtifact"] is not True
        or deployment_validation["isolatedServiceRoot"] is not True
        or deployment_validation["runitSupervisorObserved"] is not True
        or deployment_validation["realLoopbackProbes"] is not True
        or deployment_validation["probeFaultInjectionBounded"] is not True
        or deployment_validation["outputNoClobber"] is not True
        or deployment_validation["workspaceRemoved"] is not True
        or deployment_validation["serviceRemoved"] is not True
        or deployment_validation["runsvdirTerminated"] is not True
        or deployment_validation["physicalCertification"] != "not_run"
        or not isinstance(deployment_validation["scenarioResults"], list)
        or len(deployment_validation["scenarioResults"]) != 6
    ):
        fail("deployment_gate_missing")
    scenario_results = deployment_validation["scenarioResults"]
    if [item.get("id") for item in scenario_results] != SCENARIO_IDS:
        fail("deployment_gate_missing")
    expected_outcomes = ["pass", "recovered", "restarted", "recovered", "removed", "clean"]
    expected_boundaries = [
        "none", "target_readiness_probe", "supervised_process",
        "target_readiness_probe", "none", "none",
    ]
    for index, item in enumerate(scenario_results):
        exact_keys(
            item,
            {"id", "execution", "outcome", "faultBoundary"},
            "deployment_gate_missing",
        )
        if (
            item["execution"] != "native"
            or item["outcome"] != expected_outcomes[index]
            or item["faultBoundary"] != expected_boundaries[index]
        ):
            fail("deployment_gate_missing")

    retained_runtime = validate_retained_runtime(
        runtime_values,
        runtime_records,
        candidate,
        aggregate_record["_environment"],
        deployment_environment,
    )

    workflow_runs = {
        key: {
            "runId": candidate[f"{key}RunId"] if key != "android" else candidate["androidRunId"],
            "attempt": 1,
            "event": "push",
            "ref": "refs/heads/main",
            "headCommit": candidate["commit"],
            "conclusion": "success",
        }
        for key in ("ci", "security", "android")
    }
    environment = {
        "executionMode": "official-termux-docker-native-arm64",
        "architecture": "aarch64",
        "runtimeImageDigest": deployment_environment["runtimeImageDigest"],
        "rootfsUserland": {
            "image": "termux/termux-docker:aarch64",
            "digest": deployment_environment["rootfsDigest"],
            "imageId": deployment_environment["rootfsImageId"],
            "prefix": "/data/data/com.termux/files/usr",
        },
        "androidRuntime": {
            "abi": "android-bionic",
            "linkerPath": "/system/bin/linker64",
            "linkerSha256": android_linker["sha256"],
            "linkerBytes": android_linker["bytes"],
            "linkerIdentity": "aarch64-android-bionic-elf",
        },
    }
    envelope = {
        "schemaVersion": 1,
        "envelopeVersion": "1",
        "status": "pass",
        "failureCode": None,
        "releaseEligible": True,
        "qualificationClass": CLASS,
        "repository": REPOSITORY,
        "commit": candidate["commit"],
        "version": candidate["version"],
        "workflowRuns": workflow_runs,
        "qualificationRun": {
            "runId": args.qualification_run_id,
            "attempt": 1,
            "event": "workflow_run",
            "sourceWorkflow": "Android Cross Compile",
            "sourceRunId": candidate["androidRunId"],
        },
        "claimBoundary": CLAIM_BOUNDARY,
        "environment": environment,
        "retainedRuntime": retained_runtime,
        "policy": {
            "fileName": policy_path.name,
            "sha256": POLICY_SHA256,
        },
        "scenarioSet": expected_scenario_record,
        "artifacts": artifacts,
        "evidence": {
            "aggregate": {
                key: value for key, value in evidence_records["aggregate"].items()
                if not key.startswith("_")
            },
            "deployment": evidence_records["deployment"],
            "classifier": evidence_records["classifier"],
            "specialized": [
                evidence_records["battery"],
                evidence_records["volume"],
                evidence_records["volume-control"],
                evidence_records["command"],
            ],
        },
        "gates": {
            "firstAttemptMainWorkflows": "pass",
            "artifactLineage": "pass",
            "officialTermuxNativeRuntime": "pass",
            "aggregateComposition": "pass",
            "specializedProviderBoundaries": "pass",
            "isolatedDeploymentRecovery": "pass",
            "automatedReleaseClassification": "pass",
        },
    }
    ensure_no_forbidden_fields(envelope, "forbidden_qualification_claim_field")

    output = pathlib.Path(args.output)
    if not output.is_absolute() or output.name != "automated-qualification-v1.json":
        fail("output_path_invalid")
    if output.exists() or output.is_symlink():
        fail("output_already_exists")
    parent = output.parent
    try:
        resolved_parent = parent.resolve(strict=True)
        parent_stat = parent.stat(follow_symlinks=False)
    except (OSError, RuntimeError):
        fail("output_parent_invalid")
    if (
        resolved_parent != parent
        or not stat.S_ISDIR(parent_stat.st_mode)
        or stat.S_ISLNK(parent_stat.st_mode)
        or stat.S_IMODE(parent_stat.st_mode) != 0o700
    ):
        fail("output_parent_invalid")
    encoded = (json.dumps(envelope, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
    if len(encoded) > 262144:
        fail("envelope_size_invalid")
    temporary = None
    previous_signal_handlers = {}

    def interrupt_publication(_signal_number, _frame):
        fail("output_publication_interrupted")

    for signal_number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        previous_signal_handlers[signal_number] = signal.signal(
            signal_number, interrupt_publication
        )
    try:
        fd, temporary_name = tempfile.mkstemp(prefix=".automated-qualification.", dir=parent)
        temporary = pathlib.Path(temporary_name)
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb") as destination:
            destination.write(encoded)
            destination.flush()
            os.fsync(destination.fileno())
        encoded_sha256 = hashlib.sha256(encoded).hexdigest()
        commit_helper = os.environ.get("COMMIT_HELPER")
        if (
            not isinstance(commit_helper, str)
            or not os.path.isabs(commit_helper)
            or pathlib.Path(commit_helper).name != "commit_verified_file.py"
        ):
            fail("commit_helper_invalid")
        result = subprocess.run(
            [
                sys.executable,
                commit_helper,
                "--source",
                str(temporary),
                "--destination",
                str(output),
                "--sha256",
                encoded_sha256,
                "--mode",
                "600",
            ],
            check=False,
            stdin=subprocess.DEVNULL,
        )
        if result.returncode != 0:
            fail("output_publication_failed")
        os.unlink(temporary)
        temporary = None
    except OSError:
        fail("output_publication_failed")
    finally:
        if temporary is not None:
            try:
                temporary.unlink()
            except OSError:
                pass
        for signal_number, previous in previous_signal_handlers.items():
            signal.signal(signal_number, previous)


try:
    main(sys.argv[1:])
except QualificationError as error:
    print(
        f"AUTOMATED_QUALIFICATION_PACKAGE_RESULT=FAIL reason={error.code}",
        file=sys.stderr,
    )
    raise SystemExit(1)
except Exception:
    print(
        "AUTOMATED_QUALIFICATION_PACKAGE_RESULT=FAIL reason=internal_validation_error",
        file=sys.stderr,
    )
    raise SystemExit(1)

print("AUTOMATED_QUALIFICATION_PACKAGE_RESULT=PASS")
PY
