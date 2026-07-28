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

The assets directory and receipt must be named "assets" and
"release-publication-receipt-v1.json" beneath one common absent bundle
directory. That bundle's parent must be an existing caller-owned mode-0700
directory. The stage tar must be an absolute canonical, caller-owned mode-0600
regular file. This command validates the complete stage and copies only the 16
fixed publication assets. It never builds, repackages, renames governed bytes,
calls a network service, creates a tag, or publishes. The complete bundle is
committed with one atomic no-replace directory rename; the bundle is therefore
absent on incomplete execution and contains both the assets and receipt when
committed.
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
BUNDLE_DIR=""
OUTPUT_PARENT=""
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
    && [[ -n "$WORK_ROOT" && -n "$OUTPUT_PARENT" \
      && "$WORK_ROOT" == "$OUTPUT_PARENT"/.release-publication-assets.* ]]; then
    rm -rf -- "$WORK_ROOT" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

for command_name in awk basename chmod cp dirname file id mkdir mktemp python3 realpath rm sha256sum stat; do
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
((stage_bytes > 0 && stage_bytes <= 2147483647)) || fail stage_tar_invalid
[[ "$(stat -c '%a' -- "$STAGE_TAR" 2>/dev/null)" == 600 ]] || fail stage_tar_mode_invalid
[[ "$(stat -c '%u' -- "$STAGE_TAR" 2>/dev/null)" == "$(id -u)" ]] || fail stage_tar_owner_invalid

