# Safe-root file copy contract

`copy_file` copies one bounded regular file between configured filesystem safe roots without returning file contents. It is a Class 2 safe-rooted mutation only when the caller explicitly supplies `dry_run: false`; omitted `dry_run` and `dry_run: true` perform the same fully validated preview without publishing a destination.

## Closed request schema

| Field | Type | Required | Contract |
| --- | --- | --- | --- |
| `source_path` | string | yes | Absolute path to one regular file inside a configured safe root. |
| `destination_path` | string | yes | Absolute path to one absent destination whose parent already exists inside a configured safe root. |
| `dry_run` | boolean | no | Defaults to `true`; only explicit `false` authorizes mutation. |

Unknown fields, missing required fields, wrong JSON types, relative paths, NUL bytes, parent traversal, the safe-root directory itself, and equal normalized source/destination paths are rejected. Source and destination may be under different configured safe roots; both are authorized independently.

## Fixed limits and result

- maximum source size: 1,048,576 bytes;
- accepted data: arbitrary bytes, including empty files and non-UTF-8 content;
- destination mode: fixed `0600`, independent of source mode and process umask;
- complete JSON-RPC success response: at most 16,384 bytes;
- response content: normalized paths, mode, byte count, and fixed limits only; file bytes are never returned.

The successful `structuredContent` object is exactly:

```json
{
  "sourcePath": "/configured/root/source.bin",
  "destinationPath": "/configured/root/destination.bin",
  "dryRun": false,
  "sizeBytes": 123,
  "mode": "0600",
  "maxFileBytes": 1048576,
  "maxResponseBytes": 16384
}
```

Before any mutation, the transport constructs a worst-case success result using the fixed maximum byte count and verifies that the complete caller-specific JSON-RPC envelope—including the request id—fits the 16 KiB ceiling. A response that cannot fit is rejected before a staging object is created.

## Descriptor-relative execution

The operation does not invoke a shell, subprocess, platform copy utility, archive tool, or external provider.

1. Anchor source and destination lexically beneath the longest matching configured safe roots.
2. Open each safe-root directory and traverse every descendant component descriptor-relatively with no-follow semantics.
3. Inspect the source final component without following links and require a regular file at or below the fixed limit.
4. Open the source with `O_NOFOLLOW`, `O_NONBLOCK`, and close-on-exec; verify that device, inode, type, and size match the pre-open observation.
5. Read at most 1 MiB plus one byte from that exact held descriptor and verify its type, identity, and size again after the read.
6. Open and retain the destination-parent descriptor; require the final destination component to be absent without following links.
7. For preview, return only after all source and destination validation has succeeded.
8. For explicit mutation, create an unpredictable same-directory staging file with exclusive no-follow creation, force mode `0600`, write the bounded bytes, sync it, and verify its held identity, type, mode, and size.
9. Publish with atomic `RENAME_NOREPLACE`, verify both the published path and still-held descriptor against the captured staging identity and contract, then sync the destination parent.

The held source descriptor prevents a later pathname exchange from redirecting reads. The held destination-parent descriptor prevents a later parent exchange from redirecting staging, publication, cleanup, or durability sync. Atomic no-replace publication means a concurrently inserted destination wins and is never overwritten.

## Cleanup and failure semantics

Staging cleanup is armed only after the newly created file identity is captured. Cleanup stats the current name without following links and removes it only when it is still a regular file with the captured device and inode. After publication, cleanup follows the final name under the same held parent descriptor. A replacement object is preserved. Successful cleanup syncs the parent best-effort; successful publication requires parent sync.

Any failed explicit copy leaves the source untouched and must not replace an existing destination. If failure occurs after publication but before the durability boundary, identity-checked cleanup removes only this operation's published file when it is still the captured object.

The operation verifies a stable source identity and size around its bounded read. It cannot detect an in-place writer that changes bytes without changing file identity or final size. Operators requiring an application-level snapshot must quiesce source writers or provide an immutable source file before calling `copy_file`.

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
- `response_size_limit_exceeded`;
- `filesystem_copy_failed`.

## Deliberate non-capabilities

`copy_file` does not provide recursive or directory copy, overwrite, append, sparse-file preservation, hard-link or symlink copy, link following, caller-selected permissions, ownership/group preservation, timestamps, extended attributes, ACLs, Android media metadata, progress streaming, remote transfer, archive extraction, or cancellation-as-rollback. Larger or multi-object transfers require a separately reviewed bounded contract rather than expanding this tool.

## Required release evidence

Release validation must prove default preview and explicit binary copy, exact 1 MiB acceptance, one-byte-over rejection, fixed `0600` mode, absent-destination/no-replace behavior, same/missing/outside/symlink/directory denials, cross-root operation, descriptor exchange resistance, identity-safe cleanup, pre-mutation response bounding, content-private responses and audit counters, official Android cross-builds, and native Termux execution.
