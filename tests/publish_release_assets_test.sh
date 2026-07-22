#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'
export LC_ALL=C

ROOT="$(mktemp -d)"
chmod 700 "$ROOT"
cleanup() {
  local status=$?
  if ((status != 0)); then
    for diagnostic in "$ROOT"/*.stderr; do
      [[ -f "$diagnostic" ]] || continue
      printf '%s:\n' "$(basename -- "$diagnostic")" >&2
      sed -n '1,120p' "$diagnostic" >&2
    done
  fi
  rm -rf -- "$ROOT"
  exit "$status"
}
trap cleanup EXIT INT TERM

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/publish_release_assets.sh"
REAL_PATH="$PATH"
REPOSITORY="CyberBASSLord-666/termux-mcp-edge"
COMMIT="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
TAG_OBJECT_SHA="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
VERSION="0.6.0"
ARTIFACT_ID="55"
STAGE_RUN_ID="77"
RELEASE_ID="99"
STAGE_NAME="termux-mcp-server-v${VERSION}-release-stage-${COMMIT:0:12}.tar"

fail_test() {
  printf 'release publisher test failed: %s\n' "$1" >&2
  exit 1
}

assert_contains() {
  local expected="$1" path="$2"
  grep -Fq -- "$expected" "$path" \
    || { sed -n '1,120p' "$path" >&2; fail_test "missing expected text: $expected"; }
}

assert_full_output() {
  local path="$1"
  mapfile -t output_lines <"$path"
  ((${#output_lines[@]} == 8)) || fail_test publisher_output_line_count_changed
  [[ "${output_lines[0]}" == "stage_run_id=$STAGE_RUN_ID" ]] || fail_test stage_output_changed
  [[ "${output_lines[1]}" =~ ^release_body_sha256=[0-9a-f]{64}$ ]] || fail_test body_digest_output_changed
  [[ "${output_lines[2]}" =~ ^release_assets_sha256=[0-9a-f]{64}$ ]] || fail_test assets_digest_output_changed
  [[ "${output_lines[3]}" == "release_url=https://github.mock.invalid/$REPOSITORY/releases/tag/v$VERSION" ]] \
    || fail_test release_url_output_changed
  [[ "${output_lines[4]}" == "release_id=$RELEASE_ID" ]] || fail_test release_id_output_changed
  [[ "${output_lines[5]}" == "release_tag=v$VERSION" ]] || fail_test release_tag_output_changed
  [[ "${output_lines[6]}" == "release_commit=$COMMIT" ]] || fail_test release_commit_output_changed
  [[ "${output_lines[7]}" == "release_asset_count=16" ]] || fail_test release_asset_count_output_changed
}

assert_identity_record() {
  local path="$1" phase="$2"
  [[ -f "$path" && ! -L "$path" && "$(stat -c '%a' -- "$path")" == 600 ]] \
    || fail_test identity_record_file_invalid
  jq -e \
    --arg phase "$phase" --arg repository "$REPOSITORY" --arg commit "$COMMIT" \
    --arg version "$VERSION" --arg tag_object "$TAG_OBJECT_SHA" \
    --arg stage_run "$STAGE_RUN_ID" --argjson artifact_id "$ARTIFACT_ID" \
    --argjson release_id "$RELEASE_ID" '
      keys == ["phase","recordType","release","repository","schemaVersion","source","stage","workflow"]
      and .schemaVersion == 1
      and .recordType == "release-publication-identity"
      and .phase == $phase
      and .repository == $repository
      and (.workflow | keys == ["name","path","runAttempt","runId","runUrl"])
      and .workflow.name == "Publish Immutable Release"
      and .workflow.path == ".github/workflows/publish-release.yml"
      and .workflow.runId == "8801"
      and .workflow.runAttempt == "1"
      and (.source | keys == ["commit","tag","tagObjectSha","version"])
      and .source.commit == $commit
      and .source.version == $version
      and .source.tag == ("v" + $version)
      and .source.tagObjectSha == $tag_object
      and (.stage | keys == ["artifactId","artifactName","artifactSha256","artifactSize","runId"])
      and .stage.runId == $stage_run
      and .stage.artifactId == $artifact_id
      and (.release | keys == ["assetCount","assets","bodySha256","expectedAssetSetSha256","id","serverAssetIdentitiesSha256","url"])
      and .release.id == $release_id
      and .release.assetCount == 16
      and (.release.bodySha256 | test("^[0-9a-f]{64}$"))
      and (.release.expectedAssetSetSha256 | test("^[0-9a-f]{64}$"))
      and (.release.serverAssetIdentitiesSha256 | test("^[0-9a-f]{64}$"))
      and (.release.assets | length == 16)
      and all(.release.assets[];
        keys == ["apiUrl","contentType","digest","downloadUrl","id","name","size","state"]
        and (.id | type == "number" and floor == . and . >= 1)
        and (.name | test("^[A-Za-z0-9][A-Za-z0-9._-]{0,191}$"))
        and (.size | type == "number" and floor == . and . >= 1)
        and (.digest | test("^sha256:[0-9a-f]{64}$"))
        and .state == "uploaded"
        and .contentType == "application/octet-stream")
    ' "$path" >/dev/null || fail_test identity_record_contract_changed
}

[[ -f "$SCRIPT" && ! -L "$SCRIPT" && -x "$SCRIPT" ]] \
  || fail_test publisher_missing_linked_or_not_executable

PUBLICATION_ROOT="$ROOT/publication"
ASSETS_DIR="$PUBLICATION_ROOT/assets"
RECEIPT="$PUBLICATION_ROOT/release-publication-receipt-v1.json"
ATTACH_RECORD="$PUBLICATION_ROOT/release-attachment-record-v1.json"
VERIFY_RECORD="$PUBLICATION_ROOT/release-draft-verification-record-v1.json"
STAGE_ROOT="$ROOT/stage"
mkdir -m 700 -p -- "$ASSETS_DIR" "$STAGE_ROOT"
chmod 700 "$PUBLICATION_ROOT"

postures=(
  default
  mcp-runtime
  android-battery-status
  android-volume-status
  android-volume-control
  command-execution
  full-suite
)

for posture in "${postures[@]}"; do
  name="termux-mcp-server-v${VERSION}-aarch64-linux-android-${posture}"
  printf 'fixture release binary: %s\n' "$posture" >"$ASSETS_DIR/$name"
  digest="$(sha256sum -- "$ASSETS_DIR/$name" | awk '{print $1}')"
  printf '%s  %s\n' "$digest" "$name" >"$ASSETS_DIR/$name.sha256"
done

(
  cd -- "$ASSETS_DIR"
  for posture in "${postures[@]}"; do
    name="termux-mcp-server-v${VERSION}-aarch64-linux-android-${posture}"
    sha256sum -- "$name"
  done
) >"$ASSETS_DIR/SHA256SUMS"

jq -S -n \
  --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg version "$VERSION" '
    {
      schemaVersion:1,
      publicationState:"staged_not_released",
      releaseEligible:false,
      repository:$repository,
      commit:$commit,
      version:$version,
      target:"aarch64-linux-android",
      workflowRuns:{android:"5103",ci:"5101",security:"5102"},
      artifacts:[{},{},{},{},{},{},{}],
      checksums:{},
      evidence:{},
      license:{}
    }
  ' >"$STAGE_ROOT/release-staging-manifest-v1.json"
tar --format=ustar -C "$STAGE_ROOT" -cf "$ASSETS_DIR/$STAGE_NAME" \
  ./release-staging-manifest-v1.json

STAGE_SHA="$(sha256sum -- "$ASSETS_DIR/$STAGE_NAME" | awk '{print $1}')"
STAGE_SIZE="$(stat -c '%s' -- "$ASSETS_DIR/$STAGE_NAME")"
RECEIPT_ASSETS="$ROOT/receipt-assets.jsonl"
: >"$RECEIPT_ASSETS"
while IFS= read -r name; do
  path="$ASSETS_DIR/$name"
  digest="$(sha256sum -- "$path" | awk '{print $1}')"
  size="$(stat -c '%s' -- "$path")"
  if [[ "$name" == "$STAGE_NAME" ]]; then
    source_member=null
  else
    source_member="$(jq -Rn --arg value "$name" '$value')"
  fi
  jq -c -n \
    --arg name "$name" --arg digest "$digest" --argjson size "$size" \
    --argjson source "$source_member" \
    '{name:$name,sha256:$digest,size:$size,sourceStageMember:$source}' \
    >>"$RECEIPT_ASSETS"
done < <(find "$ASSETS_DIR" -mindepth 1 -maxdepth 1 -type f -printf '%f\n' | sort)

jq -S -n \
  --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg version "$VERSION" \
  --arg stage_name "$STAGE_NAME" --arg stage_sha "$STAGE_SHA" \
  --argjson stage_size "$STAGE_SIZE" --slurpfile assets "$RECEIPT_ASSETS" '
    {
      schemaVersion:1,
      repository:$repository,
      commit:$commit,
      version:$version,
      stageTar:{name:$stage_name,sha256:$stage_sha,size:$stage_size},
      assets:$assets
    }
  ' >"$RECEIPT"
chmod 600 "$RECEIPT"

MOCK_ROOT="$ROOT/mock"
FAKE_BIN="$ROOT/fake-bin"
mkdir -m 700 -p -- "$MOCK_ROOT" "$FAKE_BIN"

cat >"$FAKE_BIN/curl" <<'PY'
#!/usr/bin/env python3
import hashlib
import json
import os
import pathlib
import shutil
import sys
import urllib.parse

root = pathlib.Path(os.environ["MOCK_ROOT"])
state_path = root / "state.json"
log_path = root / "requests.jsonl"
server_root = root / "server-assets"
server_root.mkdir(parents=True, exist_ok=True)

repository = os.environ["MOCK_REPOSITORY"]
commit = os.environ["MOCK_COMMIT"]
tag_object = os.environ["MOCK_TAG_OBJECT_SHA"]
version = os.environ["MOCK_VERSION"]
artifact_id = int(os.environ["MOCK_ARTIFACT_ID"])
stage_run_id = int(os.environ["MOCK_STAGE_RUN_ID"])
release_id = int(os.environ["MOCK_RELEASE_ID"])
stage_name = os.environ["MOCK_STAGE_NAME"]
stage_sha = os.environ["MOCK_STAGE_SHA"]
stage_size = int(os.environ["MOCK_STAGE_SIZE"])
api_base = os.environ["MOCK_API_BASE"]
server_base = os.environ["MOCK_SERVER_BASE"]
api_root = f"{api_base}/repos/{repository}"
upload_root = f"https://uploads.github.com/repos/{repository}"
fault = os.environ.get("MOCK_FAULT", "")

args = sys.argv[1:]
method = "GET"
output = None
headers_output = None
data_path = None
headers = []
url = None
i = 0
value_options = {
    "--connect-timeout", "--max-time", "--request", "--output", "--write-out",
    "--dump-header", "--header", "--data-binary", "--proto",
}
flag_options = {"--silent", "--show-error", "--tlsv1.2"}
while i < len(args):
    arg = args[i]
    if arg in flag_options:
        i += 1
        continue
    if arg in value_options:
        if i + 1 >= len(args):
            raise SystemExit(90)
        value = args[i + 1]
        if arg == "--request":
            method = value
        elif arg == "--output":
            output = pathlib.Path(value)
        elif arg == "--dump-header":
            headers_output = pathlib.Path(value)
        elif arg == "--header":
            headers.append(value)
        elif arg == "--data-binary":
            if not value.startswith("@"):
                raise SystemExit(91)
            data_path = pathlib.Path(value[1:])
        i += 2
        continue
    if arg.startswith("-"):
        raise SystemExit(92)
    url = arg
    i += 1

if output is None or url is None:
    raise SystemExit(93)

auth_values = [h.split(":", 1)[1].strip() for h in headers if h.lower().startswith("authorization:")]
has_api_version = any(h.lower().startswith("x-github-api-version:") for h in headers)
is_admin = auth_values == ["Bearer admin-token"]
is_contents = auth_values == ["Bearer contents-token"]

data_kind = "none"
payload = None
if data_path is not None:
    try:
        payload = json.loads(data_path.read_text(encoding="utf-8"))
        if sorted(payload) == ["body"]:
            data_kind = "body"
        elif payload == {"draft": False, "make_latest": "true", "prerelease": False}:
            data_kind = "publish"
        else:
            data_kind = "other-json"
    except (UnicodeDecodeError, json.JSONDecodeError):
        data_kind = "binary"

with log_path.open("a", encoding="utf-8") as log:
    log.write(json.dumps({
        "method": method,
        "url": url,
        "authenticated": bool(auth_values),
        "admin": is_admin,
        "apiVersion": has_api_version,
        "dataKind": data_kind,
    }, sort_keys=True) + "\n")

if state_path.exists():
    state = json.loads(state_path.read_text(encoding="utf-8"))
else:
    state = {
        "draft": True,
        "immutable": False,
        "body": "",
        "published_at": None,
        "assets": [],
        "next_asset_id": 1000,
    }

def save_state():
    state_path.write_text(json.dumps(state, sort_keys=True), encoding="utf-8")

def write_response(code, value=None, raw=None, location=None, exit_code=0):
    output.parent.mkdir(parents=True, exist_ok=True)
    if raw is not None:
        output.write_bytes(raw)
    elif value is not None:
        output.write_text(json.dumps(value, sort_keys=True), encoding="utf-8")
    else:
        output.write_bytes(b"")
    if headers_output is not None:
        lines = [f"HTTP/1.1 {code} Mock"]
        if location is not None:
            # curl commonly serializes HTTP/2 response fields in lowercase.
            lines.append(f"location: {location}")
        headers_output.write_text("\r\n".join(lines) + "\r\n\r\n", encoding="utf-8")
    sys.stdout.write(str(code))
    raise SystemExit(exit_code)

def asset_json(asset):
    name = asset["name"]
    asset_id_value = asset["id"]
    return {
        "id": asset_id_value,
        "name": name,
        "state": "uploaded",
        "content_type": "application/octet-stream",
        "size": asset["size"],
        "digest": "sha256:" + asset["sha256"],
        "url": f"{api_root}/releases/assets/{asset_id_value}",
        "browser_download_url": f"{server_base}/{repository}/releases/download/v{version}/{name}",
    }

def release_json():
    return {
        "id": release_id,
        "url": f"{api_root}/releases/{release_id}",
        "assets_url": f"{api_root}/releases/{release_id}/assets",
        "upload_url": f"{upload_root}/releases/{release_id}/assets{{?name,label}}",
        "html_url": f"{server_base}/{repository}/releases/tag/v{version}",
        "tag_name": f"v{version}",
        "name": f"v{version}",
        "draft": state["draft"],
        "prerelease": False,
        "immutable": state["immutable"],
        "published_at": state["published_at"],
        "body": state["body"],
    }

def require_api_auth(admin=False):
    if admin:
        if not is_admin:
            write_response(403, {"message": "admin token required"})
    elif not is_contents:
        write_response(403, {"message": "contents token required"})

parsed = urllib.parse.urlsplit(url)
path = parsed.path
query = urllib.parse.parse_qs(parsed.query)

transient_api_marker = root / "transient-api-fired"
if fault == "persistent_api" and url == api_root:
    write_response(503, {"message": "persistent API failure"})
if fault == "transient_api" and url == api_root and not transient_api_marker.exists():
    transient_api_marker.touch()
    write_response(503, {"message": "transient API failure"})

if url.startswith("https://download.mock.invalid/assets/"):
    if auth_values or has_api_version:
        write_response(500, {"message": "credential leaked to redirect"})
    selected_id = int(path.rsplit("/", 1)[1])
    selected = next((item for item in state["assets"] if item["id"] == selected_id), None)
    if selected is None:
        write_response(404, {"message": "asset absent"})
    transient_download_marker = root / "transient-download-fired"
    if fault == "transient_download" and selected["name"] == "SHA256SUMS" and not transient_download_marker.exists():
        transient_download_marker.touch()
        write_response(503, {"message": "transient download failure"})
    data = (server_root / str(selected_id)).read_bytes()
    if fault in {"corrupt_download", "corrupt_public_download"} and selected["name"] == "SHA256SUMS":
        data += b"corruption"
    write_response(200, raw=data)

if url.startswith(f"{server_base}/{repository}/releases/download/v{version}/"):
    if auth_values or has_api_version:
        write_response(500, {"message": "credential sent to public URL"})
    name = urllib.parse.unquote(path.rsplit("/", 1)[1])
    selected = next((item for item in state["assets"] if item["name"] == name), None)
    if selected is None or state["draft"]:
        write_response(404, {"message": "asset not public"})
    if fault == "direct_public_download":
        write_response(200, raw=(server_root / str(selected["id"])).read_bytes())
    location = f"https://download.mock.invalid/assets/{selected['id']}"
    write_response(302, location=location)

if url.startswith(api_base) or url.startswith(upload_root):
    require_api_auth(admin=(path == f"/repos/{repository}/immutable-releases"))
    if not has_api_version:
        write_response(400, {"message": "API version absent"})

if method == "GET" and url == api_root:
    write_response(200, {
        "id": 123,
        "full_name": repository,
        "name": "termux-mcp-edge",
        "default_branch": "main",
    })

if method == "GET" and url == f"{api_root}/git/ref/heads/main":
    main_sha = "c" * 40 if fault == "wrong_main" else commit
    write_response(200, {"ref": "refs/heads/main", "object": {"type": "commit", "sha": main_sha}})

workflow_run = {
    "id": stage_run_id,
    "repository_id": 123,
    "head_repository_id": 123,
    "head_branch": "main",
    "head_sha": commit,
}
artifact = {
    "id": artifact_id,
    "name": (f"termux-mcp-server-v{version}-release-stage-{commit}"
             if fault == "full_sha_name_as_artifact" else stage_name),
    "digest": "sha256:" + (("0" * 64) if fault == "wrong_digest" else stage_sha),
    "expired": False,
    "size_in_bytes": stage_size,
    "workflow_run": workflow_run,
}

if method == "GET" and url == f"{api_root}/actions/artifacts/{artifact_id}":
    write_response(200, artifact)

if method == "GET" and url == f"{api_root}/actions/runs/{stage_run_id}":
    write_response(200, {
        "id": stage_run_id,
        "name": "Stage Release Assets",
        "path": ".github/workflows/stage-release-assets.yml",
        "event": "workflow_dispatch",
        "head_branch": "main",
        "head_sha": commit,
        "status": "completed",
        "conclusion": "success",
        "run_attempt": 2 if fault == "run_attempt_two" else 1,
        "repository": {"full_name": repository},
        "head_repository": {"full_name": repository},
    })

if method == "GET" and url == f"{api_root}/actions/runs/{stage_run_id}/artifacts?per_page=100":
    artifacts = [artifact]
    if fault == "multiple_stage_artifacts":
        artifacts.append(dict(artifact, id=artifact_id + 1, name="unexpected"))
    write_response(200, {"total_count": len(artifacts), "artifacts": artifacts})

if method == "GET" and url == f"{api_root}/git/ref/tags/v{version}":
    tag_type = "commit" if fault == "lightweight_tag" else "tag"
    write_response(200, {"ref": f"refs/tags/v{version}", "object": {"type": tag_type, "sha": tag_object}})

if method == "GET" and url == f"{api_root}/git/tags/{tag_object}":
    write_response(200, {
        "sha": tag_object,
        "tag": f"v{version}",
        "message": "Release v" + version,
        "tagger": {"name": "Release Bot", "email": "release@example.invalid", "date": "2026-07-22T00:00:00Z"},
        "object": {"type": "commit", "sha": commit},
        "verification": {"verified": True, "reason": "valid"},
    })

if method == "GET" and url == f"{api_root}/immutable-releases":
    if fault == "immutable_disabled":
        write_response(404, {"message": "immutable releases are disabled"})
    if fault == "record_race":
        raced_record = pathlib.Path(os.environ["MOCK_RECORD_RACE_PATH"])
        raced_record.write_text("occupied by race\n", encoding="utf-8")
        raced_record.chmod(0o600)
    write_response(200, {"enabled": True, "enforced_by_owner": True})

if method == "GET" and url == f"{api_root}/releases/{release_id}":
    write_response(200, release_json())

if method == "GET" and url == f"{api_root}/releases/{release_id}/assets?per_page=100":
    write_response(200, [asset_json(item) for item in state["assets"]])

if method == "GET" and path == f"/repos/{repository}/releases" and query.get("per_page") == ["100"]:
    page = int(query.get("page", ["1"])[0])
    write_response(200, [release_json()] if page == 1 else [])

if method == "GET" and url == f"{api_root}/releases/tags/v{version}":
    write_response(200 if not state["draft"] else 404, release_json() if not state["draft"] else {"message": "not found"})

if method == "GET" and url == f"{api_root}/releases/latest":
    write_response(200 if not state["draft"] else 404, release_json() if not state["draft"] else {"message": "not found"})

if method == "GET" and path.startswith(f"/repos/{repository}/releases/assets/"):
    selected_id = int(path.rsplit("/", 1)[1])
    selected = next((item for item in state["assets"] if item["id"] == selected_id), None)
    if selected is None:
        write_response(404, {"message": "asset absent"})
    if fault == "direct_asset_download":
        write_response(200, raw=(server_root / str(selected_id)).read_bytes())
    write_response(302, location=f"https://download.mock.invalid/assets/{selected_id}")

if method == "PATCH" and url == f"{api_root}/releases/{release_id}":
    if data_kind == "body" and payload is not None:
        if not state["draft"] or state["body"] or state["assets"]:
            write_response(409, {"message": "not virgin"})
        state["body"] = payload["body"]
        save_state()
        write_response(200, release_json())
    if data_kind == "publish":
        if fault == "publish_timeout_draft":
            write_response(000, exit_code=28)
        state["draft"] = False
        state["immutable"] = fault != "publish_not_immutable"
        state["published_at"] = "2026-07-22T02:00:00Z"
        save_state()
        if fault == "publish_timeout":
            write_response(000, exit_code=28)
        write_response(200, release_json())
    write_response(422, {"message": "mutation payload broadened"})

if method == "POST" and url.startswith(f"{upload_root}/releases/{release_id}/assets?"):
    if data_kind != "binary" or data_path is None or query.get("name") is None:
        write_response(422, {"message": "invalid upload"})
    if not state["draft"] or not state["body"]:
        write_response(409, {"message": "unbound draft"})
    name = query["name"][0]
    if any(item["name"] == name for item in state["assets"]):
        write_response(422, {"message": "duplicate asset"})
    data = data_path.read_bytes()
    selected_id = state["next_asset_id"]
    state["next_asset_id"] += 1
    selected = {
        "id": selected_id,
        "name": name,
        "size": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    }
    state["assets"].append(selected)
    (server_root / str(selected_id)).write_bytes(data)
    save_state()
    response = asset_json(selected)
    if fault == "bad_upload_response" and len(state["assets"]) == 1:
        response["digest"] = "sha256:" + ("0" * 64)
    write_response(201, response)

write_response(404, {"message": f"unhandled mock route: {method} {url}"})
PY
chmod 700 "$FAKE_BIN/curl"

COMMON_ENV=(
  "PATH=$FAKE_BIN:$REAL_PATH"
  "MOCK_ROOT=$MOCK_ROOT"
  "MOCK_REPOSITORY=$REPOSITORY"
  "MOCK_COMMIT=$COMMIT"
  "MOCK_TAG_OBJECT_SHA=$TAG_OBJECT_SHA"
  "MOCK_VERSION=$VERSION"
  "MOCK_ARTIFACT_ID=$ARTIFACT_ID"
  "MOCK_STAGE_RUN_ID=$STAGE_RUN_ID"
  "MOCK_RELEASE_ID=$RELEASE_ID"
  "MOCK_STAGE_NAME=$STAGE_NAME"
  "MOCK_STAGE_SHA=$STAGE_SHA"
  "MOCK_STAGE_SIZE=$STAGE_SIZE"
  "MOCK_RECORD_RACE_PATH=$ATTACH_RECORD"
  "MOCK_API_BASE=https://api.mock.invalid"
  "MOCK_SERVER_BASE=https://github.mock.invalid"
  "GITHUB_API_URL=https://api.mock.invalid"
  "GITHUB_SERVER_URL=https://github.mock.invalid"
  "GH_TOKEN=contents-token"
  "GH_ADMIN_READ_TOKEN=admin-token"
  "GITHUB_RUN_ID=8801"
  "GITHUB_RUN_ATTEMPT=1"
)

resolve_args=(
  --repository "$REPOSITORY"
  --commit "$COMMIT"
  --version "$VERSION"
  --staged-artifact-id "$ARTIFACT_ID"
  --staged-artifact-sha256 "$STAGE_SHA"
)
full_args=(
  "${resolve_args[@]}"
  --tag-object-sha "$TAG_OBJECT_SHA"
  --draft-release-id "$RELEASE_ID"
  --assets-dir "$ASSETS_DIR"
  --receipt "$RECEIPT"
)

reset_mock() {
  rm -f -- \
    "$MOCK_ROOT/state.json" \
    "$MOCK_ROOT/requests.jsonl" \
    "$MOCK_ROOT/transient-api-fired" \
    "$MOCK_ROOT/transient-download-fired" \
    "$ATTACH_RECORD" \
    "$VERIFY_RECORD"
  find "$MOCK_ROOT/server-assets" -mindepth 1 -maxdepth 1 -type f -delete 2>/dev/null || true
}

run_mode() {
  local mode="$1" fault="${2:-}"
  local -a record_args=()
  shift 2 || true
  case "$mode" in
    attach) record_args=(--record "$ATTACH_RECORD") ;;
    verify) record_args=(--record "$VERIFY_RECORD") ;;
    publish) record_args=(--verification-record "$VERIFY_RECORD") ;;
  esac
  env "${COMMON_ENV[@]}" "MOCK_FAULT=$fault" \
    bash "$SCRIPT" "$mode" "${full_args[@]}" "${record_args[@]}" "$@"
}

run_resolve() {
  local fault="${1:-}"
  env "${COMMON_ENV[@]}" "MOCK_FAULT=$fault" \
    bash "$SCRIPT" resolve-stage "${resolve_args[@]}"
}

expect_failure() {
  local mode="$1" fault="$2" expected="$3"
  if run_mode "$mode" "$fault" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "$mode unexpectedly succeeded for fault $fault"
  fi
  assert_contains "$expected" "$ROOT/last.stderr"
}

expect_resolve_failure() {
  local fault="$1" expected="$2"
  if run_resolve "$fault" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "resolve-stage unexpectedly succeeded for fault $fault"
  fi
  assert_contains "$expected" "$ROOT/last.stderr"
}

# The resolver emits one machine-readable line and binds the artifact to the
# exact successful first-attempt staging run on current main.
reset_mock
[[ "$(run_resolve)" == "stage_run_id=$STAGE_RUN_ID" ]] \
  || fail_test resolve_stage_output_changed
[[ "$(run_resolve transient_api)" == "stage_run_id=$STAGE_RUN_ID" ]] \
  || fail_test transient_api_read_was_not_retried
[[ -f "$MOCK_ROOT/transient-api-fired" ]] || fail_test transient_api_fault_was_not_exercised
reset_mock
expect_resolve_failure persistent_api api_get_status_invalid
[[ "$(jq -s --arg url "https://api.mock.invalid/repos/$REPOSITORY" '[.[] | select(.method == "GET" and .url == $url)] | length' "$MOCK_ROOT/requests.jsonl")" == 3 ]] \
  || fail_test read_retry_bound_changed
expect_resolve_failure wrong_digest staged_artifact_identity_mismatch
expect_resolve_failure full_sha_name_as_artifact staged_artifact_identity_mismatch
expect_resolve_failure run_attempt_two staging_run_identity_mismatch
expect_resolve_failure multiple_stage_artifacts staging_run_artifact_set_mismatch

# A virgin draft can be preflighted, bound once, independently verified, and
# then published once. The simulated lost publish response must be resolved by
# readback, not by retrying the mutation.
reset_mock
run_mode preflight "" >"$ROOT/preflight.stdout" 2>"$ROOT/preflight.stderr"
assert_full_output "$ROOT/preflight.stdout"
run_mode attach "" >"$ROOT/attach.stdout" 2>"$ROOT/attach.stderr"
assert_full_output "$ROOT/attach.stdout"
assert_identity_record "$ATTACH_RECORD" attach
[[ "$(jq -s '[.[] | select(.method == "POST" and (.url | contains("/assets?name=")))] | length' "$MOCK_ROOT/requests.jsonl")" == 16 ]] \
  || fail_test attach_did_not_upload_exactly_sixteen_assets
[[ "$(jq -s '[.[] | select(.method == "PATCH" and .dataKind == "body")] | length' "$MOCK_ROOT/requests.jsonl")" == 1 ]] \
  || fail_test release_body_was_not_bound_exactly_once
[[ "$(jq -s '[.[] | select(.method == "DELETE")] | length' "$MOCK_ROOT/requests.jsonl")" == 0 ]] \
  || fail_test destructive_request_detected
[[ "$(jq -s '[.[] | select(.url | startswith("https://download.mock.invalid/")) | select(.authenticated or .apiVersion)] | length' "$MOCK_ROOT/requests.jsonl")" == 0 ]] \
  || fail_test redirect_received_github_credentials

patches_before="$(jq -s '[.[] | select(.method == "PATCH")] | length' "$MOCK_ROOT/requests.jsonl")"
expect_failure publish "" verification_record_invalid
[[ "$(jq -s '[.[] | select(.method == "PATCH")] | length' "$MOCK_ROOT/requests.jsonl")" == "$patches_before" ]] \
  || fail_test missing_verification_record_reached_publish_mutation

posts_before="$(jq -s '[.[] | select(.method == "POST")] | length' "$MOCK_ROOT/requests.jsonl")"
patches_before="$(jq -s '[.[] | select(.method == "PATCH")] | length' "$MOCK_ROOT/requests.jsonl")"
rm -f -- "$ATTACH_RECORD"
expect_failure attach "" draft_release_not_virgin
[[ "$(jq -s '[.[] | select(.method == "POST")] | length' "$MOCK_ROOT/requests.jsonl")" == "$posts_before" ]] \
  || fail_test attach_rerun_mutated_assets
[[ "$(jq -s '[.[] | select(.method == "PATCH")] | length' "$MOCK_ROOT/requests.jsonl")" == "$patches_before" ]] \
  || fail_test attach_rerun_mutated_release

run_mode verify transient_download >"$ROOT/verify.stdout" 2>"$ROOT/verify.stderr"
assert_full_output "$ROOT/verify.stdout"
assert_identity_record "$VERIFY_RECORD" verify
[[ -f "$MOCK_ROOT/transient-download-fired" ]] || fail_test transient_download_fault_was_not_exercised
rm -f -- "$VERIFY_RECORD"
run_mode verify direct_asset_download >"$ROOT/verify-direct.stdout" 2>"$ROOT/verify-direct.stderr"
assert_full_output "$ROOT/verify-direct.stdout"
assert_identity_record "$VERIFY_RECORD" verify
run_mode publish publish_timeout >"$ROOT/publish.stdout" 2>"$ROOT/publish.stderr"
assert_full_output "$ROOT/publish.stdout"
assert_contains "publish response ambiguous; verifying exact readback" "$ROOT/publish.stderr"
[[ "$(jq -s '[.[] | select(.method == "PATCH" and .dataKind == "publish")] | length' "$MOCK_ROOT/requests.jsonl")" == 1 ]] \
  || fail_test ambiguous_publish_was_retried
[[ "$(jq -r '.draft == false and .immutable == true' "$MOCK_ROOT/state.json")" == true ]] \
  || fail_test publication_state_not_immutable
expect_failure publish "" draft_release_binding_mismatch
[[ "$(jq -s '[.[] | select(.method == "PATCH" and .dataKind == "publish")] | length' "$MOCK_ROOT/requests.jsonl")" == 1 ]] \
  || fail_test publish_rerun_mutated_release
run_mode postverify direct_public_download >"$ROOT/postverify.stdout" 2>"$ROOT/postverify.stderr"
assert_full_output "$ROOT/postverify.stdout"

# Fail closed before mutation on identity, tag, and policy mismatches.
reset_mock
expect_failure preflight lightweight_tag annotated_tag_ref_mismatch
[[ ! -e "$MOCK_ROOT/state.json" ]] || fail_test tag_failure_changed_release_state
reset_mock
expect_failure attach immutable_disabled immutable_release_policy_disabled
[[ ! -e "$MOCK_ROOT/state.json" ]] || fail_test policy_failure_changed_release_state
reset_mock
expect_failure attach record_race identity_record_publish_raced
[[ "$(<"$ATTACH_RECORD")" == "occupied by race" ]] \
  || fail_test raced_identity_record_was_overwritten
[[ "$(stat -c '%a' -- "$ATTACH_RECORD")" == 600 ]] \
  || fail_test raced_identity_record_mode_changed
reset_mock
if env "${COMMON_ENV[@]}" GH_ADMIN_READ_TOKEN=contents-token MOCK_FAULT="" \
  bash "$SCRIPT" attach "${full_args[@]}" --record "$ATTACH_RECORD" \
    >"$ROOT/same-token.stdout" 2>"$ROOT/same-token.stderr"; then
  fail_test same_token_unexpectedly_accepted
fi
assert_contains admin_read_token_not_separate "$ROOT/same-token.stderr"

# An upload response mismatch leaves a non-resumable partial draft and never
# triggers a retry, delete, or implicit cleanup mutation.
reset_mock
expect_failure attach bad_upload_response asset_upload_response_mismatch
[[ "$(jq -s '[.[] | select(.method == "POST")] | length' "$MOCK_ROOT/requests.jsonl")" == 1 ]] \
  || fail_test failed_upload_was_retried
[[ "$(jq -s '[.[] | select(.method == "DELETE")] | length' "$MOCK_ROOT/requests.jsonl")" == 0 ]] \
  || fail_test failed_upload_triggered_delete
expect_failure attach "" draft_release_not_virgin

# Every remote byte is downloaded and rehashed. Corruption and an ambiguous
# publish that remained a draft are both hard failures.
reset_mock
run_mode attach "" >"$ROOT/attach-corruption-fixture.stdout" 2>"$ROOT/attach-corruption-fixture.stderr"
expect_failure verify corrupt_download downloaded_asset_size_mismatch

reset_mock
run_mode attach "" >"$ROOT/attach-timeout-fixture.stdout" 2>"$ROOT/attach-timeout-fixture.stderr"
run_mode verify "" >"$ROOT/verify-timeout-fixture.stdout" 2>"$ROOT/verify-timeout-fixture.stderr"
expect_failure publish publish_timeout_draft published_release_identity_mismatch
[[ "$(jq -s '[.[] | select(.method == "PATCH" and .dataKind == "publish")] | length' "$MOCK_ROOT/requests.jsonl")" == 1 ]] \
  || fail_test failed_ambiguous_publish_was_retried
[[ "$(jq -r '.draft' "$MOCK_ROOT/state.json")" == true ]] \
  || fail_test failed_ambiguous_publish_changed_mock_state

# The final job must consume the exact same-run independent-verification
# record. A server-identity change in that otherwise valid JSON fails before
# the single publish mutation.
reset_mock
run_mode attach "" >"$ROOT/attach-record-fixture.stdout" 2>"$ROOT/attach-record-fixture.stderr"
run_mode verify "" >"$ROOT/verify-record-fixture.stdout" 2>"$ROOT/verify-record-fixture.stderr"
jq '.release.assets[0].id += 1' "$VERIFY_RECORD" >"$ROOT/tampered-record.json"
mv -T -- "$ROOT/tampered-record.json" "$VERIFY_RECORD"
chmod 600 "$VERIFY_RECORD"
patches_before="$(jq -s '[.[] | select(.method == "PATCH")] | length' "$MOCK_ROOT/requests.jsonl")"
expect_failure publish "" draft_verification_record_mismatch
[[ "$(jq -s '[.[] | select(.method == "PATCH")] | length' "$MOCK_ROOT/requests.jsonl")" == "$patches_before" ]] \
  || fail_test tampered_verification_record_reached_publish_mutation

# Static mutation surface: only asset uploads and the two exact Release PATCH
# payloads are present. Redirects are intentionally handled without -L.
if grep -Eq -- '(^|[[:space:]])DELETE([[:space:]]|$)|/git/refs|target_commitish|--clobber|curl[[:space:]].*(-L|--location)' "$SCRIPT"; then
  fail_test forbidden_mutation_or_automatic_redirect_present
fi
[[ "$(grep -Fc 'request POST "$GH_TOKEN"' "$SCRIPT")" == 1 ]] \
  || fail_test upload_post_surface_changed
grep -Fq "'{body:\$body}'" "$SCRIPT" || fail_test body_patch_payload_changed
grep -Fq "'{draft:false,prerelease:false,make_latest:\"true\"}'" "$SCRIPT" \
  || fail_test publish_patch_payload_changed
grep -Fq 'mv -T -n -- "$temporary" "$RECORD"' "$SCRIPT" \
  || fail_test identity_record_publish_must_be_no_clobber
grep -Fq '[[ ! -e "$temporary" && ! -L "$temporary" ]]' "$SCRIPT" \
  || fail_test identity_record_publish_race_check_missing

printf 'GitHub Release publication state-machine tests passed\n'
