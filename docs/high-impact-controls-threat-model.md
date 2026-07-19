# High-impact controls threat model

## Purpose

This document defines the threat model and approval gate for any future high-impact MCP capability. Termux MCP Edge is intended for developers and advanced power users, but high-impact actions can change host, Android, service, package, network, or filesystem state in ways that are difficult to reverse. These actions require a stronger gate than read-only metadata, safe-rooted file access, or bounded command profiles.

This document does not enable general high-impact controls. One separately reviewed exact-stream volume capability satisfies a deliberately narrower subset of this model; its authoritative contract is [`ANDROID_VOLUME_CONTROL.md`](ANDROID_VOLUME_CONTROL.md).

The live `create_directory` request grant is a narrower, purpose-built authorization layer for one already-confined filesystem mutation. It is not a general high-impact token and grants no package, service, Android, network, process, secret, deletion, permission, or shell authority. Its independent contract is [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md).

The live Android volume grant is likewise purpose-built. It grants only one exact stream/level mutation in one authenticated session, uses a distinct signed capability code, expires after 60 seconds, and cannot authorize filesystem, shell, package, service, network, microphone, routing, playback, or other Android actions. The project-wide internal request-grant registry fixes directory at `1`, volume at `2`, write at `3`, and reserves copy at `4`; exhaustive cross-family tests prove each live grant is privately rejected by every other authority without consumption. Public directory-creation, file-write, and volume-control APIs expose preview only, while their prepared mutation values and execution paths remain crate-private to the grant-aware transport. The legacy public `copy_file` live path remains limited to trusted embeddings until that reserved copy family is implemented.

## High-impact action categories

A future tool must be treated as high-impact if it can perform any of the following:

- Package installation, removal, upgrade, downgrade, or repository configuration.
- Service start, stop, restart, reload, enable, disable, or supervision changes.
- Network configuration changes, tunnel creation/removal, firewall changes, port binding changes, routing changes, DNS changes, or proxy configuration changes.
- Android device-control actions, including notifications, intents, sensors, camera, microphone, location, accessibility, clipboard mutation, SMS, contacts, accounts, or settings changes.
- Filesystem deletion, recursive movement, permission changes, ownership changes, or writes outside the existing staged safe-root write policy.
- Credential, token, key, certificate, or secret-store mutation.
- Process termination or signal delivery.
- Long-running background jobs, scheduled tasks, or persistence changes.
- Any operation that could materially affect availability, confidentiality, integrity, billing, device state, or user safety.

## Threat actors and failure modes

The model assumes advanced operators but still accounts for:

- Accidental misuse by an authenticated operator.
- Prompt-injection or tool-confusion attempts from upstream MCP clients.
- Compromised browser-origin or local client contexts.
- Misconfigured tunnels, proxies, or LAN exposure.
- Stale capability tokens.
- Partial operation failures that leave the device or service in an inconsistent state.
- Output truncation hiding important warnings.
- Concurrent invocations racing against each other.

## Capability-token model

High-impact tools require a dedicated capability token or equivalent confirmation design separate from normal transport authentication.

Minimum properties:

- Disabled by default.
- Tool-family specific, not global.
- Short-lived where practical.
- Bound to the exact high-impact action category.
- Bound to dry-run or mutating mode.
- Revocable by configuration reload or process restart.
- Never logged in plaintext.
- Never accepted through a raw command string.

A bearer token that authorizes transport access is not sufficient by itself for high-impact mutation.

## Confirmation model

A future high-impact request should require an explicit confirmation payload that includes:

- Tool name.
- Action category.
- Target identifier.
- Dry-run result identifier or preview hash when applicable.
- Requested mutating mode.
- Operator-supplied confirmation token or capability binding.

Confirmation must not be inferred from the presence of a valid transport token alone.

## Dry-run and preview requirements

Dry-run or preview mode is required wherever feasible.

Preview output should include:

- Action category.
- Target summary.
- Files, services, packages, routes, or configuration keys that would change.
- Estimated reversibility.
- Required capability token scope.
- Audit reason code.

Preview output must not include secrets, raw credential material, full environment dumps, private file contents, or unrelated host paths.

