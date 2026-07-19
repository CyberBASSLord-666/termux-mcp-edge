# Safe-root UTF-8 text range contract

`read_text_range` is the baseline Class 1 tool for paginating larger UTF-8 text files without authorizing an unbounded read or splitting a Unicode code point. It extends read authority only inside configured filesystem safe roots and does not increase the separate 1 MiB whole-file authority of `read_file`.

## Discovery and input

The tool is present only on the authenticated `mcp-runtime` surface. Its schema is closed:

```json
{
  "type": "object",
  "properties": {
    "path": {"type": "string"},
    "offset_bytes": {"type": "integer", "minimum": 0, "maximum": 67108864},
    "max_bytes": {"type": "integer", "minimum": 4, "maximum": 262144}
  },
  "required": ["path", "offset_bytes", "max_bytes"],
  "additionalProperties": false
}
```

- `path` identifies one absolute regular file beneath a configured safe root.
- `offset_bytes` is a zero-based UTF-8 byte boundary. Offset equal to EOF succeeds with an empty result; offset beyond EOF or on a continuation byte is rejected.
- `max_bytes` is the maximum source bytes to inspect and return. The fixed minimum of four bytes guarantees forward progress from every valid non-EOF boundary because a UTF-8 code point is at most four bytes.
- The held file may be at most 67,108,864 bytes (64 MiB), and one page may contain at most 262,144 UTF-8 bytes (256 KiB).

No alternate root, negative value, open-ended read, caller-selected encoding, replacement-character mode, link-following option, mutation option, or caller-selected response ceiling is accepted.

## Code-point boundary behavior

The selected page must start on a UTF-8 code-point boundary. If the fixed byte ceiling ends partway through an otherwise valid code point and the file contains more bytes, that incomplete suffix is deferred. The result contains only the valid prefix and `nextOffsetBytes` points to the deferred code point's first byte.

Malformed UTF-8 inside the selected page is rejected. An incomplete code point at actual EOF is also rejected rather than silently omitted or replaced. Validation is page-local: callers continue with the returned `nextOffsetBytes`; the tool does not scan unrelated file regions or claim that bytes outside the selected page are valid text.

## Result

A successful first page for the UTF-8 bytes of `aé🙂z` with `max_bytes: 4` returns exactly:

```json
{
  "content": "aé",
  "offsetBytes": 0,
  "nextOffsetBytes": 3,
  "sizeBytes": 3,
  "fileSizeBytes": 8,
  "eof": false,
  "maxReadBytes": 262144,
  "maxFileBytes": 67108864,
  "maxResponseBytes": 1703936
}
```

- `content` contains complete, valid UTF-8 only.
- `offsetBytes` is the accepted starting byte offset.
- `nextOffsetBytes` is the first unread UTF-8 boundary and is the value callers use for the next page.
- `sizeBytes` is the UTF-8 byte length of `content`, not its Unicode scalar count or JSON-escaped length.
- `fileSizeBytes` is the size verified on the held descriptor before the read and confirmed unchanged afterward.
- `eof` is true when `nextOffsetBytes` reaches EOF, including a call whose start offset is exactly EOF.
- The result never includes the path, filename, inode, device, UID, GID, mode, timestamps, link target, request ID, host error, or bytes from outside the selected page.

## Descriptor-relative confinement and consistency

The implementation:

1. validates the fixed numeric contract before filesystem work;
2. anchors the absolute path to one configured safe-root descriptor;
3. rejects parent traversal, NUL path data, outside-root paths, and symlinked descendants;
4. walks parent components descriptor-relatively with no-follow directory opens;
5. performs a no-follow final-object lookup and requires a regular file no larger than 64 MiB;
6. opens the final file read-only, no-follow, nonblocking, and close-on-exec;
7. requires the opened descriptor to remain a regular file and match the pre-open device/inode identity;
8. repeats the type and size ceiling checks on that descriptor;
9. rejects offset past EOF or beginning on a continuation byte;
10. seeks and reads at most the requested 256 KiB through that exact held descriptor;
11. repeats descriptor metadata inspection and rejects the entire result if the file size changed; and
12. validates UTF-8, defers only a partial non-EOF trailing code point, and calculates the next boundary with checked arithmetic.

Holding the verified descriptor prevents a post-open pathname exchange from redirecting the read. The nonblocking final open prevents a concurrent regular-file-to-FIFO exchange from stalling a worker. The post-read size check detects truncation or growth during the bounded operation; it does not claim snapshot semantics for same-size in-place writes or across separate page calls.

## Response preflight and memory bound

Before parsing arguments or accessing the filesystem, the transport serializes the maximum success envelope for the actual JSON-RPC ID and reserves six output bytes for every possible source byte. Six bytes covers the worst JSON string escape such as `\u0000`. If that maximum cannot fit within 1,703,936 bytes, the call returns a bounded payload-too-large response without touching the path. The actual success response also passes through the shared full-response limiter.

Peak content storage is bounded by the 256 KiB source vector, the validated UTF-8 string, and bounded JSON serialization. The tool does not map the 64 MiB file, allocate its file ceiling, stream an unbounded response, normalize text, or invoke a subprocess.

## Stable failures and audits

Invalid or missing arguments, invalid ranges, invalid or truncated selected UTF-8, outside-root paths, missing targets, unsupported object types, oversized files, detected size changes, response violations, and internal failures use stable JSON-RPC/HTTP categories. A detected size change returns HTTP 409 with JSON-RPC code `-32004`; no partial content is returned. Error responses do not echo the requested path, content, host error, or invalid bytes.

Allowed calls use audit reason `safe_root_text_range_read`. Tool-specific denied reasons are:

- `filesystem_text_range_target_not_found`
- `filesystem_text_range_type_unsupported`
- `filesystem_text_range_invalid`
- `filesystem_text_range_file_too_large`
- `filesystem_text_range_encoding_invalid`
- `filesystem_text_range_changed_during_read`
- `filesystem_text_range_failed`

Shared argument, safe-root, and response-limit reasons are reused where applicable. Audits never retain the path, filename, offset, requested/returned size, content, file size or identity, request ID, invalid bytes, or host error.

## Validation requirements

Changes to this tool are blocked unless tests and release gates prove:

- exact ordered discovery, closed schema, and runtime capability metadata;
- multi-byte pagination, deferred trailing code points, exact continuation offsets, final short pages, and empty EOF results;
- exact 256 KiB page success and one-byte-over rejection;
- exact 64 MiB sparse-file success and one-byte-over rejection;
- missing, outside-root, final-symlink, linked-parent, directory, socket, and descriptor-exchange behavior;
- midpoint, offset-past-EOF, malformed UTF-8, and truncated-at-EOF rejection;
- detected concurrent size-change rejection without partial output;
- response preflight before argument parsing and filesystem access;
- worst-case NUL escaping within the exact full-response ceiling;
- path/content/host-metadata-private results and audit counters;
- parity across baseline and every optional artifact discovery posture; and
- release-validator v8, device-harness v8, Android cross-builds, and native official-Termux ARM64 execution for the exact candidate.

## Non-goals

`read_text_range` does not authorize shared-storage access, recursive reads, directory archives, symlink following, alternate encodings, lossy decoding, Unicode normalization, line indexing, delimiter-aware paging, content search, mutation, upload, deletion, command execution, or network access. It provides bounded byte-offset pagination, not a persistent handle or a multi-call snapshot guarantee.
