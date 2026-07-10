# Capability-token evaluation contract

Termux MCP Edge treats high-impact capability tokens as future authorization metadata, not as an enabled runtime surface. The current primitives in `src/capability_token.rs` are intentionally inert: they model decisions for later review without accepting, generating, persisting, serializing, or validating raw bearer tokens or secrets.

This contract defines the minimum integration boundary for any later PR that evaluates high-impact capability grants.

## Current status

- Capability-token policy primitives exist for review and tests.
- No high-impact MCP tools are exposed.
- No command execution, package management, Android device control, network mutation, or project-service mutation is enabled by these primitives.
- Runtime behavior remains default-deny unless a later focused PR explicitly changes a narrowly scoped gate.

## Required evaluation inputs

Future callers may evaluate only stable, non-secret authorization metadata:

- `CapabilityRequirement`
  - `capability_class`: one explicit high-impact class.
  - `scope`: a bounded scope label such as `project-service:restart` or `command-profile:diagnostics`.
  - `confirmation_required`: whether separate operator confirmation is mandatory.
- `CapabilityGrant`
  - `grant_id`: an opaque identifier suitable for audit correlation.
  - `capability_class`: the class granted by the operator-approved policy.
  - `scope`: the exact bounded scope granted.
  - `expires_unix_seconds`: absolute expiry time.
  - `active`: whether the grant is currently enabled.
  - `confirmation_satisfied`: whether the separate confirmation step has already been completed.
- `now_unix_seconds`: caller-supplied evaluation time.

Future integrations must not pass raw bearer tokens, access tokens, refresh tokens, passwords, environment values, file contents, command output, Android identifiers, private filesystem paths, or user secrets into capability-token evaluation.

## Decision contract

Capability evaluation is exact-match and fail-closed.

An evaluation is allowed only when all of the following are true:

1. A grant is present.
2. The grant is active.
3. The grant has not expired.
4. The grant capability class equals the requirement capability class.
5. The grant scope equals the requirement scope.
6. Required operator confirmation has been satisfied.

All other cases return a denied decision with a stable reason code:

- `capability_grant_missing`
- `capability_grant_inactive`
- `capability_grant_expired`
- `capability_class_mismatch`
- `capability_scope_mismatch`
- `capability_confirmation_required`

Allowed evaluations use `capability_grant_allowed`.

## Audit expectations

Any future runtime wiring must emit or count only stable, non-sensitive metadata:

- Tool name
- Gate name
- Capability class
- Bounded scope label
- Dry-run, preview, or mutating mode where relevant
- Allowed or denied decision
- Stable reason code
- Opaque grant identifier when safe and useful for correlation

Audit records and counters must not store:

- Raw tokens or token material
- Passwords, secrets, API keys, or environment values
- Raw command lines beyond fixed allowlist names
- Command stdout or stderr
- File contents
- Private filesystem paths
- Android identifiers
- Global process inventories

## Runtime integration requirements

Before any future PR wires these primitives into a live runtime gate, that PR must prove all of the following:

1. The affected capability is explicitly listed in documentation and discovery behavior.
2. The tool remains unavailable unless its compile-time and runtime gates are both satisfied.
3. Inputs are allowlisted and bounded.
4. Mutating behavior has dry-run or preview behavior where feasible.
5. Missing, inactive, expired, mismatched, and unconfirmed grants are denied with structured failures.
6. Audit coverage records non-sensitive decision metadata.
7. Tests cover allowed and denied decisions, sensitive-data exclusion, and default-disabled behavior.
8. Existing lower-risk tools keep their response contracts unless the PR explicitly documents an additive change.

## Non-goals

This contract does not introduce:

- A token issuance system
- Token storage
- Raw bearer-token parsing
- Arbitrary command execution
- Shell access
- Package installation or removal
- Android control actions
- Network mutation
- Project-service mutation
- Global process listing
- Environment-variable exposure
- Filesystem access beyond existing safe-rooted tools

Any future high-impact runtime surface must be implemented in a separate focused issue and PR with its own tests, audit coverage, and explicit opt-in gate.

Originally added for #133; synchronized to current project governance by #165.
