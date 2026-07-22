#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077

usage() {
  cat <<'EOF'
Usage: package_android_artifact.sh \
  --binary FILE --output-dir DIR --repository OWNER/REPO --commit SHA \
  --workflow-run-id ID --artifact-name NAME \
  --posture default|mcp-runtime|android-battery-status|android-volume-status|android-volume-control|command-execution|full-suite \
  --version VERSION
EOF
}

BINARY=""
OUTPUT_DIR=""
REPOSITORY=""
COMMIT=""
WORKFLOW_RUN_ID=""
ARTIFACT_NAME=""
POSTURE=""
VERSION=""
STAGING_DIR=""
COMPLETED=0

while (($# > 0)); do
  case "$1" in
    --binary) (($# >= 2)) || { usage >&2; exit 2; }; BINARY="$2"; shift 2 ;;
    --output-dir) (($# >= 2)) || { usage >&2; exit 2; }; OUTPUT_DIR="$2"; shift 2 ;;
    --repository) (($# >= 2)) || { usage >&2; exit 2; }; REPOSITORY="$2"; shift 2 ;;
    --commit) (($# >= 2)) || { usage >&2; exit 2; }; COMMIT="$2"; shift 2 ;;
    --workflow-run-id) (($# >= 2)) || { usage >&2; exit 2; }; WORKFLOW_RUN_ID="$2"; shift 2 ;;
    --artifact-name) (($# >= 2)) || { usage >&2; exit 2; }; ARTIFACT_NAME="$2"; shift 2 ;;
    --posture) (($# >= 2)) || { usage >&2; exit 2; }; POSTURE="$2"; shift 2 ;;
    --version) (($# >= 2)) || { usage >&2; exit 2; }; VERSION="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

fail() {
  printf '[artifact-package] ERROR: %s\n' "$1" >&2
  exit 1
}

cleanup() {
  if ((COMPLETED == 0)) && [[ -n "$STAGING_DIR" && "$STAGING_DIR" == "$OUTPUT_DIR.staging.$$" ]]; then
    rm -rf -- "$STAGING_DIR" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

for command_name in awk chmod date dirname file install jq mkdir mv rm sha256sum stat; do
  command -v "$command_name" >/dev/null 2>&1 || fail required_command_missing
done

[[ -f "$BINARY" && ! -L "$BINARY" && -x "$BINARY" ]] || fail binary_invalid
[[ "$REPOSITORY" == CyberBASSLord-666/termux-mcp-edge ]] || fail repository_invalid
[[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail commit_invalid
[[ "$WORKFLOW_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail workflow_run_id_invalid
[[ "$VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail version_invalid
[[ -n "$OUTPUT_DIR" && "$OUTPUT_DIR" != / && ! -e "$OUTPUT_DIR" && ! -L "$OUTPUT_DIR" ]] || fail output_directory_invalid
output_parent="$(dirname "$OUTPUT_DIR")"
[[ -d "$output_parent" && ! -L "$output_parent" ]] || fail output_parent_invalid

case "$POSTURE" in
  default)
    expected_artifact_name=termux-mcp-server-aarch64-linux-android-default
    features='[]'
    ;;
  mcp-runtime)
    expected_artifact_name=termux-mcp-server-aarch64-linux-android-mcp-runtime
    features='["mcp-runtime"]'
    ;;
  android-battery-status)
    expected_artifact_name=termux-mcp-server-aarch64-linux-android-android-battery-status
    features='["android-battery-status"]'
    ;;
  android-volume-status)
    expected_artifact_name=termux-mcp-server-aarch64-linux-android-android-volume-status
    features='["android-volume-status"]'
    ;;
  android-volume-control)
    expected_artifact_name=termux-mcp-server-aarch64-linux-android-android-volume-control
    features='["android-volume-control"]'
    ;;
  command-execution)
    expected_artifact_name=termux-mcp-server-aarch64-linux-android-command-execution
    features='["command-execution"]'
    ;;
  full-suite)
    expected_artifact_name=termux-mcp-server-aarch64-linux-android-full-suite
    features='["full-suite"]'
    ;;
  *) fail posture_invalid ;;
esac
[[ "$ARTIFACT_NAME" == "$expected_artifact_name" ]] || fail artifact_name_posture_mismatch

source_bytes="$(stat -c '%s' "$BINARY" 2>/dev/null)" || fail binary_stat_failed
[[ "$source_bytes" =~ ^[0-9]+$ ]] || fail binary_stat_failed
((source_bytes > 0 && source_bytes <= 67108864)) || fail binary_size_invalid

STAGING_DIR="$OUTPUT_DIR.staging.$$"
[[ ! -e "$STAGING_DIR" && ! -L "$STAGING_DIR" ]] || fail staging_directory_exists
mkdir -m 700 -- "$STAGING_DIR" 2>/dev/null || fail staging_directory_create_failed
install -m 700 "$BINARY" "$STAGING_DIR/termux-mcp-server" 2>/dev/null || fail binary_copy_failed
bytes="$(stat -c '%s' "$STAGING_DIR/termux-mcp-server" 2>/dev/null)" || fail binary_stat_failed
[[ "$bytes" == "$source_bytes" ]] || fail binary_copy_size_mismatch
identity="$(file -b -- "$STAGING_DIR/termux-mcp-server" 2>/dev/null)" || fail binary_identity_failed
[[ "$identity" == *ELF* && "$identity" == *"ARM aarch64"* ]] || fail binary_architecture_mismatch
[[ "$identity" == *Android* || "$identity" == *"/system/bin/linker64"* ]] || fail binary_android_identity_missing
digest="$(sha256sum -- "$STAGING_DIR/termux-mcp-server" 2>/dev/null | awk '{print $1}')" || fail binary_digest_failed
[[ "$digest" =~ ^[0-9a-f]{64}$ ]] || fail binary_digest_failed

printf '%s  %s\n' "$digest" termux-mcp-server >"$STAGING_DIR/SHA256SUMS" 2>/dev/null || fail checksum_write_failed
jq -n \
  --arg repository "$REPOSITORY" \
  --arg commit "$COMMIT" \
  --arg workflow_run_id "$WORKFLOW_RUN_ID" \
  --arg artifact_name "$ARTIFACT_NAME" \
  --arg posture "$POSTURE" \
  --arg version "$VERSION" \
  --arg sha256 "$digest" \
  --argjson bytes "$bytes" \
  --argjson features "$features" \
  --arg created_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{
    schemaVersion: 1,
    repository: $repository,
    commit: $commit,
    workflowRunId: $workflow_run_id,
    artifactName: $artifact_name,
    posture: $posture,
    features: $features,
    target: "aarch64-linux-android",
    fileName: "termux-mcp-server",
    version: $version,
    sha256: $sha256,
    bytes: $bytes,
    elf: "aarch64-android-elf",
    createdAt: $created_at
  }' >"$STAGING_DIR/artifact-manifest.json" 2>/dev/null || fail manifest_write_failed
chmod 600 "$STAGING_DIR/SHA256SUMS" "$STAGING_DIR/artifact-manifest.json" 2>/dev/null || fail metadata_mode_failed
(cd "$STAGING_DIR" && sha256sum -c SHA256SUMS >/dev/null 2>&1) || fail checksum_verification_failed
jq -e --arg commit "$COMMIT" --arg sha "$digest" \
  '.schemaVersion == 1 and .commit == $commit and .sha256 == $sha' \
  "$STAGING_DIR/artifact-manifest.json" >/dev/null 2>&1 || fail manifest_verification_failed
mv -T -- "$STAGING_DIR" "$OUTPUT_DIR" 2>/dev/null || fail bundle_publication_failed
STAGING_DIR=""
COMPLETED=1
printf '[artifact-package] result=PASS\n'
