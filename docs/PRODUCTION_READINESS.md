# Production Readiness Checklist

This checklist defines the evidence required to merge, release, and operate the current Termux MCP Edge codebase. It distinguishes seven governed compile-time postures: six least-privilege artifacts and the explicit `full-suite` aggregate. The aggregate compiles every supported optional provider but leaves every runtime flag and request grant independent. Ordinary release readiness depends on exact-candidate filesystem, deployment, configuration, packaging, recovery, native official-Termux, policy, and protected-publication evidence.

## Supported Compile-Time Postures

| Surface | Default | `mcp-runtime` | Battery | Volume status | Volume control | Command | `full-suite` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `GET /health` | Enabled | Enabled | Enabled | Enabled | Enabled | Enabled | Enabled |
| `/mcp` stable transport | Absent | Protected | Protected | Protected | Protected | Protected | Protected |
| Optional tools when their flags are on | None | None | Battery | Volume status | Volume control | Fixed diagnostics | All four |
| Tool count with optional flags off | 0 | 17 | 17 | 17 | 17 | 17 | 17 |
| Maximum enabled tool count | 0 | 17 | 18 | 18 | 19 | 18 | 21 |
| Broader Android/shell/arbitrary-command/service control | Disabled | Disabled | Disabled | Disabled | Disabled | Disabled | Disabled |

All postures validate startup authentication configuration. Static-token mode is the default. Unauthenticated development requires an explicit opt-in and a loopback bind.

The `mcp-runtime` build negotiates protocol version `2025-11-25`, validates initialize metadata, issues cryptographically random bounded sessions, gates normal operations on `notifications/initialized`, enforces POST media negotiation and the subsequent-request protocol/session headers, accepts compliant client notifications and responses with HTTP 202, and supports DELETE termination. Its default JSON posture uses the specification-permitted GET 405. The separate SSE runtime opt-in provides only finite primed responses and bounded exact-stream replay; long-lived server queues and broadcast remain absent. Baseline `create_directory`, `copy_file`, `trash_file`, and `write_file` discovery remains preview-only unless each tool's independent default-false runtime gate, static authentication, and capability key pair are active; every live mutation still requires its exact request grant.

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

Source remediation alone is not a release declaration. A candidate is production-ready only after the exact commit completes every applicable PR/release gate below, every published Android posture and all retained-runtime evidence are verified, and the automated full-suite install/upgrade/rollback recovery procedure succeeds without waived failures. An on-device repetition is required only for the separate optional physical-certification tier or an operator-specific deployment claim.

## Required Pull Request Gate

Every implementation pull request must satisfy all applicable items:

1. The diff is focused on one tracked concern and is based on current `main`.
2. Exact-head CI passes formatting, named full-suite and raw all-feature Clippy/tests with warnings denied, and Termux deployment shell tests.
3. Exact-head Android validation passes for all seven AArch64 artifacts: default, `mcp-runtime`, battery, volume-status, volume-control, fixed-command, and `full-suite`.
4. Exact-head Security passes when Cargo metadata, `Cargo.lock`, or the Security workflow changes.
5. Dependency alerts are reviewed after dependency changes.
6. All actionable review threads are resolved and the head SHA has not changed since validation.
7. Documentation and tests match the resulting compiled behavior.
8. No change combines protocol migration, dependency maintenance, and unrelated high-impact capability exposure.

Documentation-only changes may document why path-filtered workflow non-runs are acceptable. Changes to Rust source comments still match `src/**` workflow filters and require the checks they trigger.

## Release Candidate Checklist

Run the host gates with the pinned toolchain:

```bash
cargo metadata --locked --all-features --format-version 1 --no-deps >/dev/null
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --features full-suite -- -D warnings
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --features full-suite
cargo test --locked --workspace --all-targets --all-features
bash tests/termux_deploy_test.sh
cargo build --release --locked
cargo build --release --locked --features mcp-runtime
cargo build --release --locked --features android-battery-status
cargo build --release --locked --features android-volume-status
cargo build --release --locked --features android-volume-control
cargo build --release --locked --features command-execution
cargo build --release --locked --features full-suite
```

For Android, require all posture-specific artifacts described in [`ANDROID_ARTIFACTS.md`](ANDROID_ARTIFACTS.md):

- `termux-mcp-server-aarch64-linux-android-default`;
- `termux-mcp-server-aarch64-linux-android-mcp-runtime`;
- `termux-mcp-server-aarch64-linux-android-android-battery-status`.
- `termux-mcp-server-aarch64-linux-android-android-volume-status`.
- `termux-mcp-server-aarch64-linux-android-android-volume-control`.
- `termux-mcp-server-aarch64-linux-android-command-execution`.
- `termux-mcp-server-aarch64-linux-android-full-suite`.

Across the seven-artifact release set, digest, manifest, ELF, and version checks
apply to every posture. Runtime provider checks apply to their matching
postures, while the isolated install/recovery transaction is performed against
the governed `full-suite` artifact. This does not claim that all seven binaries
were individually installed:

