# Production Readiness Checklist

This checklist defines the evidence required to merge, release, and operate the current Termux MCP Edge codebase. It distinguishes the two supported compile-time postures and does not treat the staged MCP transport as fully protocol-conformant or broadly production-ready.

## Supported Compile-Time Postures

| Surface | Default build | `mcp-runtime` build |
| --- | --- | --- |
| `GET /health` | Enabled, unauthenticated, coarse response | Enabled, unauthenticated, coarse response |
| `GET /ready` | Enabled, unauthenticated, coarse response | Enabled, unauthenticated, includes non-sensitive request-limit metadata |
| `POST /mcp` | Not compiled | Compiled as an authenticated, resource-bounded staged transport |
| MCP tools | None | `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, `write_file` |
| Android control, shell, command execution, arbitrary service control, and other high-impact actions | Disabled | Disabled |

Both postures validate startup authentication configuration. Static-token mode is the default. Unauthenticated development requires an explicit opt-in and a loopback bind.

The `mcp-runtime` build currently implements a custom POST-only JSON-RPC transport that reports protocol version `2024-11-05`. It does not yet implement the complete stable MCP 2025-11-25 Streamable HTTP lifecycle, media negotiation, protocol-version header, or optional session behavior. Track that work separately under #199.

## Open Production Blockers

Do not describe the staged MCP posture as fully production-ready while these confirmed lanes remain open:

- #198: runtime tool arguments do not yet enforce every advertised closed-schema rule consistently;
- #199: stable MCP 2025-11-25 lifecycle and Streamable HTTP conformance are incomplete;
- #200: filesystem operations retain canonicalize-then-use symlink race windows;
- #203: runit service transitions and failed-first-install cleanup are not fully atomic;
- #204: invalid-Unicode environment handling and port/list configuration need uniform fail-closed behavior;
- #205: package metadata, dependency features, and shipped license materials require reconciliation;
- #206: filesystem response byte bounds, determinism, and happy/boundary coverage remain incomplete.

These blockers do not erase the controls already present. They define the remaining evidence required before a broad readiness claim.

## Required Pull Request Gate

Every implementation pull request must satisfy all applicable items:

1. The diff is focused on one tracked concern and is based on current `main`.
2. Exact-head CI passes formatting, all-target/all-feature Clippy with warnings denied, the full all-feature test suite, and Termux deployment shell tests.
3. Exact-head Android validation passes for both the default and `mcp-runtime` AArch64 postures when Rust source, toolchain, dependencies, cross-compilation, or device artifacts can change.
4. Exact-head Security passes when Cargo metadata, `Cargo.lock`, or the Security workflow changes.
5. Dependency alerts are reviewed after dependency changes.
6. All actionable review threads are resolved and the head SHA has not changed since validation.
7. Documentation and tests match the resulting compiled behavior.
8. No change combines protocol migration, dependency maintenance, and unrelated high-impact capability exposure.

Documentation-only changes may document why path-filtered workflow non-runs are acceptable. Changes to Rust source comments still match `src/**` workflow filters and require the checks they trigger.

## Release Candidate Checklist

Run the host gates with the pinned toolchain:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
bash tests/termux_deploy_test.sh
cargo build --release
cargo build --release --features mcp-runtime
```

For Android, require both posture-specific artifacts described in [`ANDROID_ARTIFACTS.md`](ANDROID_ARTIFACTS.md):

- `termux-mcp-server-aarch64-linux-android-default`;
- `termux-mcp-server-aarch64-linux-android-mcp-runtime`.

For each released artifact:

1. Record the exact commit and workflow run.
2. Verify the SHA-256 digest, AArch64 Android ELF identity, size, and embedded `--version` output.
3. Install through `scripts/termux_deploy.sh`; do not mix it with the legacy runit path.
4. Confirm private non-symlink `runtime.env` configuration and the intended authentication posture.
5. Confirm runit state, `GET /health`, and `GET /ready`.
6. For the `mcp-runtime` artifact, prove unauthenticated rejection, authenticated discovery, representative allowed/denied tool calls, request-limit behavior, and filesystem boundaries.
7. Exercise upgrade failure recovery and explicit rollback before replacing the prior known-good release.
8. Validate sustained behavior under the target device's battery, thermal, and child-process restrictions.

## Current MCP Runtime Gate

A change to the staged transport or tool registry must prove:

- bearer authentication remains outside request-limit accounting and message handling;
- localhost-only unauthenticated mode cannot bind to a non-loopback address;
- unexpected `Host` and browser `Origin` values fail before JSON-RPC dispatch;
- malformed JSON and invalid JSON-RPC request objects remain distinct;
- notification-shaped tool calls cannot dispatch or mutate state;
- unauthenticated callers cannot discover or invoke tools;
- discovery lists only the current seven-tool allowlist;
- filesystem tools remain safe-rooted, bounded, and dry-run-first for writes;
- read-only metadata excludes persistent identifiers, secrets, environments, process inventory, and control behavior;
- errors and audit counters retain only stable non-sensitive data;
- command execution, Android control, shell fallback, and other high-impact tools remain absent.

Protocol migration work must additionally satisfy the stable MCP 2025-11-25 requirements documented in [`MCP_RESTORATION_VALIDATION.md`](MCP_RESTORATION_VALIDATION.md).

## High-Impact Capability Gate

Any future tool that executes commands, controls Android or services, accesses broad/shared storage, performs network or package mutation, automates a browser, handles credentials, or otherwise expands device authority requires:

1. a dedicated compile-time and runtime opt-in;
2. a fixed allowlist and bounded inputs/outputs;
3. explicit operator consent or capability-grant semantics appropriate to the action;
4. deterministic allowed, denied, boundary, timeout, cancellation, cleanup, and rollback tests;
5. non-sensitive audit coverage for every decision;
6. operator documentation and on-device validation;
7. an independently reviewable pull request.

Inert policy modules are not authorization to expose a live capability.

## Stop Conditions

Do not merge or release when any applicable condition is true:

- exact-head CI, Android, or Security evidence is missing, stale, cancelled, or failing;
- an artifact's feature posture or source commit is ambiguous;
- actionable review feedback remains unresolved;
- documentation claims behavior or conformance the code does not implement;
- unauthenticated clients can reach MCP discovery or invocation in static-token mode;
- browser-reachable MCP traffic lacks exact Host/Origin enforcement;
- errors, logs, or audit data can expose tokens, private paths, raw I/O text, or caller payloads;
- filesystem mutation can occur without explicit `dry_run:false` and safe-root validation;
- a dependency advisory is unresolved without a documented accepted-risk decision;
- a high-impact capability appears without its independent gate and validation evidence.
