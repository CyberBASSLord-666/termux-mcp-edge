#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077

usage() {
  cat <<'EOF'
Usage:
  publish_release_assets.sh resolve-stage \
    --repository OWNER/REPO --commit SHA --version VERSION \
    --staged-artifact-id ID --staged-artifact-sha256 SHA256

  publish_release_assets.sh preflight|attach|verify|publish|postverify \
    --repository OWNER/REPO --commit SHA --version VERSION \
    --tag-object-sha SHA --staged-artifact-id ID \
    --staged-artifact-sha256 SHA256 --draft-release-id ID \
    --assets-dir ABSOLUTE_DIR --receipt ABSOLUTE_FILE \
    [--record ABSOLUTE_NEW_FILE] [--verification-record ABSOLUTE_FILE]

Environment:
  GH_TOKEN                 Actions/Contents token (required in every mode)
  GH_ADMIN_READ_TOKEN      Administration-read token (attach and publish only)

The workflow consumes an already-created empty draft Release. It never creates
or deletes a Release, tag, or ref; it never deletes or replaces an asset and
never rebuilds release bytes.

The attach and verify modes require --record and write a closed JSON identity
record. The publish mode requires the exact verify-mode record through
--verification-record and rejects publication if it does not reproduce the
current draft, server-assigned asset identities, and workflow-run identity.
EOF
}

fail() {
  printf '[release-publish] ERROR: %s\n' "$1" >&2
  exit 1
}

MODE="${1:-}"
[[ -n "$MODE" ]] || { usage >&2; exit 2; }
shift

case "$MODE" in
  resolve-stage|preflight|attach|verify|publish|postverify) ;;
  *) usage >&2; exit 2 ;;
esac

REPOSITORY=""
COMMIT=""
VERSION=""
TAG_OBJECT_SHA=""
STAGED_ARTIFACT_ID=""
STAGED_ARTIFACT_SHA256=""
DRAFT_RELEASE_ID=""
ASSETS_DIR=""
RECEIPT=""
RECORD=""
VERIFICATION_RECORD=""