1. Record the exact commit and workflow run.
2. Verify the SHA-256 digest, AArch64 Android ELF identity, size, and embedded `--version` output.
3. Install through `scripts/termux_deploy.sh`; do not mix it with the legacy runit path.
4. Confirm private non-symlink `runtime.env` configuration and the intended authentication posture.
5. Confirm runit state, `GET /health`, and `GET /ready`.
6. For the `mcp-runtime` artifact, prove unauthenticated rejection, authenticated discovery, representative allowed/denied tool calls, request-limit behavior, and filesystem boundaries. Exercise all four independent filesystem mutation gates and exact-binary issuers. For `trash_file`, prove identity/content/recovery-posture binding, exact 1 MiB and 16 KiB boundaries, exact-inode `NOREPLACE` retention, separate reserved-namespace isolation and per-parent capacity/lock denial, `recoveryArtifactRetained:true`, and private response/audit evidence. Retain the existing write content/disposition/existing-identity, mode-`0600` create, irreversible replace, replay, and displaced-object recovery checks. Trash replay and concurrent-replay denial are required automated core/integration-test evidence, not a direct artifact-gate claim.
7. For the battery artifact, prove disabled-default discovery and enabled fixed-path, zero-argument, cleared-environment, bounded, normalized, redacted, audited behavior without enabling device control or command execution. Exercise immediate endless-output rejection, isolated process-group termination, pipe-holding descendant cleanup, caller cancellation, authoritative direct-child reaping, and cleanup-reserve exhaustion precedence through repository and native ARM64 Termux gates.
8. For the volume artifact, prove disabled-default discovery and enabled fixed `termux-volume` zero-argument execution, cleared environment, exact six-stream parsing, canonical ordering, unknown-field rejection, bounded output, stable audited failures, and shared-supervisor process/descendant/cancellation cleanup without enabling volume mutation, device control, or command execution.
9. For the control artifact, prove incompatible-build rejection, disabled/enabled truth, closed schema, preview non-consumption, exact grants, fixed setter, fresh bounds, verification, recovery, concurrency, cancellation cleanup, and private counters.
10. For the command artifact, prove default-build compile rejection, disabled discovery, the exact three-profile closed schema, binary-only enablement, exact-name candidate-to-loaded-image device/inode attestation, `/proc/self/exe` spawning, descriptor-pinned non-root safe cwd after pathname replacement, empty environment, null stdin, immutable 5-second/16 KiB stdout/4 KiB stderr maxima, override rejection, and audit counters while arbitrary commands and unrelated high-impact controls remain disabled. Require strict v3 evidence with exactly 29 MCP requests plus the separate wrong-name construction-failure phase: `McpRouterBuildError::CommandClientUnavailable`, no request serving or service-start log, and no bearer-token or filesystem-path disclosure. Retain the complete candidate/artifact/environment checks and the separately bound base-userland and derived runtime-image identities.
11. Exercise isolated upgrade-failure recovery, supervised restart, rollback-failure recovery, uninstall, and bounded cleanup. The automated scenario uses a second release target containing the same exact candidate artifact, so it proves transaction recovery and target isolation rather than cross-version compatibility.
12. Run the bounded native stress and provider-simulation contracts under the digest-pinned official Termux userland on native ARM64. Treat physical battery, thermal, radio, Doze, OEM, and Android-framework observation as a separate optional certification tier.

Run exact downloaded artifacts through the native ARM64 official-Termux gate in [`EMULATED_RELEASE_GATE.md`](EMULATED_RELEASE_GATE.md). The resulting `official_termux_native_automated_v1` envelope is the ordinary release qualification. Run [`DEVICE_PRODUCTION_GATE.md`](DEVICE_PRODUCTION_GATE.md) only when separate physical certification is requested; historical physical observation and inheritance records do not substitute for, or broaden, the automated class.

The independent manual
[`Android Rish Physical Identity`](SHIZUKU_RISH_PHYSICAL_WORKFLOW.md)
workflow is narrower still: it can establish only the development S2.5
Shizuku/rish UID-2000 identity probe for an exact open same-repository PR
head. Its closed evidence always records `releaseEligible:false` and
`productionControlQualified:false`; it is not an eighth governed Android
release artifact, physical production certification, S3 attestation, or
authority for any device mutation.

Release validator v11 and the native official-Termux gates must execute deterministic authorization contracts for all four filesystem mutation families against the exact workflow artifact. They must also prove the full-suite 17-disabled/21-enabled truth table while keeping every optional provider flag and request-grant family independent. Reversible trash evidence must prove default-disabled discovery and denial, exact grant issuance, target identity/content binding, authorized recovery retention, mismatch denial, preflight preservation, private response/audit evidence, separate quarantine isolation/capacity, and service cleanup through deployment upgrade/rollback/uninstall. Device harness v11 is required only when separate physical certification is requested; it is not an input to ordinary automated release qualification.

