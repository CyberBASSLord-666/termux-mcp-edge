#!/usr/bin/env python3
from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}: {old[:120]!r}")
    path.write_text(text.replace(old, new, 1))


validator = Path("scripts/termux_release_validate.sh")
replace_once(
    validator,
    '''  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/created-directory" '{"jsonrpc":"2.0","id":"create-directory","method":"tools/call","params":{"name":"create_directory","arguments":{"path":$path,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status create_directory "$status" 200 create_directory_succeeded
  jq -e --arg path "$VALIDATION_SAFE_ROOT/created-directory" '
    .result.structuredContent == {
      path:$path,
      dryRun:false,
      mode:"0700",
      maxResponseBytes:16384
    }
  ' "$body" >/dev/null 2>&1 || fail create_directory_contract_invalid
  [[ -d "$VALIDATION_SAFE_ROOT/created-directory" ]] || fail create_directory_target_missing
  [[ "$(stat -c '%a' "$VALIDATION_SAFE_ROOT/created-directory" 2>/dev/null)" == 700 ]] || fail create_directory_mode_invalid
  record_result runtime create_directory pass safe_root_directory_creation_verified

  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status create_directory_existing "$status" 400 create_directory_existing_rejected
  jq -e '.error.code == -32602' "$body" >/dev/null 2>&1 || fail create_directory_existing_body_invalid
''',
    '''  payload="$(jq -cn --arg path "$VALIDATION_SAFE_ROOT/created-directory" '{"jsonrpc":"2.0","id":"create-directory","method":"tools/call","params":{"name":"create_directory","arguments":{"path":$path,"dry_run":false}}}')"
  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status create_directory_gate_closed "$status" 200 create_directory_gate_closed
  jq -e '
    .result.isError == true
    and .result.structuredContent.error == "filesystem_directory_create_unauthorized"
    and .result.structuredContent.reasonCode == "directory_mutation_authorization_unavailable"
  ' "$body" >/dev/null 2>&1 || fail create_directory_gate_contract_invalid
  [[ ! -e "$VALIDATION_SAFE_ROOT/created-directory" && ! -L "$VALIDATION_SAFE_ROOT/created-directory" ]] || fail create_directory_gate_mutated
  record_result runtime create_directory pass create_directory_mutation_gate_closed_verified

  status="$(mcp_post "$body" "$payload" "$MCP_SESSION_ID")"
  expect_status create_directory_gate_repeat "$status" 200 create_directory_gate_repeat_closed
  jq -e '
    .result.isError == true
    and .result.structuredContent.error == "filesystem_directory_create_unauthorized"
    and .result.structuredContent.reasonCode == "directory_mutation_authorization_unavailable"
  ' "$body" >/dev/null 2>&1 || fail create_directory_gate_repeat_contract_invalid
  [[ ! -e "$VALIDATION_SAFE_ROOT/created-directory" && ! -L "$VALIDATION_SAFE_ROOT/created-directory" ]] || fail create_directory_gate_repeat_mutated
''',
)

release_test = Path("tests/termux_release_validate_test.sh")
replace_once(
    release_test,
    'and ([.results[].code] | index("safe_root_directory_creation_verified") != null)',
    'and ([.results[].code] | index("create_directory_mutation_gate_closed_verified") != null)',
)

fixture = Path("tests/fixtures/release_validator_mock_server.py")
replace_once(
    fixture,
    '''def result(identifier: Any, structured: dict[str, Any]) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": identifier,
        "result": {
            "content": [{"type": "text", "text": "fixture-result"}],
            "structuredContent": structured,
            "isError": False,
        },
    }


''',
    '''def result(identifier: Any, structured: dict[str, Any]) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": identifier,
        "result": {
            "content": [{"type": "text", "text": "fixture-result"}],
            "structuredContent": structured,
            "isError": False,
        },
    }


def tool_error_result(identifier: Any, error: str, reason_code: str) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": identifier,
        "result": {
            "content": [{"type": "text", "text": "fixture-tool-error"}],
            "structuredContent": {"error": error, "reasonCode": reason_code},
            "isError": True,
        },
    }


''',
)
replace_once(
    fixture,
    '''            if not dry_run:
                target.mkdir(mode=0o700, parents=False, exist_ok=False)
                target.chmod(0o700)
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "path": str(target),
                        "dryRun": dry_run,
                        "mode": "0700",
                        "maxResponseBytes": 16384,
                    },
                ),
            )
''',
    '''            if not dry_run:
                self.send_json(
                    200,
                    tool_error_result(
                        identifier,
                        "filesystem_directory_create_unauthorized",
                        "directory_mutation_authorization_unavailable",
                    ),
                )
                return
            self.send_json(
                200,
                result(
                    identifier,
                    {
                        "path": str(target),
                        "dryRun": True,
                        "mode": "0700",
                        "maxResponseBytes": 16384,
                    },
                ),
            )
''',
)
