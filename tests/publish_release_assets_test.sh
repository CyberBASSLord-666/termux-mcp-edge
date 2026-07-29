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
REAL_MV="$(command -v mv)"
REPOSITORY="CyberBASSLord-666/termux-mcp-edge"
COMMIT="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
TAG_OBJECT_SHA="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
VERSION="0.6.0"
ARTIFACT_ID="55"
STAGE_RUN_ID="77"
RELEASE_ID="99"
QUALIFICATION_RUN_ID="5104"
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
      keys == ["phase","qualification","recordType","release","repository","schemaVersion","source","stage","workflow"]
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
      and (.qualification | keys == ["claimBoundary","class","runId"])
      and .qualification.class == "official_termux_native_automated_v1"
      and .qualification.runId == "5104"
      and .qualification.claimBoundary == {
        physicalDeviceObserved:false,
        androidFrameworkObserved:false,
        sustainedPhysicalSoak:false,
        physicalCertification:"not_run"
      }
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
grep -Fq 'reverified_bundle="$reverified_root/publication-inputs"' "$SCRIPT" \
  || fail_test publisher_reverification_does_not_use_an_absent_atomic_bundle
grep -Fq 'reverified_assets="$reverified_bundle/assets"' "$SCRIPT" \
  || fail_test publisher_reverification_assets_escape_atomic_bundle
grep -Fq 'reverified_receipt="$reverified_bundle/release-publication-receipt-v1.json"' "$SCRIPT" \
  || fail_test publisher_reverification_receipt_escapes_atomic_bundle
if grep -Fq 'mkdir -m 700 -- "$reverified_bundle"' "$SCRIPT"; then
  fail_test publisher_precreates_atomic_bundle_destination
fi
grep -Fq '2147483647' "$SCRIPT" \
  || fail_test strict_release_asset_size_limit_missing
grep -Fq '1610612736' "$SCRIPT" \
  || fail_test retained_runtime_archive_limit_missing
grep -Fq '16777216' "$SCRIPT" \
  || fail_test retained_runtime_json_limit_missing
for runtime_name in \
  evidence/runtime/termux-qualified-runtime-image-v1.tar.gz \
  evidence/runtime/termux-runtime-package-lock-v1.json \
  evidence/runtime/termux-runtime-snapshot-v1.json \
  evidence/runtime/termux-runtime-snapshot-replay-v1.json
do
  grep -Fq "$runtime_name" "$SCRIPT" \
    || fail_test "publisher retained-runtime record missing: $runtime_name"
done

PUBLICATION_ROOT="$ROOT/publication"
ASSETS_DIR="$PUBLICATION_ROOT/assets"
RECEIPT="$PUBLICATION_ROOT/release-publication-receipt-v1.json"
ATTACH_RECORD="$PUBLICATION_ROOT/release-attachment-record-v1.json"
VERIFY_RECORD="$PUBLICATION_ROOT/release-draft-verification-record-v1.json"
PREP_FIXTURE_EXPORT_DIR="$PUBLICATION_ROOT" \
  bash "$REPO_ROOT/tests/prepare_release_publication_assets_test.sh" \
  >"$ROOT/fixture-export.stdout"
[[ -d "$ASSETS_DIR" && ! -L "$ASSETS_DIR" && -f "$RECEIPT" && ! -L "$RECEIPT" ]] \
  || fail_test canonical_publication_fixture_missing
STAGE_SHA="$(sha256sum -- "$ASSETS_DIR/$STAGE_NAME" | awk '{print $1}')"
STAGE_SIZE="$(stat -c '%s' -- "$ASSETS_DIR/$STAGE_NAME")"

make_publication_case() {
  local name="$1" case_root
  case_root="$ROOT/input-cases/$name"
  mkdir -m 700 -p -- "$case_root"
  cp -a -- "$ASSETS_DIR" "$case_root/assets"
  cp -p -- "$RECEIPT" "$case_root/release-publication-receipt-v1.json"
  printf '%s\n' "$case_root"
}

