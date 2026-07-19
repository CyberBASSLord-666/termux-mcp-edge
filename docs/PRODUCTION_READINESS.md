# Production Readiness Checklist

This checklist defines the evidence required to merge, release, and operate the current Termux MCP Edge codebase. It distinguishes six supported compile-time postures: the optional runtime implements the stable MCP 2025-11-25 Streamable HTTP lifecycle around a deliberately staged tool surface; battery, volume-status, and fixed-command features each add one separately gated read-only tool; and volume control adds one separately authorized preview-first mutation. Release readiness still depends on the exact-candidate filesystem, deployment, configuration, packaging, recovery, and applicable physical-device evidence below.

## Supported Compile-Time Postures

| Surface | Default build | `mcp-runtime` build | `android-battery-status` build | `android-volume-status` build | `android-volume-control` build | `command-execution` build |
| --- | --- | --- | --- | --- | --- | --- |
| `GET /health` | Enabled, coarse | Enabled, coarse | Enabled, coarse | Enabled, coarse | Enabled, coarse | Enabled, coarse |
| `GET /ready` | Enabled, coarse | Enabled with bounded-limit metadata | Same | Same | Same | Same |
| `/mcp` stable transport | Not compiled | Authenticated and resource-bounded; JSON/GET-405 default, bounded SSE opt-in | Same | Same | Same | Same |
| Optional tool | None | None | `android_battery_status` | `android_volume_status` | `set_android_volume` | `run_command_profile` |
| Exact-stream volume control | Disabled | Disabled | Disabled | Disabled | Preview-first; live use exact-grant authorized | Disabled |
| Broader Android/shell/arbitrary-command/service control | Disabled | Disabled | Disabled | Disabled | Disabled | Disabled |

All postures validate startup authentication configuration. Static-token mode is the default. Unauthenticated development requires an explicit opt-in and a loopback bind.

The `mcp-runtime` build negotiates protocol version `2025-11-25`, validates initialize metadata, issues cryptographically random bounded sessions, gates normal operations on `notifications/initialized`, enforces POST media negotiation and the subsequent-request protocol/session headers, accepts compliant client notifications and responses with HTTP 202, and supports DELETE termination. Its default JSON posture uses the specification-permitted GET 405. The separate SSE runtime opt-in provides only finite primed responses and bounded exact-stream replay; long-lived server queues and broadcast remain absent. Baseline `write_file` discovery remains preview-only unless its independent default-false runtime gate, static authentication, and capability key pair are active; every live create or replace still requires an exact request grant.

## Remediated Production Lanes

The confirmed implementation lanes have focused merge evidence:

- #200: descriptor-relative no-follow filesystem operations and adversarial race coverage;
- #203: atomic runit publication, shutdown confirmation, interruption recovery, and failed-first-install cleanup;
- #204: uniform fail-closed environment parsing and listener/safe-root validation;
- #205/#218: reconciled package licensing/metadata and minimized dependency features;
- #206: deterministic response byte/cardinality bounds and happy/boundary coverage.
- #240: descriptor-relative literal text search with fixed traversal, file, byte, match, response, and audit bounds.
- #242: descriptor-relative single-object metadata with content/identifier minimization and a fixed full-response bound.
- #247: bounded binary-safe file copy with held source/destination descriptors, atomic no-replace publication, fixed mode, response preflight, identity-safe cleanup, and content-private audit evidence.
- #244: dry-run-first one-directory creation with fixed mode, no-replace publication, durability sync, and identity-checked cleanup.
- #248: default-disabled one-directory mutation with exact-binary offline issuance, short-lived principal/session/root/target binding, atomic single-use consumption, private stable denials, and release/device evidence.

Source remediation alone is not a release declaration. A candidate is production-ready only after the exact commit completes every applicable PR/release gate below, every published Android posture is retained and verified, and the on-device install/upgrade/rollback smoke procedure succeeds without waived failures.

## Required Pull Request Gate

Every implementation pull request must satisfy all applicable items:

