# Safe-root file copy contract

`copy_file` copies one bounded regular file between configured filesystem safe roots without returning file contents or paths. Omitted `dry_run` and explicit `dry_run:true` perform the same fully validated preview without publishing a destination. Class 2 mutation additionally requires the independent default-false copy gate and one exact request-scoped grant; explicit `dry_run:false` alone is denied. The complete authorization and issuance contract is [copy-file capability grants](COPY_FILE_CAPABILITY_GRANTS.md).

## Closed request schema

| Field | Type | Required | Contract |
| --- | --- | --- | --- |
| `source_path` | string | yes | Absolute path to one regular file inside a configured safe root. |
| `destination_path` | string | yes | Absolute path to one absent destination whose parent already exists inside a configured safe root. |
| `dry_run` | boolean | no | Defaults to `true`; explicit `false` requests the separately gated and grant-authorized mutation path. |

Unknown fields, missing required fields, wrong JSON types, relative paths, NUL bytes, parent traversal, the safe-root directory itself, and equal normalized source/destination paths are rejected. Source and destination may be under different configured safe roots; both are authorized independently.

## Fixed limits and result

- maximum source size: 1,048,576 bytes;
- accepted data: arbitrary bytes, including empty files and non-UTF-8 content;
- destination mode: fixed `0600`, independent of source mode and process umask;
- complete JSON-RPC success response: at most 16,384 bytes;
- response content: dry-run posture, mode, byte count, and fixed limits only; paths and file bytes are never returned.

The successful `structuredContent` object is exactly:

```json
{
  "dryRun": false,
  "sizeBytes": 123,
  "mode": "0600",
  "maxFileBytes": 1048576,
  "maxResponseBytes": 16384
}
```

Before filesystem preparation or grant consumption, the transport constructs a worst-case success result using the fixed maximum byte count and verifies that the complete caller-specific JSON-RPC envelope—including the request id—fits the 16 KiB ceiling. A response that cannot fit is rejected before source access, grant consumption, or staging.

## Descriptor-relative execution

The operation does not invoke a shell, subprocess, platform copy utility, archive tool, or external provider.

1. Anchor source and destination lexically beneath the longest matching configured safe roots.
2. Duplicate and identity-verify each selected root's lifetime-pinned descriptor, then traverse every descendant component descriptor-relatively with no-follow semantics.
3. Inspect the source final component without following links and require a regular file at or below the fixed limit.
4. Open the source with `O_NOFOLLOW`, `O_NONBLOCK`, and close-on-exec; verify that device, inode, type, and size match the pre-open observation.
5. Read at most 1 MiB plus one byte from that exact held descriptor and verify its type, identity, and size again after the read.
6. Compute SHA-256 over the exact held bytes and bind it with source device, inode, size, high-resolution change time, one-link count, anchored root identity, and normalized source components.
7. Retain the duplicated destination-root and destination-parent descriptors; require the final destination component to be absent without following links and bind both destination root identity and normalized components.
8. For preview, return only after all source and destination validation has succeeded.
9. For explicit mutation, acquire the shared process publication lock, then revalidate both root identities, held and named source identity, exact bytes and SHA-256, destination-parent identity, destination absence, and hidden staging capacity before cancellation ownership and grant consumption.
10. Create an unpredictable staging file exclusively inside the destination parent's reserved mode-`0700` `.termux-mcp-write-quarantine`, force mode `0600`, write the grant-bound bytes, sync it, and verify its held and named identity, type, mode, link count, and size.
11. Publish from the hidden quarantine to the held destination parent with atomic `RENAME_NOREPLACE`, verify both the published name and still-held descriptor against the captured staging identity and contract, sync both directories, and revalidate quarantine bounds.

The held source descriptor prevents a later pathname exchange from redirecting reads. The held destination-parent descriptor prevents a later parent exchange from redirecting staging, publication, cleanup, or durability sync. The process lock serializes cooperating create/copy/write instances. Atomic no-replace publication means a concurrently inserted destination wins and is never overwritten.

## Cleanup and failure semantics

Staging cleanup is armed only after the newly created file identity is captured. Cleanup uses the authoritative quarantine or destination-parent descriptor, stats the current name without following links, and removes it only when it is still the captured single-link regular file. After publication, cleanup follows the final name under the held destination parent. A replacement object or unknown identity/type is preserved. Successful cleanup syncs its parent best-effort; successful publication requires destination-parent and quarantine sync.

Any failed explicit copy leaves the source untouched and must not replace an existing destination. If failure occurs after publication but before the durability boundary, identity-checked cleanup removes only this operation's published file when it is still the captured object.

The operation verifies source identity, high-resolution change time, link count, exact bytes, size, and SHA-256 during initial preparation and again under the publication lock. The copied bytes are the in-memory bytes covered by the grant, so a source change after the final revalidation cannot redirect or alter the authorized destination content. A same-UID external writer can still force bounded denial by racing namespace or metadata checks; operators must give the service exclusive operational ownership of live-mutation safe roots.

## Stable audit surface

Audit events and aggregate counters contain only tool, gate, dry-run/mutating mode, decision, and low-cardinality reason code. They never contain paths, content, request ids, operating-system errors, device/inode identifiers, or temporary names.

Allowed reasons:

- `dry_run_preview`;
- `safe_root_file_copied`.

Denied reasons:

- `missing_arguments`;
- `invalid_arguments`;
- `safe_root_rejected`;
- `filesystem_copy_source_not_found`;
- `filesystem_copy_parent_not_found`;
- `filesystem_copy_same_path`;
- `filesystem_destination_exists`;
- `filesystem_copy_source_type_unsupported`;
- `filesystem_copy_source_too_large`;
- `copy_file_mutation_disabled`;
- `filesystem_copy_source_changed`;
- `filesystem_copy_destination_changed`;
- the stable `capability_*` authorization reasons;
- `filesystem_mutation_worker_capacity_exceeded`;
- `filesystem_mutation_request_cancelled`;
- `response_size_limit_exceeded`;
- `filesystem_copy_failed`.

## Deliberate non-capabilities

`copy_file` does not provide recursive or directory copy, overwrite, append, sparse-file preservation, hard-link or symlink copy, link following, caller-selected permissions, ownership/group preservation, timestamps, extended attributes, ACLs, Android media metadata, progress streaming, remote transfer, archive extraction, or cancellation-as-rollback. Larger or multi-object transfers require a separately reviewed bounded contract rather than expanding this tool.

## Required release evidence

Release validation must prove default-disabled discovery and denial, enabled exact-binary grant issuance, preview non-consumption, binding and replay denial, explicit binary copy, exact 1 MiB acceptance, one-byte-over rejection, hidden staging, fixed `0600` mode, absent-destination/no-replace behavior, stale source/destination non-consumption, same/missing/outside/symlink/directory denials, cross-root operation, descriptor exchange resistance, identity-safe foreign-object preservation, pre-access actual-id response bounding, path/content/digest/grant-private responses and audit counters, official Android cross-builds, emulated gates, and native Termux execution.