while (($# > 0)); do
  case "$1" in
    --repository) (($# >= 2)) || { usage >&2; exit 2; }; REPOSITORY="$2"; shift 2 ;;
    --commit) (($# >= 2)) || { usage >&2; exit 2; }; COMMIT="$2"; shift 2 ;;
    --version) (($# >= 2)) || { usage >&2; exit 2; }; VERSION="$2"; shift 2 ;;
    --tag-object-sha) (($# >= 2)) || { usage >&2; exit 2; }; TAG_OBJECT_SHA="$2"; shift 2 ;;
    --staged-artifact-id) (($# >= 2)) || { usage >&2; exit 2; }; STAGED_ARTIFACT_ID="$2"; shift 2 ;;
    --staged-artifact-sha256) (($# >= 2)) || { usage >&2; exit 2; }; STAGED_ARTIFACT_SHA256="$2"; shift 2 ;;
    --draft-release-id) (($# >= 2)) || { usage >&2; exit 2; }; DRAFT_RELEASE_ID="$2"; shift 2 ;;
    --assets-dir) (($# >= 2)) || { usage >&2; exit 2; }; ASSETS_DIR="$2"; shift 2 ;;
    --receipt) (($# >= 2)) || { usage >&2; exit 2; }; RECEIPT="$2"; shift 2 ;;
    --record) (($# >= 2)) || { usage >&2; exit 2; }; RECORD="$2"; shift 2 ;;
    --verification-record) (($# >= 2)) || { usage >&2; exit 2; }; VERIFICATION_RECORD="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

for command_name in awk basename cmp curl dirname find grep jq mktemp mv realpath rm sed sha256sum sleep sort stat tail tar wc; do
  command -v "$command_name" >/dev/null 2>&1 || fail required_command_missing
done

[[ "$REPOSITORY" == "CyberBASSLord-666/termux-mcp-edge" ]] || fail repository_invalid
[[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail commit_invalid
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail version_invalid
[[ "$STAGED_ARTIFACT_ID" =~ ^[1-9][0-9]*$ ]] || fail staged_artifact_id_invalid
[[ "$STAGED_ARTIFACT_SHA256" =~ ^[0-9a-f]{64}$ ]] || fail staged_artifact_sha256_invalid
[[ -n "${GH_TOKEN:-}" && "$GH_TOKEN" != *$'\n'* && "$GH_TOKEN" != *$'\r'* ]] \
  || fail gh_token_invalid

TAG="v$VERSION"
# upload-artifact raw mode (`archive: false`) uses the uploaded file basename
# as both the Actions artifact name and the downloaded filename.
STAGE_TAR_NAME="termux-mcp-server-v${VERSION}-release-stage-${COMMIT:0:12}.tar"
API_BASE="${GITHUB_API_URL:-https://api.github.com}"
SERVER_BASE="${GITHUB_SERVER_URL:-https://github.com}"
[[ "$API_BASE" =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?$ ]] || fail github_api_url_invalid
[[ "$SERVER_BASE" =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?$ ]] || fail github_server_url_invalid
API_ROOT="$API_BASE/repos/$REPOSITORY"
UPLOAD_ROOT="https://uploads.github.com/repos/$REPOSITORY"

if [[ "$MODE" != resolve-stage ]]; then
  [[ "$TAG_OBJECT_SHA" =~ ^[0-9a-f]{40}$ ]] || fail tag_object_sha_invalid
  [[ "$DRAFT_RELEASE_ID" =~ ^[1-9][0-9]*$ ]] || fail draft_release_id_invalid
  [[ "$ASSETS_DIR" == /* && -d "$ASSETS_DIR" && ! -L "$ASSETS_DIR" ]] || fail assets_directory_invalid
  [[ "$(realpath -e -- "$ASSETS_DIR")" == "$ASSETS_DIR" ]] || fail assets_directory_not_canonical
  [[ "$(stat -c '%a' -- "$ASSETS_DIR")" == 700 ]] || fail assets_directory_mode_invalid
  [[ "$RECEIPT" == /* && -f "$RECEIPT" && ! -L "$RECEIPT" ]] || fail receipt_invalid
  [[ "$(realpath -e -- "$RECEIPT")" == "$RECEIPT" ]] || fail receipt_not_canonical
  [[ "$(stat -c '%a' -- "$RECEIPT")" == 600 ]] || fail receipt_mode_invalid
  [[ "$(basename -- "$RECEIPT")" == release-publication-receipt-v1.json ]] || fail receipt_name_invalid
  [[ "$(dirname -- "$RECEIPT")" == "$(dirname -- "$ASSETS_DIR")" ]] \
    || fail publication_inputs_not_siblings
  [[ "$(stat -c '%a' -- "$(dirname -- "$ASSETS_DIR")")" == 700 ]] \
    || fail publication_parent_mode_invalid
fi

validate_new_record_path() {
  local expected_name="$1" parent
  [[ "$RECORD" == /* && "$(basename -- "$RECORD")" == "$expected_name" ]] \
    || fail identity_record_path_invalid
  [[ "$(realpath -m -- "$RECORD")" == "$RECORD" ]] || fail identity_record_path_not_canonical
  parent="$(dirname -- "$RECORD")"
  [[ -d "$parent" && ! -L "$parent" && "$(realpath -e -- "$parent")" == "$parent" ]] \
    || fail identity_record_parent_invalid
  [[ "$(stat -c '%a' -- "$parent")" == 700 ]] || fail identity_record_parent_mode_invalid
  [[ ! -e "$RECORD" && ! -L "$RECORD" ]] || fail identity_record_already_exists
}

validate_existing_verification_record_path() {
  local parent
  [[ "$VERIFICATION_RECORD" == /* \
    && "$(basename -- "$VERIFICATION_RECORD")" == release-draft-verification-record-v1.json \
    && -f "$VERIFICATION_RECORD" && ! -L "$VERIFICATION_RECORD" ]] \
    || fail verification_record_invalid
  [[ "$(realpath -e -- "$VERIFICATION_RECORD")" == "$VERIFICATION_RECORD" ]] \
    || fail verification_record_not_canonical
  [[ "$(stat -c '%a' -- "$VERIFICATION_RECORD")" == 600 ]] \
    || fail verification_record_mode_invalid
  [[ "$(stat -c '%s' -- "$VERIFICATION_RECORD")" -ge 1 \
    && "$(stat -c '%s' -- "$VERIFICATION_RECORD")" -le 1048576 ]] \
    || fail verification_record_size_invalid
  parent="$(dirname -- "$VERIFICATION_RECORD")"
  [[ -d "$parent" && ! -L "$parent" && "$(realpath -e -- "$parent")" == "$parent" ]] \
    || fail verification_record_parent_invalid
  [[ "$(stat -c '%a' -- "$parent")" == 700 ]] \
    || fail verification_record_parent_mode_invalid
  jq -e . "$VERIFICATION_RECORD" >/dev/null 2>&1 || fail verification_record_json_invalid
}

case "$MODE" in
  attach)
    [[ -n "$RECORD" && -z "$VERIFICATION_RECORD" ]] || fail identity_record_arguments_invalid
    validate_new_record_path release-attachment-record-v1.json
    ;;
  verify)
    [[ -n "$RECORD" && -z "$VERIFICATION_RECORD" ]] || fail identity_record_arguments_invalid
    validate_new_record_path release-draft-verification-record-v1.json
    ;;
  publish)
    [[ -z "$RECORD" && -n "$VERIFICATION_RECORD" ]] || fail identity_record_arguments_invalid
    validate_existing_verification_record_path
    ;;
  resolve-stage|preflight|postverify)
    [[ -z "$RECORD" && -z "$VERIFICATION_RECORD" ]] || fail identity_record_arguments_invalid
    ;;
esac

if [[ "$MODE" == attach || "$MODE" == verify || "$MODE" == publish ]]; then
  [[ "${GITHUB_RUN_ID:-}" =~ ^[1-9][0-9]*$ ]] || fail workflow_run_id_invalid
  [[ "${GITHUB_RUN_ATTEMPT:-}" == 1 ]] || fail workflow_run_attempt_invalid
fi

if [[ "$MODE" == attach || "$MODE" == publish ]]; then
  [[ -n "${GH_ADMIN_READ_TOKEN:-}" \
    && "$GH_ADMIN_READ_TOKEN" != *$'\n'* \
    && "$GH_ADMIN_READ_TOKEN" != *$'\r'* ]] || fail admin_read_token_invalid
  [[ "$GH_ADMIN_READ_TOKEN" != "$GH_TOKEN" ]] || fail admin_read_token_not_separate
fi

WORK_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/termux-mcp-release-publish.XXXXXXXX")" \
  || fail temporary_directory_create_failed
[[ "$WORK_ROOT" == "${TMPDIR:-/tmp}"/termux-mcp-release-publish.* ]] \
  || fail temporary_directory_invalid
cleanup() {
  if [[ -n "${WORK_ROOT:-}" && -d "$WORK_ROOT" \
    && "$WORK_ROOT" == "${TMPDIR:-/tmp}"/termux-mcp-release-publish.* ]]; then
    rm -rf -- "$WORK_ROOT" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

LAST_CURL_EXIT=0
LAST_HTTP_CODE=""

request() {
  local method="$1" token="$2" accept="$3" url="$4" output="$5" max_time="$6"
  local data_file="${7:-}" content_type="${8:-}" headers_file="${9:-}"
  local -a arguments
  arguments=(
    --silent --show-error
    --proto '=https' --tlsv1.2
    --connect-timeout 10 --max-time "$max_time"
    --request "$method"
    --output "$output"
    --write-out '%{http_code}'
  )
  if [[ -n "$headers_file" ]]; then
    arguments+=(--dump-header "$headers_file")
  fi
  if [[ -n "$token" ]]; then
    arguments+=(--header "Authorization: Bearer $token")
  fi
  if [[ -n "$accept" ]]; then
    arguments+=(--header "Accept: $accept")
  fi
  if [[ "$url" == "$API_BASE"/* || "$url" == "$UPLOAD_ROOT"/* ]]; then
    arguments+=(--header 'X-GitHub-Api-Version: 2026-03-10')
  fi
  if [[ -n "$content_type" ]]; then
    arguments+=(--header "Content-Type: $content_type")
  fi
  if [[ -n "$data_file" ]]; then
    arguments+=(--data-binary "@$data_file")
  fi
  arguments+=("$url")
  set +e
  LAST_HTTP_CODE="$(curl "${arguments[@]}")"
  LAST_CURL_EXIT=$?
  set -e
  [[ "$LAST_HTTP_CODE" =~ ^[0-9]{3}$ ]] || LAST_HTTP_CODE=000
}

readonly_failure_is_transient() {
  ((LAST_CURL_EXIT != 0)) && return 0
  case "$LAST_HTTP_CODE" in
    408|425|429|500|502|503|504) return 0 ;;
    *) return 1 ;;
  esac
}

readonly_request() {
  local token="$1" accept="$2" url="$3" output="$4" max_time="$5"
  local headers_file="${6:-}" attempt
  for attempt in 1 2 3; do
    request GET "$token" "$accept" "$url" "$output" "$max_time" "" "" "$headers_file"
    if ! readonly_failure_is_transient || ((attempt == 3)); then
      return
    fi
    sleep "$attempt"
  done
}

api_get() {
  local token="$1" url="$2" output="$3"
  readonly_request "$token" application/vnd.github+json "$url" "$output" 30
  ((LAST_CURL_EXIT == 0)) || fail api_get_transport_failed
  [[ "$LAST_HTTP_CODE" == 200 ]] || fail api_get_status_invalid
  jq -e . "$output" >/dev/null 2>&1 || fail api_get_json_invalid
}

mutate_json() {
  local method="$1" url="$2" payload="$3" output="$4" expected_status="$5"
  request "$method" "$GH_TOKEN" application/vnd.github+json "$url" "$output" 60 \
    "$payload" application/json
  ((LAST_CURL_EXIT == 0)) || return 1
  [[ "$LAST_HTTP_CODE" == "$expected_status" ]] || return 1
  jq -e . "$output" >/dev/null 2>&1 || return 1
}

upload_asset() {
  local name="$1" path="$2" output="$3" encoded_name
  encoded_name="$(jq -nr --arg value "$name" '$value | @uri')" || fail asset_name_encoding_failed
  [[ -n "$encoded_name" ]] || fail asset_name_encoding_failed
  request POST "$GH_TOKEN" application/vnd.github+json \
    "$UPLOAD_ROOT/releases/$DRAFT_RELEASE_ID/assets?name=$encoded_name" \
    "$output" 600 "$path" application/octet-stream
  ((LAST_CURL_EXIT == 0)) || fail asset_upload_transport_failed
  [[ "$LAST_HTTP_CODE" == 201 ]] || fail asset_upload_status_invalid
  jq -e . "$output" >/dev/null 2>&1 || fail asset_upload_json_invalid
}

plain_download() {
  local url="$1" output="$2" headers="$3" body="$4" location
  local -a locations
  [[ "$url" =~ ^https://[^[:space:]]+$ ]] || fail download_url_invalid
  readonly_request "" "" "$url" "$body" 600 "$headers"
  ((LAST_CURL_EXIT == 0)) || fail public_download_transport_failed
  if [[ "$LAST_HTTP_CODE" == 200 ]]; then
    mv -T -- "$body" "$output" || fail public_download_move_failed
    return
  fi
  [[ "$LAST_HTTP_CODE" == 302 ]] || fail public_download_status_invalid
  mapfile -t locations < <(
    awk 'tolower($0) ~ /^location:[[:space:]]/ {
      sub(/\r$/, ""); sub(/^[^:]*:[[:space:]]*/, ""); print
    }' "$headers"
  )
  ((${#locations[@]} == 1)) || fail public_download_location_invalid
  location="${locations[0]}"
  [[ "$location" =~ ^https://[^[:space:]]+$ ]] || fail public_download_location_invalid
  readonly_request "" "" "$location" "$output" 600
  ((LAST_CURL_EXIT == 0)) || fail redirected_download_transport_failed
  [[ "$LAST_HTTP_CODE" == 200 ]] || fail redirected_download_status_invalid
}

authenticated_asset_download() {
  local asset_id="$1" output="$2" headers="$WORK_ROOT/asset-$asset_id.headers"
  local body="$WORK_ROOT/asset-$asset_id.first-body" location
  local -a locations
  readonly_request "$GH_TOKEN" application/octet-stream \
    "$API_ROOT/releases/assets/$asset_id" "$body" 600 "$headers"
  ((LAST_CURL_EXIT == 0)) || fail asset_download_transport_failed
  if [[ "$LAST_HTTP_CODE" == 200 ]]; then
    mv -T -- "$body" "$output" || fail asset_download_move_failed
    return
  fi
  [[ "$LAST_HTTP_CODE" == 302 ]] || fail asset_download_status_invalid
  mapfile -t locations < <(
    awk 'tolower($0) ~ /^location:[[:space:]]/ {
      sub(/\r$/, ""); sub(/^[^:]*:[[:space:]]*/, ""); print
    }' "$headers"
  )
  ((${#locations[@]} == 1)) || fail asset_download_location_invalid
  location="${locations[0]}"
  [[ "$location" =~ ^https://[^[:space:]]+$ ]] || fail asset_download_location_invalid
  # The presigned redirect receives no Authorization or GitHub API header.
  readonly_request "" "" "$location" "$output" 600
  ((LAST_CURL_EXIT == 0)) || fail redirected_asset_download_transport_failed
  [[ "$LAST_HTTP_CODE" == 200 ]] || fail redirected_asset_download_status_invalid
}

REPOSITORY_ID=""
STAGE_RUN_ID=""
STAGE_SIZE=""

resolve_stage() {
  local repository_json="$WORK_ROOT/repository.json"
  local main_json="$WORK_ROOT/main-ref.json"
  local artifact_json="$WORK_ROOT/staged-artifact.json"
  local run_json="$WORK_ROOT/staging-run.json"
  local artifacts_json="$WORK_ROOT/staging-run-artifacts.json"
  api_get "$GH_TOKEN" "$API_ROOT" "$repository_json"
  jq -e --arg repository "$REPOSITORY" '
    .full_name == $repository
    and .name == "termux-mcp-edge"
    and .default_branch == "main"
    and (.id | type == "number" and floor == . and . >= 1)
  ' "$repository_json" >/dev/null || fail repository_identity_mismatch
  REPOSITORY_ID="$(jq -r '.id | tostring' "$repository_json")"

  api_get "$GH_TOKEN" "$API_ROOT/git/ref/heads/main" "$main_json"
  jq -e --arg commit "$COMMIT" '
    .ref == "refs/heads/main"
    and .object.type == "commit"
    and .object.sha == $commit
  ' "$main_json" >/dev/null || fail current_main_mismatch

  api_get "$GH_TOKEN" "$API_ROOT/actions/artifacts/$STAGED_ARTIFACT_ID" "$artifact_json"
  jq -e \
    --argjson artifact_id "$STAGED_ARTIFACT_ID" \
    --arg name "$STAGE_TAR_NAME" \
    --arg digest "sha256:$STAGED_ARTIFACT_SHA256" \
    --arg commit "$COMMIT" \
    --argjson repository_id "$REPOSITORY_ID" '
      .id == $artifact_id
      and .name == $name
      and .digest == $digest
      and .expired == false
      and (.size_in_bytes | type == "number" and floor == . and . >= 1 and . <= 536870912)
      and (.workflow_run.id | type == "number" and floor == . and . >= 1)
      and .workflow_run.repository_id == $repository_id
      and .workflow_run.head_repository_id == $repository_id
      and .workflow_run.head_branch == "main"
      and .workflow_run.head_sha == $commit
    ' "$artifact_json" >/dev/null || fail staged_artifact_identity_mismatch
  STAGE_RUN_ID="$(jq -r '.workflow_run.id | tostring' "$artifact_json")"
  STAGE_SIZE="$(jq -r '.size_in_bytes | tostring' "$artifact_json")"
  [[ "$STAGE_RUN_ID" =~ ^[1-9][0-9]*$ ]] || fail staging_run_id_invalid

  api_get "$GH_TOKEN" "$API_ROOT/actions/runs/$STAGE_RUN_ID" "$run_json"
  jq -e \
    --argjson run_id "$STAGE_RUN_ID" \
    --arg repository "$REPOSITORY" \
    --arg commit "$COMMIT" '
      .id == $run_id
      and .name == "Stage Release Assets"
      and .path == ".github/workflows/stage-release-assets.yml"
      and .event == "workflow_dispatch"
      and .head_branch == "main"
      and .head_sha == $commit
      and .status == "completed"
      and .conclusion == "success"
      and .run_attempt == 1
      and .repository.full_name == $repository
      and .head_repository.full_name == $repository
    ' "$run_json" >/dev/null || fail staging_run_identity_mismatch

  api_get "$GH_TOKEN" "$API_ROOT/actions/runs/$STAGE_RUN_ID/artifacts?per_page=100" "$artifacts_json"
  jq -e \
    --argjson artifact_id "$STAGED_ARTIFACT_ID" \
    --arg name "$STAGE_TAR_NAME" \
    --arg digest "sha256:$STAGED_ARTIFACT_SHA256" \
    --arg commit "$COMMIT" \
    --argjson repository_id "$REPOSITORY_ID" \
    --argjson run_id "$STAGE_RUN_ID" '
      .total_count == 1
      and (.artifacts | length) == 1
      and .artifacts[0].id == $artifact_id
      and .artifacts[0].name == $name
      and .artifacts[0].digest == $digest
      and .artifacts[0].expired == false
      and .artifacts[0].workflow_run.id == $run_id
      and .artifacts[0].workflow_run.repository_id == $repository_id
      and .artifacts[0].workflow_run.head_repository_id == $repository_id
      and .artifacts[0].workflow_run.head_branch == "main"
      and .artifacts[0].workflow_run.head_sha == $commit
    ' "$artifacts_json" >/dev/null || fail staging_run_artifact_set_mismatch
}

if [[ "$MODE" == resolve-stage ]]; then
  resolve_stage
  printf 'stage_run_id=%s\n' "$STAGE_RUN_ID"
  exit 0
fi

EXPECTED_NAMES_FILE="$WORK_ROOT/expected-asset-names"
{
  for posture in \
    default mcp-runtime android-battery-status android-volume-status \
    android-volume-control command-execution full-suite
  do
    printf 'termux-mcp-server-v%s-aarch64-linux-android-%s\n' "$VERSION" "$posture"
    printf 'termux-mcp-server-v%s-aarch64-linux-android-%s.sha256\n' "$VERSION" "$posture"
  done
  printf 'SHA256SUMS\n'
  printf '%s\n' "$STAGE_TAR_NAME"
} | sort >"$EXPECTED_NAMES_FILE"
[[ "$(wc -l <"$EXPECTED_NAMES_FILE")" == 16 ]] || fail expected_asset_set_invalid
EXPECTED_NAMES_JSON="$(jq -R -s 'split("\n")[:-1]' "$EXPECTED_NAMES_FILE")" \
  || fail expected_asset_set_invalid

validate_local_assets() {
  local actual_entries name path expected_size expected_sha actual_size actual_sha
  jq -e \
    --arg repository "$REPOSITORY" \
    --arg commit "$COMMIT" \
    --arg version "$VERSION" \
    --arg tar_name "$STAGE_TAR_NAME" \
    --arg tar_sha "$STAGED_ARTIFACT_SHA256" \
    --argjson names "$EXPECTED_NAMES_JSON" '
      .stageTar.size as $tar_size
      | (keys == ["assets","commit","repository","schemaVersion","stageTar","version"])
      and .schemaVersion == 1
      and .repository == $repository
      and .commit == $commit
      and .version == $version
      and (.stageTar | keys == ["name","sha256","size"])
      and .stageTar.name == $tar_name
      and .stageTar.sha256 == $tar_sha
      and (.stageTar.size | type == "number" and floor == . and . >= 1 and . <= 536870912)
      and (.assets | type == "array" and length == 16)
      and ([.assets[].name] == $names)
      and ([.assets[].name] | unique | length == 16)
      and all(.assets[];
        (keys == ["name","sha256","size","sourceStageMember"])
        and (.name | test("^[A-Za-z0-9][A-Za-z0-9._-]{0,191}$"))
        and (.sha256 | test("^[0-9a-f]{64}$"))
        and (.size | type == "number" and floor == . and . >= 1 and . <= 536870912)
        and (if .name == $tar_name
          then .sourceStageMember == null
          else .sourceStageMember == .name
          end))
      and ([.assets[] | select(.name == $tar_name)] | length == 1)
      and ([.assets[] | select(.name == $tar_name)][0]
        | .sha256 == $tar_sha and .size == $tar_size)
    ' "$RECEIPT" >/dev/null || fail receipt_contract_mismatch

  actual_entries="$(find "$ASSETS_DIR" -mindepth 1 -maxdepth 1 -printf '%f\n' 2>/dev/null | sort)" \
    || fail assets_directory_enumeration_failed
  [[ "$actual_entries" == "$(<"$EXPECTED_NAMES_FILE")" ]] || fail local_asset_set_mismatch
  if find "$ASSETS_DIR" -mindepth 1 -maxdepth 1 ! -type f -print -quit | grep -q .; then
    fail local_asset_type_invalid
  fi
  if find "$ASSETS_DIR" -mindepth 1 -maxdepth 1 -type l -print -quit | grep -q .; then
    fail local_asset_link_detected
  fi

  while IFS= read -r name; do
    path="$ASSETS_DIR/$name"
    [[ -f "$path" && ! -L "$path" ]] || fail local_asset_invalid
    expected_size="$(jq -er --arg name "$name" '.assets[] | select(.name == $name) | .size' "$RECEIPT")"
    expected_sha="$(jq -er --arg name "$name" '.assets[] | select(.name == $name) | .sha256' "$RECEIPT")"
    actual_size="$(stat -c '%s' -- "$path")" || fail local_asset_stat_failed
    actual_sha="$(sha256sum -- "$path" | awk '{print $1}')" || fail local_asset_digest_failed
    [[ "$actual_size" == "$expected_size" ]] || fail local_asset_size_mismatch
    [[ "$actual_sha" == "$expected_sha" ]] || fail local_asset_digest_mismatch
  done <"$EXPECTED_NAMES_FILE"

  [[ "$(jq -r '.stageTar.size | tostring' "$RECEIPT")" == \
    "$(stat -c '%s' -- "$ASSETS_DIR/$STAGE_TAR_NAME")" ]] || fail stage_tar_size_mismatch
}

STAGING_MANIFEST="$WORK_ROOT/release-staging-manifest-v1.json"
CI_RUN_ID=""
SECURITY_RUN_ID=""
ANDROID_RUN_ID=""
RUST_VERSION=""
NDK_VERSION=""
RELEASE_BODY="$WORK_ROOT/release-body.md"
RELEASE_BODY_SHA256=""
RELEASE_ASSETS_SHA256=""

load_release_metadata() {
  tar -xOf "$ASSETS_DIR/$STAGE_TAR_NAME" ./release-staging-manifest-v1.json \
    >"$STAGING_MANIFEST" 2>/dev/null || fail staging_manifest_extract_failed
  jq -e \
    --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg version "$VERSION" '
      (keys == ["artifacts","checksums","commit","evidence","license","publicationState","releaseEligible","repository","schemaVersion","target","version","workflowRuns"])
      and .schemaVersion == 1
      and .publicationState == "staged_not_released"
      and .releaseEligible == false
      and .repository == $repository
      and .commit == $commit
      and .version == $version
      and .target == "aarch64-linux-android"
      and (.workflowRuns | keys == ["android","ci","security"])
      and all(.workflowRuns[]; type == "string" and test("^[1-9][0-9]*$"))
      and (.artifacts | length == 7)
    ' "$STAGING_MANIFEST" >/dev/null || fail staging_manifest_identity_mismatch
  CI_RUN_ID="$(jq -r '.workflowRuns.ci' "$STAGING_MANIFEST")"
  SECURITY_RUN_ID="$(jq -r '.workflowRuns.security' "$STAGING_MANIFEST")"
  ANDROID_RUN_ID="$(jq -r '.workflowRuns.android' "$STAGING_MANIFEST")"

  local script_dir repository_root
  script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)" \
    || fail script_directory_resolution_failed
  repository_root="$(cd -- "$script_dir/.." && pwd -P)" \
    || fail repository_root_resolution_failed
  [[ -f "$repository_root/rust-toolchain.toml" && ! -L "$repository_root/rust-toolchain.toml" ]] \
    || fail rust_toolchain_file_invalid
  [[ -f "$repository_root/.github/workflows/android-cross-compile.yml" \
    && ! -L "$repository_root/.github/workflows/android-cross-compile.yml" ]] \
    || fail android_workflow_file_invalid
  RUST_VERSION="$(sed -n 's/^channel = "\([0-9][0-9.]*\)"$/\1/p' \
    "$repository_root/rust-toolchain.toml")"
  NDK_VERSION="$(sed -n 's/^[[:space:]]*ndk-version:[[:space:]]*\([^[:space:]#]*\).*$/\1/p' \
    "$repository_root/.github/workflows/android-cross-compile.yml" | sort -u)"
  [[ "$RUST_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail rust_toolchain_version_invalid
  [[ "$NDK_VERSION" =~ ^r[0-9]+[a-z]$ ]] || fail android_ndk_version_invalid
}

generate_release_body() {
  {
    printf '# termux-mcp-server v%s\n\n' "$VERSION"
    printf 'Production Android/Termux release built and qualified from exact `main`.\n\n'
    printf '## Immutable provenance\n\n'
    printf -- '- Source commit: [`%s`](%s/%s/commit/%s)\n' \
      "$COMMIT" "$SERVER_BASE" "$REPOSITORY" "$COMMIT"
    printf -- '- Annotated tag object: `%s` (`%s`)\n' "$TAG_OBJECT_SHA" "$TAG"
    printf -- '- Staging workflow run: [`%s`](%s/%s/actions/runs/%s)\n' \
      "$STAGE_RUN_ID" "$SERVER_BASE" "$REPOSITORY" "$STAGE_RUN_ID"
    printf -- '- Staged Actions artifact ID: `%s`\n' "$STAGED_ARTIFACT_ID"
    printf -- '- Staged tar SHA-256: `%s`\n' "$STAGED_ARTIFACT_SHA256"
    printf -- '- CI / Security / Android runs: `%s` / `%s` / `%s`\n' \
      "$CI_RUN_ID" "$SECURITY_RUN_ID" "$ANDROID_RUN_ID"
    printf -- '- Rust / Android target / NDK: `%s` / `aarch64-linux-android` / `%s`\n\n' \
      "$RUST_VERSION" "$NDK_VERSION"
    printf '## Release assets\n\n'
    printf '| Asset | Bytes | SHA-256 |\n'
    printf '| --- | ---: | --- |\n'
    jq -r '.assets[] | "| `\(.name)` | \(.size) | `\(.sha256)` |"' "$RECEIPT"
    printf '\nThe provenance tar is the unchanged staged Actions artifact. The other assets are exact member bytes from that tar; no release-time rebuild occurred.\n\n'
    printf '## Operational boundary\n\n'
    printf -- '- Select the least-privilege posture that provides the required capability set.\n'
    printf -- '- High-impact filesystem and volume mutations remain separately runtime-gated and grant-gated.\n'
    printf -- '- Sessions and replay state are process-local and intentionally do not survive restart.\n'
    printf -- '- No byte-for-byte reproducible-build claim is made.\n'
    printf -- '- This is the first governed public release; public rollback becomes available only after a later complete release is installed as an upgrade.\n\n'
    printf 'See [deployment](%s/%s/blob/%s/docs/TERMUX_DEPLOYMENT.md), [operations](%s/%s/blob/%s/docs/OPERATIONS.md), and [release governance](%s/%s/blob/%s/docs/RELEASE_GOVERNANCE.md).\n' \
      "$SERVER_BASE" "$REPOSITORY" "$TAG" \
      "$SERVER_BASE" "$REPOSITORY" "$TAG" \
      "$SERVER_BASE" "$REPOSITORY" "$TAG"
  } >"$RELEASE_BODY" || fail release_body_write_failed
  RELEASE_BODY_SHA256="$(sha256sum -- "$RELEASE_BODY" | awk '{print $1}')" \
    || fail release_body_digest_failed
  RELEASE_ASSETS_SHA256="$(jq -cS '.assets' "$RECEIPT" | sha256sum | awk '{print $1}')" \
    || fail release_assets_digest_failed
  [[ "$RELEASE_BODY_SHA256" =~ ^[0-9a-f]{64}$ ]] || fail release_body_digest_failed
  [[ "$RELEASE_ASSETS_SHA256" =~ ^[0-9a-f]{64}$ ]] || fail release_assets_digest_failed
}

verify_tag() {
  local ref_json="$WORK_ROOT/tag-ref.json" tag_json="$WORK_ROOT/tag-object.json"
  api_get "$GH_TOKEN" "$API_ROOT/git/ref/tags/$TAG" "$ref_json"
  jq -e --arg tag "$TAG" --arg object_sha "$TAG_OBJECT_SHA" '
    .ref == ("refs/tags/" + $tag)
    and .object.type == "tag"
    and .object.sha == $object_sha
  ' "$ref_json" >/dev/null || fail annotated_tag_ref_mismatch
  api_get "$GH_TOKEN" "$API_ROOT/git/tags/$TAG_OBJECT_SHA" "$tag_json"
  jq -e --arg tag "$TAG" --arg object_sha "$TAG_OBJECT_SHA" --arg commit "$COMMIT" '
    .sha == $object_sha
    and .tag == $tag
    and (.message | type == "string" and length >= 1 and length <= 65536)
    and (.tagger.name | type == "string" and length >= 1)
    and (.tagger.email | type == "string" and length >= 3)
    and (.tagger.date | type == "string" and length >= 10)
    and .object.type == "commit"
    and .object.sha == $commit
    and (
      (.verification.verified == true and .verification.reason == "valid")
      or (.verification.verified == false and .verification.reason == "unsigned")
    )
  ' "$tag_json" >/dev/null || fail annotated_tag_object_mismatch
}

RELEASE_JSON="$WORK_ROOT/release.json"
ASSETS_JSON="$WORK_ROOT/release-assets.json"

refresh_release() {
  api_get "$GH_TOKEN" "$API_ROOT/releases/$DRAFT_RELEASE_ID" "$RELEASE_JSON"
  api_get "$GH_TOKEN" "$API_ROOT/releases/$DRAFT_RELEASE_ID/assets?per_page=100" "$ASSETS_JSON"
  jq -e 'type == "array" and length < 100' "$ASSETS_JSON" >/dev/null \
    || fail release_asset_page_invalid
}

verify_release_uniqueness() {
  local page=1 page_json length matches total_matches=0 matched_id=""
  while ((page <= 20)); do
    page_json="$WORK_ROOT/releases-page-$page.json"
    api_get "$GH_TOKEN" "$API_ROOT/releases?per_page=100&page=$page" "$page_json"
    jq -e 'type == "array" and length <= 100' "$page_json" >/dev/null \
      || fail release_page_invalid
    length="$(jq -r 'length' "$page_json")"
    matches="$(jq -r --arg tag "$TAG" '[.[] | select(.tag_name == $tag)] | length' "$page_json")"
    if ((matches > 0)); then
      total_matches=$((total_matches + matches))
      matched_id="$(jq -r --arg tag "$TAG" '.[] | select(.tag_name == $tag) | .id' "$page_json" | tail -n 1)"
    fi
    ((length < 100)) && break
    page=$((page + 1))
  done
  ((page <= 20)) || fail release_pagination_bound_exceeded
  [[ "$total_matches" == 1 && "$matched_id" == "$DRAFT_RELEASE_ID" ]] \
    || fail release_tag_uniqueness_mismatch
}

assert_release_base() {
  jq -e \
    --argjson release_id "$DRAFT_RELEASE_ID" \
    --arg tag "$TAG" \
    --arg api_url "$API_ROOT/releases/$DRAFT_RELEASE_ID" \
    --arg assets_url "$API_ROOT/releases/$DRAFT_RELEASE_ID/assets" \
    --arg upload_url "$UPLOAD_ROOT/releases/$DRAFT_RELEASE_ID/assets{?name,label}" \
    --arg html_url "$SERVER_BASE/$REPOSITORY/releases/tag/$TAG" '
      .id == $release_id
      and .url == $api_url
      and .assets_url == $assets_url
      and .upload_url == $upload_url
      and .html_url == $html_url
      and .tag_name == $tag
      and .name == $tag
      and .prerelease == false
    ' "$RELEASE_JSON" >/dev/null || fail draft_release_identity_mismatch
}

assert_virgin_draft() {
  assert_release_base
  jq -e '
    .draft == true
    and .immutable == false
    and .published_at == null
    and (.body == null or .body == "")
  ' "$RELEASE_JSON" >/dev/null || fail draft_release_not_virgin
  jq -e 'length == 0' "$ASSETS_JSON" >/dev/null || fail draft_release_not_empty
}

assert_bound_draft() {
  assert_release_base
  jq -e --rawfile body "$RELEASE_BODY" '
    .draft == true
    and .immutable == false
    and .published_at == null
    and .body == $body
  ' "$RELEASE_JSON" >/dev/null || fail draft_release_binding_mismatch
}

assert_published_release() {
  assert_release_base
  jq -e --rawfile body "$RELEASE_BODY" '
    .draft == false
    and .immutable == true
    and .published_at != null
    and (.published_at | type == "string" and length >= 10)
    and .body == $body
  ' "$RELEASE_JSON" >/dev/null || fail published_release_identity_mismatch
}

assert_server_asset_prefix() {
  local prefix_count="$1" expected
  [[ "$prefix_count" =~ ^[0-9]+$ ]] || fail asset_prefix_invalid
  ((prefix_count >= 0 && prefix_count <= 16)) || fail asset_prefix_invalid
  expected="$(jq -c --argjson count "$prefix_count" '
    .assets[0:$count]
    | map({name, size, digest:("sha256:" + .sha256), state:"uploaded", content_type:"application/octet-stream"})
  ' "$RECEIPT")" || fail expected_server_assets_invalid
  jq -e --argjson expected "$expected" --arg api_root "$API_ROOT" '
    (length == ($expected | length))
    and ([.[] | {name,size,digest,state,content_type}] | sort_by(.name))
      == ($expected | sort_by(.name))
    and ([.[].id] | unique | length) == length
    and all(.[];
      (.id | type == "number" and floor == . and . >= 1)
      and .url == ($api_root + "/releases/assets/" + (.id | tostring))
      and (.browser_download_url | type == "string" and startswith("https://")))
  ' "$ASSETS_JSON" >/dev/null || fail release_asset_set_mismatch
}

verify_remote_asset_bytes() {
  local visibility="$1" name expected_size expected_sha asset_id browser_url output headers body
  while IFS= read -r name; do
    expected_size="$(jq -er --arg name "$name" '.assets[] | select(.name == $name) | .size' "$RECEIPT")"
    expected_sha="$(jq -er --arg name "$name" '.assets[] | select(.name == $name) | .sha256' "$RECEIPT")"
    asset_id="$(jq -er --arg name "$name" '.[] | select(.name == $name) | .id' "$ASSETS_JSON")"
    [[ "$asset_id" =~ ^[1-9][0-9]*$ ]] || fail release_asset_id_invalid
    output="$WORK_ROOT/download-$asset_id"
    if [[ "$visibility" == draft ]]; then
      authenticated_asset_download "$asset_id" "$output"
    elif [[ "$visibility" == public ]]; then
      browser_url="$(jq -er --arg name "$name" '.[] | select(.name == $name) | .browser_download_url' "$ASSETS_JSON")"
      [[ "$browser_url" == "$SERVER_BASE/$REPOSITORY/releases/download/$TAG/$name" ]] \
        || fail public_asset_url_mismatch
      headers="$WORK_ROOT/public-$asset_id.headers"
      body="$WORK_ROOT/public-$asset_id.first-body"
      plain_download "$browser_url" "$output" "$headers" "$body"
    else
      fail asset_visibility_invalid
    fi
    [[ -f "$output" && ! -L "$output" ]] || fail downloaded_asset_invalid
    [[ "$(stat -c '%s' -- "$output")" == "$expected_size" ]] \
      || fail downloaded_asset_size_mismatch
    [[ "$(sha256sum -- "$output" | awk '{print $1}')" == "$expected_sha" ]] \
      || fail downloaded_asset_digest_mismatch
  done <"$EXPECTED_NAMES_FILE"
}

verify_immutable_policy() {
  local policy_json="$WORK_ROOT/immutable-policy.json"
  readonly_request "$GH_ADMIN_READ_TOKEN" application/vnd.github+json \
    "$API_ROOT/immutable-releases" "$policy_json" 30
  ((LAST_CURL_EXIT == 0)) || fail immutable_release_policy_query_transport_failed
  [[ "$LAST_HTTP_CODE" != 404 ]] || fail immutable_release_policy_disabled
  [[ "$LAST_HTTP_CODE" == 200 ]] || fail immutable_release_policy_query_status_invalid
  jq -e . "$policy_json" >/dev/null 2>&1 || fail immutable_release_policy_query_json_invalid
  jq -e '
    .enabled == true
    and (.enforced_by_owner | type == "boolean")
  ' "$policy_json" >/dev/null || fail immutable_release_policy_disabled
}

refresh_full_state() {
  resolve_stage
  [[ "$STAGE_SIZE" == "$(jq -r '.stageTar.size | tostring' "$RECEIPT")" ]] \
    || fail staged_artifact_size_mismatch
  verify_tag
  refresh_release
  verify_release_uniqueness
}

verify_published_indexes() {
  local by_tag_json="$WORK_ROOT/release-by-tag.json" latest_json="$WORK_ROOT/latest-release.json"
  api_get "$GH_TOKEN" "$API_ROOT/releases/tags/$TAG" "$by_tag_json"
  api_get "$GH_TOKEN" "$API_ROOT/releases/latest" "$latest_json"
  for path in "$by_tag_json" "$latest_json"; do
    jq -e --argjson release_id "$DRAFT_RELEASE_ID" --arg tag "$TAG" --rawfile body "$RELEASE_BODY" '
      .id == $release_id
      and .tag_name == $tag
      and .name == $tag
      and .draft == false
      and .prerelease == false
      and .immutable == true
      and .published_at != null
      and .body == $body
    ' "$path" >/dev/null || fail published_release_index_mismatch
  done
}

build_identity_record() {
  local phase="$1" output="$2"
  local projected_assets="$WORK_ROOT/identity-record-assets-$phase.json"
  local server_assets_sha256
  [[ "$phase" == attach || "$phase" == verify ]] || fail identity_record_phase_invalid
  jq -cS '
    [ .[]
      | {
          id,
          name,
          size,
          digest,
          state,
          contentType:.content_type,
          apiUrl:.url,
          downloadUrl:.browser_download_url
        }
    ] | sort_by(.name)
  ' "$ASSETS_JSON" >"$projected_assets" || fail identity_record_assets_invalid
  server_assets_sha256="$(sha256sum -- "$projected_assets" | awk '{print $1}')" \
    || fail identity_record_assets_digest_failed
  [[ "$server_assets_sha256" =~ ^[0-9a-f]{64}$ ]] \
    || fail identity_record_assets_digest_failed

  jq -S -n \
    --arg phase "$phase" \
    --arg repository "$REPOSITORY" \
    --arg workflow_run_id "$GITHUB_RUN_ID" \
    --arg workflow_run_attempt "$GITHUB_RUN_ATTEMPT" \
    --arg workflow_run_url "$SERVER_BASE/$REPOSITORY/actions/runs/$GITHUB_RUN_ID" \
    --arg commit "$COMMIT" \
    --arg version "$VERSION" \
    --arg tag "$TAG" \
    --arg tag_object_sha "$TAG_OBJECT_SHA" \
    --arg stage_run_id "$STAGE_RUN_ID" \
    --argjson staged_artifact_id "$STAGED_ARTIFACT_ID" \
    --arg staged_artifact_name "$STAGE_TAR_NAME" \
    --arg staged_artifact_sha256 "$STAGED_ARTIFACT_SHA256" \
    --argjson staged_artifact_size "$STAGE_SIZE" \
    --argjson release_id "$DRAFT_RELEASE_ID" \
    --arg release_url "$SERVER_BASE/$REPOSITORY/releases/tag/$TAG" \
    --arg release_body_sha256 "$RELEASE_BODY_SHA256" \
    --arg expected_asset_set_sha256 "$RELEASE_ASSETS_SHA256" \
    --arg server_assets_sha256 "$server_assets_sha256" \
    --slurpfile assets "$projected_assets" '
      {
        schemaVersion:1,
        recordType:"release-publication-identity",
        phase:$phase,
        repository:$repository,
        workflow:{
          name:"Publish Immutable Release",
          path:".github/workflows/publish-release.yml",
          runId:$workflow_run_id,
          runAttempt:$workflow_run_attempt,
          runUrl:$workflow_run_url
        },
        source:{
          commit:$commit,
          version:$version,
          tag:$tag,
          tagObjectSha:$tag_object_sha
        },
        stage:{
          runId:$stage_run_id,
          artifactId:$staged_artifact_id,
          artifactName:$staged_artifact_name,
          artifactSha256:$staged_artifact_sha256,
          artifactSize:$staged_artifact_size
        },
        release:{
          id:$release_id,
          url:$release_url,
          bodySha256:$release_body_sha256,
          expectedAssetSetSha256:$expected_asset_set_sha256,
          serverAssetIdentitiesSha256:$server_assets_sha256,
          assetCount:($assets[0] | length),
          assets:$assets[0]
        }
      }
    ' >"$output" || fail identity_record_write_failed
  chmod 600 "$output" || fail identity_record_mode_failed
}

write_identity_record() {
  local phase="$1" temporary
  temporary="$WORK_ROOT/$phase-identity-record-v1.json"
  build_identity_record "$phase" "$temporary"
  mv -T -n -- "$temporary" "$RECORD" 2>/dev/null || true
  [[ ! -e "$temporary" && ! -L "$temporary" ]] || fail identity_record_publish_raced
  [[ -f "$RECORD" && ! -L "$RECORD" && "$(stat -c '%a' -- "$RECORD")" == 600 ]] \
    || fail identity_record_publish_failed
}

validate_draft_verification_record() {
  local expected="$WORK_ROOT/expected-draft-verification-record-v1.json"
  build_identity_record verify "$expected"
  cmp -s -- "$VERIFICATION_RECORD" "$expected" || fail draft_verification_record_mismatch
}

emit_outputs() {
  printf 'stage_run_id=%s\n' "$STAGE_RUN_ID"
  printf 'release_body_sha256=%s\n' "$RELEASE_BODY_SHA256"
  printf 'release_assets_sha256=%s\n' "$RELEASE_ASSETS_SHA256"
  printf 'release_url=%s/%s/releases/tag/%s\n' "$SERVER_BASE" "$REPOSITORY" "$TAG"
  printf 'release_id=%s\n' "$DRAFT_RELEASE_ID"
  printf 'release_tag=%s\n' "$TAG"
  printf 'release_commit=%s\n' "$COMMIT"
  printf 'release_asset_count=16\n'
}

validate_local_assets
load_release_metadata
resolve_stage
[[ "$STAGE_SIZE" == "$(jq -r '.stageTar.size | tostring' "$RECEIPT")" ]] \
  || fail staged_artifact_size_mismatch
generate_release_body
verify_tag
refresh_release
verify_release_uniqueness

case "$MODE" in
  preflight)
    assert_virgin_draft
    ;;

  attach)
    # A partially populated or already-bound draft is never a resumable mutation target.
    assert_virgin_draft
    verify_immutable_policy
    refresh_full_state
    assert_virgin_draft

    body_payload="$WORK_ROOT/release-body-patch.json"
    body_response="$WORK_ROOT/release-body-response.json"
    jq -n --rawfile body "$RELEASE_BODY" '{body:$body}' >"$body_payload" \
      || fail release_body_payload_failed
    mutate_json PATCH "$API_ROOT/releases/$DRAFT_RELEASE_ID" \
      "$body_payload" "$body_response" 200 || fail release_body_update_failed
    jq -e --argjson release_id "$DRAFT_RELEASE_ID" --arg tag "$TAG" --rawfile body "$RELEASE_BODY" '
      .id == $release_id and .tag_name == $tag and .name == $tag
      and .draft == true and .prerelease == false and .immutable == false
      and .published_at == null and .body == $body
    ' "$body_response" >/dev/null || fail release_body_update_response_mismatch

    uploaded_count=0
    while IFS= read -r asset_name; do
      refresh_release
      assert_bound_draft
      assert_server_asset_prefix "$uploaded_count"
      upload_response="$WORK_ROOT/upload-$uploaded_count.json"
      upload_asset "$asset_name" "$ASSETS_DIR/$asset_name" "$upload_response"
      expected_size="$(jq -er --arg name "$asset_name" '.assets[] | select(.name == $name) | .size' "$RECEIPT")"
      expected_sha="$(jq -er --arg name "$asset_name" '.assets[] | select(.name == $name) | .sha256' "$RECEIPT")"
      jq -e \
        --arg name "$asset_name" --arg digest "sha256:$expected_sha" \
        --argjson size "$expected_size" --arg api_root "$API_ROOT" '
          (.id | type == "number" and floor == . and . >= 1)
          and .name == $name and .state == "uploaded"
          and .content_type == "application/octet-stream"
          and .size == $size and .digest == $digest
          and .url == ($api_root + "/releases/assets/" + (.id | tostring))
        ' "$upload_response" >/dev/null || fail asset_upload_response_mismatch
      uploaded_count=$((uploaded_count + 1))
    done <"$EXPECTED_NAMES_FILE"
    [[ "$uploaded_count" == 16 ]] || fail asset_upload_count_mismatch

    refresh_full_state
    assert_bound_draft
    assert_server_asset_prefix 16
    verify_remote_asset_bytes draft
    verify_immutable_policy
    write_identity_record attach
    ;;

  verify)
    assert_bound_draft
    assert_server_asset_prefix 16
    verify_remote_asset_bytes draft
    refresh_full_state
    assert_bound_draft
    assert_server_asset_prefix 16
    write_identity_record verify
    ;;

  publish)
    # Publishing is single-shot: an already-public release is rejected here.
    assert_bound_draft
    assert_server_asset_prefix 16
    validate_draft_verification_record
    verify_immutable_policy
    verify_remote_asset_bytes draft

    # Close the approval-time window immediately before the only publish mutation.
    refresh_full_state
    assert_bound_draft
    assert_server_asset_prefix 16
    validate_draft_verification_record
    verify_immutable_policy

    publish_payload="$WORK_ROOT/publish-patch.json"
    publish_response="$WORK_ROOT/publish-response.json"
    jq -n '{draft:false,prerelease:false,make_latest:"true"}' >"$publish_payload" \
      || fail publish_payload_failed
    if mutate_json PATCH "$API_ROOT/releases/$DRAFT_RELEASE_ID" \
      "$publish_payload" "$publish_response" 200; then
      jq -e --argjson release_id "$DRAFT_RELEASE_ID" --arg tag "$TAG" --rawfile body "$RELEASE_BODY" '
        .id == $release_id and .tag_name == $tag and .name == $tag
        and .draft == false and .prerelease == false and .immutable == true
        and .published_at != null and .body == $body
      ' "$publish_response" >/dev/null || fail publish_response_mismatch
    else
      # Never retry an ambiguous PATCH. A readback may prove that the exact
      # immutable publication completed despite a lost response.
      printf '[release-publish] WARN: publish response ambiguous; verifying exact readback\n' >&2
    fi

    refresh_full_state
    assert_published_release
    assert_server_asset_prefix 16
    verify_published_indexes
    verify_remote_asset_bytes public
    ;;

  postverify)
    assert_published_release
    assert_server_asset_prefix 16
    verify_published_indexes
    verify_remote_asset_bytes public
    refresh_full_state
    assert_published_release
    assert_server_asset_prefix 16
    ;;
esac

emit_outputs
