# Runtime audit counters

Termux MCP Edge exposes staged audit counters through the `runtime_status` tool in the `structuredContent.auditCounters` field. The counters are intentionally small, in-memory, and backend-neutral. They are designed to help advanced operators verify that capability gates are being exercised without turning the MCP runtime into a log store or observability backend.

## Current scope

The current staged runtime records aggregate decisions for the enabled read-only/status surfaces that are wired into the audit counter path:

- `runtime_status`
- `platform_info`
- `android_status`
- `project_service_status`

The counters are additive runtime metadata. They do not change the availability, authorization, output shape, or behavior of the staged tools. They are reset when the process restarts.

Filesystem tools remain governed by safe-root validation, bounded reads, and dry-run-by-default writes. Future filesystem audit coverage must preserve the same non-sensitive counter contract before it is wired into the runtime.

## Counter shape

`auditCounters` contains deterministic aggregate counts:

```json
{
  "allowed_total": 3,
  "denied_total": 1,
  "by_tool": {
    "android_status": {
      "allowed": 1,
      "denied": 0
    },
    "project_service_status": {
      "allowed": 1,
      "denied": 1
    },
    "runtime_status": {
      "allowed": 1,
      "denied": 0
    }
  },
  "by_reason_code": {
    "allowlisted_project_service": {
      "allowed": 1,
      "denied": 0
    },
    "allowlisted_status_metadata": {
      "allowed": 1,
      "denied": 0
    },
    "staged_runtime_metadata": {
      "allowed": 1,
      "denied": 0
    },
    "unsupported_service": {
      "allowed": 0,
      "denied": 1
    }
  }
}
```

Sparse maps are omitted when there are no recorded decisions. If the counter mutex is poisoned, `runtime_status` reports an unavailable audit snapshot instead of exposing partial internal state.

## Non-sensitive observability contract

Audit counters may store only stable labels and aggregate counts:

- tool names
- gate names, where represented by the event source
- reason codes
- allowed and denied totals

Audit counters must not store or serialize:

- raw filesystem paths
- file contents
- command output
- command arguments
- environment variable names or values
- hostnames, usernames, Android identifiers, or private device metadata
- secrets, bearer values, passwords, API keys, or raw capability tokens
- arbitrary caller-supplied strings

The `AuditCounters` implementation deliberately ignores event metadata so bounded metadata used in local policy tests cannot accidentally become a runtime telemetry payload.

## Reason-code expectations

Reason codes are stable, low-cardinality labels. They are suitable for assertions and coarse operational monitoring, but they are not a substitute for full request logging.

Current examples include:

- `staged_runtime_metadata`
- `read_only_platform_metadata`
- `arguments_not_supported`
- `allowlisted_status_metadata`
- `arguments_not_empty_or_not_object`
- `allowlisted_project_service`
- `missing_service_name`
- `invalid_service_arguments`
- `unsupported_service`

New reason codes should be short, snake_case, and tied to a policy decision rather than a caller value.

## Expansion rules

Future audit expansion must remain staged and explicit:

1. Add or reuse backend-neutral audit primitives first.
2. Prefer counters over retained event logs unless a later design explicitly defines storage, retention, redaction, and operator controls.
3. Keep labels low-cardinality and stable.
4. Never record raw paths, file content, command output, environment values, private host details, or raw tokens.
5. Preserve existing MCP response contracts unless a focused PR intentionally changes them.
6. Keep high-impact controls disabled unless a separate capability gate, allowlist, confirmation, dry-run or preview model, structured failure mode, and audit contract are implemented.

## Relationship to higher-risk surfaces

Audit counters are not an authorization mechanism. They provide visibility into decisions made by staged gates. Command execution, Android platform control, package or service mutation, network mutation, and other high-impact controls remain unavailable until separately implemented behind explicit opt-in policy and capability gates.
