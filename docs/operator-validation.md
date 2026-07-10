# Operator runtime validation checklist

This checklist gives advanced Termux MCP Edge operators a repeatable way to validate the staged runtime without expanding the MCP surface.

Use it after a local build, configuration change, release candidate, or manual dispatch/tag build when you need evidence that the runtime still matches the staged capability model.

## Validation posture

The expected posture is narrow and fail-closed:

- In static-token mode, the complete `/mcp` route requires the configured bearer token before transport validation, JSON-RPC parsing, discovery, or invocation.
- Explicit unauthenticated development mode is accepted only when startup validates a loopback bind.
- `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file` are the only staged MCP tools currently expected in authenticated discovery.
- `write_file` remains dry-run by default and must require explicit `dry_run:false` plus safe-root validation before mutation.
- Filesystem reads, listings, and writes remain bounded to configured safe roots.
- `project_service_status` remains limited to explicitly allowlisted project-owned logical services.
- Android status remains read-only allowlisted metadata, not Android platform control.
- Shell access, arbitrary command execution, global process inventory, service mutation, package management, network mutation, and high-impact device controls remain unavailable.

## Preflight

Before validating behavior, confirm the operator configuration is deliberately narrow:

1. Build with the intended feature set, normally `--features mcp-runtime` for staged MCP validation.
2. Use a strong static bearer token for any deployment that is not explicitly loopback-development only.
3. Protect the token file with mode `0600`; do not echo the token or use shell tracing while it is loaded.
4. Use localhost-only unauthenticated mode only when the server is bound to a loopback address and not exposed through a tunnel, LAN listener, or reverse proxy.
5. Keep `MCP__TRANSPORT__ALLOWED_HOSTS` and `MCP__TRANSPORT__ALLOWED_ORIGINS` exact and minimal.
6. Keep filesystem safe roots limited to a dedicated project directory, not broad shared storage such as `/storage/emulated/0` or `/sdcard`.

## Authentication checks

For static-token validation, load the protected token into a temporary shell variable without printing it:

```bash
MCP_TEST_TOKEN="$(cat "$HOME/.termux_mcp_token")"
```

Prove all of the following:

- A `/mcp` request with no `Authorization` header receives HTTP 401.
- The response includes `WWW-Authenticate: Bearer` and `Cache-Control: no-store`.
- Missing, malformed, oversized, and incorrect credentials produce the same non-sensitive `unauthorized` response shape.
- The response never includes the configured or presented token.
- A correct `Authorization: Bearer ${MCP_TEST_TOKEN}` header reaches transport validation and MCP handling.
- Authentication rejection happens before invalid Host/Origin or malformed JSON is processed.
- `/health` and `/ready` remain available without credentials and return only coarse non-secret operational status.

Clear the temporary variable after validation:

```bash
unset MCP_TEST_TOKEN
```

## Discovery checks

A valid runtime discovery pass proves presence and absence:

- An unauthenticated caller receives no tool list in static-token mode.
- An authenticated `tools/list` call includes the current staged tools listed above.
- `tools/list` does not include command execution, Android control, process listing, service mutation, package management, arbitrary network mutation, environment inspection, or token-management tools.
- Tool descriptions and schemas continue to communicate safe-root, read-only, dry-run, and allowlist boundaries where applicable.

Discovery is not sufficient by itself. A tool being absent from discovery is the first guardrail, but each boundary below should also be checked through representative authenticated calls.

## Runtime status and audit-counter checks

Call `runtime_status` before and after representative allowed and denied authenticated tool calls.

Expected evidence:

- `structuredContent.auditCounters` is present when the audit snapshot is available.
- Allowed and denied totals move only in response to staged tool-gate decisions.
- Authentication failures do not enter MCP tool dispatch or expose tool audit data.
- `by_tool` uses stable staged tool names.
- `by_reason_code` uses stable low-cardinality reason codes.
- Counters do not include raw paths, file contents, command output, command arguments, environment values, hostnames, usernames, Android identifiers, private device metadata, bearer values, raw capability tokens, or arbitrary caller-provided strings.
- Restarting the process resets the in-memory counters.

Audit counters are evidence of gate decisions, not an authorization mechanism and not a retained activity log. The authoritative counter contract is maintained in [`runtime-audit-counters.md`](runtime-audit-counters.md).

## Filesystem checks

Use a dedicated safe-root test directory. Validate all of the following with authenticated calls in static-token mode:

- Listing a safe-rooted directory succeeds with a `safe_root_listing`-style allowed decision.
- Reading a bounded UTF-8 file under a safe root succeeds with a `safe_root_read`-style allowed decision.
- Reading or listing a path outside the configured safe root is denied with a stable outside-safe-root reason code.
- Excessive read or write sizes are denied with stable byte-limit reason codes.
- `write_file` with omitted `dry_run` or `dry_run:true` returns a preview and does not mutate the file.
- `write_file` with `dry_run:false` mutates only a safe-rooted target and is still bounded by size and path validation.
- Symlink escapes remain denied.

Filesystem counter expectations are maintained in [`filesystem-audit-counter-contract.md`](filesystem-audit-counter-contract.md).

## Project service status checks

Use the documented project-owned service name first.

Expected evidence:

- `project_service_status` succeeds for an explicitly allowlisted project-owned logical service such as `mcp_runtime`.
- Missing, malformed, or unsupported service names fail with structured errors and stable reason codes.
- The tool does not expose arbitrary service discovery, global process lists, PIDs, command lines, environment values, service control, or supervision mutation.

## Android status checks

Expected evidence:

- `android_status` returns only read-only allowlisted Android/Termux status metadata.
- It does not expose contacts, SMS, notifications, accounts, location, camera, microphone, accessibility state, installed package inventory, persistent device identifiers, user secrets, shell fallback, or device-control actions.
- Read-only Android status must not be treated as completion of the Android platform-control gate.

## Capability-token boundary checks

Capability-token primitives are currently inert policy scaffolding for future high-impact gates. They are separate from the static bearer token used to authenticate the MCP transport.

Expected evidence:

- No raw high-impact capability-token issuance, persistence, bearer parsing, validation, or serialization is exposed by the runtime.
- No high-impact MCP tool is enabled by the presence of capability-token primitives.
- Future capability-token evaluation must remain exact-match, fail-closed, bounded to non-secret metadata, and audited only with stable non-sensitive labels.

The capability-token evaluation contract is maintained in [`capability-token-evaluation-contract.md`](capability-token-evaluation-contract.md).

## Failure interpretation

Treat any of the following as a blocker for a staged runtime PR or release candidate:

- Static-token mode permits unauthenticated `/mcp` discovery or invocation.
- Authentication failures reveal token values or reach JSON-RPC/tool dispatch.
- Discovery exposes a tool outside the staged baseline.
- A read-only metadata tool exposes private identifiers, secrets, environment values, filesystem paths outside filesystem tools, process inventory, or command output.
- Filesystem tools can escape configured safe roots or mutate without explicit `dry_run:false`.
- Audit counters serialize raw caller values or high-cardinality private metadata.
- Capability-token primitives become a live high-impact authorization surface without a separate focused gate.
- Any command execution, Android control, service mutation, package management, network mutation, or high-impact action appears without its own documented opt-in gate, tests, and audit contract.

When a blocker is found, keep remediation narrow: preserve existing response contracts unless the fix explicitly documents an additive change, and do not combine runtime behavior changes with dependency or workflow maintenance.
