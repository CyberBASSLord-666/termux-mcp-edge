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

## Grant authority and key separation

Only a separately trusted capability-grant authority may mint mutation grants. The server must verify an exact configured issuer and audience in addition to the cryptographic signature or MAC. Authentication credentials, MCP bearer tokens, session secrets, transport credentials, audit keys, and replay-ledger digest keys must never be accepted as grant-signing keys or reused to mint grants.

Grant verification uses an explicit algorithm allowlist and a fail-closed keyring loaded from owner-only configuration. Caller-selected algorithms, embedded keys, remote key URLs, key discovery, algorithm substitution, and fallback to authentication secrets are prohibited. Duplicate key identifiers, unknown issuers or audiences, malformed key material, unsafe permissions, missing active verification keys, and ambiguous configuration prevent mutation capability startup.

Signing/verification keys and replay-ledger digest keys require domain separation and independent rotation. A retiring verification key must remain available until every grant it could have signed is expired plus the fixed clock-skew allowance. A retiring ledger-digest key must remain available until every replay record derived from it is safely expired and compacted. Key removal, upgrade, or rollback must never make an unexpired grant or consumed identifier unverifiable in a way that permits mutation; uncertainty fails closed.

## Trusted identity provenance

Grant binding must use identity values derived by the server only after successful authentication. Tool arguments, JSON-RPC identifiers, query parameters, forwarded headers, caller-selected labels, and unvalidated authorization metadata are not identity sources.

The authenticated principal binding must be a stable, non-secret credential identity assigned by trusted local configuration or derived with a dedicated keyed one-way function from the accepted credential. It must not be the raw bearer token, a loggable token prefix, or a value selected by the caller. Credential rotation must define an explicit overlap or replacement policy so that a grant cannot silently migrate to a different principal and an old credential cannot retain mutation authority beyond its configured retirement boundary.

The session binding must use the server-created MCP session record after lookup and phase validation. Merely echoing an `mcp-session-id` header is insufficient. The grant must bind the exact active server-side session identity and its authenticated-principal association. Missing, unknown, closed, expired, cross-principal, or re-created sessions fail closed. Session identifiers supplied before successful authentication must not influence grant verification.

If the configured authentication mode cannot produce a stable principal identity and a validated server-side session association, mutation capability startup must fail closed. Implementations must not weaken this requirement by treating every authenticated caller as an implicit shared principal unless that single-principal deployment mode is explicitly configured and documented; grants from that mode must remain non-transferable across credential rotation and session boundaries.

## Grant binding

The trusted grant authority must cryptographically bind:

- exact issuer and audience;
- server-derived authenticated principal identity;
- exact validated server-side MCP session identity and principal association;
- capability identifier `filesystem.create-directory`;
- selected safe-root identity;
- normalized root-relative target components;
- mutating posture;
- grant format version, algorithm, and allowlisted key identifier;
- a unique high-entropy grant identifier;
- issuance time, not-before time where present, and a short server-enforced expiry.

Confinement occurs before grant-path comparison. Unknown versions, issuers, audiences, keys, or algorithms; malformed grants; excessive lifetimes; future issuance beyond fixed skew; not-yet-valid or expired grants; clock rollback; binding mismatch; and replay fail closed with stable non-sensitive errors. Verification must cover the exact serialized claims representation and use constant-time comparison for authentication tags and fixed-size sensitive identifiers.

## Consumption and replay resistance

The server atomically consumes the grant immediately before the first mutation attempt. Concurrent replay must produce one winner and at most one mutation attempt. A consumed grant remains consumed after creation, verification, synchronization, response serialization, rollback, or cleanup failure. Dry-run requests do not consume mutating grants.

Replay state must be concurrency-safe, bounded, retained through expiry, and unavailable to caller-controlled identifiers or labels.

Consumption must also survive process crashes, runit restarts, package upgrades, and abrupt Android process death for the full remaining grant lifetime. An in-memory-only replay cache is insufficient. Before the first filesystem mutation, the implementation must durably publish the consumed identifier to a descriptor-confined, owner-only replay ledger and synchronize the ledger and its parent directory. Mutation is forbidden unless durable consumption succeeds. Recovery must reject every unexpired consumed identifier, tolerate a torn final record without accepting replay, compact only expired entries, and fail closed on corruption, rollback, permission drift, unsafe links, or storage exhaustion. Cleanup or compaction failure must never resurrect a consumed grant.

The durable ledger must contain only a keyed, non-reversible digest of the grant identifier plus the minimum expiry/version/key metadata required for replay enforcement. It must not contain grant material, principal/session identifiers, safe-root paths, target components, or other caller-derived labels. Ledger size, record count, startup scan work, compaction work, and retained lifetime must have fixed server-side ceilings suitable for Termux devices.

## Discovery and dispatch

The mutation path must remain unavailable while the dedicated runtime gate is disabled. Tool discovery and dispatch must use the same effective gate decision, with no time-of-check/time-of-use divergence. Dry-run availability must not permit a caller to bypass the mutating authorization path.

## Preserved controls

This change must not weaken authentication ordering, exact Host/Origin validation, request or response ceilings, descriptor-relative no-follow traversal, atomic no-replace publication, fixed mode `0700`, durability synchronization, identity-checked rollback, cleanup-failure precedence, or audit privacy.

## Required regression evidence

Tests must prove:

- disabled-gate discovery and dispatch denial;
- authorization-context-only grant transport and closed tool arguments;
- exact issuer/audience enforcement and rejection of caller-selected algorithms, embedded keys, remote key URLs, and authentication-secret fallback;
- separation of grant verification, authentication, transport, audit, and replay-ledger keys;
- fail-closed startup for missing, malformed, duplicate, permission-unsafe, rolled-back, or ambiguous key configuration;
- rejection of caller-controlled principal or session identity sources, including tool arguments, forwarded headers, query parameters, JSON-RPC identifiers, and unauthenticated session headers;
- stable server-derived principal binding without raw-token storage or logging;
- exact active server-side session binding, principal/session association validation, and denial of unknown, closed, expired, cross-principal, or re-created sessions;
- credential and session rotation behavior that prevents grants from migrating across principals, retired credentials, or replacement sessions;
- fail-closed mutation startup when the authentication mode cannot provide trustworthy principal and session provenance;
- principal/session, capability, root, path, posture, version/algorithm/key, issuance, not-before, expiry, and unique-ID binding;
- malformed, expired, future-issued, not-yet-valid, excessive-lifetime, unknown-version/key/issuer/audience, mismatched, replayed, and clock-rollback denial;
- retiring-key overlap through the maximum grant and replay-record lifetimes, including upgrade and rollback cases;
- one winner and at most one mutation attempt under concurrent replay;
- permanent consumption after all post-consumption failures;
- dry-run non-consumption;
- replay denial after clean restart, crash immediately after durable consumption, runit restart, and abrupt process termination;
- no mutation when replay-ledger publication or synchronization fails;
- fail-closed behavior for torn records, corruption, permission drift, symlink substitution, rollback, storage exhaustion, and bounded-capacity exhaustion;
- bounded startup recovery and compaction with no resurrection of unexpired consumed identifiers;
- replay-ledger privacy: keyed digests only, with no raw grant, principal/session, root, or target data;
- complete grant-secret and identifier redaction;
- unchanged authentication, Host/Origin, limits, safe-root, durability, rollback, and audit contracts;
- exact-head CI, Security, Android Cross Compile, device-smoke, and native official-Termux ARM64 execution.
