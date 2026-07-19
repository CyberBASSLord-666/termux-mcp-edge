# Safe-root file hashing contract

`hash_file` computes a SHA-256 digest for one bounded regular file inside a configured filesystem safe root. It is a Class 1 read-only capability: it does not mutate the file, invoke a subprocess, return file content, or expose the requested path in its result or audit counters.

## Closed request schema

| Field | Type | Required | Contract |
| --- | --- | --- | --- |
| `path` | string | yes | Absolute path to one regular file inside a configured safe root. |

Unknown fields, missing `path`, wrong JSON types, relative paths, NUL bytes, explicit parent traversal, paths outside configured roots, symlinked parent or final components, directories, sockets, FIFOs, devices, and other non-regular objects are rejected.

## Fixed limits and result

- algorithm: SHA-256 only;
- maximum file size: 16,777,216 bytes;
- accepted data: arbitrary bytes, including empty and non-UTF-8 files;
- read buffer: 65,536 bytes;
- complete JSON-RPC response: at most 16,384 bytes;
- result privacy: no path, filename, content, metadata, partial digest, or raw operating-system error.

Successful `structuredContent` contains exactly:

```json
{
  "algorithm": "sha256",
  "digest": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
  "sizeBytes": 0
}
```

`digest` is always 64 lowercase hexadecimal characters. `sizeBytes` is the number of bytes consumed from the retained descriptor. The transport preflights the complete caller-specific success envelope, including the request id, against the 16 KiB ceiling before reading the file. An envelope that cannot fit fails with a bounded payload-too-large error and no hash operation.

## Descriptor-relative execution

The implementation does not authorize a pathname and reopen it later.

1. Select the longest matching configured safe root and retain normalized relative components.
2. Duplicate and identity-verify that root's lifetime-pinned descriptor, then walk every parent component relative to held descriptors with no-follow directory opens.
3. Inspect the final component with no-follow metadata and require a regular file no larger than 16 MiB.
4. Open that final component relative to the retained parent with read-only, nonblocking, no-follow, and close-on-exec flags. Nonblocking open prevents a concurrent swap to a FIFO from stalling a worker before final-type validation.
5. Verify that the opened descriptor is regular and that its device and inode match the no-follow path observation. A concurrent replacement therefore fails or leaves the operation attached to the already-open object; it cannot redirect hashing through a replacement link.
6. Stream at most 16 MiB plus one byte from the exact descriptor through SHA-256. The runtime byte counter independently rejects growth past the ceiling even if the pre-open size was within bounds.
7. Publish the digest only after end-of-file. Limit or I/O failures return no partial digest.

Hashing through a retained descriptor prevents pathname replacement from redirecting the read. It does not turn a mutable regular file into an atomic snapshot: an in-place writer can change bytes while the descriptor is being read. Operators who need reproducible artifact verification must quiesce writers or hash an immutable, atomically published file.

## Stable errors and audit privacy

Invalid schemas and rejected paths/types map to bounded client errors. Files over the byte ceiling or responses over the full-envelope ceiling map to bounded payload-too-large errors. Unexpected descriptor or read failures map to a stable internal failure without reflecting the raw error, path, content, or partial digest.

Successful calls record `hash_file` / `safe_root_file_hashed`. Denied calls use existing low-cardinality reasons such as `missing_arguments`, `invalid_arguments`, `safe_root_rejected`, `response_size_limit_exceeded`, or `filesystem_operation_failed`. Dedicated byte-limit metrics count oversize rejections, while aggregate audit counters retain no sizes or caller data.

Audit surfaces never retain:

- requested or normalized paths and filenames;
- content, digest values, or partial hashes;
- request or session identifiers;
- device/inode identities or other file metadata;
- raw operating-system errors.

## Deliberate non-capabilities

`hash_file` does not provide caller-selected algorithms, keyed hashes, signatures, recursive tree hashing, directory manifests, chunk digests, remote URLs, link following, unbounded files, progress streams, persistent checksum storage, file comparison, integrity enforcement, or mutation. Those would require separate bounded contracts and threat review.

## Required release evidence

Release validation must prove closed discovery schema, binary and empty-file correctness, exact 16 MiB acceptance, one-byte-over rejection, missing/outside/symlink/parent-link/non-regular denials, concurrent final-object exchange resistance, full-response bounding before the read, runtime growth enforcement, digest/path/content-private audit counters, canonical validator and device-smoke execution, all-feature Android cross-builds, and native official-Termux execution through the automated release gate.
