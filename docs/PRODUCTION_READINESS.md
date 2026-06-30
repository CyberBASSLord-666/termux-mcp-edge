# Production Readiness

## Current Release Line

The current `main` branch is a conservative Axum health-check runtime. It does not expose MCP transport or MCP tools. Transport restoration should be implemented in smaller validated changes.

## Merge Gates

Every pull request must satisfy these gates before merge:

1. Exact-head CI passes.
2. Exact-head Security passes.
3. Dependency alerts are reviewed after dependency or workflow changes.
4. Documentation matches the compiled runtime behavior.
5. Runtime behavior is tested or covered by a smoke-test note.

## Dependency Policy

- Avoid adding dependencies for code paths that are not compiled or used.
- Remove unused dependency surfaces rather than carrying advisory risk.
- Use the Security workflow as the minimum dependency-audit gate.
- Re-check GitHub Security after dependency changes merge.

## Transport Restoration Policy

MCP transport restoration must be staged. A broad feature PR should not restore transport and all tool classes at once without a complete threat model. See [MCP Transport Threat Model](TRANSPORT_THREAT_MODEL.md) before restoring any transport route.

Minimum staged path:

1. Restore a patched transport dependency.
2. Add Host and Origin or equivalent anti-rebinding protections.
3. Add MCP tool-discovery smoke coverage.
4. Add one low-risk read-only tool with smoke coverage.
5. Add higher-impact tools only after explicit authorization policy is documented.

## Runtime Exposure Policy

- Prefer localhost binding by default.
- Require a non-empty bearer token for non-local or shared access paths.
- Keep filesystem scope restricted to dedicated project directories.
- Treat broad Android shared storage as an exception requiring review.
- Do not claim production MCP readiness until transport and tool behavior are validated on the exact release candidate.
- See [MCP Tool Authorization Policy](TOOL_AUTHORIZATION_POLICY.md) before exposing MCP tool discovery or invocation.

## Release Checklist

Before tagging a release, confirm format, lint, tests, dependency audit, release build, Android cross-compile, and supervised Termux startup validation.