extract_case_stage() {
  local case_root="$1" payload="$case_root/stage-payload"
  local stage="$case_root/assets/$STAGE_NAME"
  rm -rf -- "$payload"
  mkdir -m 700 -- "$payload"
  tar -xf "$stage" -C "$payload"
}

repack_case_stage() {
  local case_root="$1" payload="$case_root/stage-payload"
  local stage="$case_root/assets/$STAGE_NAME"
  rm -- "$stage"
  tar --format=gnu --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
    --mode='u+rwX,go+rX,go-w' -C "$payload" -cf "$stage" .
  chmod 600 "$stage"
}

refresh_case_stage_receipt() {
  local case_root="$1" receipt="$case_root/release-publication-receipt-v1.json"
  local stage="$case_root/assets/$STAGE_NAME" digest bytes
  digest="$(sha256sum "$stage" | awk '{print $1}')"
  bytes="$(stat -c '%s' "$stage")"
  jq --arg name "$STAGE_NAME" --arg sha "$digest" --argjson bytes "$bytes" '
    .stageTar.sha256 = $sha
    | .stageTar.size = $bytes
    | (.assets[] | select(.name == $name) | .sha256) = $sha
    | (.assets[] | select(.name == $name) | .size) = $bytes
  ' "$receipt" >"$case_root/receipt.next"
  mv "$case_root/receipt.next" "$receipt"
  chmod 600 "$receipt"
  printf '%s\n' "$digest"
}

MOCK_ROOT="$ROOT/mock"
FAKE_BIN="$ROOT/fake-bin"
mkdir -m 700 -p -- "$MOCK_ROOT" "$FAKE_BIN"

cat >"$FAKE_BIN/file" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
printf '%s\n' 'ELF 64-bit LSB pie executable, ARM aarch64, interpreter /system/bin/linker64, for Android 24'
EOF
chmod 700 "$FAKE_BIN/file"

cat >"$FAKE_BIN/mv" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
target="${@: -1}"
"$PUBLISH_REAL_MV" "$@"
if [[ "${MOCK_FAULT:-}" == record_post_move_replacement \
  && "$target" == "$MOCK_RECORD_RACE_PATH" \
  && ! -e "$MOCK_RECORD_POST_MOVE_MARKER" ]]
then
  : >"$MOCK_RECORD_POST_MOVE_MARKER"
  rm -f -- "$target"
  printf 'replaced after move\n' >"$target"
  chmod 600 "$target"
fi
EOF
chmod 700 "$FAKE_BIN/mv"

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
ci_run_id = int(os.environ["MOCK_CI_RUN_ID"])
security_run_id = int(os.environ["MOCK_SECURITY_RUN_ID"])
android_run_id = int(os.environ["MOCK_ANDROID_RUN_ID"])
qualification_run_id = int(os.environ["MOCK_QUALIFICATION_RUN_ID"])
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

push_runs = {
    ci_run_id: ("CI", ".github/workflows/ci.yml"),
    security_run_id: ("Security", ".github/workflows/security.yml"),
    android_run_id: ("Android Cross Compile", ".github/workflows/android-cross-compile.yml"),
}

def push_run(
    run_id,
    name,
    path,
    *,
    conclusion="success",
    attempt=1,
    created_at="2026-07-23T00:00:00Z",
    run_started_at="2026-07-23T00:00:01Z",
):
    return {
        "id": run_id,
        "name": name,
        "path": path,
        "event": "push",
        "head_branch": "main",
        "head_sha": commit,
        "status": "completed",
        "conclusion": conclusion,
        "run_attempt": attempt,
        "created_at": created_at,
        "run_started_at": run_started_at,
        "repository": {"full_name": repository},
        "head_repository": {"full_name": repository},
    }

