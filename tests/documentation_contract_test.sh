#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$REPO_ROOT"

fail() {
  printf 'documentation contract failed: %s\n' "$1" >&2
  exit 1
}

mapfile -t markdown_files < <(git ls-files '*.md')
((${#markdown_files[@]} > 0)) || fail no_markdown_files

if grep -Fn 'current/bin/termux-mcp-server' "${markdown_files[@]}"; then
  fail obsolete_deployed_binary_path
fi

catalog=docs/CAPABILITIES.md
[[ -f "$catalog" ]] || fail capability_catalog_missing

python3 - "$catalog" <<'PY'
import pathlib
import re
import sys

catalog = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
source = pathlib.Path("src/mcp_transport.rs").read_text(encoding="utf-8")
all_source = "\n".join(
    path.read_text(encoding="utf-8") for path in sorted(pathlib.Path("src").rglob("*.rs"))
)

constants = dict(
    re.findall(r'(?:pub\s+)?const\s+([A-Z0-9_]+_TOOL):\s*&str\s*=\s*"([^"]+)";', all_source)
)
array_match = re.search(
    r'const\s+BASE_AVAILABLE_TOOLS:\s*\[&str;\s*(\d+)\]\s*=\s*\[(.*?)\];',
    source,
    re.DOTALL,
)
if array_match is None:
    raise SystemExit("BASE_AVAILABLE_TOOLS could not be parsed")

declared_count = int(array_match.group(1))
symbols = re.findall(r'\b[A-Z0-9_]+_TOOL\b', array_match.group(2))
if len(symbols) != declared_count:
    raise SystemExit("BASE_AVAILABLE_TOOLS declared and parsed counts differ")

optional_symbols = [
    "ANDROID_BATTERY_STATUS_TOOL",
    "ANDROID_VOLUME_STATUS_TOOL",
    "SET_ANDROID_VOLUME_TOOL",
    "RUN_COMMAND_PROFILE_TOOL",
]
for symbol in symbols + optional_symbols:
    tool = constants.get(symbol)
    if tool is None:
        raise SystemExit(f"tool constant could not be resolved: {symbol}")
    if f"`{tool}`" not in catalog:
        raise SystemExit(f"capability catalog omits source tool: {tool}")

if declared_count != 17:
    raise SystemExit("baseline tool count changed without documentation-contract review")
if "Up to 21" not in catalog:
    raise SystemExit("capability catalog omits the current maximum tool count")
PY

postures=(
  mcp-runtime
  android-battery-status
  android-volume-status
  android-volume-control
  command-execution
)
for posture in "${postures[@]}"; do
  grep -Fq -- "BUILD_FEATURES=$posture" CONTRIBUTING.md \
    || fail "contributing_posture_missing_$posture"
done

grep -Fq '`trash_file`' SECURITY.md || fail root_security_trash_tool_missing
grep -Fq '`read_text_range`' SECURITY.md || fail root_security_text_range_missing
grep -Fq 'trash (`5`)' SECURITY.md || fail root_security_trash_family_missing
grep -Fq 'finite request-response SSE' SECURITY.md || fail root_security_sse_posture_missing
grep -Fq 'Copy, trash, and write results disclose neither' SECURITY.md \
  || fail root_security_result_privacy_scope_missing
grep -Fq 'Directory creation returns its normalized safe-rooted path' SECURITY.md \
  || fail root_security_create_result_scope_missing
grep -Fq '### `trash_file` request grant' docs/capability-gates.md \
  || fail trash_capability_gate_missing
grep -Fq '## v0.6.0 — Release Candidate (Unreleased)' CHANGELOG.md \
  || fail changelog_release_candidate_heading_missing
if grep -Eq '^## (Unreleased|[0-9]{4}-[0-9]{2}-[0-9]{2} — v0\.6\.0)' CHANGELOG.md; then
  fail changelog_release_state_ambiguous
fi

if grep -Eiq -- '--private|create a new private repository' docs/GITHUB_IMPORT.md; then
  fail canonical_repository_visibility_stale
fi
grep -Fq 'https://github.com/CyberBASSLord-666/termux-mcp-edge.git' docs/GITHUB_IMPORT.md \
  || fail canonical_clone_url_missing
grep -Fq 'docs/**/*.md' .github/workflows/ci.yml || fail documentation_ci_path_filter_missing
grep -Fq 'pgrep -af "$PREFIX/bin/runsvdir"' README.md \
  || fail readme_service_supervisor_preflight_missing

python3 - "${markdown_files[@]}" <<'PY'
import pathlib
import re
import sys
import urllib.parse

link_pattern = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
failures: list[str] = []

for raw_name in sys.argv[1:]:
    document = pathlib.Path(raw_name)
    text = document.read_text(encoding="utf-8")
    for match in link_pattern.finditer(text):
        raw_target = match.group(1).strip()
        if not raw_target or raw_target.startswith(("#", "http://", "https://", "mailto:")):
            continue
        if raw_target.startswith("<") and raw_target.endswith(">"):
            raw_target = raw_target[1:-1]
        target_without_fragment = raw_target.split("#", 1)[0]
        target_without_fragment = urllib.parse.unquote(target_without_fragment)
        if not target_without_fragment:
            continue
        target = document.parent / target_without_fragment
        if not target.exists():
            line = text.count("\n", 0, match.start()) + 1
            failures.append(f"{document}:{line}: {raw_target}")

if failures:
    print("broken relative Markdown links:", file=sys.stderr)
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    raise SystemExit(1)
PY

printf 'Documentation capability, deployment-path, posture, and link contracts passed\n'