Run complete downloaded workflow bundles—binary, `SHA256SUMS`, and `artifact-manifest.json`—through the exact native workflow. The final exact-main commit needs passing aggregate [`emulated-release-evidence-schema-v4.json`](emulated-release-evidence-schema-v4.json), four specialized provider reports, classifier v3, the committed isolated deployment/recovery gate, and a closed [`release-automated-qualification-schema-v1.json`](release-automated-qualification-schema-v1.json) envelope. The Android run must expose exactly nine 30-day artifacts; the qualifier must expose one exact twelve-member artifact. The envelope binds all seven workflow manifests and binaries, immutable policy/scenario digests, the content-addressed runtime archive, package lock, snapshot, and no-network replay. It explicitly records `physicalDeviceObserved:false`, `androidFrameworkObserved:false`, `sustainedPhysicalSoak:false`, `physicalCertification:"not_run"`, and `rebuildReproducibilityClaim:false`.

After automated qualification, follow [`PUBLIC_RELEASE.md`](PUBLIC_RELEASE.md). The `release-qualification` environment must already have a non-initiating required reviewer, prevent-self-review, a main-only branch policy, disabled administrator bypass, no secrets, and the documented guard variable. Protected staging must consume the exact first-attempt Android run and independently discovered qualifier, repeat every source/run/artifact/evidence/runtime check after approval, copy all seven binaries and four retained-runtime members byte-for-byte, and emit a deterministic closed-manifest tar with `publicationState: "staged_not_released"`. A stage is not authorization to tag or publish.

Publication additionally requires release immutability, a no-bypass `v*` tag ruleset, one protected annotated tag at the exact staged commit, and one pre-created exact-tag draft with zero assets. `release-production` and `release-final` must have disjoint eligible-reviewer sets, prevent self review and administrator bypass, accept only `main`, and expose their documented environment-only guards and separate Administration-read policy credentials. Before final approval, administrators must establish the documented bounded exclusive mutation freeze covering every other Release writer, immutable-release-settings writer, protected-tag or ruleset writer, `main` update, and relevant workflow rerun; repeated REST reads are latest-observed checks rather than an atomic lock. The publisher must revalidate all four runtime members and their staging-manifest records, then attach exactly sixteen assets—seven binaries, seven sidecars, `SHA256SUMS`, and the unchanged raw staging tar. Runtime evidence remains inside that tar; the private receipt is never uploaded. Fresh read-only draft verification remains mandatory before final approval. Attachment GET-to-PATCH/POST races can only leave an unpublished, non-resumable partial draft that automation never deletes or repairs. Production readiness requires the published response to say `immutable: true` and a public re-download to prove every asset byte. A tag, stage, populated draft, or automated `releaseEligible:true` envelope alone is insufficient.

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
- discovery lists exactly 17 baseline tools, plus only those battery, volume-status, volume-control, and fixed-command tools whose independent gates are active (18 with one through 21 with all four);
- every tool call enforces its advertised closed input schema, including the omitted-or-empty contract for no-argument tools;
- filesystem tools remain safe-rooted and bounded; mutations remain dry-run-first and independently default-disabled. Directory creation is exact-target grant-gated, fixed-mode/no-replace/non-recursive, and single-use. File copy is exact source/content/destination grant-gated, single-regular-file, 1 MiB, binary-safe, fixed-mode, content-private, and no-replace. `trash_file` is exact principal/session/root/path/identity/content/recovery-posture grant-gated, 1 MiB, 16 KiB response-bounded, atomic-no-replace and recovery-retained in a separate hidden bounded quarantine, with no MCP purge or restore. `write_file` remains exact content/disposition/old-identity grant-gated, 1 MiB, target-mode `0600`, 16 KiB content/path-free, create-`NOREPLACE` without retention, and irreversible replace-`EXCHANGE` with bounded displaced-object preservation. Path discovery, hashing, reads, metadata, and search retain their documented descriptor and privacy bounds;
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
- filesystem mutation can occur without explicit `dry_run:false` and safe-root validation, or create/copy/trash/write mutation can occur without its own enabled gate and exact request-scoped single-use grant;
- reversible trash can overwrite, unlink, purge, recurse, expose recovery material, start before complete-response/capacity preflight and grant consumption, or succeed without retaining and verifying the exact bound inode/content in its private bounded quarantine;
- a file-write grant is not bound to exact content, create/replace disposition, or the old replacement identity; create can overwrite; replacement can destructively clean an uncertain object, skip bounded recovery retention, or claim hostile same-UID atomic rollback; mutation can begin before complete-response/quarantine-capacity preflight and atomic consumption; or its response/audits expose content, paths, digests, grants, sessions, JTIs, identities, or artifact names;
- a dependency advisory is unresolved without a documented accepted-risk decision;
- a high-impact capability appears without its independent gate and validation evidence.
