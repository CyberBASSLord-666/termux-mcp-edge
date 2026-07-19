# Safe-rooted path metadata

`path_metadata` is the baseline read-only MCP capability for inspecting one regular file or directory beneath a configured filesystem safe root. It avoids parent-directory enumeration and file-content reads while preserving the same descriptor-relative no-follow boundary as the other filesystem tools.

## Request contract

The closed input schema accepts exactly one field:

- `path`: a required absolute path beneath one configured safe root.

Relative paths, parent traversal, NUL bytes, paths outside every safe root, unknown fields, missing objects, symlink components, and unsupported object types fail closed. There is no recursion, glob, query, content, hash, caller-selected response limit, or mutation option.

## Result contract

A successful structured result contains exactly:

- `path`: the normalized safe-rooted path;
- `kind`: `regular_file` or `directory`;
- `sizeBytes`: the regular-file byte size, or `null` for a directory;
- `modified`: an RFC 3339 UTC timestamp when the platform timestamp is representable, otherwise `null`;
- `maxResponseBytes`: the fixed full-response ceiling, `16384`.

The result does not expose file content, inode or device numbers, UID/GID values, raw permission bits, access or creation times, extended attributes, link targets, or raw operating-system errors. The text content is a fixed summary and does not repeat metadata or caller input.

## Descriptor and race boundary

The server anchors the request beneath the longest matching configured root label, duplicates and identity-verifies that root's lifetime-pinned no-follow descriptor, and walks every parent component relative to the retained duplicate. The final object is opened with Linux path-descriptor and no-follow semantics, then classified with `fstat` on that exact descriptor. The configured safe-root directory itself is inspected through the verified duplicate rather than by reopening its pathname.

Symlink final components are opened only as links long enough to classify and reject them; link targets are never resolved or returned. Sockets, FIFOs, devices, and other non-regular types are rejected. Holding the final descriptor prevents a concurrent rename or path exchange from redirecting metadata lookup to an outside object.

All blocking descriptor work runs through `spawn_blocking`. The complete JSON-RPC response is capped at 16 KiB, including the caller-controlled request identifier and envelope.

## Stable decisions and audit privacy

Successful calls use `safe_root_metadata_read`. Denials use only stable low-cardinality reasons such as `missing_arguments`, `invalid_arguments`, `safe_root_rejected`, `filesystem_path_not_found`, `filesystem_path_type_unsupported`, `response_size_limit_exceeded`, or `filesystem_metadata_failed`.

Audit counters retain only the tool, gate, mode, outcome, and reason labels. They never retain the path, filename, object kind, size, timestamp, request identifier, or raw error.

## Validation

Repository coverage includes regular and empty files, directories, the safe-root directory itself, exact byte size, timestamp shape, missing and outside paths, symlink parents and final components, unsupported FIFOs, final-object exchanges after descriptor acquisition, closed MCP arguments, full-response bounds, aggregate audit privacy, release-validator fixtures, device smoke checks, and native AArch64 Android execution in the pinned official Termux environment.
