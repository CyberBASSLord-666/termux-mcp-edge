#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C
umask 077
set +x

usage() {
  cat <<'EOF'
Usage: resolve_shizuku_rish_candidate.sh \
  --repository OWNER/REPO --commit SHA --pull-request NUMBER --output FILE

Requires GH_TOKEN. Resolves the current open same-repository pull-request head
with current trusted exact-head approval(s) and the latest exact-head,
first-attempt, successful CI, Security, and Android Cross Compile pull-request
runs. Writes one private closed JSON identity file.

Approval policy (development physical lane only):
  Default: two distinct non-author MEMBER/OWNER/COLLABORATOR exact-head
  APPROVED reviews.
  Solo-operator: set ANDROID_RISH_PHYSICAL_SOLO_OPERATOR=reviewed-v1 to accept
  one OWNER exact-head APPROVED review (author may self-approve). This mode is
  for the non-release physical_shizuku_rish_identity_development_v1 lane only.
EOF
}

REPOSITORY=""
COMMIT=""
PULL_REQUEST=""
OUTPUT=""
TEMP_ROOT=""
SCRIPT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
RUN_SELECTOR="$SCRIPT_ROOT/latest_workflow_run.jq"

fail() {
  printf '[android-rish-candidate] ERROR: %s\n' "$1" >&2
  exit 1
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM HUP
  if [[ -n "$TEMP_ROOT" && -d "$TEMP_ROOT" && ! -L "$TEMP_ROOT" ]]; then
    rm -rf -- "$TEMP_ROOT" >/dev/null 2>&1 || true
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

while (($# > 0)); do
  case "$1" in
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
    --pull-request)
      (($# >= 2)) || fail missing_pull_request
      PULL_REQUEST="$2"
      shift 2
      ;;
    --output)
      (($# >= 2)) || fail missing_output
      OUTPUT="$2"
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

for command_name in chmod curl dirname jq mkdir mktemp mv realpath rm; do
  command -v "$command_name" >/dev/null 2>&1 || fail required_command_missing
done
[[ -f "$RUN_SELECTOR" && ! -L "$RUN_SELECTOR" ]] || fail run_selector_invalid
[[ "$REPOSITORY" == CyberBASSLord-666/termux-mcp-edge ]] || fail repository_invalid
[[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || fail commit_invalid
[[ "$PULL_REQUEST" =~ ^[1-9][0-9]*$ ]] || fail pull_request_invalid
[[ -n "${GH_TOKEN:-}" ]] || fail github_token_missing
[[ "$OUTPUT" == /* && "$OUTPUT" != / && ! -e "$OUTPUT" && ! -L "$OUTPUT" ]] \
  || fail output_invalid
OUTPUT_PARENT="$(dirname -- "$OUTPUT")"
[[ "$OUTPUT_PARENT" == /* && -d "$OUTPUT_PARENT" && ! -L "$OUTPUT_PARENT" ]] \
  || fail output_parent_invalid
[[ "$(realpath -e -- "$OUTPUT_PARENT" 2>/dev/null)" == "$OUTPUT_PARENT" ]] \
  || fail output_parent_invalid

API_ROOT="${GITHUB_API_URL:-https://api.github.com}/repos/$REPOSITORY"
TEMP_ROOT="$(mktemp -d "$OUTPUT_PARENT/.android-rish-candidate.XXXXXX")" \
  || fail temporary_directory_create_failed
chmod 700 "$TEMP_ROOT" || fail temporary_directory_mode_failed

api_get() {
  curl --fail --silent --show-error \
    --connect-timeout 10 \
    --max-time 30 \
    --max-filesize 16777216 \
    -H "Authorization: Bearer $GH_TOKEN" \
    -H 'Accept: application/vnd.github+json' \
    -H 'X-GitHub-Api-Version: 2022-11-28' \
    "$1"
}

api_get "$API_ROOT/pulls/$PULL_REQUEST" >"$TEMP_ROOT/pull-request.json" \
  || fail pull_request_query_failed
jq -e \
  --arg repository "$REPOSITORY" \
  --arg commit "$COMMIT" \
  --argjson pull_request "$PULL_REQUEST" '
  .number == $pull_request
  and .state == "open"
  and .draft == false
  and .head.sha == $commit
  and .head.repo.full_name == $repository
  and .base.repo.full_name == $repository
  and .base.ref == "main"
  and (.head.ref | type == "string" and length >= 1 and length <= 255)
  and (.user.login | type == "string"
    and test("^[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$"))
  ' "$TEMP_ROOT/pull-request.json" >/dev/null \
  || fail pull_request_identity_invalid
HEAD_BRANCH="$(jq -er '.head.ref' "$TEMP_ROOT/pull-request.json")" \
  || fail pull_request_identity_invalid
PR_AUTHOR="$(jq -er '.user.login | ascii_downcase' "$TEMP_ROOT/pull-request.json")" \
  || fail pull_request_identity_invalid

# Solo-operator mode unblocks the development-only physical lane when the
# repository has a single OWNER and cannot collect two non-author approvals.
SOLO_OPERATOR=0
if [[ "${ANDROID_RISH_PHYSICAL_SOLO_OPERATOR:-}" == "reviewed-v1" ]]; then
  SOLO_OPERATOR=1
fi

api_get "$API_ROOT/pulls/$PULL_REQUEST/reviews?per_page=100" \
  >"$TEMP_ROOT/reviews.json" || fail pull_request_reviews_query_failed
jq -e \
  --arg author "$PR_AUTHOR" \
  --arg commit "$COMMIT" \
  --argjson solo "$SOLO_OPERATOR" '
  def timestamp:
    type == "string"
    and test(
      "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"
    );
  def trusted_approvals($allow_author):
    [
      (
        sort_by(.user.login | ascii_downcase)
        | group_by(.user.login | ascii_downcase)[]
        | sort_by(
            (.submitted_at // "9999-12-31T23:59:59Z"),
            .id
          )
        | last
        | select(
            .state == "APPROVED"
            and .commit_id == $commit
            and (
              $allow_author
              or (.user.login | ascii_downcase) != $author
            )
            and (.author_association | IN(
              "MEMBER",
              "OWNER",
              "COLLABORATOR"
            ))
            and (
              ($allow_author | not)
              or .author_association == "OWNER"
            )
          )
        | .user.login
        | ascii_downcase
      )
    ]
    | unique;
  type == "array"
  and length >= (if $solo == 1 then 1 else 2 end)
  and length < 100
  and all(.[];
    (.id | type == "number" and . >= 1 and floor == .)
    and (.user | type == "object")
    and (.user.login | type == "string"
      and test("^[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$"))
    and .user.type == "User"
    and (.author_association | type == "string")
    and (.state | IN(
      "APPROVED",
      "CHANGES_REQUESTED",
      "COMMENTED",
      "DISMISSED",
      "PENDING"
    ))
    and (
      .commit_id == null
      or (
        .commit_id
        | type == "string" and test("^[0-9a-f]{40}$")
      )
    )
    and (.submitted_at == null or (.submitted_at | timestamp))
  )
  and ([.[].id] | unique | length) == length
  and (
    if $solo == 1 then
      (trusted_approvals(true) | length) >= 1
    else
      (trusted_approvals(false) | length) >= 2
    end
  )
  ' "$TEMP_ROOT/reviews.json" >/dev/null \
  || fail pull_request_reviews_invalid

api_get "$API_ROOT/actions/runs?head_sha=$COMMIT&event=pull_request&per_page=100" \
  >"$TEMP_ROOT/runs.json" || fail workflow_runs_query_failed

resolve_run() {
  local name="$1" path="$2" output="$3" selected run_id
  selected="$(
    jq -c -L "$SCRIPT_ROOT" \
      --arg name "$name" \
      --arg path ".github/workflows/$path" \
      --arg repository "$REPOSITORY" \
      --arg commit "$COMMIT" \
      --arg branch "$HEAD_BRANCH" \
      --argjson pull_request "$PULL_REQUEST" '
      include "latest_workflow_run";
      complete_workflow_run_page
      |
      [
        .[]
        | select(
            .name == $name
            and .path == $path
            and .event == "pull_request"
            and .head_sha == $commit
            and .head_branch == $branch
            and .repository.full_name == $repository
            and .head_repository.full_name == $repository
            and (.pull_requests | type == "array")
            and any(.pull_requests[]?; .number == $pull_request)
          )
      ]
      | latest_workflow_run
      ' "$TEMP_ROOT/runs.json"
  )" || fail workflow_run_selection_failed
  run_id="$(jq -er '.id' <<<"$selected")" || fail workflow_run_selection_failed
  [[ "$run_id" =~ ^[1-9][0-9]*$ ]] || fail workflow_run_selection_failed
  api_get "$API_ROOT/actions/runs/$run_id" >"$output" \
    || fail workflow_run_query_failed
  jq -e \
    --arg name "$name" \
    --arg path ".github/workflows/$path" \
    --arg repository "$REPOSITORY" \
    --arg commit "$COMMIT" \
    --arg branch "$HEAD_BRANCH" \
    --argjson pull_request "$PULL_REQUEST" \
    --argjson run_id "$run_id" '
    .id == $run_id
    and .name == $name
    and .path == $path
    and .event == "pull_request"
    and .head_sha == $commit
    and .head_branch == $branch
    and .status == "completed"
    and .conclusion == "success"
    and .run_attempt == 1
    and .repository.full_name == $repository
    and .head_repository.full_name == $repository
    and (.pull_requests | type == "array")
    and any(.pull_requests[]?; .number == $pull_request)
    ' "$output" >/dev/null || fail workflow_run_identity_invalid
  printf '%s\n' "$run_id"
}

CI_RUN_ID="$(resolve_run CI ci.yml "$TEMP_ROOT/ci.json")"
SECURITY_RUN_ID="$(resolve_run Security security.yml "$TEMP_ROOT/security.json")"
ANDROID_RUN_ID="$(
  resolve_run \
    "Android Cross Compile" \
    android-cross-compile.yml \
    "$TEMP_ROOT/android.json"
)"

STAGED_OUTPUT="$TEMP_ROOT/candidate.json"
jq -cn \
  --arg repository "$REPOSITORY" \
  --arg commit "$COMMIT" \
  --arg head_branch "$HEAD_BRANCH" \
  --argjson pull_request "$PULL_REQUEST" \
  --argjson ci_run_id "$CI_RUN_ID" \
  --argjson security_run_id "$SECURITY_RUN_ID" \
  --argjson android_run_id "$ANDROID_RUN_ID" '
  {
    schemaVersion: 1,
    repository: $repository,
    commit: $commit,
    pullRequestNumber: $pull_request,
    headBranch: $head_branch,
    workflowEvent: "pull_request",
    runAttempt: 1,
    ciRunId: $ci_run_id,
    securityRunId: $security_run_id,
    androidRunId: $android_run_id
  }' >"$STAGED_OUTPUT" || fail output_write_failed
chmod 600 "$STAGED_OUTPUT" || fail output_mode_failed
jq -e \
  --arg repository "$REPOSITORY" \
  --arg commit "$COMMIT" \
  --argjson pull_request "$PULL_REQUEST" '
  (keys == [
    "androidRunId",
    "ciRunId",
    "commit",
    "headBranch",
    "pullRequestNumber",
    "repository",
    "runAttempt",
    "schemaVersion",
    "securityRunId",
    "workflowEvent"
  ])
  and .schemaVersion == 1
  and .repository == $repository
  and .commit == $commit
  and .pullRequestNumber == $pull_request
  and .workflowEvent == "pull_request"
  and .runAttempt == 1
  and (.ciRunId | type == "number" and floor == . and . >= 1)
  and (.securityRunId | type == "number" and floor == . and . >= 1)
  and (.androidRunId | type == "number" and floor == . and . >= 1)
  ' "$STAGED_OUTPUT" >/dev/null || fail output_contract_invalid
mv -Tn -- "$STAGED_OUTPUT" "$OUTPUT" || fail output_publication_failed
printf '[android-rish-candidate] result=PASS\n'
