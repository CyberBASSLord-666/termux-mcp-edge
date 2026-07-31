#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077
set +x

usage() {
  cat <<'EOF'
Usage: package_shizuku_rish_test_artifact.sh \
  --binary FILE --cargo-lock FILE --output-dir DIR \
  --repository CyberBASSLord-666/termux-mcp-edge \
  --commit SHA --workflow-run-id ID --version VERSION

Creates one closed, development-only Android rish artifact bundle. The output is
never release eligible and never qualifies production device control.
EOF
}

BINARY=""
CARGO_LOCK=""
OUTPUT_DIR=""
REPOSITORY=""
COMMIT=""
WORKFLOW_RUN_ID=""
VERSION=""
STAGING_DIR=""
COMPLETED=0

fail() {
  printf '[android-rish-development-artifact] ERROR: %s\n' "$1" >&2
  exit 1
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP
  if ((COMPLETED == 0)) && [[ -n "$STAGING_DIR" ]]; then
    rm -rf -- "$STAGING_DIR" >/dev/null 2>&1 || true
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

while (($# > 0)); do
  case "$1" in
    --binary)
      (($# >= 2)) || fail missing_binary
      BINARY="$2"
      shift 2
      ;;
    --cargo-lock)
      (($# >= 2)) || fail missing_cargo_lock
      CARGO_LOCK="$2"
      shift 2
      ;;
    --output-dir)
      (($# >= 2)) || fail missing_output_directory
      OUTPUT_DIR="$2"
      shift 2
      ;;
    --repository)
      (($# >= 2)) || fail missing_repository
      REPOSITORY="$2"
      shift 2
      ;;
    --commit)
      (($# >= 2)) || fail missing_commit
      COMMIT="$2"
      shift 2
      ;;
    --workflow-run-id)
      (($# >= 2)) || fail missing_workflow_run_id
      WORKFLOW_RUN_ID="$2"
      shift 2
      ;;
    --version)
      (($# >= 2)) || fail missing_version
      VERSION="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail unknown_argument
      ;;
  esac
done

for command_name in awk chmod date dirname file install jq mkdir mktemp mv realpath rm sha256sum stat; do
  command -v "$command_name" >/dev/null 2>&1 || fail required_command_missing
done

[[ -f "$BINARY" && ! -L "$BINARY" && -x "$BINARY" ]] || fail binary_invalid
[[ -f "$CARGO_LOCK" && ! -L "$CARGO_LOCK" ]] || fail cargo_lock_invalid
[[ "$(stat -c '%s' -- "$CARGO_LOCK" 2>/dev/null)" =~ ^[1-9][0-9]*$ ]] \
  || fail cargo_lock_invalid
(( $(stat -c '%s' -- "$CARGO_LOCK") <= 4194304 )) || fail cargo_lock_invalid
[[ "$REPOSITORY" == CyberBASSLord-666/termux-mcp-edge ]] || fail repository_invalid
[[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail commit_invalid
[[ "$WORKFLOW_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail workflow_run_id_invalid
[[ "$VERSION" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || fail version_invalid
[[ -n "$OUTPUT_DIR" && "$OUTPUT_DIR" == /* && "$OUTPUT_DIR" != / ]] \
  || fail output_directory_invalid
[[ ! -e "$OUTPUT_DIR" && ! -L "$OUTPUT_DIR" ]] || fail output_directory_invalid
OUTPUT_PARENT="$(dirname -- "$OUTPUT_DIR")"
[[ "$OUTPUT_PARENT" == /* && -d "$OUTPUT_PARENT" && ! -L "$OUTPUT_PARENT" ]] \
  || fail output_parent_invalid
[[ "$(realpath -e -- "$OUTPUT_PARENT" 2>/dev/null)" == "$OUTPUT_PARENT" ]] \
  || fail output_parent_invalid

SOURCE_BYTES="$(stat -c '%s' -- "$BINARY" 2>/dev/null)" || fail binary_stat_failed
[[ "$SOURCE_BYTES" =~ ^[1-9][0-9]*$ ]] || fail binary_stat_failed
((SOURCE_BYTES <= 67108864)) || fail binary_size_invalid
CARGO_LOCK_SHA256="$(sha256sum -- "$CARGO_LOCK" 2>/dev/null | awk '{print $1}')" \
  || fail cargo_lock_digest_failed
[[ "$CARGO_LOCK_SHA256" =~ ^[0-9a-f]{64}$ ]] || fail cargo_lock_digest_failed

STAGING_DIR="$(mktemp -d "$OUTPUT_PARENT/.android-rish-development.XXXXXX" 2>/dev/null)" \
  || fail staging_directory_create_failed
[[ -d "$STAGING_DIR" && ! -L "$STAGING_DIR" ]] || fail staging_directory_create_failed
chmod 700 "$STAGING_DIR" || fail staging_directory_mode_failed
install -m 700 -- "$BINARY" "$STAGING_DIR/termux-mcp-server" \
  || fail binary_copy_failed

BYTES="$(stat -c '%s' -- "$STAGING_DIR/termux-mcp-server" 2>/dev/null)" \
  || fail binary_stat_failed
[[ "$BYTES" == "$SOURCE_BYTES" ]] || fail binary_copy_size_mismatch
IDENTITY="$(file -b -- "$STAGING_DIR/termux-mcp-server" 2>/dev/null)" \
  || fail binary_identity_failed
[[ "$IDENTITY" == *ELF* && "$IDENTITY" == *"ARM aarch64"* ]] \
  || fail binary_architecture_mismatch
[[ "$IDENTITY" == *Android* || "$IDENTITY" == *"/system/bin/linker64"* ]] \
  || fail binary_android_identity_missing
SHA256="$(sha256sum -- "$STAGING_DIR/termux-mcp-server" 2>/dev/null | awk '{print $1}')" \
  || fail binary_digest_failed
[[ "$SHA256" =~ ^[0-9a-f]{64}$ ]] || fail binary_digest_failed

printf '%s  %s\n' "$SHA256" termux-mcp-server >"$STAGING_DIR/SHA256SUMS" \
  || fail checksum_write_failed
jq -n \
  --arg repository "$REPOSITORY" \
  --arg commit "$COMMIT" \
  --arg workflow_run_id "$WORKFLOW_RUN_ID" \
  --arg version "$VERSION" \
  --arg sha256 "$SHA256" \
  --arg cargo_lock_sha256 "$CARGO_LOCK_SHA256" \
  --arg created_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --argjson bytes "$BYTES" '
  {
    schemaVersion: 1,
    artifactClass: "android_rish_development_only_v1",
    releaseEligible: false,
    productionControlQualified: false,
    repository: $repository,
    commit: $commit,
    workflowRunId: $workflow_run_id,
    artifactName: "termux-mcp-server-aarch64-linux-android-android-rish-development",
    posture: "android-rish-development",
    features: ["android-rish"],
    target: "aarch64-linux-android",
    fileName: "termux-mcp-server",
    version: $version,
    sha256: $sha256,
    bytes: $bytes,
    elf: "aarch64-android-elf",
    cargoLockSha256: $cargo_lock_sha256,
    createdAt: $created_at
  }' >"$STAGING_DIR/artifact-manifest.json" || fail manifest_write_failed
chmod 600 "$STAGING_DIR/SHA256SUMS" "$STAGING_DIR/artifact-manifest.json" \
  || fail metadata_mode_failed

(cd "$STAGING_DIR" && sha256sum --check --strict SHA256SUMS >/dev/null) \
  || fail checksum_verification_failed
jq -e \
  --arg commit "$COMMIT" \
  --arg sha256 "$SHA256" \
  --arg cargo_lock_sha256 "$CARGO_LOCK_SHA256" '
  (keys == [
    "artifactClass",
    "artifactName",
    "bytes",
    "cargoLockSha256",
    "commit",
    "createdAt",
    "elf",
    "features",
    "fileName",
    "posture",
    "productionControlQualified",
    "releaseEligible",
    "repository",
    "schemaVersion",
    "sha256",
    "target",
    "version",
    "workflowRunId"
  ])
  and .schemaVersion == 1
  and .artifactClass == "android_rish_development_only_v1"
  and .releaseEligible == false
  and .productionControlQualified == false
  and .repository == "CyberBASSLord-666/termux-mcp-edge"
  and .commit == $commit
  and .artifactName == "termux-mcp-server-aarch64-linux-android-android-rish-development"
  and .posture == "android-rish-development"
  and .features == ["android-rish"]
  and .target == "aarch64-linux-android"
  and .fileName == "termux-mcp-server"
  and .sha256 == $sha256
  and .cargoLockSha256 == $cargo_lock_sha256
  and .elf == "aarch64-android-elf"
  ' "$STAGING_DIR/artifact-manifest.json" >/dev/null \
  || fail manifest_verification_failed

[[ "$(find "$STAGING_DIR" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 3 ]] \
  || fail bundle_file_contract_invalid
[[ -z "$(find "$STAGING_DIR" -mindepth 1 -maxdepth 1 ! -type f -print -quit)" ]] \
  || fail bundle_file_contract_invalid
mv -Tn -- "$STAGING_DIR" "$OUTPUT_DIR" || fail bundle_publication_failed
STAGING_DIR=""
COMPLETED=1
printf '[android-rish-development-artifact] result=PASS\n'
