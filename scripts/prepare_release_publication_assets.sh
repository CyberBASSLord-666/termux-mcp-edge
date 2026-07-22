#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077

usage() {
  cat <<'EOF'
Usage: prepare_release_publication_assets.sh \
  --stage-tar /absolute/path/termux-mcp-server-vVERSION-release-stage-SHA12.tar \
  --staged-artifact-sha256 SHA256 \
  --assets-dir /absolute/new/path/release-assets \
  --receipt /absolute/new/path/release-publication-receipt-v1.json \
  --repository OWNER/REPO \
  --commit SHA \
  --version VERSION

The assets directory and receipt must be absent siblings under one existing,
caller-owned mode-0700 directory. The stage tar must be an absolute canonical,
caller-owned mode-0600 regular file. This command validates the complete stage
and copies only the 16 fixed publication assets. It never builds, repackages,
renames governed bytes, calls a network service, creates a tag, or publishes.
The receipt is the completion marker. A concurrent receipt-path conflict fails
closed without a receipt and may leave the already-validated assets directory
for cleanup inside the caller's private output parent.
EOF
}

STAGE_TAR=""
STAGED_ARTIFACT_SHA256=""
ASSETS_DIR=""
RECEIPT=""
REPOSITORY=""
COMMIT=""
VERSION=""
WORK_ROOT=""
COMPLETED=0