for upstream_run_id, (upstream_name, workflow_path) in push_runs.items():
    workflow_file = workflow_path.rsplit("/", 1)[-1]
    if method == "GET" and url == f"{api_root}/actions/runs/{upstream_run_id}":
        attempt = 2 if fault == "upstream_attempt_two" and upstream_run_id == ci_run_id else 1
        write_response(
            200,
            push_run(upstream_run_id, upstream_name, workflow_path, attempt=attempt),
        )
    list_url = (
        f"{api_root}/actions/workflows/{workflow_file}/runs"
        f"?branch=main&event=push&head_sha={commit}&per_page=100"
    )
    if method == "GET" and url == list_url:
        runs = [push_run(upstream_run_id, upstream_name, workflow_path)]
        if (
            upstream_run_id == ci_run_id
            and (
                fault == "newer_ci_failure"
                or (fault == "newer_ci_failure_after_body" and state["body"] != "")
            )
        ):
            runs.insert(
                0,
                push_run(
                    upstream_run_id + 10000,
                    upstream_name,
                    workflow_path,
                    conclusion="failure",
                    created_at="2026-07-23T00:01:00Z",
                    run_started_at="2026-07-23T00:01:01Z",
                ),
            )
        write_response(200, {"total_count": len(runs), "workflow_runs": runs})

qualification_title = f"Qualify Android run {android_run_id} at {commit}"
qualification_run = {
    "id": qualification_run_id,
    "name": "Automated Release Qualification",
    "display_title": qualification_title,
    "path": ".github/workflows/automated-release-qualification.yml",
    "event": "workflow_run",
    "head_branch": "main",
    "head_sha": commit,
    "status": "completed",
    "conclusion": "success",
    "run_attempt": 1,
    "created_at": "2026-07-23T00:02:00Z",
    "run_started_at": "2026-07-23T00:02:01Z",
    "repository": {"full_name": repository},
    "head_repository": {"full_name": repository},
}
if method == "GET" and url == f"{api_root}/actions/runs/{qualification_run_id}":
    write_response(200, qualification_run)
if method == "GET" and url == (
    f"{api_root}/actions/workflows/automated-release-qualification.yml/runs"
    f"?branch=main&event=workflow_run&head_sha={commit}&per_page=100"
):
    runs = [qualification_run]
    if fault == "newer_qualification_failure":
        runs.insert(
            0,
            dict(
                qualification_run,
                id=qualification_run_id + 10000,
                conclusion="failure",
                created_at="2026-07-23T00:03:00Z",
                run_started_at="2026-07-23T00:03:01Z",
            ),
        )
    write_response(200, {"total_count": len(runs), "workflow_runs": runs})

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
  "MOCK_CI_RUN_ID=5101"
  "MOCK_SECURITY_RUN_ID=5102"
  "MOCK_ANDROID_RUN_ID=5103"
  "MOCK_QUALIFICATION_RUN_ID=$QUALIFICATION_RUN_ID"
  "MOCK_RELEASE_ID=$RELEASE_ID"
  "MOCK_STAGE_NAME=$STAGE_NAME"
  "MOCK_STAGE_SHA=$STAGE_SHA"
  "MOCK_STAGE_SIZE=$STAGE_SIZE"
  "MOCK_RECORD_RACE_PATH=$ATTACH_RECORD"
  "MOCK_RECORD_POST_MOVE_MARKER=$MOCK_ROOT/record-post-move-fired"
  "PUBLISH_REAL_MV=$REAL_MV"
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
    "$MOCK_ROOT/record-post-move-fired" \
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
  local fault="${1:-}" stage_size="${2:-$STAGE_SIZE}"
  env "${COMMON_ENV[@]}" "MOCK_FAULT=$fault" "MOCK_STAGE_SIZE=$stage_size" \
    bash "$SCRIPT" resolve-stage "${resolve_args[@]}"
}

