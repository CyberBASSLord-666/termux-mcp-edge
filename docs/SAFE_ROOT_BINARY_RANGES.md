# Safe-root binary range read contract

`read_binary_range` is the baseline Class 1 tool for retrieving one bounded arbitrary-byte slice from a larger regular file. It extends read capability only to the configured filesystem safe roots and does not increase the 1 MiB whole-file authority of `read_binary_file`.

## Discovery and input

The tool is present only on the authenticated `mcp-runtime` surface. Its schema is closed:

```json
{
  "type": "object",
  "properties": {
    "path": {"type": "string"},
    "offset_bytes": {"type": "integer", "minimum": 0, "maximum": 67108864},
    "length_bytes": {"type": "integer", "minimum": 1, "maximum": 262144}
  },
  "required": ["path", "offset_bytes", "length_bytes"],
  "additionalProperties": false
}
```

- `path` identifies one absolute regular file beneath a configured safe root.
- `offset_bytes` is the zero-based raw-byte offset. Offset equal to EOF is valid and returns an empty result; offset beyond EOF is rejected.
- `length_bytes` is the maximum number of raw bytes to return. A final short range succeeds and reports EOF.
- The file may be at most 67,108,864 bytes (64 MiB), and one result may contain at most 262,144 raw bytes (256 KiB).

No alternate root, negative value, open-ended length, encoding selector, link-following option, mutation option, or caller-selected response ceiling is accepted.

## Result

A successful call returns exactly these `structuredContent` fields:

```json
{
  "encoding": "base64",
  "data": "gGEKAQ==",
  "offsetBytes": 2,
  "sizeBytes": 4,
  "fileSizeBytes": 7,
  "eof": false,
  "maxReadBytes": 262144,
  "maxFileBytes": 67108864,
  "maxResponseBytes": 393216
}
```

- `data` is canonical padded RFC 4648 base64 using the standard alphabet.
- `offsetBytes` is the accepted raw-byte offset.
- `sizeBytes` is the returned raw-byte count, not the encoded length.
- `fileSizeBytes` is the size verified on the held descriptor before the read and confirmed unchanged after it.
- `eof` is true when the returned range reaches EOF, including an offset exactly equal to EOF.
- The result never includes the path, filename, inode, device, UID, GID, mode, timestamps, link target, MIME guess, digest, or host error text.

The largest raw range encodes to 349,528 base64 bytes. The complete JSON-RPC response, including the caller's response ID, is capped at 393,216 bytes.

## Descriptor-relative confinement and consistency

The implementation:

1. validates the fixed numeric contract before filesystem work;
2. anchors the absolute path to one configured safe-root descriptor;
3. rejects parent traversal, NUL data, outside-root paths, and symlinked descendant components;
4. walks parent components descriptor-relatively with no-follow directory opens;
5. performs a no-follow final-object metadata lookup and requires a regular file no larger than 64 MiB;
6. opens the final file read-only, no-follow, nonblocking, and close-on-exec;
7. requires the opened descriptor to remain a regular file and match the pre-open device/inode identity;
8. repeats the type and size ceiling checks on the opened descriptor;
9. rejects an offset beyond the opened size;
10. seeks and reads through that exact held descriptor with the requested 256 KiB-or-smaller limit; and
11. repeats descriptor metadata inspection and rejects the entire result if the file size changed.

Holding the verified descriptor prevents a post-open pathname exchange from redirecting the read. The nonblocking final open prevents a concurrent regular-file-to-FIFO exchange from stalling a worker. The post-read size check detects truncation or growth during the bounded operation; it does not claim snapshot semantics for same-size in-place writes.

## Response preflight and memory bound

Before argument parsing or file access, the transport serializes the maximum success envelope for the actual JSON-RPC ID and adds the exact maximum base64 payload length. If it cannot fit, the call returns a bounded payload-too-large response without touching the file. The actual success response also passes through the shared full-response limiter.

Peak content storage is bounded by the 256 KiB raw vector, the 349,528-byte encoded string, and bounded JSON serialization. The tool does not map the file, allocate the 64 MiB file ceiling, stream an unbounded response, or invoke a subprocess.

## Stable failures and audits

Invalid or missing arguments, invalid ranges, outside-root paths, missing targets, unsupported object types, oversized files, detected size changes, response violations, and internal failures use stable JSON-RPC/HTTP categories. A detected size change returns HTTP 409 with JSON-RPC code `-32004`; no partial data is returned. Responses never echo a requested path or host error.

Allowed calls use audit reason `safe_root_binary_range_read`. Binary-range-specific denied reasons are:

- `filesystem_binary_range_target_not_found`
- `filesystem_binary_range_type_unsupported`
- `filesystem_binary_range_invalid`
- `filesystem_binary_range_file_too_large`
- `filesystem_binary_range_changed_during_read`
- `filesystem_binary_range_failed`

Shared argument, safe-root, and response-limit reasons are reused where applicable. Audits never retain the path, filename, offset, requested or returned length, raw bytes, base64 data, file size or identity, request ID, or host error.

## Validation requirements

Changes to this tool are blocked unless tests and release gates prove:

- exact ordered discovery, closed schema, and runtime capability metadata;
- canonical arbitrary-byte slices, short final ranges, and explicit empty EOF results;
- exact 256 KiB range success and one-byte-over rejection;
- exact 64 MiB sparse-file success and one-byte-over rejection;
- missing, outside-root, final-symlink, linked-parent, directory, socket, and descriptor-exchange behavior;
- offset-past-EOF and detected concurrent size-change rejection without partial output;
- response preflight before argument parsing and file access;
- complete response bounding at the exact range limit;
- path/content/host-metadata-private results and audit counters;
- parity across baseline and every optional artifact discovery posture; and
- release-validator v8, device-harness v8, Android cross-builds, and native official-Termux ARM64 execution for the exact candidate.

## Non-goals

`read_binary_range` does not authorize shared-storage access, whole files larger than the separate 1 MiB whole-file contract, recursive reads, directory archives, symlink following, alternate encodings, content-type detection, decompression, decryption, mutation, upload, deletion, command execution, or network access. It provides bounded range retrieval, not a persistent open-file handle or a multi-call snapshot guarantee.
