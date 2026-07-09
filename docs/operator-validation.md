# Operator runtime validation checklist

This checklist gives advanced Termux MCP Edge operators a repeatable way to validate the staged runtime without expanding the MCP surface. It is intentionally documentation-only: it does not introduce new tools, routes, dependencies, secrets, command execution, Android control, or high-impact behavior.

Use it after a local build, configuration change, release candidate, or manual dispatch/tag build when you need evidence that the runtime still matches the staged capability model.

## Validation posture

The expected posture is narrow and fail-closed:

- `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file` are the only staged MCP tools currently expected in discovery.
- `write_file` remains dry-run by default and must require explicit `dry_run:false` plus safe-root validation before mutation.
- Filesystem reads, listings, and writes remain bounded to configured safe roots.
- `project_service_status` remains limited to explicitly allowlisted project-owned logical services.
- Android status remains read-only allowlisted metadata, not Android platform control.
- Shell access, arbitrary command execution, global process inventory, service mutation, package management, network mutation, and high-impact device controls remain unavailable.

## Preflight

Before validating behavior, confirm the operator configuration is deliberately narrow:

1. Build with the intended feature set, normally `--features mcp-runtime` for staged MCP validation.
2. Use a strong static bearer token for any non-local deployment.
3. Use localhost-only unauthenticated mode only when the server is bound to a loopback address and not exposed through a tunnel, LAN listener, or reverse proxy.
4. Keep `MCP__TRANSPORT__ALLOWED_HOSTS` and `MCP__TRANSPORT__ALLOWED_ORIGINS` exact and minimal.
5. Keep filesystem safe roots limited to a dedicated project directory, not broad shared storage such as `/storage/emulated/0` or `/sdcard`.

## Discovery checks

A valid runtime discovery pass proves presence and absence:

- `tools/list` includes the current staged tools listed above.
- `tools/list` does not include command execution, Android control, process listing, service mutation, package management, arbitrary network mutation, environment inspection, or token management tools.
- Tool descriptions and schemas continue to communicate safe-root, read-only, dry-run, and allowlist boundaries where applicable.

Discovery is not sufficient by itself. A tool being absent from discovery is the first guardrail, but each boundary below should also be checked through representative calls.

## Runtime status and audit-counter checks

Call `runtime_status` before and after representative allowed and denied tool calls.

Expected evidence:

- `structuredContent.auditCounters` is present when the audit snapshot is available.
- Allowed and denied totals move only in response to staged gate decisions.
- `by_tool` uses stable staged tool names.
- `by_reason_code` uses stable low-cardinality reason codes.
- Counters do not include raw paths, file contents, command output, command arguments, environment values, hostnames, usernames, Android identifiers, private device metadata, bearer values, raw capability tokens, or arbitrary caller-provided strings.
- Restarting the process resets the in-memory counters.

Audit counters are evidence of gate decisions, not an authorization mechanism and not a retained activity log. The authoritative counter contract is maintained in [`runtime-audit-counters.md`](runtime-audit-counters.md).

## Filesystem checks

Use a dedicated safe-root test directory. Validate all of the following:

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

Capability-token primitives are currently inert policy scaffolding for future high-impact gates.

Expected evidence:

- No raw bearer token parsing, issuance, persistence, validation, or serialization is exposed by the runtime.
- No high-impact MCP tool is enabled by the presence of capability-token primitives.
- Future capability-token evaluation must remain exact-match, fail-closed, bounded to non-secret metadata, and audited only with stable non-sensitive labels.

The capability-token evaluation contract is maintained in [`capability-token-evaluation-contract.md`](capability-token-evaluation-contract.md).

## Failure interpretation

Treat any of the following as a blocker for a staged runtime PR or release candidate:

- Discovery exposes a tool outside the staged baseline.
- A read-only metadata tool exposes private identifiers, secrets, environment values, filesystem paths outside filesystem tools, process inventory, or command output.
- Filesystem tools can escape configured safe roots or mutate without explicit `dry_run:false`.
- Audit counters serialize raw caller values or high-cardinality private metadata.
- Capability-token primitives become a live authorization or bearer-token surface without a separate focused gate.
- Any command execution, Android control, service mutation, package management, network mutation, or high-impact action appears without its own documented opt-in gate, tests, and audit contract.

When a blocker is found, keep remediation narrow: preserve existing response contracts unless the fix explicitly documents an additive change, and do not combine runtime behavior changes with dependency or workflow maintenance.