[[ "$ASSETS_DIR" == /* && "$(realpath -m -- "$ASSETS_DIR" 2>/dev/null)" == "$ASSETS_DIR" ]] \
  || fail assets_dir_path_invalid
[[ "$RECEIPT" == /* && "$(realpath -m -- "$RECEIPT" 2>/dev/null)" == "$RECEIPT" ]] \
  || fail receipt_path_invalid
[[ "$(basename -- "$ASSETS_DIR")" == assets ]] || fail assets_dir_name_invalid
[[ "$ASSETS_DIR" != / && ! -e "$ASSETS_DIR" && ! -L "$ASSETS_DIR" ]] \
  || fail assets_dir_invalid
[[ "$RECEIPT" != / && ! -e "$RECEIPT" && ! -L "$RECEIPT" ]] || fail receipt_invalid

BUNDLE_DIR="$(dirname -- "$ASSETS_DIR")"
receipt_parent="$(dirname -- "$RECEIPT")"
[[ "$BUNDLE_DIR" == "$receipt_parent" ]] || fail output_parent_mismatch
[[ "$BUNDLE_DIR" != / && ! -e "$BUNDLE_DIR" && ! -L "$BUNDLE_DIR" ]] \
  || fail bundle_dir_invalid
OUTPUT_PARENT="$(dirname -- "$BUNDLE_DIR")"
[[ -d "$OUTPUT_PARENT" && ! -L "$OUTPUT_PARENT" ]] || fail output_parent_invalid
[[ "$(realpath -- "$OUTPUT_PARENT" 2>/dev/null)" == "$OUTPUT_PARENT" ]] \
  || fail output_parent_invalid
[[ "$(stat -c '%a' -- "$OUTPUT_PARENT" 2>/dev/null)" == 700 ]] \
  || fail output_parent_mode_invalid
[[ "$(stat -c '%u' -- "$OUTPUT_PARENT" 2>/dev/null)" == "$(id -u)" ]] \
  || fail output_parent_owner_invalid

script_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
schema="$script_root/docs/release-staging-manifest-schema-v2.json"
[[ -f "$schema" && ! -L "$schema" ]] || fail staging_schema_invalid

WORK_ROOT="$(mktemp -d "$OUTPUT_PARENT/.release-publication-assets.XXXXXX")" \
  || fail work_root_create_failed
[[ -d "$WORK_ROOT" && ! -L "$WORK_ROOT" ]] || fail work_root_create_failed
[[ "$(stat -c '%a' -- "$WORK_ROOT" 2>/dev/null)" == 700 ]] \
  || fail work_root_create_failed
SNAPSHOT="$WORK_ROOT/$expected_stage_name"
EXTRACTED="$WORK_ROOT/extracted"
BUNDLE_WORK="$WORK_ROOT/bundle"
ASSET_WORK="$BUNDLE_WORK/assets"
RECEIPT_WORK="$BUNDLE_WORK/release-publication-receipt-v1.json"
mkdir -m 700 -- "$EXTRACTED" "$BUNDLE_WORK" "$ASSET_WORK" \
  || fail work_root_create_failed

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
    "const", "contains", "description", "else", "enum", "format", "if", "items",
    "maxContains", "maxItems", "maxLength", "maximum", "minContains",
    "minItems", "minLength", "minimum", "not", "oneOf", "pattern",
    "prefixItems", "properties", "required", "then", "title", "type",
    "uniqueItems",
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

    one_of = node.get("oneOf", [])
    if not isinstance(one_of, list):
        fail("staging_schema_unsupported")
    if one_of:
        matches = 0
        for child in one_of:
            try:
                schema_check(instance, child, root, location)
                matches += 1
            except SchemaMismatch:
                pass
        if matches != 1:
            raise SchemaMismatch

    if "not" in node:
        try:
            schema_check(instance, node["not"], root, location)
        except SchemaMismatch:
            pass
        else:
            raise SchemaMismatch

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
        if "contains" in node:
            minimum_contains = node.get("minContains", 1)
            maximum_contains = node.get("maxContains")
            if (
                not isinstance(minimum_contains, int)
                or isinstance(minimum_contains, bool)
                or minimum_contains < 0
                or (
                    maximum_contains is not None
                    and (
                        not isinstance(maximum_contains, int)
                        or isinstance(maximum_contains, bool)
                        or maximum_contains < 0
                        or maximum_contains < minimum_contains
                    )
                )
            ):
                fail("staging_schema_unsupported")
            contains_count = 0
            for index, value in enumerate(instance):
                try:
                    schema_check(
                        value,
                        node["contains"],
                        root,
                        f"{location}[{index}]",
                    )
                    contains_count += 1
                except SchemaMismatch:
                    pass
            if contains_count < minimum_contains:
                raise SchemaMismatch
            if maximum_contains is not None and contains_count > maximum_contains:
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
    "evidence/termux-native-aggregate-evidence-v4.json",
    "evidence/automated-native-deployment-v1.json",
    "evidence/termux-observation-requirement-v3.json",
    "evidence/automated-qualification-v1.json",
    "evidence/termux-battery-emulated-evidence.json",
    "evidence/termux-volume-emulated-evidence.json",
    "evidence/termux-volume-control-emulated-evidence.json",
    "evidence/termux-command-emulated-evidence.json",
]
runtime_names = [
    "evidence/runtime/termux-qualified-runtime-image-v1.tar.gz",
    "evidence/runtime/termux-runtime-package-lock-v1.json",
    "evidence/runtime/termux-runtime-snapshot-v1.json",
    "evidence/runtime/termux-runtime-snapshot-replay-v1.json",
]
expected_files = set(
    release_names
    + checksum_names
    + workflow_manifest_names
    + evidence_names
    + runtime_names
    + ["SHA256SUMS", "LICENSE", "release-staging-manifest-v2.json"]
)
expected_member_names = sorted(
    [".", "./evidence", "./evidence/runtime"]
    + [f"./{name}" for name in expected_files]
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
max_sizes[runtime_names[0]] = 1_610_612_736
for name in runtime_names[1:]:
    max_sizes[name] = 16_777_216
max_sizes["SHA256SUMS"] = 2_048
max_sizes["LICENSE"] = 1_048_576
max_sizes["release-staging-manifest-v2.json"] = 1_048_576

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
            elif member.name in ("./evidence", "./evidence/runtime"):
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
        if total_size > 2_147_483_647:
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
    manifest_path = extracted / "release-staging-manifest-v2.json"
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
    or manifest["qualificationClass"] != "official_termux_native_automated_v1"
    or manifest["claimBoundary"] != {
        "physicalDeviceObserved": False,
        "androidFrameworkObserved": False,
        "sustainedPhysicalSoak": False,
        "physicalCertification": "not_run",
    }
):
    fail("staging_manifest_identity_mismatch")
run_ids = manifest["workflowRuns"]
if any(
    re.fullmatch(r"[1-9][0-9]*", run_ids[key]) is None
    for key in ("ci", "security", "android", "qualification")
):
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
validate_record(manifest["evidence"]["deployment"], evidence_names[1], "evidence_record_mismatch")
validate_record(manifest["evidence"]["classifier"], evidence_names[2], "evidence_record_mismatch")
validate_record(manifest["evidence"]["qualification"], evidence_names[3], "evidence_record_mismatch")
for record, expected_name in zip(manifest["evidence"]["specialized"], evidence_names[4:]):
    validate_record(record, expected_name, "evidence_record_mismatch")
runtime_manifest_records = manifest["evidence"]["runtime"]
for key, expected_name in zip(
    ("archive", "packageLock", "snapshot", "replay"),
    runtime_names,
):
    validate_record(
        runtime_manifest_records[key],
        expected_name,
        "runtime_evidence_record_mismatch",
    )

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


def load_governed_runtime(runtime_index, schema_name, mismatch_code):
    runtime_path = extracted / runtime_names[runtime_index]
    runtime_schema_path = schema_path.parent / schema_name
    try:
        value = read_json(runtime_path)
        runtime_schema = read_json(runtime_schema_path)
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError):
        fail(mismatch_code)
    if (
        runtime_schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema"
        or runtime_schema.get("type") != "object"
        or runtime_schema.get("additionalProperties") is not False
    ):
        fail("runtime_evidence_schema_invalid")
    schema_validate(value, runtime_schema, runtime_schema, mismatch_code=mismatch_code)
    return value


def timestamp_order_valid(started, completed):
    try:
        started_at = datetime.datetime.strptime(started, "%Y-%m-%dT%H:%M:%SZ")
        completed_at = datetime.datetime.strptime(completed, "%Y-%m-%dT%H:%M:%SZ")
    except (TypeError, ValueError):
        return False
    return completed_at >= started_at


aggregate = load_governed_evidence(
    0, "emulated-release-evidence-schema-v4.json", "aggregate_evidence_mismatch"
)
claim_boundary = {
    "physicalDeviceObserved": False,
    "androidFrameworkObserved": False,
    "sustainedPhysicalSoak": False,
    "physicalCertification": "not_run",
}
aggregate_candidate = aggregate["candidate"]
aggregate_environment = aggregate["environment"]
if (
    not timestamp_order_valid(aggregate["startedAt"], aggregate["completedAt"])
    or aggregate["schemaVersion"] != 4
    or aggregate["gateVersion"] != "4"
    or aggregate["status"] != "pass"
    or aggregate["failureCode"] is not None
    or aggregate["releaseQualificationEligible"] is not False
    or aggregate["claimBoundary"] != claim_boundary
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
    or aggregate["environment"]["runtimeImageDigest"]
    == aggregate["environment"]["rootfsImageId"]
    or aggregate["environment"]["androidLinker"] is not True
    or aggregate["runtimeValidation"]["status"] != "pass"
    or aggregate["runtimeValidation"]["phases"]["preflight"] != "pass"
    or aggregate["runtimeValidation"]["phases"]["runtime"] != "pass"
    or aggregate["aggregateValidation"]["status"] != "pass"
    or aggregate["aggregateValidation"]["requests"] < 14
    or aggregate["aggregateValidation"]["automatedQualificationComponent"] is not True
    or aggregate["stress"]["status"] != "pass"
    or aggregate["stress"]["servicePidStable"] is not True
    or aggregate["stress"]["healthReadyStable"] is not True
    or aggregate["stress"]["longObservationRequired"] is not False
):
    fail("aggregate_evidence_mismatch")

specialized_environment = aggregate_environment
specialized_specs = [
    (4, "android-battery-emulated-evidence-schema-v3.json", 3, "3", 2, None, None),
    (5, "android-volume-emulated-evidence-schema-v2.json", 2, "2", 3, None, None),
    (
        6,
        "android-volume-control-emulated-evidence-schema-v2.json",
        2,
        "2",
        4,
        "incompatibleArtifact",
        3,
    ),
    (7, "command-emulated-evidence-schema-v3.json", 3, "3", 5, "defaultArtifact", 0),
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
        or specialized["environment"] != specialized_environment
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
    2,
    "release-observation-requirement-schema-v3.json",
    "observation_requirement_mismatch",
)
if (
    observation_requirement["schemaVersion"] != 3
    or observation_requirement["classifierVersion"] != "3"
    or observation_requirement["status"] != "pass"
    or observation_requirement["failureCode"] is not None
    or observation_requirement["releaseQualificationEligible"] is not False
    or observation_requirement["claimBoundary"] != claim_boundary
    or observation_requirement["evidenceMode"] != "automated_release_qualification"
    or observation_requirement["reasonCode"]
    != "automated_native_termux_evidence_required"
    or observation_requirement["inheritanceCandidate"] is not False
    or observation_requirement["nextGate"] != "assemble_automated_release_qualification"
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
    or observation_requirement["emulation"]["imageDigest"]
    != aggregate_environment["imageDigest"]
    or observation_requirement["emulation"]["samples"] != aggregate["stress"]["samples"]
):
    fail("observation_requirement_mismatch")

try:
    deployment_path = extracted / evidence_names[1]
    qualification_path = extracted / evidence_names[3]
    deployment = read_json(deployment_path)
    qualification = read_json(qualification_path)
except (OSError, UnicodeError, json.JSONDecodeError, ValueError):
    fail("qualification_json_invalid")

deployment = load_governed_evidence(
    1,
    "automated-native-deployment-evidence-schema-v1.json",
    "deployment_evidence_mismatch",
)
qualification = load_governed_evidence(
    3,
    "release-automated-qualification-schema-v1.json",
    "automated_qualification_mismatch",
)
scenario_path = schema_path.parent / "automated-native-deployment-scenarios-v1.json"
policy_path = schema_path.parent / "release-qualification-policy-v1.json"
try:
    scenario_set = read_json(scenario_path)
    policy = read_json(policy_path)
except (OSError, UnicodeError, json.JSONDecodeError, ValueError):
    fail("qualification_policy_invalid")

deployment_artifact = {
    "artifactName": workflow_artifacts[6],
    "posture": "full-suite",
    "features": ["full-suite"],
    "sha256": binary_digests[6],
    "manifestSha256": manifest_digests[6],
    "bytes": binary_sizes[6],
    "target": "aarch64-linux-android",
    "elf": "aarch64-android-elf",
}
scenario_record = {
    "fileName": "automated-native-deployment-scenarios-v1.json",
    "schemaVersion": 1,
    "scenarioSetVersion": "1",
    "sha256": sha256_path(scenario_path),
    "scenarioCount": 6,
}

scenario_ids = [
    "isolated_fresh_deploy",
    "failed_upgrade_recovery",
    "supervised_restart",
    "rollback_recovery",
    "uninstall",
    "bounded_cleanup",
]
scenario_record["scenarioIds"] = scenario_ids
deployment_environment = deployment["environment"]
deployment_linker = deployment_environment["androidLinker"]
if (
    not timestamp_order_valid(deployment["startedAt"], deployment["completedAt"])
    or deployment["schemaVersion"] != 1
    or deployment["gateVersion"] != "1"
    or deployment["status"] != "pass"
    or deployment["failureCode"] is not None
    or deployment["releaseQualificationEligible"] is not False
    or deployment["qualificationClass"] != "official_termux_native_automated_v1"
    or deployment["candidate"] != {
        "repository": repository,
        "commit": commit,
        "version": version,
        "ciRunId": run_ids["ci"],
        "securityRunId": run_ids["security"],
        "nativeRunId": run_ids["android"],
        "artifact": deployment_artifact,
    }
    or deployment["scenarioSet"] != scenario_record
    or deployment_environment["executionMode"] != aggregate_environment["executionMode"]
    or deployment_environment["architecture"] != "aarch64"
    or deployment_environment["rootfsImage"] != aggregate_environment["image"]
    or deployment_environment["rootfsDigest"] != aggregate_environment["imageDigest"]
    or deployment_environment["rootfsImageId"]
    != aggregate_environment["rootfsImageId"]
    or deployment_environment["runtimeImageDigest"]
    != aggregate_environment["runtimeImageDigest"]
    or deployment_environment["runtimeImageDigest"]
    == deployment_environment["rootfsImageId"]
    or deployment_environment["termuxPrefix"] != "/data/data/com.termux/files/usr"
    or deployment_environment["androidLinker"]["observed"] is not True
    or deployment_environment["androidLinker"]["path"] != "/system/bin/linker64"
    or deployment_environment["androidFrameworkObserved"] is not False
    or deployment_environment["physicalHardwareObserved"] is not False
    or deployment_environment["physicalDeviceObserved"] is not False
    or deployment_environment["sustainedPhysicalSoak"] is not False
    or deployment["validation"]["status"] != "pass"
    or deployment["validation"]["physicalCertification"] != "not_run"
):
    fail("deployment_evidence_mismatch")

runtime_package_lock = load_governed_runtime(
    1,
    "runtime-package-lock-schema-v1.json",
    "runtime_package_lock_mismatch",
)
runtime_snapshot = load_governed_runtime(
    2,
    "runtime-snapshot-schema-v1.json",
    "runtime_snapshot_mismatch",
)
runtime_replay = load_governed_runtime(
    3,
    "runtime-snapshot-replay-schema-v1.json",
    "runtime_replay_mismatch",
)


def runtime_file_record(index):
    path = extracted / runtime_names[index]
    return {
        "fileName": pathlib.PurePosixPath(runtime_names[index]).name,
        "sha256": sha256_path(path),
        "bytes": path.stat().st_size,
    }


runtime_archive_record = runtime_file_record(0)
runtime_package_lock_record = runtime_file_record(1)
runtime_snapshot_record = runtime_file_record(2)
runtime_replay_record = runtime_file_record(3)
runtime_archive_identity = {
    **runtime_archive_record,
    "format": "docker-image-archive-v1",
    "compression": "gzip-no-name",
}
expected_runtime_base = {
    "image": aggregate_environment["image"],
    "digest": aggregate_environment["imageDigest"],
    "imageId": aggregate_environment["rootfsImageId"],
}
expected_runtime_verification = {
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
expected_runtime_linker = {
    "path": deployment_linker["path"],
    "sha256": deployment_linker["sha256"],
    "bytes": deployment_linker["bytes"],
}
if (
    runtime_package_lock["repository"] != repository
    or runtime_package_lock["commit"] != commit
    or runtime_package_lock["androidRunId"] != run_ids["android"]
    or runtime_package_lock["base"] != expected_runtime_base
    or runtime_package_lock["requestedPackages"]
    != ["file", "jq", "python", "termux-services"]
    or runtime_package_lock["resolution"]
    != {
        "resolver": "termux-apt-download-only",
        "repositoryMetadataAuthenticated": True,
        "packageBytesFrozenBeforeBuild": True,
        "finalImageBuildNetwork": "none",
    }
):
    fail("runtime_package_lock_mismatch")
if (
    runtime_snapshot["repository"] != repository
    or runtime_snapshot["commit"] != commit
    or runtime_snapshot["androidRunId"] != run_ids["android"]
    or runtime_snapshot["base"] != expected_runtime_base
    or runtime_snapshot["runtimeImageId"]
    != aggregate_environment["runtimeImageDigest"]
    or runtime_snapshot["platform"] != {"os": "linux", "architecture": "arm64"}
    or runtime_snapshot["packageLock"] != runtime_package_lock_record
    or runtime_snapshot["archive"] != runtime_archive_identity
    or runtime_snapshot["claimBoundary"] != claim_boundary
    or runtime_snapshot["rebuildReproducibilityClaim"] is not False
):
    fail("runtime_snapshot_mismatch")
if (
    runtime_replay["repository"] != repository
    or runtime_replay["commit"] != commit
    or runtime_replay["runtimeImageId"] != runtime_snapshot["runtimeImageId"]
    or runtime_replay["snapshot"]
    != {
        "manifest": runtime_snapshot_record,
        "archive": runtime_archive_identity,
    }
    or runtime_replay["packageLock"] != runtime_package_lock_record
    or runtime_replay["installedPackages"]
    != {
        "sha256": runtime_snapshot["installedPackages"]["sha256"],
        "count": runtime_snapshot["installedPackages"]["count"],
    }
    or runtime_replay["androidLinker"] != expected_runtime_linker
    or runtime_replay["verification"] != expected_runtime_verification
    or runtime_replay["claimBoundary"] != claim_boundary
    or runtime_replay["rebuildReproducibilityClaim"] is not False
):
    fail("runtime_replay_mismatch")

expected_artifacts = [
    {
        "posture": postures[index],
        "features": features[index],
        "workflowArtifactName": workflow_artifacts[index],
        "sha256": binary_digests[index],
        "bytes": binary_sizes[index],
        "manifestSha256": manifest_digests[index],
    }
    for index in range(7)
]
expected_evidence = {
    "aggregate": {
        "fileName": pathlib.PurePosixPath(evidence_names[0]).name,
        "sha256": sha256_path(extracted / evidence_names[0]),
        "bytes": (extracted / evidence_names[0]).stat().st_size,
    },
    "deployment": {
        "fileName": pathlib.PurePosixPath(evidence_names[1]).name,
        "sha256": sha256_path(extracted / evidence_names[1]),
        "bytes": (extracted / evidence_names[1]).stat().st_size,
    },
    "classifier": {
        "fileName": pathlib.PurePosixPath(evidence_names[2]).name,
        "sha256": sha256_path(extracted / evidence_names[2]),
        "bytes": (extracted / evidence_names[2]).stat().st_size,
    },
    "specialized": [
        {
            "fileName": pathlib.PurePosixPath(name).name,
            "sha256": sha256_path(extracted / name),
            "bytes": (extracted / name).stat().st_size,
        }
        for name in evidence_names[4:]
    ],
}
expected_workflow_runs = {
    key: {
        "runId": run_ids[key],
        "attempt": 1,
        "event": "push",
        "ref": "refs/heads/main",
        "headCommit": commit,
        "conclusion": "success",
    }
    for key in ("ci", "security", "android")
}
expected_qualification_run = {
    "runId": run_ids["qualification"],
    "attempt": 1,
    "event": "workflow_run",
    "sourceWorkflow": "Android Cross Compile",
    "sourceRunId": run_ids["android"],
}
expected_qualification_environment = {
    "executionMode": deployment_environment["executionMode"],
    "architecture": deployment_environment["architecture"],
    "runtimeImageDigest": deployment_environment["runtimeImageDigest"],
    "rootfsUserland": {
        "image": deployment_environment["rootfsImage"],
        "digest": deployment_environment["rootfsDigest"],
        "imageId": deployment_environment["rootfsImageId"],
        "prefix": deployment_environment["termuxPrefix"],
    },
    "androidRuntime": {
        "abi": "android-bionic",
        "linkerPath": deployment_linker["path"],
        "linkerSha256": deployment_linker["sha256"],
        "linkerBytes": deployment_linker["bytes"],
        "linkerIdentity": "aarch64-android-bionic-elf",
    },
}
expected_retained_runtime = {
    "runtimeImageId": runtime_snapshot["runtimeImageId"],
    "base": expected_runtime_base,
    "archive": runtime_archive_record,
    "packageLock": runtime_package_lock_record,
    "snapshot": runtime_snapshot_record,
    "replay": runtime_replay_record,
    "installedPackages": runtime_replay["installedPackages"],
    "androidLinker": expected_runtime_linker,
    "verification": expected_runtime_verification,
    "claimBoundary": claim_boundary,
    "rebuildReproducibilityClaim": False,
}
if (
    qualification["schemaVersion"] != 1
    or qualification["envelopeVersion"] != "1"
    or qualification["status"] != "pass"
    or qualification["failureCode"] is not None
    or qualification["releaseEligible"] is not True
    or qualification["qualificationClass"] != "official_termux_native_automated_v1"
    or qualification["repository"] != repository
    or qualification["commit"] != commit
    or qualification["version"] != version
    or qualification["workflowRuns"] != expected_workflow_runs
    or qualification["qualificationRun"] != expected_qualification_run
    or qualification["claimBoundary"] != claim_boundary
    or qualification["environment"] != expected_qualification_environment
    or qualification["retainedRuntime"] != expected_retained_runtime
    or qualification["artifacts"] != expected_artifacts
    or qualification["evidence"] != expected_evidence
    or qualification["policy"] != {
        "fileName": "release-qualification-policy-v1.json",
        "sha256": sha256_path(policy_path),
    }
    or qualification["scenarioSet"] != scenario_record
    or qualification["gates"] != {
        "firstAttemptMainWorkflows": "pass",
        "artifactLineage": "pass",
        "officialTermuxNativeRuntime": "pass",
        "aggregateComposition": "pass",
        "specializedProviderBoundaries": "pass",
        "isolatedDeploymentRecovery": "pass",
        "automatedReleaseClassification": "pass",
    }
    or manifest["qualificationClass"] != qualification["qualificationClass"]
    or manifest["claimBoundary"] != qualification["claimBoundary"]
):
    fail("automated_qualification_mismatch")

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

commit_publication_bundle() {
  python3 - \
    "$1" "$2" "$REPOSITORY" "$COMMIT" "$VERSION" "$STAGED_ARTIFACT_SHA256" <<'PY'
import ctypes
import errno
import hashlib
import json
import os
import pathlib
import re
import stat
import sys

bundle_source = pathlib.Path(sys.argv[1])
bundle_destination = pathlib.Path(sys.argv[2])
repository, commit, version, stage_sha = sys.argv[3:]
source_parent_fd = -1
destination_parent_fd = -1
bundle_fd = -1
assets_fd = -1
receipt_fd = -1
asset_fds = []


def fail(code):
    print(f"[release-publication-assets] ERROR: {code}", file=sys.stderr)
    raise SystemExit(1)


def stable_identity(value):
    return (
        value.st_dev,
        value.st_ino,
        value.st_mode,
        value.st_uid,
        value.st_size,
        value.st_mtime_ns,
        value.st_ctime_ns,
    )


def read_stable(fd, maximum):
    before = os.fstat(fd)
    if not stat.S_ISREG(before.st_mode) or before.st_size < 1 or before.st_size > maximum:
        raise ValueError
    os.lseek(fd, 0, os.SEEK_SET)
    content = bytearray()
    while len(content) <= maximum:
        chunk = os.read(fd, min(1024 * 1024, maximum + 1 - len(content)))
        if not chunk:
            break
        content.extend(chunk)
    after = os.fstat(fd)
    if (
        stable_identity(before) != stable_identity(after)
        or len(content) != after.st_size
        or len(content) > maximum
    ):
        raise ValueError
    return bytes(content), after


def digest_fd(fd, maximum):
    before = os.fstat(fd)
    if not stat.S_ISREG(before.st_mode) or before.st_size < 1 or before.st_size > maximum:
        raise ValueError
    value = hashlib.sha256()
    os.lseek(fd, 0, os.SEEK_SET)
    total = 0
    while True:
        chunk = os.read(fd, 1024 * 1024)
        if not chunk:
            break
        total += len(chunk)
        if total > maximum:
            raise ValueError
        value.update(chunk)
    after = os.fstat(fd)
    if stable_identity(before) != stable_identity(after) or total != after.st_size:
        raise ValueError
    return value.hexdigest(), after


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
        not bundle_source.is_absolute()
        or not bundle_destination.is_absolute()
        or bundle_source.name != "bundle"
        or not bundle_destination.name
    ):
        raise ValueError
    source_parent_fd = os.open(
        bundle_source.parent,
        os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW,
    )
    destination_parent_fd = os.open(
        bundle_destination.parent,
        os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW,
    )
    source_parent_stat = os.fstat(source_parent_fd)
    destination_parent_stat = os.fstat(destination_parent_fd)
    for parent_stat in (source_parent_stat, destination_parent_stat):
        if (
            not stat.S_ISDIR(parent_stat.st_mode)
            or stat.S_IMODE(parent_stat.st_mode) != 0o700
            or parent_stat.st_uid != os.getuid()
        ):
            raise ValueError
    bundle_fd = os.open(
        bundle_source.name,
        os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW,
        dir_fd=source_parent_fd,
    )
    bundle_stat = os.fstat(bundle_fd)
    if (
        not stat.S_ISDIR(bundle_stat.st_mode)
        or stat.S_IMODE(bundle_stat.st_mode) != 0o700
        or bundle_stat.st_uid != os.getuid()
        or sorted(os.listdir(bundle_fd)) != ["assets", "release-publication-receipt-v1.json"]
    ):
        raise ValueError
    assets_fd = os.open(
        "assets",
        os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW,
        dir_fd=bundle_fd,
    )
    assets_stat = os.fstat(assets_fd)
    if (
        not stat.S_ISDIR(assets_stat.st_mode)
        or stat.S_IMODE(assets_stat.st_mode) != 0o700
        or assets_stat.st_uid != os.getuid()
    ):
        raise ValueError
    receipt_fd = os.open(
        "release-publication-receipt-v1.json",
        os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
        dir_fd=bundle_fd,
    )
    receipt_bytes, receipt_stat = read_stable(receipt_fd, 1048576)
    if (
        not stat.S_ISREG(receipt_stat.st_mode)
        or stat.S_IMODE(receipt_stat.st_mode) != 0o600
        or receipt_stat.st_uid != os.getuid()
    ):
        raise ValueError
    receipt_sha = hashlib.sha256(receipt_bytes).hexdigest()
    receipt = json.loads(
        receipt_bytes.decode("utf-8"),
        parse_constant=reject_json_constant,
        object_pairs_hook=reject_duplicate_json_keys,
    )
    if set(receipt) != {"assets", "commit", "repository", "schemaVersion", "stageTar", "version"}:
        raise ValueError
    stage_name = f"termux-mcp-server-v{version}-release-stage-{commit[:12]}.tar"
    if (
        type(receipt["schemaVersion"]) is not int
        or receipt["schemaVersion"] != 1
        or receipt["repository"] != repository
        or receipt["commit"] != commit
        or receipt["version"] != version
    ):
        raise ValueError
    records = receipt["assets"]
    expected_names = [record["name"] for record in records]
    actual_names = sorted(os.listdir(assets_fd))
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
    validated_asset_stats = {}
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
        expected_source = None if record["name"] == stage_name else record["name"]
        expected_mode = 0o600 if record["name"] == stage_name else (0o755 if record["name"] in binaries else 0o644)
        asset_fd = os.open(
            record["name"],
            os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
            dir_fd=assets_fd,
        )
        asset_fds.append(asset_fd)
        maximum = 2147483647 if record["name"] == stage_name else 67108864
        asset_digest, asset_stat = digest_fd(asset_fd, maximum)
        if (
            not stat.S_ISREG(asset_stat.st_mode)
            or asset_digest != record["sha256"]
            or asset_stat.st_size != record["size"]
            or record["sourceStageMember"] != expected_source
            or stat.S_IMODE(asset_stat.st_mode) != expected_mode
            or asset_stat.st_uid != os.getuid()
        ):
            raise ValueError
        validated_asset_stats[record["name"]] = asset_stat
    stage_record = next(record for record in records if record["name"] == stage_name)
    if (
        receipt["stageTar"]
        != {
            "name": stage_name,
            "sha256": stage_sha,
            "size": stage_record["size"],
        }
    ):
        raise ValueError

    # Bind every validated descriptor back to the exact private directory
    # entries immediately before the one public namespace mutation.
    if stable_identity(os.fstat(bundle_fd)) != stable_identity(bundle_stat):
        raise ValueError
    if stable_identity(os.fstat(assets_fd)) != stable_identity(assets_stat):
        raise ValueError
    if stable_identity(
        os.stat("assets", dir_fd=bundle_fd, follow_symlinks=False)
    ) != stable_identity(assets_stat):
        raise ValueError
    if stable_identity(
        os.stat(
            "release-publication-receipt-v1.json",
            dir_fd=bundle_fd,
            follow_symlinks=False,
        )
    ) != stable_identity(receipt_stat):
        raise ValueError
    for name, asset_fd, asset_stat in zip(actual_names, asset_fds, (
        validated_asset_stats[name] for name in actual_names
    )):
        if (
            stable_identity(os.fstat(asset_fd)) != stable_identity(asset_stat)
            or stable_identity(
                os.stat(name, dir_fd=assets_fd, follow_symlinks=False)
            )
            != stable_identity(asset_stat)
        ):
            raise ValueError

    libc = ctypes.CDLL(None, use_errno=True)
    renameat2 = getattr(libc, "renameat2", None)
    if renameat2 is None:
        fail("bundle_atomic_rename_unavailable")
    renameat2.argtypes = [
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_uint,
    ]
    renameat2.restype = ctypes.c_int
    if renameat2(
        source_parent_fd,
        os.fsencode(bundle_source.name),
        destination_parent_fd,
        os.fsencode(bundle_destination.name),
        1,
    ) != 0:
        error_number = ctypes.get_errno()
        if error_number in (errno.EEXIST, errno.ENOTEMPTY):
            fail("bundle_publication_conflict")
        fail("bundle_publication_failed")
except SystemExit:
    raise
except (OSError, UnicodeError, json.JSONDecodeError, KeyError, StopIteration, TypeError, ValueError):
    fail("private_bundle_validation_failed")
finally:
    for asset_fd in asset_fds:
        os.close(asset_fd)
    if receipt_fd >= 0:
        os.close(receipt_fd)
    if assets_fd >= 0:
        os.close(assets_fd)
    if bundle_fd >= 0:
        os.close(bundle_fd)
    if destination_parent_fd >= 0:
        os.close(destination_parent_fd)
    if source_parent_fd >= 0:
        os.close(source_parent_fd)
print(receipt_sha)
PY
}

receipt_sha="$(
  commit_publication_bundle "$BUNDLE_WORK" "$BUNDLE_DIR"
)" || exit $?
[[ "$receipt_sha" =~ ^[0-9a-f]{64}$ ]] || fail receipt_digest_failed
COMPLETED=1
rm -rf -- "$WORK_ROOT" >/dev/null 2>&1 || true
WORK_ROOT=""
printf '[release-publication-assets] assets=16 stageSha256=%s receiptSha256=%s\n' \
  "$STAGED_ARTIFACT_SHA256" "$receipt_sha"