while (($# > 0)); do
  case "$1" in
    --stage-tar) (($# >= 2)) || { usage >&2; exit 2; }; STAGE_TAR="$2"; shift 2 ;;
    --staged-artifact-sha256) (($# >= 2)) || { usage >&2; exit 2; }; STAGED_ARTIFACT_SHA256="$2"; shift 2 ;;
    --assets-dir) (($# >= 2)) || { usage >&2; exit 2; }; ASSETS_DIR="$2"; shift 2 ;;
    --receipt) (($# >= 2)) || { usage >&2; exit 2; }; RECEIPT="$2"; shift 2 ;;
    --repository) (($# >= 2)) || { usage >&2; exit 2; }; REPOSITORY="$2"; shift 2 ;;
    --commit) (($# >= 2)) || { usage >&2; exit 2; }; COMMIT="$2"; shift 2 ;;
    --version) (($# >= 2)) || { usage >&2; exit 2; }; VERSION="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

fail() {
  printf '[release-publication-assets] ERROR: %s\n' "$1" >&2
  exit 1
}

cleanup() {
  if ((COMPLETED == 0)) \
    && [[ -n "$WORK_ROOT" && -n "$ASSETS_DIR" && "$WORK_ROOT" == "$ASSETS_DIR.staging.$$" ]]; then
    rm -rf -- "$WORK_ROOT" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

for command_name in awk basename chmod cp dirname file id mkdir mv python3 realpath rm sha256sum stat; do
  command -v "$command_name" >/dev/null 2>&1 || fail required_command_missing
done

required_values=(
  "$STAGE_TAR" "$STAGED_ARTIFACT_SHA256" "$ASSETS_DIR" "$RECEIPT"
  "$REPOSITORY" "$COMMIT" "$VERSION"
)
for required_value in "${required_values[@]}"; do
  [[ -n "$required_value" ]] || fail required_argument_missing
done

[[ "$REPOSITORY" == "CyberBASSLord-666/termux-mcp-edge" ]] || fail repository_invalid
[[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail commit_invalid
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail version_invalid
[[ "$STAGED_ARTIFACT_SHA256" =~ ^[0-9a-f]{64}$ ]] || fail staged_artifact_sha256_invalid

expected_stage_name="termux-mcp-server-v${VERSION}-release-stage-${COMMIT:0:12}.tar"
[[ "$(basename -- "$STAGE_TAR")" == "$expected_stage_name" ]] || fail stage_tar_name_invalid
[[ "$(basename -- "$RECEIPT")" == "release-publication-receipt-v1.json" ]] \
  || fail receipt_name_invalid

[[ "$STAGE_TAR" == /* && "$(realpath -- "$STAGE_TAR" 2>/dev/null)" == "$STAGE_TAR" ]] \
  || fail stage_tar_path_invalid
[[ -f "$STAGE_TAR" && ! -L "$STAGE_TAR" ]] || fail stage_tar_invalid
stage_bytes="$(stat -c '%s' -- "$STAGE_TAR" 2>/dev/null)" || fail stage_tar_invalid
[[ "$stage_bytes" =~ ^[0-9]+$ ]] || fail stage_tar_invalid
((stage_bytes > 0 && stage_bytes <= 536870912)) || fail stage_tar_invalid
[[ "$(stat -c '%a' -- "$STAGE_TAR" 2>/dev/null)" == 600 ]] || fail stage_tar_mode_invalid
[[ "$(stat -c '%u' -- "$STAGE_TAR" 2>/dev/null)" == "$(id -u)" ]] || fail stage_tar_owner_invalid

[[ "$ASSETS_DIR" == /* && "$(realpath -m -- "$ASSETS_DIR" 2>/dev/null)" == "$ASSETS_DIR" ]] \
  || fail assets_dir_path_invalid
[[ "$RECEIPT" == /* && "$(realpath -m -- "$RECEIPT" 2>/dev/null)" == "$RECEIPT" ]] \
  || fail receipt_path_invalid
[[ "$ASSETS_DIR" != / && ! -e "$ASSETS_DIR" && ! -L "$ASSETS_DIR" ]] \
  || fail assets_dir_invalid
[[ "$RECEIPT" != / && ! -e "$RECEIPT" && ! -L "$RECEIPT" ]] || fail receipt_invalid

output_parent="$(dirname -- "$ASSETS_DIR")"
receipt_parent="$(dirname -- "$RECEIPT")"
[[ "$output_parent" == "$receipt_parent" ]] || fail output_parent_mismatch
[[ -d "$output_parent" && ! -L "$output_parent" ]] || fail output_parent_invalid
[[ "$(realpath -- "$output_parent" 2>/dev/null)" == "$output_parent" ]] || fail output_parent_invalid
[[ "$(stat -c '%a' -- "$output_parent" 2>/dev/null)" == 700 ]] || fail output_parent_mode_invalid
[[ "$(stat -c '%u' -- "$output_parent" 2>/dev/null)" == "$(id -u)" ]] || fail output_parent_owner_invalid

script_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
schema="$script_root/docs/release-staging-manifest-schema-v1.json"
[[ -f "$schema" && ! -L "$schema" ]] || fail staging_schema_invalid

WORK_ROOT="$ASSETS_DIR.staging.$$"
[[ ! -e "$WORK_ROOT" && ! -L "$WORK_ROOT" ]] || fail work_root_exists
SNAPSHOT="$WORK_ROOT/$expected_stage_name"
EXTRACTED="$WORK_ROOT/extracted"
ASSET_WORK="$WORK_ROOT/assets"
RECEIPT_WORK="$WORK_ROOT/release-publication-receipt-v1.json"
mkdir -m 700 -- "$WORK_ROOT" "$EXTRACTED" "$ASSET_WORK" || fail work_root_create_failed

# Validate only the private snapshot. If the caller replaces or mutates the
# input while it is copied, the expected server digest rejects the snapshot.
cp -P --reflink=never -- "$STAGE_TAR" "$SNAPSHOT" || fail stage_snapshot_failed
[[ -f "$SNAPSHOT" && ! -L "$SNAPSHOT" ]] || fail stage_snapshot_failed
[[ "$(stat -c '%u' -- "$SNAPSHOT" 2>/dev/null)" == "$(id -u)" ]] \
  || fail stage_snapshot_failed
chmod 600 -- "$SNAPSHOT" || fail stage_snapshot_failed
snapshot_sha="$(sha256sum -- "$SNAPSHOT" | awk '{print $1}')" || fail stage_digest_failed
[[ "$snapshot_sha" == "$STAGED_ARTIFACT_SHA256" ]] || fail stage_digest_mismatch
[[ "$(stat -c '%s' -- "$SNAPSHOT")" == "$stage_bytes" ]] || fail stage_snapshot_size_mismatch

python3 - \
  "$SNAPSHOT" "$EXTRACTED" "$ASSET_WORK" "$RECEIPT_WORK" "$schema" \
  "$REPOSITORY" "$COMMIT" "$VERSION" "$STAGED_ARTIFACT_SHA256" <<'PY'
import hashlib
import datetime
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tarfile

(
    stage_tar_text,
    extracted_text,
    assets_text,
    receipt_text,
    schema_text,
    repository,
    commit,
    version,
    expected_stage_sha,
) = sys.argv[1:]

stage_tar = pathlib.Path(stage_tar_text)
extracted = pathlib.Path(extracted_text)
assets = pathlib.Path(assets_text)
receipt = pathlib.Path(receipt_text)
schema_path = pathlib.Path(schema_text)


def fail(code):
    print(f"[release-publication-assets] ERROR: {code}", file=sys.stderr)
    raise SystemExit(1)


def sha256_path(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def reject_json_constant(_value):
    raise ValueError("non-standard JSON numeric constant")


def reject_duplicate_json_keys(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate JSON object key")
        value[key] = item
    return value


def read_json(path):
    return json.loads(
        path.read_text(encoding="utf-8"),
        parse_constant=reject_json_constant,
        object_pairs_hook=reject_duplicate_json_keys,
    )


def exact_keys(value, expected, code):
    if not isinstance(value, dict) or set(value) != set(expected):
        fail(code)


def json_equal(left, right):
    # JSON Schema treats booleans as a distinct primitive type. Python's
    # ``True == 1`` must therefore never satisfy a numeric const or enum.
    if isinstance(left, bool) or isinstance(right, bool):
        return type(left) is type(right) and left == right
    if left is None or right is None:
        return left is None and right is None
    if (
        isinstance(left, (int, float))
        and isinstance(right, (int, float))
        and not isinstance(left, bool)
        and not isinstance(right, bool)
    ):
        return left == right
    return type(left) is type(right) and left == right


def matches_schema_type(instance, expected_type):
    if expected_type == "object":
        return isinstance(instance, dict)
    if expected_type == "array":
        return isinstance(instance, list)
    if expected_type == "string":
        return isinstance(instance, str)
    if expected_type == "integer":
        return isinstance(instance, int) and not isinstance(instance, bool)
    if expected_type == "number":
        return isinstance(instance, (int, float)) and not isinstance(instance, bool)
    if expected_type == "boolean":
        return isinstance(instance, bool)
    if expected_type == "null":
        return instance is None
    fail("staging_schema_unsupported")


def resolve_ref(root, reference):
    if not reference.startswith("#/"):
        fail("staging_schema_unsupported")
    value = root
    for component in reference[2:].split("/"):
        component = component.replace("~1", "/").replace("~0", "~")
        if not isinstance(value, dict) or component not in value:
            fail("staging_schema_unsupported")
        value = value[component]
    return value


class SchemaMismatch(Exception):
    pass


supported_schema_keywords = {
    "$defs", "$id", "$ref", "$schema", "additionalProperties", "allOf",
    "const", "description", "else", "enum", "format", "if", "items",
    "maxItems", "maxLength", "maximum", "minItems", "minLength", "minimum",
    "pattern", "prefixItems", "properties", "required", "then", "title",
    "type", "uniqueItems",
}


def schema_check(instance, node, root, location="$"):
    if node is True:
        return
    if node is False:
        raise SchemaMismatch
    if not isinstance(node, dict) or set(node) - supported_schema_keywords:
        fail("staging_schema_unsupported")

    if "$ref" in node:
        if not isinstance(node["$ref"], str):
            fail("staging_schema_unsupported")
        schema_check(instance, resolve_ref(root, node["$ref"]), root, location)

    all_of = node.get("allOf", [])
    if not isinstance(all_of, list):
        fail("staging_schema_unsupported")
    for child in all_of:
        schema_check(instance, child, root, location)

    if "if" in node:
        try:
            schema_check(instance, node["if"], root, location)
            condition_matches = True
        except SchemaMismatch:
            condition_matches = False
        branch_name = "then" if condition_matches else "else"
        if branch_name in node:
            schema_check(instance, node[branch_name], root, location)

    expected_type = node.get("type")
    if expected_type is not None:
        alternatives = expected_type if isinstance(expected_type, list) else [expected_type]
        if not alternatives or not all(isinstance(value, str) for value in alternatives):
            fail("staging_schema_unsupported")
        if not any(matches_schema_type(instance, value) for value in alternatives):
            raise SchemaMismatch

    if "const" in node and not json_equal(instance, node["const"]):
        raise SchemaMismatch
    if "enum" in node:
        if not isinstance(node["enum"], list):
            fail("staging_schema_unsupported")
        if not any(json_equal(instance, value) for value in node["enum"]):
            raise SchemaMismatch

    if isinstance(instance, dict):
        required = node.get("required", [])
        if not isinstance(required, list) or not all(isinstance(key, str) for key in required):
            fail("staging_schema_unsupported")
        if any(key not in instance for key in required):
            raise SchemaMismatch
        properties = node.get("properties", {})
        if not isinstance(properties, dict):
            fail("staging_schema_unsupported")
        for key, child in properties.items():
            if not isinstance(key, str):
                fail("staging_schema_unsupported")
            if key in instance:
                schema_check(instance[key], child, root, f"{location}.{key}")
        additional = node.get("additionalProperties", True)
        unexpected = set(instance) - set(properties)
        if additional is False and unexpected:
            raise SchemaMismatch
        if isinstance(additional, dict):
            for key in unexpected:
                schema_check(instance[key], additional, root, f"{location}.{key}")
        elif additional not in (True, False):
            fail("staging_schema_unsupported")

    if isinstance(instance, list):
        if "minItems" in node and len(instance) < node["minItems"]:
            raise SchemaMismatch
        if "maxItems" in node and len(instance) > node["maxItems"]:
            raise SchemaMismatch
        unique_items = node.get("uniqueItems", False)
        if not isinstance(unique_items, bool):
            fail("staging_schema_unsupported")
        if unique_items:
            encoded = [json.dumps(item, sort_keys=True, separators=(",", ":")) for item in instance]
            if len(encoded) != len(set(encoded)):
                raise SchemaMismatch
        prefix = node.get("prefixItems", [])
        if not isinstance(prefix, list):
            fail("staging_schema_unsupported")
        for index, child in enumerate(prefix):
            if index < len(instance):
                schema_check(instance[index], child, root, f"{location}[{index}]")
        items = node.get("items", True)
        if items is False and len(instance) > len(prefix):
            raise SchemaMismatch
        if isinstance(items, dict):
            for index in range(len(prefix), len(instance)):
                schema_check(instance[index], items, root, f"{location}[{index}]")
        elif items not in (True, False):
            fail("staging_schema_unsupported")

    if isinstance(instance, str):
        if "minLength" in node and len(instance) < node["minLength"]:
            raise SchemaMismatch
        if "maxLength" in node and len(instance) > node["maxLength"]:
            raise SchemaMismatch
        if "pattern" in node:
            try:
                matched = re.search(node["pattern"], instance)
            except (re.error, TypeError):
                fail("staging_schema_unsupported")
            if matched is None:
                raise SchemaMismatch
        if node.get("format") == "date-time":
            try:
                datetime.datetime.strptime(instance, "%Y-%m-%dT%H:%M:%SZ")
            except ValueError:
                raise SchemaMismatch
        elif "format" in node:
            fail("staging_schema_unsupported")
    if isinstance(instance, (int, float)) and not isinstance(instance, bool):
        if "minimum" in node and instance < node["minimum"]:
            raise SchemaMismatch
        if "maximum" in node and instance > node["maximum"]:
            raise SchemaMismatch


def schema_validate(instance, node, root, location="$", mismatch_code="staging_manifest_schema_mismatch"):
    try:
        schema_check(instance, node, root, location)
    except SchemaMismatch:
        fail(mismatch_code)


postures = [
    "default",
    "mcp-runtime",
    "android-battery-status",
    "android-volume-status",
    "android-volume-control",
    "command-execution",
    "full-suite",
]
features = [
    [],
    ["mcp-runtime"],
    ["android-battery-status"],
    ["android-volume-status"],
    ["android-volume-control"],
    ["command-execution"],
    ["full-suite"],
]
workflow_artifacts = [
    "termux-mcp-server-aarch64-linux-android-default",
    "termux-mcp-server-aarch64-linux-android-mcp-runtime",
    "termux-mcp-server-aarch64-linux-android-android-battery-status",
    "termux-mcp-server-aarch64-linux-android-android-volume-status",
    "termux-mcp-server-aarch64-linux-android-android-volume-control",
    "termux-mcp-server-aarch64-linux-android-command-execution",
    "termux-mcp-server-aarch64-linux-android-full-suite",
]
release_names = [
    f"termux-mcp-server-v{version}-aarch64-linux-android-{posture}"
    for posture in postures
]
checksum_names = [f"{name}.sha256" for name in release_names]
workflow_manifest_names = [f"{name}.workflow-manifest.json" for name in release_names]
evidence_names = [
    "evidence/emulated-release-v3.json",
    "evidence/release-validator-v11.json",
    "evidence/physical-qualification-v1.json",
    "evidence/android-battery-emulated-v2.json",
    "evidence/android-volume-emulated-v1.json",
    "evidence/android-volume-control-emulated-v1.json",
    "evidence/command-emulated-v2.json",
    "evidence/release-observation-requirement-v2.json",
]
expected_files = set(
    release_names
    + checksum_names
    + workflow_manifest_names
    + evidence_names
    + ["SHA256SUMS", "LICENSE", "release-staging-manifest-v1.json"]
)
expected_member_names = sorted(
    [".", "./evidence"] + [f"./{name}" for name in expected_files]
)

max_sizes = {}
for name in release_names:
    max_sizes[name] = 67_108_864
for name in checksum_names:
    max_sizes[name] = 256
for name in workflow_manifest_names:
    max_sizes[name] = 65_536
for name in evidence_names:
    max_sizes[name] = 1_048_576
max_sizes["evidence/physical-qualification-v1.json"] = 65_536
max_sizes["SHA256SUMS"] = 2_048
max_sizes["LICENSE"] = 1_048_576
max_sizes["release-staging-manifest-v1.json"] = 1_048_576

try:
    with tarfile.open(stage_tar, mode="r:") as archive:
        members = archive.getmembers()
        names = [member.name for member in members]
        if names != expected_member_names or len(names) != len(set(names)):
            fail("archive_members_invalid")
        total_size = 0
        expected_offset = 0
        for member in members:
            if member.offset != expected_offset or member.offset_data != member.offset + 512:
                fail("archive_layout_not_canonical")
            expected_offset = member.offset_data + ((member.size + 511) // 512) * 512
            if member.pax_headers or member.uname or member.gname:
                fail("archive_header_metadata_mismatch")
            if member.name == ".":
                if not member.isdir() or member.mode != 0o755 or member.linkname:
                    fail("archive_root_invalid")
            elif member.name == "./evidence":
                if not member.isdir() or member.mode != 0o755 or member.linkname:
                    fail("archive_directory_invalid")
            else:
                if not member.name.startswith("./"):
                    fail("archive_member_path_invalid")
                normalized = member.name[2:]
                path = pathlib.PurePosixPath(normalized)
                if (
                    path.is_absolute()
                    or not path.parts
                    or any(part in ("", ".", "..") for part in path.parts)
                    or "\\" in normalized
                    or normalized not in expected_files
                ):
                    fail("archive_member_path_invalid")
                if not member.isfile() or member.type not in (tarfile.REGTYPE, tarfile.AREGTYPE):
                    fail("archive_link_or_special_file")
                if member.linkname:
                    fail("archive_header_metadata_mismatch")
                if member.size < 1 or member.size > max_sizes[normalized]:
                    fail("archive_member_oversized")
                expected_mode = 0o755 if normalized in release_names else 0o644
                if member.mode != expected_mode:
                    fail("archive_member_mode_mismatch")
                total_size += member.size
            if (
                member.uid != 0
                or member.gid != 0
                or member.mtime != 0
                or member.devmajor != 0
                or member.devminor != 0
            ):
                fail("archive_header_metadata_mismatch")
        if total_size > 536_870_912:
            fail("archive_uncompressed_size_invalid")
        archive_size = stage_tar.stat().st_size
        canonical_size = ((expected_offset + 1024 + 10239) // 10240) * 10240
        if archive_size != canonical_size:
            fail("archive_layout_not_canonical")
        with stage_tar.open("rb") as raw_archive:
            for member in members:
                if not member.isfile() or member.size % 512 == 0:
                    continue
                padding_size = 512 - (member.size % 512)
                raw_archive.seek(member.offset_data + member.size)
                if raw_archive.read(padding_size) != b"\0" * padding_size:
                    fail("archive_layout_not_canonical")
            raw_archive.seek(expected_offset)
            if raw_archive.read() != b"\0" * (archive_size - expected_offset):
                fail("archive_layout_not_canonical")

        for member in members:
            if not member.isfile():
                continue
            normalized = member.name[2:]
            source = archive.extractfile(member)
            if source is None:
                fail("archive_extract_failed")
            target = extracted.joinpath(*pathlib.PurePosixPath(normalized).parts)
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            with target.open("xb") as output:
                shutil.copyfileobj(source, output, length=1024 * 1024)
            if target.stat().st_size != member.size:
                fail("archive_extract_size_mismatch")
            target.chmod(member.mode)
except (OSError, tarfile.TarError, ValueError):
    fail("archive_invalid")

try:
    schema = read_json(schema_path)
    manifest_path = extracted / "release-staging-manifest-v1.json"
    manifest = read_json(manifest_path)
except (OSError, UnicodeError, json.JSONDecodeError, ValueError):
    fail("staging_manifest_json_invalid")

if (
    schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema"
    or schema.get("type") != "object"
    or schema.get("additionalProperties") is not False
):
    fail("staging_schema_invalid")
schema_validate(manifest, schema, schema)

if (
    manifest["repository"] != repository
    or manifest["commit"] != commit
    or manifest["version"] != version
    or manifest["target"] != "aarch64-linux-android"
    or manifest["publicationState"] != "staged_not_released"
    or manifest["releaseEligible"] is not False
):
    fail("staging_manifest_identity_mismatch")
run_ids = manifest["workflowRuns"]
if any(re.fullmatch(r"[1-9][0-9]*", run_ids[key]) is None for key in ("ci", "security", "android")):
    fail("staging_manifest_run_id_invalid")


def validate_record(record, expected_name, code):
    exact_keys(record, ["fileName", "sha256", "bytes"], code)
    if record["fileName"] != expected_name:
        fail(code)
    path = extracted / expected_name
    if path.stat().st_size != record["bytes"] or sha256_path(path) != record["sha256"]:
        fail(code)


validate_record(manifest["license"], "LICENSE", "license_record_mismatch")
validate_record(manifest["evidence"]["aggregate"], evidence_names[0], "evidence_record_mismatch")
validate_record(manifest["evidence"]["validator"], evidence_names[1], "evidence_record_mismatch")
validate_record(manifest["evidence"]["physicalQualification"], evidence_names[2], "evidence_record_mismatch")
for record, expected_name in zip(manifest["evidence"]["specialized"], evidence_names[3:]):
    validate_record(record, expected_name, "evidence_record_mismatch")

binary_digests = []
binary_sizes = []
manifest_digests = []
for index, artifact in enumerate(manifest["artifacts"]):
    binary_name = release_names[index]
    checksum_name = checksum_names[index]
    workflow_manifest_name = workflow_manifest_names[index]
    if (
        artifact["posture"] != postures[index]
        or artifact["features"] != features[index]
        or artifact["workflowArtifactName"] != workflow_artifacts[index]
        or artifact["workflowFileName"] != "termux-mcp-server"
        or artifact["workflowManifestFileName"] != workflow_manifest_name
        or artifact["releaseFileName"] != binary_name
        or artifact["checksumFileName"] != checksum_name
        or artifact["elf"] != "aarch64-android-elf"
    ):
        fail("artifact_record_identity_mismatch")
    binary_path = extracted / binary_name
    checksum_path = extracted / checksum_name
    workflow_manifest_path = extracted / workflow_manifest_name
    digest = sha256_path(binary_path)
    size = binary_path.stat().st_size
    manifest_digest = sha256_path(workflow_manifest_path)
    if digest != artifact["sha256"] or size != artifact["bytes"]:
        fail("artifact_record_digest_mismatch")
    if manifest_digest != artifact["workflowManifestSha256"]:
        fail("workflow_manifest_record_mismatch")
    expected_checksum = f"{digest}  {binary_name}\n".encode("ascii")
    if checksum_path.read_bytes() != expected_checksum:
        fail("per_file_checksum_mismatch")
    identity = subprocess.run(
        ["file", "-b", "--", str(binary_path)],
        check=False,
        capture_output=True,
        text=True,
    )
    if identity.returncode != 0:
        fail("binary_identity_failed")
    identity_text = identity.stdout.strip()
    if (
        "ELF" not in identity_text
        or "ARM aarch64" not in identity_text
        or ("Android" not in identity_text and "/system/bin/linker64" not in identity_text)
    ):
        fail("binary_architecture_mismatch")
    try:
        workflow_manifest = read_json(workflow_manifest_path)
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError):
        fail("workflow_manifest_json_invalid")
    exact_keys(
        workflow_manifest,
        [
            "schemaVersion", "repository", "commit", "workflowRunId", "artifactName",
            "posture", "features", "target", "fileName", "version", "sha256",
            "bytes", "elf", "createdAt",
        ],
        "workflow_manifest_closed_schema_mismatch",
    )
    if workflow_manifest != {
        **workflow_manifest,
        "schemaVersion": 1,
        "repository": repository,
        "commit": commit,
        "workflowRunId": run_ids["android"],
        "artifactName": workflow_artifacts[index],
        "posture": postures[index],
        "features": features[index],
        "target": "aarch64-linux-android",
        "fileName": "termux-mcp-server",
        "version": version,
        "sha256": digest,
        "bytes": size,
        "elf": "aarch64-android-elf",
    }:
        fail("workflow_manifest_identity_mismatch")
    if not isinstance(workflow_manifest["createdAt"], str) or re.fullmatch(
        r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z",
        workflow_manifest["createdAt"],
    ) is None:
        fail("workflow_manifest_created_at_invalid")
    binary_digests.append(digest)
    binary_sizes.append(size)
    manifest_digests.append(manifest_digest)

if len(set(binary_digests)) != 7:
    fail("artifact_digests_not_distinct")
expected_combined = "".join(
    f"{digest}  {name}\n" for digest, name in zip(binary_digests, release_names)
).encode("ascii")
if (extracted / "SHA256SUMS").read_bytes() != expected_combined:
    fail("combined_checksum_mismatch")


def load_governed_evidence(evidence_index, schema_name, mismatch_code):
    evidence_path = extracted / evidence_names[evidence_index]
    evidence_schema_path = schema_path.parent / schema_name
    try:
        value = read_json(evidence_path)
        evidence_schema = read_json(evidence_schema_path)
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError):
        fail(mismatch_code)
    if (
        evidence_schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema"
        or evidence_schema.get("type") != "object"
        or evidence_schema.get("additionalProperties") is not False
    ):
        fail("evidence_schema_invalid")
    schema_validate(value, evidence_schema, evidence_schema, mismatch_code=mismatch_code)
    return value


def timestamp_order_valid(started, completed):
    try:
        started_at = datetime.datetime.strptime(started, "%Y-%m-%dT%H:%M:%SZ")
        completed_at = datetime.datetime.strptime(completed, "%Y-%m-%dT%H:%M:%SZ")
    except (TypeError, ValueError):
        return False
    return completed_at >= started_at


aggregate = load_governed_evidence(
    0, "emulated-release-evidence-schema-v3.json", "aggregate_evidence_mismatch"
)
aggregate_candidate = aggregate["candidate"]
if (
    not timestamp_order_valid(aggregate["startedAt"], aggregate["completedAt"])
    or aggregate["schemaVersion"] != 3
    or aggregate["gateVersion"] != "3"
    or aggregate["status"] != "pass"
    or aggregate["failureCode"] is not None
    or aggregate["releaseQualificationEligible"] is not False
    or aggregate_candidate["commit"] != commit
    or aggregate_candidate["version"] != version
    or aggregate_candidate["ciRunId"] != run_ids["ci"]
    or aggregate_candidate["securityRunId"] != run_ids["security"]
    or aggregate_candidate["androidRunId"] != run_ids["android"]
    or aggregate_candidate["defaultArtifact"]
    != {"sha256": binary_digests[0], "bytes": binary_sizes[0]}
    or aggregate_candidate["mcpRuntimeArtifact"]
    != {"sha256": binary_digests[1], "bytes": binary_sizes[1]}
    or aggregate_candidate["androidVolumeControlArtifact"]
    != {"sha256": binary_digests[4], "bytes": binary_sizes[4]}
    or aggregate_candidate["fullSuiteArtifact"]
    != {
        "sha256": binary_digests[6],
        "bytes": binary_sizes[6],
        "manifestSha256": manifest_digests[6],
        "artifactName": workflow_artifacts[6],
        "posture": "full-suite",
        "features": ["full-suite"],
        "fileName": "termux-mcp-server",
    }
    or aggregate["environment"]["executionMode"]
    != "official-termux-docker-native-arm64"
    or aggregate["environment"]["architecture"] not in ("aarch64", "arm64")
    or aggregate["environment"]["androidLinker"] is not True
    or aggregate["runtimeValidation"]["status"] != "pass"
    or aggregate["runtimeValidation"]["phases"]["preflight"] != "pass"
    or aggregate["runtimeValidation"]["phases"]["runtime"] != "pass"
    or aggregate["aggregateValidation"]["status"] != "pass"
    or aggregate["aggregateValidation"]["requests"] < 14
    or aggregate["stress"]["status"] != "pass"
    or aggregate["stress"]["servicePidStable"] is not True
    or aggregate["stress"]["healthReadyStable"] is not True
    or aggregate["stress"]["longObservationRequired"] is not False
):
    fail("aggregate_evidence_mismatch")

specialized_specs = [
    (3, "android-battery-emulated-evidence-schema-v2.json", 2, "2", 2, None, None),
    (4, "android-volume-emulated-evidence-schema-v1.json", 1, "1", 3, None, None),
    (
        5,
        "android-volume-control-emulated-evidence-schema-v1.json",
        1,
        "1",
        4,
        "incompatibleArtifact",
        3,
    ),
    (6, "command-emulated-evidence-schema-v2.json", 2, "2", 5, "defaultArtifact", 0),
]
for evidence_index, schema_name, schema_version, gate_version, artifact_index, related_key, related_index in specialized_specs:
    specialized = load_governed_evidence(
        evidence_index, schema_name, "specialized_evidence_mismatch"
    )
    expected_candidate = {
        "commit": commit,
        "version": version,
        "ciRunId": run_ids["ci"],
        "securityRunId": run_ids["security"],
        "androidRunId": run_ids["android"],
        "artifact": {
            "sha256": binary_digests[artifact_index],
            "bytes": binary_sizes[artifact_index],
        },
    }
    if related_key is not None:
        expected_candidate[related_key] = {
            "sha256": binary_digests[related_index],
            "bytes": binary_sizes[related_index],
        }
    validation = specialized["validation"]
    boolean_results = [
        value
        for key, value in validation.items()
        if isinstance(value, bool) and key != "longObservationRequired"
    ]
    if (
        not timestamp_order_valid(specialized["startedAt"], specialized["completedAt"])
        or specialized["schemaVersion"] != schema_version
        or specialized["gateVersion"] != gate_version
        or specialized["status"] != "pass"
        or specialized["failureCode"] is not None
        or specialized["releaseQualificationEligible"] is not False
        or specialized["candidate"] != expected_candidate
        or specialized["environment"]["executionMode"]
        != "official-termux-docker-native-arm64"
        or specialized["environment"]["architecture"] not in ("aarch64", "arm64")
        or specialized["environment"]["androidLinker"] is not True
        or validation["status"] != "pass"
        or validation["requests"] < 1
        or validation["exactArtifact"] is not True
        or not boolean_results
        or not all(boolean_results)
        or (
            "longObservationRequired" in validation
            and validation["longObservationRequired"] is not False
        )
    ):
        fail("specialized_evidence_mismatch")

observation_requirement = load_governed_evidence(
    7,
    "release-observation-requirement-schema-v2.json",
    "observation_requirement_mismatch",
)
if (
    observation_requirement["schemaVersion"] != 2
    or observation_requirement["classifierVersion"] != "2"
    or observation_requirement["status"] != "pass"
    or observation_requirement["failureCode"] is not None
    or observation_requirement["releaseQualificationEligible"] is not False
    or observation_requirement["reasonCode"]
    != "full_suite_direct_physical_observation_required"
    or observation_requirement["inheritanceCandidate"] is not False
    or observation_requirement["nextGate"] != "direct_physical_device_observation"
    or observation_requirement["candidate"]
    != {
        "commit": commit,
        "version": version,
        "ciRunId": run_ids["ci"],
        "securityRunId": run_ids["security"],
        "androidRunId": run_ids["android"],
        "fullSuiteArtifactSha256": binary_digests[6],
        "fullSuiteManifestSha256": manifest_digests[6],
    }
    or observation_requirement["emulation"]["reportSha256"]
    != sha256_path(extracted / evidence_names[0])
    or observation_requirement["emulation"]["status"] != "pass"
    or observation_requirement["emulation"]["executionMode"]
    != "official-termux-docker-native-arm64"
    or "full_suite_artifact" not in observation_requirement["changedInputClasses"]
):
    fail("observation_requirement_mismatch")

try:
    validator_path = extracted / evidence_names[1]
    physical_path = extracted / evidence_names[2]
    validator = read_json(validator_path)
    physical = read_json(physical_path)
except (OSError, UnicodeError, json.JSONDecodeError, ValueError):
    fail("qualification_json_invalid")

exact_keys(
    validator,
    [
        "artifacts", "completedAt", "deploymentCandidate", "environment", "failureCode",
        "phases", "releaseEligible", "repository", "requestedPhase", "results",
        "schemaVersion", "startedAt", "status", "sustainedObservation", "validatorVersion",
    ],
    "validator_closed_schema_mismatch",
)
exact_keys(validator["repository"], ["androidRunId", "ciRunId", "commit", "securityRunId", "version"], "validator_repository_mismatch")
if (
    validator["schemaVersion"] != 2
    or validator["validatorVersion"] != "11"
    or validator["status"] != "pass"
    or validator["failureCode"] is not None
    or validator["releaseEligible"] is not True
    or validator["requestedPhase"] != "all"
    or validator["repository"] != {
        "androidRunId": run_ids["android"],
        "ciRunId": run_ids["ci"],
        "commit": commit,
        "securityRunId": run_ids["security"],
        "version": version,
    }
    or validator["phases"] != {"preflight": "pass", "runtime": "pass", "deployment": "pass"}
    or validator["deploymentCandidate"] != {"posture": "full-suite", "productionAction": None}
):
    fail("validator_eligibility_mismatch")
try:
    started_at = datetime.datetime.strptime(validator["startedAt"], "%Y-%m-%dT%H:%M:%SZ")
    completed_at = datetime.datetime.strptime(validator["completedAt"], "%Y-%m-%dT%H:%M:%SZ")
except (TypeError, ValueError):
    fail("validator_timestamp_mismatch")
if completed_at < started_at:
    fail("validator_timestamp_mismatch")
exact_keys(validator["environment"], ["architecture", "fixtureMode", "tools"], "validator_environment_mismatch")
if (
    validator["environment"]["architecture"] not in ("aarch64", "arm64")
    or validator["environment"]["fixtureMode"] is not False
    or not isinstance(validator["environment"]["tools"], dict)
    or set(validator["environment"]["tools"]) != {"bash", "curl", "file", "jq"}
    or any(
        not isinstance(value, str)
        or not (1 <= len(value) <= 256)
        or "\0" in value
        or "\n" in value
        or "\r" in value
        for value in validator["environment"]["tools"].values()
    )
    or len(validator["environment"]["tools"]["jq"]) > 64
):
    fail("validator_environment_mismatch")

artifact_bindings = {
    "default": 0,
    "mcpRuntime": 1,
    "androidVolumeControl": 4,
    "fullSuite": 6,
}
exact_keys(validator["artifacts"], ["androidVolumeControl", "baseline", "default", "fullSuite", "mcpRuntime"], "validator_artifacts_mismatch")
for key, index in artifact_bindings.items():
    if validator["artifacts"][key] != {
        "sha256": binary_digests[index],
        "bytes": binary_sizes[index],
        "version": version,
        "elf": "aarch64-android-elf",
    }:
        fail("validator_artifacts_mismatch")
baseline = validator["artifacts"]["baseline"]
exact_keys(baseline, ["bytes", "elf", "sha256", "version"], "validator_baseline_mismatch")
if (
    not isinstance(baseline["bytes"], int)
    or isinstance(baseline["bytes"], bool)
    or baseline["bytes"] < 1
    or baseline["bytes"] > 67_108_864
    or re.fullmatch(r"[0-9a-f]{64}", baseline["sha256"]) is None
    or not isinstance(baseline["version"], str)
    or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}", baseline["version"]) is None
    or baseline["version"] == version
    or baseline["elf"] != "aarch64-android-elf"
    or baseline["sha256"] in binary_digests
):
    fail("validator_baseline_mismatch")

required_result_codes = {
    "full_suite_default_disabled_17_tool_posture_verified",
    "full_suite_battery_runtime_gate_independence_verified",
    "full_suite_volume_status_runtime_gate_independence_verified",
    "full_suite_volume_control_runtime_gate_independence_verified",
    "full_suite_command_runtime_gate_independence_verified",
    "full_suite_enabled_21_tool_posture_verified",
    "full_suite_optional_provider_success_verified",
    "full_suite_volume_preview_and_grant_boundary_verified",
    "full_suite_command_basename_and_profile_verified",
    "full_suite_filesystem_mutations_independently_disabled",
    "full_suite_deployment_candidate_selected",
}
if not isinstance(validator["results"], list) or not (1 <= len(validator["results"]) <= 256):
    fail("validator_results_mismatch")
passed_codes = set()
for result in validator["results"]:
    exact_keys(result, ["check", "code", "outcome", "phase"], "validator_results_mismatch")
    if (
        result["outcome"] not in ("pass", "fail", "info")
        or result["outcome"] == "fail"
        or result["phase"] not in ("preflight", "runtime", "deployment")
        or not isinstance(result["check"], str)
        or re.fullmatch(r"[a-z0-9_]{1,96}", result["check"]) is None
        or not isinstance(result["code"], str)
        or re.fullmatch(r"[a-z0-9_]{1,96}", result["code"]) is None
    ):
        fail("validator_results_mismatch")
    if result["outcome"] == "pass":
        passed_codes.add(result["code"])
if not required_result_codes.issubset(passed_codes):
    fail("validator_results_mismatch")

observation = validator["sustainedObservation"]
exact_keys(observation, ["minimumMinutes", "minutes", "operatorSupplied", "reasonCode", "status"], "validator_observation_mismatch")
if (
    observation["operatorSupplied"] is not True
    or observation["status"] != "pass"
    or not isinstance(observation["minutes"], int)
    or isinstance(observation["minutes"], bool)
    or not (60 <= observation["minutes"] <= 10080)
    or observation["minimumMinutes"] != 60
    or observation["reasonCode"] != "stable"
):
    fail("validator_observation_mismatch")

exact_keys(
    physical,
    [
        "androidRunId", "architecture", "ciRunId", "cleanupConfirmed", "commit",
        "envelopeVersion", "failureCode", "harnessPassed", "harnessVersion",
        "nativeFullSuiteSha256", "rawHarnessReportSha256", "releaseEligible", "repository",
        "schemaVersion", "securityRunId", "status", "validatorReportSha256",
        "validatorVersion", "version", "workflowFullSuiteSha256",
    ],
    "physical_qualification_closed_schema_mismatch",
)
if (
    physical["schemaVersion"] != 1
    or physical["envelopeVersion"] != "1"
    or physical["status"] != "pass"
    or physical["failureCode"] is not None
    or physical["releaseEligible"] is not True
    or physical["repository"] != repository
    or physical["commit"] != commit
    or physical["version"] != version
    or physical["ciRunId"] != run_ids["ci"]
    or physical["securityRunId"] != run_ids["security"]
    or physical["androidRunId"] != run_ids["android"]
    or physical["validatorVersion"] != "11"
    or physical["harnessVersion"] != "11"
    or physical["architecture"] != "aarch64"
    or physical["validatorReportSha256"] != sha256_path(validator_path)
    or physical["workflowFullSuiteSha256"] != binary_digests[6]
    or physical["harnessPassed"] is not True
    or physical["cleanupConfirmed"] is not True
    or re.fullmatch(r"[0-9a-f]{64}", physical["rawHarnessReportSha256"]) is None
    or re.fullmatch(r"[0-9a-f]{64}", physical["nativeFullSuiteSha256"]) is None
):
    fail("physical_qualification_eligibility_mismatch")

publication_members = release_names + checksum_names + ["SHA256SUMS"]
asset_records = []
for name in publication_members:
    source = extracted / name
    destination = assets / name
    shutil.copyfile(source, destination)
    destination.chmod(0o755 if name in release_names else 0o644)
    if sha256_path(destination) != sha256_path(source) or destination.stat().st_size != source.stat().st_size:
        fail("publication_asset_copy_mismatch")
    asset_records.append({
        "name": name,
        "sha256": sha256_path(destination),
        "size": destination.stat().st_size,
        "sourceStageMember": name,
    })

stage_name = stage_tar.name
stage_destination = assets / stage_name
shutil.copyfile(stage_tar, stage_destination)
stage_destination.chmod(0o600)
stage_sha = sha256_path(stage_destination)
if stage_sha != expected_stage_sha or stage_destination.stat().st_size != stage_tar.stat().st_size:
    fail("stage_asset_copy_mismatch")
asset_records.append({
    "name": stage_name,
    "sha256": stage_sha,
    "size": stage_destination.stat().st_size,
    "sourceStageMember": None,
})
asset_records.sort(key=lambda record: record["name"])

receipt_value = {
    "assets": asset_records,
    "commit": commit,
    "repository": repository,
    "schemaVersion": 1,
    "stageTar": {
        "name": stage_name,
        "sha256": expected_stage_sha,
        "size": stage_tar.stat().st_size,
    },
    "version": version,
}
receipt.write_text(
    json.dumps(receipt_value, ensure_ascii=True, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
receipt.chmod(0o600)

actual_asset_names = sorted(path.name for path in assets.iterdir())
if actual_asset_names != [record["name"] for record in asset_records] or len(actual_asset_names) != 16:
    fail("publication_asset_inventory_mismatch")
for record in asset_records:
    path = assets / record["name"]
    if not path.is_file() or path.is_symlink():
        fail("publication_asset_inventory_mismatch")
    if path.stat().st_size != record["size"] or sha256_path(path) != record["sha256"]:
        fail("publication_asset_receipt_mismatch")

PY

[[ ! -e "$ASSETS_DIR" && ! -L "$ASSETS_DIR" ]] || fail assets_dir_publication_conflict
[[ ! -e "$RECEIPT" && ! -L "$RECEIPT" ]] || fail receipt_publication_conflict
# Sibling paths cannot be committed atomically. Publish the assets first and
# the receipt last as the transaction marker. On a receipt race, deliberately
# leave the validated assets in place: deleting a path that another process can
# exchange would create a more dangerous race. The missing receipt is a
# fail-closed result and the private workflow directory is discarded by its
# caller.
mv -T --no-clobber -- "$ASSET_WORK" "$ASSETS_DIR" || fail assets_publication_failed
[[ ! -e "$ASSET_WORK" && ! -L "$ASSET_WORK" && -d "$ASSETS_DIR" && ! -L "$ASSETS_DIR" ]] \
  || fail assets_publication_failed
mv -T --no-clobber -- "$RECEIPT_WORK" "$RECEIPT" || fail receipt_publication_failed
[[ ! -e "$RECEIPT_WORK" && ! -L "$RECEIPT_WORK" && -f "$RECEIPT" && ! -L "$RECEIPT" ]] \
  || fail receipt_publication_failed

python3 - \
  "$ASSETS_DIR" "$RECEIPT" "$REPOSITORY" "$COMMIT" "$VERSION" "$STAGED_ARTIFACT_SHA256" <<'PY'
import hashlib
import json
import os
import pathlib
import re
import stat
import sys

assets = pathlib.Path(sys.argv[1])
receipt_path = pathlib.Path(sys.argv[2])
repository, commit, version, stage_sha = sys.argv[3:]

def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def reject_json_constant(_value):
    raise ValueError("non-standard JSON numeric constant")


def reject_duplicate_json_keys(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate JSON object key")
        value[key] = item
    return value


try:
    if (
        not assets.is_dir()
        or assets.is_symlink()
        or stat.S_IMODE(assets.stat().st_mode) != 0o700
        or assets.stat().st_uid != os.getuid()
        or not receipt_path.is_file()
        or receipt_path.is_symlink()
        or stat.S_IMODE(receipt_path.stat().st_mode) != 0o600
        or receipt_path.stat().st_uid != os.getuid()
    ):
        raise ValueError
    receipt = json.loads(
        receipt_path.read_text(encoding="utf-8"),
        parse_constant=reject_json_constant,
        object_pairs_hook=reject_duplicate_json_keys,
    )
    if set(receipt) != {"assets", "commit", "repository", "schemaVersion", "stageTar", "version"}:
        raise ValueError
    stage_name = f"termux-mcp-server-v{version}-release-stage-{commit[:12]}.tar"
    stage_path = assets / stage_name
    if (
        type(receipt["schemaVersion"]) is not int
        or receipt["schemaVersion"] != 1
        or receipt["repository"] != repository
        or receipt["commit"] != commit
        or receipt["version"] != version
        or receipt["stageTar"]
        != {"name": stage_name, "sha256": stage_sha, "size": stage_path.stat().st_size}
    ):
        raise ValueError
    records = receipt["assets"]
    expected_names = [record["name"] for record in records]
    actual_names = sorted(path.name for path in assets.iterdir())
    postures = [
        "default", "mcp-runtime", "android-battery-status", "android-volume-status",
        "android-volume-control", "command-execution", "full-suite",
    ]
    binaries = [
        f"termux-mcp-server-v{version}-aarch64-linux-android-{posture}"
        for posture in postures
    ]
    required_names = sorted(binaries + [f"{name}.sha256" for name in binaries] + ["SHA256SUMS", stage_name])
    if (
        not isinstance(records, list)
        or expected_names != required_names
        or actual_names != required_names
        or len(records) != 16
    ):
        raise ValueError
    for record in records:
        if set(record) != {"name", "sha256", "size", "sourceStageMember"}:
            raise ValueError
        if (
            not isinstance(record["name"], str)
            or re.fullmatch(r"[0-9a-f]{64}", record["sha256"]) is None
            or type(record["size"]) is not int
            or record["size"] < 1
        ):
            raise ValueError
        path = assets / record["name"]
        if not path.is_file() or path.is_symlink():
            raise ValueError
        if path.stat().st_size != record["size"] or digest(path) != record["sha256"]:
            raise ValueError
        expected_source = None if record["name"] == stage_name else record["name"]
        expected_mode = 0o600 if record["name"] == stage_name else (0o755 if record["name"] in binaries else 0o644)
        if (
            record["sourceStageMember"] != expected_source
            or stat.S_IMODE(path.stat().st_mode) != expected_mode
            or path.stat().st_uid != os.getuid()
        ):
            raise ValueError
except (OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError, ValueError):
    print("[release-publication-assets] ERROR: published_asset_receipt_mismatch", file=sys.stderr)
    raise SystemExit(1)
PY

receipt_sha="$(sha256sum -- "$RECEIPT" | awk '{print $1}')" || fail receipt_digest_failed
[[ "$receipt_sha" =~ ^[0-9a-f]{64}$ ]] || fail receipt_digest_failed
rm -rf -- "$WORK_ROOT" >/dev/null 2>&1 || true
WORK_ROOT=""
COMPLETED=1
printf '[release-publication-assets] assets=16 stageSha256=%s receiptSha256=%s\n' \
  "$STAGED_ARTIFACT_SHA256" "$receipt_sha"