run_case_preflight() {
  local case_root="$1" stage_sha="$2" selected_path="${3:-$FAKE_BIN:$REAL_PATH}"
  local stage_size
  stage_size="$(stat -c '%s' "$case_root/assets/$STAGE_NAME")"
  env "${COMMON_ENV[@]}" \
    "PATH=$selected_path" \
    "MOCK_FAULT=" \
    "MOCK_STAGE_SHA=$stage_sha" \
    "MOCK_STAGE_SIZE=$stage_size" \
    "PUBLISH_REAL_CP=${PUBLISH_REAL_CP:-}" \
    "PUBLISH_RACE_SOURCE=${PUBLISH_RACE_SOURCE:-}" \
    "PUBLISH_RACE_MARKER=${PUBLISH_RACE_MARKER:-}" \
    bash "$SCRIPT" preflight \
      --repository "$REPOSITORY" \
      --commit "$COMMIT" \
      --version "$VERSION" \
      --staged-artifact-id "$ARTIFACT_ID" \
      --staged-artifact-sha256 "$stage_sha" \
      --tag-object-sha "$TAG_OBJECT_SHA" \
      --draft-release-id "$RELEASE_ID" \
      --assets-dir "$case_root/assets" \
      --receipt "$case_root/release-publication-receipt-v1.json"
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

# The publisher independently replays the canonical preparer over a private
# stage snapshot. Duplicate manifests, archive/member drift, and receipt-only
# asset substitutions fail before any Release mutation.
case_root="$(make_publication_case duplicate-staging-manifest-key)"
extract_case_stage "$case_root"
manifest="$case_root/stage-payload/release-staging-manifest-v2.json"
sed '1s/^{/{"schemaVersion":2,/' "$manifest" >"$case_root/manifest.next"
mv "$case_root/manifest.next" "$manifest"
repack_case_stage "$case_root"
case_stage_sha="$(refresh_case_stage_receipt "$case_root")"
reset_mock
if run_case_preflight "$case_root" "$case_stage_sha" \
  >"$ROOT/duplicate-manifest.stdout" 2>"$ROOT/duplicate-manifest.stderr"
then
  fail_test duplicate_staging_manifest_unexpectedly_accepted
fi
assert_contains publication_inputs_reverification_failed "$ROOT/duplicate-manifest.stderr"

case_root="$(make_publication_case qualification-run-substitution)"
extract_case_stage "$case_root"
manifest="$case_root/stage-payload/release-staging-manifest-v2.json"
jq '.workflowRuns.qualification = "9999"' "$manifest" >"$case_root/manifest.next"
mv "$case_root/manifest.next" "$manifest"
repack_case_stage "$case_root"
case_stage_sha="$(refresh_case_stage_receipt "$case_root")"
reset_mock
if run_case_preflight "$case_root" "$case_stage_sha" \
  >"$ROOT/qualification-run-substitution.stdout" \
  2>"$ROOT/qualification-run-substitution.stderr"
then
  fail_test qualification_run_substitution_unexpectedly_accepted
fi
assert_contains publication_inputs_reverification_failed \
  "$ROOT/qualification-run-substitution.stderr"

case_root="$(make_publication_case staged-member-byte-drift)"
extract_case_stage "$case_root"
drift_name="termux-mcp-server-v${VERSION}-aarch64-linux-android-default"
printf 'stage-only replacement\n' >>"$case_root/stage-payload/$drift_name"
repack_case_stage "$case_root"
case_stage_sha="$(refresh_case_stage_receipt "$case_root")"
reset_mock
if run_case_preflight "$case_root" "$case_stage_sha" \
  >"$ROOT/staged-member-drift.stdout" 2>"$ROOT/staged-member-drift.stderr"
then
  fail_test staged_member_byte_drift_unexpectedly_accepted
fi
assert_contains publication_inputs_reverification_failed "$ROOT/staged-member-drift.stderr"

case_root="$(make_publication_case retained-runtime-archive-byte-drift)"
extract_case_stage "$case_root"
printf 'stage-only runtime replacement\n' \
  >>"$case_root/stage-payload/evidence/runtime/termux-qualified-runtime-image-v1.tar.gz"
repack_case_stage "$case_root"
case_stage_sha="$(refresh_case_stage_receipt "$case_root")"
reset_mock
if run_case_preflight "$case_root" "$case_stage_sha" \
  >"$ROOT/runtime-archive-drift.stdout" 2>"$ROOT/runtime-archive-drift.stderr"
then
  fail_test retained_runtime_archive_byte_drift_unexpectedly_accepted
fi
assert_contains publication_inputs_reverification_failed "$ROOT/runtime-archive-drift.stderr"

case_root="$(make_publication_case retained-runtime-replay-claim-drift)"
extract_case_stage "$case_root"
runtime_replay="$case_root/stage-payload/evidence/runtime/termux-runtime-snapshot-replay-v1.json"
jq '.verification.runtimeNetworkAccess = true' "$runtime_replay" \
  >"$case_root/runtime-replay.next"
mv "$case_root/runtime-replay.next" "$runtime_replay"
runtime_replay_sha="$(sha256sum "$runtime_replay" | awk '{print $1}')"
runtime_replay_bytes="$(stat -c '%s' "$runtime_replay")"
manifest="$case_root/stage-payload/release-staging-manifest-v2.json"
jq --arg sha "$runtime_replay_sha" --argjson bytes "$runtime_replay_bytes" '
  .evidence.runtime.replay.sha256 = $sha
  | .evidence.runtime.replay.bytes = $bytes
' "$manifest" >"$case_root/manifest.next"
mv "$case_root/manifest.next" "$manifest"
repack_case_stage "$case_root"
case_stage_sha="$(refresh_case_stage_receipt "$case_root")"
reset_mock
if run_case_preflight "$case_root" "$case_stage_sha" \
  >"$ROOT/runtime-replay-drift.stdout" 2>"$ROOT/runtime-replay-drift.stderr"
then
  fail_test retained_runtime_replay_claim_drift_unexpectedly_accepted
fi
assert_contains publication_inputs_reverification_failed "$ROOT/runtime-replay-drift.stderr"

case_root="$(make_publication_case receipt-source-member-substitution)"
receipt="$case_root/release-publication-receipt-v1.json"
jq --arg name "termux-mcp-server-v${VERSION}-aarch64-linux-android-default" '
  (.assets[] | select(.name == $name) | .sourceStageMember) = "SHA256SUMS"
' "$receipt" >"$case_root/receipt.next"
mv "$case_root/receipt.next" "$receipt"
chmod 600 "$receipt"
reset_mock
if run_case_preflight "$case_root" "$STAGE_SHA" \
  >"$ROOT/receipt-source.stdout" 2>"$ROOT/receipt-source.stderr"
then
  fail_test receipt_source_member_substitution_unexpectedly_accepted
fi
assert_contains receipt_contract_mismatch "$ROOT/receipt-source.stderr"

case_root="$(make_publication_case local-asset-stage-substitution)"
name="termux-mcp-server-v${VERSION}-aarch64-linux-android-default"
printf 'local-only replacement\n' >>"$case_root/assets/$name"
replacement_sha="$(sha256sum "$case_root/assets/$name" | awk '{print $1}')"
replacement_size="$(stat -c '%s' "$case_root/assets/$name")"
receipt="$case_root/release-publication-receipt-v1.json"
jq --arg name "$name" --arg sha "$replacement_sha" --argjson size "$replacement_size" '
  (.assets[] | select(.name == $name) | .sha256) = $sha
  | (.assets[] | select(.name == $name) | .size) = $size
' "$receipt" >"$case_root/receipt.next"
mv "$case_root/receipt.next" "$receipt"
chmod 600 "$receipt"
reset_mock
if run_case_preflight "$case_root" "$STAGE_SHA" \
  >"$ROOT/local-stage-substitution.stdout" 2>"$ROOT/local-stage-substitution.stderr"
then
  fail_test local_asset_stage_substitution_unexpectedly_accepted
fi
assert_contains publication_receipt_stage_join_mismatch "$ROOT/local-stage-substitution.stderr"

case_root="$(make_publication_case duplicate-receipt-key)"
receipt="$case_root/release-publication-receipt-v1.json"
sed '1s/^{/{"schemaVersion":1,/' "$receipt" >"$case_root/receipt.next"
mv "$case_root/receipt.next" "$receipt"
chmod 600 "$receipt"
reset_mock
if run_case_preflight "$case_root" "$STAGE_SHA" \
  >"$ROOT/duplicate-receipt.stdout" 2>"$ROOT/duplicate-receipt.stderr"
then
  fail_test duplicate_receipt_key_unexpectedly_accepted
fi
assert_contains receipt_json_invalid "$ROOT/duplicate-receipt.stderr"

mkdir -m 700 "$ROOT/racing-cp"
cat >"$ROOT/racing-cp/cp" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
source_path="${@: -2:1}"
"$PUBLISH_REAL_CP" "$@"
if [[ "$source_path" == "$PUBLISH_RACE_SOURCE" && ! -e "$PUBLISH_RACE_MARKER" ]]; then
  : >"$PUBLISH_RACE_MARKER"
  printf 'replaced after private snapshot\n' >>"$source_path"
fi
EOF
chmod 700 "$ROOT/racing-cp/cp"

case_root="$(make_publication_case local-asset-snapshot-race)"
race_source="$case_root/assets/termux-mcp-server-v${VERSION}-aarch64-linux-android-default"
reset_mock
PUBLISH_REAL_CP="$(command -v cp)" \
PUBLISH_RACE_SOURCE="$race_source" \
PUBLISH_RACE_MARKER="$ROOT/asset-race-fired" \
  run_case_preflight "$case_root" "$STAGE_SHA" "$ROOT/racing-cp:$FAKE_BIN:$REAL_PATH" \
  >"$ROOT/asset-race.stdout" 2>"$ROOT/asset-race.stderr"
assert_full_output "$ROOT/asset-race.stdout"
[[ -f "$ROOT/asset-race-fired" ]] || fail_test local_asset_snapshot_race_not_exercised
grep -Fq 'replaced after private snapshot' "$race_source" \
  || fail_test local_asset_snapshot_race_did_not_replace_source

case_root="$(make_publication_case receipt-snapshot-race)"
race_source="$case_root/release-publication-receipt-v1.json"
reset_mock
PUBLISH_REAL_CP="$(command -v cp)" \
PUBLISH_RACE_SOURCE="$race_source" \
PUBLISH_RACE_MARKER="$ROOT/receipt-race-fired" \
  run_case_preflight "$case_root" "$STAGE_SHA" "$ROOT/racing-cp:$FAKE_BIN:$REAL_PATH" \
  >"$ROOT/receipt-race.stdout" 2>"$ROOT/receipt-race.stderr"
assert_full_output "$ROOT/receipt-race.stdout"
[[ -f "$ROOT/receipt-race-fired" ]] || fail_test receipt_snapshot_race_not_exercised
grep -Fq 'replaced after private snapshot' "$race_source" \
  || fail_test receipt_snapshot_race_did_not_replace_source

# The resolver emits one machine-readable line and binds the artifact to the
# exact successful first-attempt staging run on current main.
reset_mock
[[ "$(run_resolve)" == "stage_run_id=$STAGE_RUN_ID" ]] \
  || fail_test resolve_stage_output_changed
[[ "$(run_resolve transient_api)" == "stage_run_id=$STAGE_RUN_ID" ]] \
  || fail_test transient_api_read_was_not_retried
[[ -f "$MOCK_ROOT/transient-api-fired" ]] || fail_test transient_api_fault_was_not_exercised
[[ "$(run_resolve "" 2147483647)" == "stage_run_id=$STAGE_RUN_ID" ]] \
  || fail_test exact_release_asset_size_limit_was_rejected
if run_resolve "" 2147483648 >"$ROOT/over-limit.stdout" 2>"$ROOT/over-limit.stderr"; then
  fail_test over_limit_release_asset_was_accepted
fi
assert_contains staged_artifact_identity_mismatch "$ROOT/over-limit.stderr"
reset_mock
expect_resolve_failure persistent_api api_get_status_invalid
[[ "$(jq -s --arg url "https://api.mock.invalid/repos/$REPOSITORY" '[.[] | select(.method == "GET" and .url == $url)] | length' "$MOCK_ROOT/requests.jsonl")" == 3 ]] \
  || fail_test read_retry_bound_changed
expect_resolve_failure wrong_digest staged_artifact_identity_mismatch
expect_resolve_failure full_sha_name_as_artifact staged_artifact_identity_mismatch
expect_resolve_failure run_attempt_two staging_run_identity_mismatch
expect_resolve_failure multiple_stage_artifacts staging_run_artifact_set_mismatch

for upstream_fault in upstream_attempt_two newer_ci_failure newer_qualification_failure; do
  reset_mock
  expect_failure preflight "$upstream_fault" qualification_run_
  [[ "$(jq -s '[.[] | select(.method == "POST" or .method == "PATCH")] | length' \
    "$MOCK_ROOT/requests.jsonl")" == 0 ]] \
    || fail_test "$upstream_fault reached a release mutation"
done

reset_mock
expect_failure attach newer_ci_failure_after_body qualification_run_not_latest
[[ "$(jq -s '[.[] | select(.method == "PATCH" and .dataKind == "body")] | length' \
  "$MOCK_ROOT/requests.jsonl")" == 1 ]] \
  || fail_test stateful_upstream_fault_did_not_cross_the_body_binding_boundary
[[ "$(jq -s '[.[] | select(.method == "POST")] | length' \
  "$MOCK_ROOT/requests.jsonl")" == 0 ]] \
  || fail_test stateful_upstream_fault_reached_asset_upload

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
jq -e '
  .body
  | contains("official_termux_native_automated_v1")
    and contains("Physical device observed: no.")
    and contains("Android framework observed: no.")
    and contains("Sustained physical soak: no.")
    and contains("Physical certification: not run.")
    and contains("rebuildReproducibilityClaim:false")
    and contains("runtime archive, package lock, snapshot, and offline replay record")
    and contains("not separate Release assets")
    and contains("Automated qualification run: [`5104`]")
' "$MOCK_ROOT/state.json" >/dev/null \
  || fail_test release_body_qualification_boundary_missing
[[ "$(jq -s '[.[] | select(.method == "POST" and (.url | contains("/assets?name="))) | .url] | map(select(test("termux-(qualified-runtime|runtime-package|runtime-snapshot|runtime-replay)|release-publication-receipt"))) | length' "$MOCK_ROOT/requests.jsonl")" == 0 ]] \
  || fail_test retained_runtime_or_private_receipt_was_uploaded_separately
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
expect_failure attach record_post_move_replacement identity_record_publish_replaced
[[ -f "$MOCK_ROOT/record-post-move-fired" ]] \
  || fail_test post_move_identity_record_race_was_not_exercised
[[ "$(<"$ATTACH_RECORD")" == "replaced after move" ]] \
  || fail_test post_move_identity_record_replacement_was_deleted
[[ "$(stat -c '%a' -- "$ATTACH_RECORD")" == 600 ]] \
  || fail_test post_move_identity_record_mode_changed
reset_mock

cross_filesystem_root=""
for candidate in /dev/shm; do
  [[ -d "$candidate" && -w "$candidate" ]] || continue
  [[ "$(stat -c '%d' -- "$candidate")" != "$(stat -c '%d' -- "$ROOT")" ]] || continue
  cross_filesystem_root="$(mktemp -d "$candidate/termux-mcp-publisher-test.XXXXXXXX")"
  chmod 700 "$cross_filesystem_root"
  break
done
if [[ -n "$cross_filesystem_root" ]]; then
  cross_filesystem_record="$cross_filesystem_root/release-attachment-record-v1.json"
  env "${COMMON_ENV[@]}" MOCK_FAULT="" \
    bash "$SCRIPT" attach "${full_args[@]}" --record "$cross_filesystem_record" \
      >"$ROOT/cross-filesystem-record.stdout" \
      2>"$ROOT/cross-filesystem-record.stderr"
  assert_full_output "$ROOT/cross-filesystem-record.stdout"
  assert_identity_record "$cross_filesystem_record" attach
  rm -f -- "$cross_filesystem_record"
  rmdir -- "$cross_filesystem_root"
  reset_mock
fi

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
