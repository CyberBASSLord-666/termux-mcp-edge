# Safe-root binary read contract

`read_binary_file` is the baseline Class 1 tool for reading one arbitrary-byte regular file without expanding filesystem authority or requiring callers to treat binary data as UTF-8.

## Discovery and input

The tool is present only on the authenticated `mcp-runtime` surface. Its input schema is closed:

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Absolute path to one regular file inside a configured safe root."
    }
  },
  "required": ["path"],
  "additionalProperties": false
}
```

No encoding selector, byte range, offset, length, link-following option, alternate root, or mutation option is accepted.

## Result

A successful call returns exactly these `structuredContent` fields:

```json
{
  "encoding": "base64",
  "data": "AAEC/w==",
  "sizeBytes": 4,
  "maxFileBytes": 1048576,
  "maxResponseBytes": 1507328
}
```

- `data` is canonical padded RFC 4648 base64 using the standard alphabet.
- `sizeBytes` is the exact raw byte count, not the encoded length.
- `maxFileBytes` and `maxResponseBytes` are fixed capability metadata.
- The result never includes the path, filename, inode, device, UID, GID, mode, timestamps, link target, MIME guess, digest, or host error text.
- Empty files succeed with `data:""` and `sizeBytes:0`.

The maximum raw file is 1,048,576 bytes. Its maximum encoded payload is 1,398,104 bytes. The complete JSON-RPC response, including the caller's response ID, is capped at 1,507,328 bytes.

## Descriptor-relative confinement

The implementation:

1. anchors the requested absolute path to one configured safe-root descriptor;
2. rejects parent traversal, NUL data, paths outside every safe root, and symlinked descendant components;
3. walks the parent components descriptor-relatively with no-follow directory opens;
4. performs a no-follow final-object metadata lookup and requires a regular file;
5. rejects a reported size above 1 MiB before reading;
6. opens the final file read-only, no-follow, nonblocking, and close-on-exec;
7. requires the opened descriptor to remain a regular file and match the pre-open device/inode identity;
8. repeats the size check on the opened descriptor;
9. retains and reads that exact descriptor with a max-plus-one ceiling; and
10. rejects runtime growth rather than returning truncated or partial content.

The nonblocking final open prevents a concurrent regular-file-to-FIFO exchange from stalling a worker. Holding the verified descriptor prevents a post-open rename or symlink exchange from redirecting the read.

## Response preflight and memory bound

Before argument parsing or filesystem access, the transport serializes the maximum success envelope for the actual JSON-RPC ID and adds the exact maximum base64 payload length. If that envelope cannot fit, the call fails with the bounded payload-too-large response and a null response ID where necessary. This ordering prevents an oversized caller-controlled ID from triggering file access or allocating the maximum encoded content.

After the read, the transport still serializes the actual response through the shared full-response limiter. No success response may exceed `maxResponseBytes`.

Peak content storage is bounded by the 1 MiB raw vector plus the 1,398,104-byte encoded string and bounded JSON serialization. The tool does not stream an unbounded response, map a caller-selected file, or invoke a subprocess.

## Stable failures and audits

Missing/invalid arguments, outside-root paths, missing targets, unsupported object types, size violations, response violations, and internal failures use stable JSON-RPC/HTTP categories. Responses do not echo a requested path or operating-system error.

Audit counters retain only the tool name, gate, read-only decision, and one stable low-cardinality reason. Allowed calls use `safe_root_binary_read`. Denied binary-specific reasons are:

- `filesystem_binary_read_target_not_found`
- `filesystem_binary_read_type_unsupported`
- `filesystem_binary_read_size_limit_exceeded`
- `filesystem_binary_read_failed`

Shared argument, safe-root, and response-limit reasons are reused where applicable. Audits never retain the path, filename, raw bytes, base64 data, size, file identity, request ID, or host error.

## Validation requirements

Changes to this tool are blocked unless tests and release gates prove:

- exact closed discovery and runtime capability metadata;
- RFC 4648 canonical vectors, arbitrary binary data, and empty files;
- exact 1 MiB success and one-byte-over rejection;
- missing, outside-root, final symlink, linked-parent, directory, socket, and concurrent path-exchange behavior;
- max-plus-one rejection when a file grows after metadata validation;
- response preflight before argument parsing and file access;
- complete response bounding at the exact raw-file limit;
- path/content/host-metadata-private results and audit counters;
- parity across default and optional artifact discovery postures;
- release-validator v7, device-harness v7, and native official-Termux ARM64 execution for the exact candidate.

## Non-goals

`read_binary_file` does not authorize shared-storage access, recursive reads, directory archives, link following, alternate encodings, content-type detection, decompression, decryption, file mutation, upload, deletion, command execution, or network access. Bounded byte ranges are a separate Class 1 capability with a larger file ceiling and their own contract in [`SAFE_ROOT_BINARY_RANGES.md`](SAFE_ROOT_BINARY_RANGES.md); it does not broaden whole-file reads.