If dry-run is not technically feasible, the tool design must explicitly document why and require stronger confirmation controls.

## Rollback expectations

Every high-impact tool design must classify rollback behavior:

- `automatic`: operation can be automatically reverted by the runtime.
- `guided`: runtime can provide precise manual rollback instructions.
- `best_effort`: runtime can attempt cleanup but cannot guarantee full reversal.
- `irreversible`: operation cannot be reliably reversed.

Mutating tools with `best_effort` or `irreversible` rollback must require elevated confirmation language and audit reason codes.

## Audit requirements

Every high-impact attempt must emit an audit event or equivalent structured counters for both allowed and denied decisions.

Required audit fields:

- Timestamp.
- Tool name.
- Action category.
- Target summary or safe-rooted target path where applicable.
- Dry-run versus mutating mode.
- Allowed or denied decision.
- Reason code.
- Capability-token scope, never the token value.
- Rollback classification.
- Bounded size/limit metadata where relevant.
- Duration and completion state.

Denied decisions must be audited before returning.

Audit output must not include:

- Secrets.
- Raw command output.
- Raw file contents.
- Full environment variables.
- Persistent Android identifiers.
- Contact, SMS, notification, account, camera, microphone, location, or accessibility data unless the specific future tool has its own approved data-minimization design.

## Concurrency and idempotency

High-impact tools must define concurrency behavior before implementation.

Required decisions:

- Whether identical requests are deduplicated.
- Whether conflicting requests are rejected.
- Whether the tool uses a lock file, in-memory lock, or external lock.
- What cleanup happens on cancellation, timeout, panic, or process shutdown.
- How partial state is detected on retry.

Mutating shell scripts and service/network tools must include explicit teardown traps or cleanup paths for temporary files, network binds, lock files, and transient configuration.

## Policy response model

Rejected high-impact requests should return structured policy errors with stable reason codes.

Minimum reason codes:

- `high_impact_disabled`
- `capability_token_missing`
- `capability_token_scope_mismatch`
- `confirmation_required`
- `dry_run_required_first`
- `target_not_allowlisted`
- `rollback_not_defined`
- `concurrency_conflict`
- `audit_sink_unavailable`
- `security_review_required`

## Required implementation sequence

1. Add inert action-category and policy data types.
2. Add policy tests for disabled-by-default behavior.
3. Add capability-token scope validation primitives.
4. Add dry-run/preview result model.
5. Add audit event integration for denied decisions.
6. Add one narrow high-impact family behind compile-time and runtime gates.
7. Add operator documentation and manual recovery notes.
8. Only then expose MCP discovery/tool-call handling for that family.

The exact-stream volume slice completed this sequence with a preview-first model, offline exact-binary grants, fixed execution, non-queueing concurrency, verification, automatic restoration, cancellation-independent recovery, and aggregate privacy-preserving counters. Aggregate counters are the intentionally bounded audit design for this slice; they retain stable decision/recovery labels and no per-request timestamps or targets.

## Required tests before any high-impact tool is enabled

- Disabled-by-default runtime status.
- Tool discovery does not expose high-impact tools until gates are enabled.
- Missing capability token is denied.
- Wrong token scope is denied.
- Missing confirmation is denied.
- Dry-run-first policy is enforced where applicable.
- Non-allowlisted target is denied.
- Timeout/cancellation triggers cleanup.
- Audit events are emitted for allowed and denied decisions.
- Rollback classification appears in dry-run output.
- Secrets and raw credential material are absent from responses and audit events.

## Security review requirement

Every implementation PR that enables a high-impact tool must include a security-review checklist in the PR body.

The checklist must confirm:

- Feature gate and runtime opt-in are required.
- Command execution is not implicitly enabled unless separately approved.
- Audit behavior is tested.
- Rollback behavior is documented.
- Operator confirmation is required for mutation.
- Exact-head CI passed.
- Security workflow passed if dependencies, lockfiles, workflows, or security-relevant configuration changed.

## Non-goals for this gate

- No package installation/removal implementation.
- No service restart/stop implementation.
- No network or tunnel mutation implementation.
- No Android device-control implementation beyond the separately authorized exact-stream volume slice.
- No raw shell execution.
- No broad host-control tool.
