# `create_directory` capability-grant boundary

The `create_directory` mutation is a high-impact filesystem operation. Authentication and `dry_run: false` are not sufficient authorization.

## Mandatory authorization

Mutation requires all of the following:

1. an authenticated request;
2. the existing descriptor-relative safe-root confinement succeeding;
3. a dedicated, default-disabled, fail-closed directory-mutation runtime gate;
4. explicit `dry_run: false`;
5. a valid request-scoped, single-use `filesystem.create-directory` capability grant supplied only through authenticated authorization context.

Grant material must never appear in tool arguments, URLs, responses, logs, tracing fields, metrics labels, or audit labels.

## Grant binding

The trusted grant authority must cryptographically bind:

- authenticated principal and session/authorization context;
- capability identifier `filesystem.create-directory`;
- selected safe-root identity;
- normalized root-relative target components;
- mutating posture;
- grant format version and allowlisted key identifier;
- a unique high-entropy grant identifier;
- issuance time and a short server-enforced expiry.

Confinement occurs before grant-path comparison. Unknown versions, keys, or algorithms; malformed grants; excessive lifetimes; future issuance beyond fixed skew; expiry; clock rollback; binding mismatch; and replay fail closed with stable non-sensitive errors.

## Consumption and replay resistance

The server atomically consumes the grant immediately before the first mutation attempt. Concurrent replay must produce one winner and at most one mutation attempt. A consumed grant remains consumed after creation, verification, synchronization, response serialization, rollback, or cleanup failure. Dry-run requests do not consume mutating grants.

Replay state must be concurrency-safe, bounded, retained through expiry, and unavailable to caller-controlled identifiers or labels.

## Discovery and dispatch

The mutation path must remain unavailable while the dedicated runtime gate is disabled. Tool discovery and dispatch must use the same effective gate decision, with no time-of-check/time-of-use divergence. Dry-run availability must not permit a caller to bypass the mutating authorization path.

## Preserved controls

This change must not weaken authentication ordering, exact Host/Origin validation, request or response ceilings, descriptor-relative no-follow traversal, atomic no-replace publication, fixed mode `0700`, durability synchronization, identity-checked rollback, cleanup-failure precedence, or audit privacy.

## Required regression evidence

Tests must prove:

- disabled-gate discovery and dispatch denial;
- authorization-context-only grant transport and closed tool arguments;
- principal/session, capability, root, path, posture, version/key, issuance, expiry, and unique-ID binding;
- malformed, expired, future-issued, excessive-lifetime, unknown-version/key, mismatched, replayed, and clock-rollback denial;
- one winner and at most one mutation attempt under concurrent replay;
- permanent consumption after all post-consumption failures;
- dry-run non-consumption;
- complete grant-secret and identifier redaction;
- unchanged authentication, Host/Origin, limits, safe-root, durability, rollback, and audit contracts;
- exact-head CI, Security, Android Cross Compile, device-smoke, and native official-Termux ARM64 execution.
