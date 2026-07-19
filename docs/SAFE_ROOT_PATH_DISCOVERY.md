# Safe-root path discovery contract

`find_paths` is the baseline Class 1 tool for locating regular files and directories by a bounded literal basename substring. It reads no file content, follows no links, performs no mutation, and receives no authority beyond the configured filesystem safe roots.

## Request

The closed input object accepts only:

| Field | Required | Contract |
| --- | --- | --- |
| `path` | Yes | Absolute existing directory beneath one configured safe root. |
| `query` | Yes | Non-empty, case-sensitive literal UTF-8 basename substring, at most 256 bytes. NUL, CR, LF, and `/` are rejected. |
| `kind` | No | `any`, `regular_file`, or `directory`; defaults to `any`. |
| `max_depth` | No | Integer 1 through 5; defaults to 5. Entries directly below `path` are depth 1. |

The query is never interpreted as a regular expression, glob, path, shell fragment, or locale-sensitive pattern. Matching uses exact Unicode scalar/UTF-8 string equality semantics through Rust's case-sensitive literal substring operation. The tool does not normalize case or Unicode.

## Descriptor-relative traversal

The runtime:

1. selects the most specific configured safe root that lexically contains `path`;
2. opens that root as a no-follow directory descriptor;
3. walks every starting-path component with descriptor-relative no-follow directory opens;
4. reads descendants through held directory descriptors;
5. classifies each child with no-follow descriptor-relative metadata;
6. traverses only verified directories and returns only verified regular files or directories.

Symbolic links and non-file/non-directory objects are skipped and counted as unsafe. Names that are not valid UTF-8 are skipped and counted separately because they cannot be represented unambiguously in JSON. Metadata or open failures are counted as unreadable without returning names, paths, or operating-system error text. An invalid-UTF-8 directory name is not traversed.

The call is not a filesystem snapshot. Concurrent directory changes may cause an entry to disappear before classification or a directory to become unreadable; those cases produce bounded skip counters. The tool never reclassifies a link target and never returns device, inode, owner, permission, access-time, or file-content metadata.

## Fixed limits and ordering

| Limit | Value |
| --- | ---: |
| Query bytes | 256 |
| Traversal depth | 5 |
| Entries examined | 8,192 |
| Matches returned | 512 |
| Complete JSON-RPC response | 262,144 bytes |

Entries within each opened directory are processed in lexical basename order after bounded collection, and published matches are sorted lexicographically by full returned path. Reaching the entry or match execution ceiling sets `truncated: true`. If the bounded metadata and paths would exceed the internal structured-content budget, lexicographically largest matches are removed until the complete-response contract can be satisfied and `truncated` remains true.

Before parsing tool arguments or accessing the filesystem, the transport proves that the caller-controlled JSON-RPC ID leaves room for the maximum structured-content budget and maximum summary. If it does not, the call returns HTTP 413 / JSON-RPC `-32001` with a bounded error and null ID. The actual serialized success response is checked again before publication.

## Response

Successful `structuredContent` contains exactly:

- `path`: the validated starting directory;
- `matches`: ordered objects containing only `path` and `kind`;
- `truncated`;
- `entriesExamined`;
- `skippedInvalidUtf8Entries`;
- `skippedUnsafeEntries`;
- `skippedUnreadableEntries`;
- `queryBytes`;
- `kindFilter`;
- `maxDepth`;
- `maxEntries`;
- `maxMatches`;
- `maxResponseBytes`.

The raw query is not echoed. File contents, excerpts, sizes, timestamps, identities, permissions, and host metadata are absent. Use `path_metadata`, `read_file`, `read_text_range`, `read_binary_file`, `read_binary_range`, or `hash_file` in a separate bounded call when that independently confined information is needed.

## Audit privacy

Allowed calls increment `find_paths` with reason `safe_root_paths_found`. Denials use only existing stable filesystem reasons plus:

- `find_query_invalid`;
- `filesystem_find_failed`.

Counters retain no starting path, matched path, filename, query, kind value, request ID, filesystem identity, or raw error. Only stable tool/gate/mode/decision/reason labels and aggregate counts are retained.

## Validation evidence

The exact candidate must prove:

- the closed schema, literal case-sensitive behavior, default and explicit kind/depth postures, empty results, and lexicographic publication;
- the 8,192-entry, 512-match, and 262,144-byte boundaries;
- safe-root, linked-parent, final-link, special-object, invalid-UTF-8, unreadable, and response-ID preflight behavior;
- content/query/path-private audit decisions;
- validator v7, device harness v7, every optional posture allowlist, Android cross-builds, and native official-Termux ARM64 execution.

## Non-goals

`find_paths` does not authorize content reads, regular expressions, globbing, fuzzy matching, case folding, caller-selected execution budgets, symlink following, arbitrary metadata, persistent directory handles, filesystem watching, mutation, deletion, command execution, network access, or shared-storage access outside an explicitly configured safe root.