1. The diff is focused on one tracked concern and is based on current `main`.
2. Exact-head CI passes formatting, all-target/all-feature Clippy with warnings denied, the full all-feature test suite, and Termux deployment shell tests.
3. Exact-head Android validation passes for the default, `mcp-runtime`, battery, volume-status, volume-control, and fixed-command AArch64 postures when Rust source, toolchain, dependencies, cross-compilation, or device artifacts can change.
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
cargo build --release --features android-battery-status
cargo build --release --features android-volume-status
cargo build --release --features android-volume-control
cargo build --release --features command-execution
```

For Android, require all posture-specific artifacts described in [`ANDROID_ARTIFACTS.md`](ANDROID_ARTIFACTS.md):

- `termux-mcp-server-aarch64-linux-android-default`;
- `termux-mcp-server-aarch64-linux-android-mcp-runtime`;
- `termux-mcp-server-aarch64-linux-android-android-battery-status`.
- `termux-mcp-server-aarch64-linux-android-android-volume-status`.
- `termux-mcp-server-aarch64-linux-android-android-volume-control`.
- `termux-mcp-server-aarch64-linux-android-command-execution`.

For each released artifact:

1. Record the exact commit and workflow run.
2. Verify the SHA-256 digest, AArch64 Android ELF identity, size, and embedded `--version` output.
3. Install through `scripts/termux_deploy.sh`; do not mix it with the legacy runit path.
4. Confirm private non-symlink `runtime.env` configuration and the intended authentication posture.
5. Confirm runit state, `GET /health`, and `GET /ready`.
6. For the `mcp-runtime` artifact, prove unauthenticated rejection, authenticated discovery, representative allowed/denied tool calls, request-limit behavior, and filesystem boundaries. Exercise the independent disabled/enabled `write_file` gate, exact-binary issuance from a private stable content file, content/disposition/existing-identity binding, replay/concurrent replay, exact 1 MiB and 16 KiB boundaries, target mode `0600`, create `NOREPLACE` without retention, irreversible replace `EXCHANGE` with preserved recovery material, reserved-namespace isolation, per-parent capacity/lock denial, `recoveryArtifactRetained`, and private response/audit evidence.
7. For the battery artifact, prove disabled-default discovery and enabled fixed-path, zero-argument, cleared-environment, bounded, normalized, redacted, audited behavior without enabling device control or command execution. Exercise immediate endless-output rejection, isolated process-group termination, pipe-holding descendant cleanup, caller cancellation, authoritative direct-child reaping, and cleanup-reserve exhaustion precedence through repository and native ARM64 Termux gates.
8. For the volume artifact, prove disabled-default discovery and enabled fixed `termux-volume` zero-argument execution, cleared environment, exact six-stream parsing, canonical ordering, unknown-field rejection, bounded output, stable audited failures, and shared-supervisor process/descendant/cancellation cleanup without enabling volume mutation, device control, or command execution.
9. For the control artifact, prove incompatible-build rejection, disabled/enabled truth, closed schema, preview non-consumption, exact grants, fixed setter, fresh bounds, verification, recovery, concurrency, cancellation cleanup, and private counters.
10. For the command artifact, prove default-build compile rejection, disabled discovery, the exact three-profile closed schema, fixed executable/argv, safe-root cwd, empty environment, null stdin, bounded UTF-8 output, override rejection, and audit counters while arbitrary commands and unrelated high-impact controls remain disabled.
11. Exercise upgrade failure recovery and explicit rollback before replacing the prior known-good release.
12. Validate sustained behavior under the target device's battery, thermal, and child-process restrictions when release governance requires device-specific evidence, either directly for the candidate or through a strictly verified inherited observation when runtime/deployment inputs and exact bridge artifact digests are unchanged.

Run exact downloaded artifacts through the native ARM64 official-Termux gate in [`EMULATED_RELEASE_GATE.md`](EMULATED_RELEASE_GATE.md). For behavior-changing candidates, also run [`DEVICE_PRODUCTION_GATE.md`](DEVICE_PRODUCTION_GATE.md) directly on hardware. A metadata-only descendant may inherit an already completed physical observation only when the repository verifier proves every source, dependency, deployment, bridge-digest, and emulation condition without exception.

The release validator and device-smoke harness must execute the same deterministic file-write authorization contract against the exact artifact: default-disabled discovery and denial, enabled exact grant issuance, authorized create/replace, binding mismatch and replay denial, target/artifact race preservation, content-free response/audit evidence, reserved quarantine isolation/capacity, and service cleanup through deployment upgrade/rollback/uninstall. This capability evidence does not by itself require a new fixed-duration idle observation; physical observation remains governed by the existing source/artifact and device-behavior classifier.

Run complete downloaded workflow bundles—binary, `SHA256SUMS`, and `artifact-manifest.json`—through [`RELEASE_CANDIDATE_VALIDATION.md`](RELEASE_CANDIDATE_VALIDATION.md). The exact commit must have either a direct non-fixture `releaseEligible: true` report or a passing inherited-observation report conforming to [`release-observation-inheritance-schema-v1.json`](release-observation-inheritance-schema-v1.json), backed by the direct source report and a passing exact-candidate [`emulated-release-evidence-schema-v1.json`](emulated-release-evidence-schema-v1.json) report.

## Current MCP Runtime Gate

A change to the stable transport or staged tool registry must prove:

- bearer authentication remains outside request-limit accounting and message handling;
- localhost-only unauthenticated mode cannot bind to a non-loopback address;
- unexpected `Host` and browser `Origin` values fail before JSON-RPC dispatch;
- malformed JSON and invalid JSON-RPC request objects remain distinct;
- initialization negotiates `2025-11-25`, creates no session for invalid params, and gates normal operations until `notifications/initialized`;
- POST content and accepted response media types, `MCP-Protocol-Version`, and `MCP-Session-Id` are enforced without ambiguous duplicate headers;
- sessions remain random, bounded, expiring, isolated, explicitly terminable, and subordinate to request authentication;
- notifications and client responses receive HTTP 202 with no body, batches remain rejected, the default GET returns 405 without replay state, and the opt-in SSE posture proves finite priming, exact same-stream resumption, cross-session denial, deterministic eviction, JSON fallback, and lifecycle cleanup;
- notification-shaped tool calls cannot dispatch or mutate state;
- unauthenticated callers cannot discover or invoke tools;
- discovery lists exactly sixteen baseline tools, plus only those battery, volume-status, volume-control, and fixed-command tools whose independent gates are active (seventeen with one through twenty with all four);
- every tool call enforces its advertised closed input schema, including the omitted-or-empty contract for no-argument tools;
- filesystem tools remain safe-rooted and bounded; mutations remain dry-run-first; directory creation is additionally default-disabled, exact-target grant-gated, fixed-mode/no-replace/non-recursive, and single-use; `write_file` is independently default-disabled, exact principal/session/root/target/content/disposition/old-identity grant-gated, 1 MiB, target-mode `0600`, 16 KiB content/path-free, single-use, create-`NOREPLACE` without retention, and irreversible replace-`EXCHANGE` with bounded displaced-object preservation; file copy is single-regular-file, 1 MiB, binary-safe, fixed-mode, content-private, and no-replace; path discovery is literal-basename-only, descriptor-relative, 8,192-entry/512-match/depth/response bounded, and content-free; hashing is single-regular-file, streaming SHA-256, 16 MiB, and digest/path/content-private; binary read is single-regular-file, canonical padded base64, 1 MiB raw, response-preflighted, and path/host-metadata-private; metadata is descriptor-classified/content-free; and text search is literal/content-free;
- read-only metadata excludes persistent identifiers, secrets, environments, process inventory, and control behavior;
- errors and audit counters retain only stable non-sensitive data;
- arbitrary command execution, broader Android control, shell fallback, and unrelated high-impact tools remain absent; fixed diagnostics and exact-stream volume control appear only in their explicit postures.

Stable transport regression evidence, including the independently gated SSE posture, is defined in [`MCP_RESTORATION_VALIDATION.md`](MCP_RESTORATION_VALIDATION.md). Future long-lived server-request streaming or protocol-version changes require a new focused transport gate rather than an implicit compatibility expansion.

## High-Impact Capability Gate

Any future tool that adds a new executable, accepts command parameters, mutates state, controls Android or services, accesses broad/shared storage, performs network or package mutation, automates a browser, handles credentials, or otherwise expands device authority beyond the fixed diagnostic gate requires:

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
- filesystem mutation can occur without explicit `dry_run:false` and safe-root validation, or directory/file-write mutation can occur without its own enabled gate and exact request-scoped single-use grant;
- a file-write grant is not bound to exact content, create/replace disposition, or the old replacement identity; create can overwrite; replacement can destructively clean an uncertain object, skip bounded recovery retention, or claim hostile same-UID atomic rollback; mutation can begin before complete-response/quarantine-capacity preflight and atomic consumption; or its response/audits expose content, paths, digests, grants, sessions, JTIs, identities, or artifact names;
- a dependency advisory is unresolved without a documented accepted-risk decision;
- a high-impact capability appears without its independent gate and validation evidence.
