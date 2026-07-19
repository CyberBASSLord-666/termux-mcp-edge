# Safe-root text-search contract

`search_text` is the read-only literal text-search capability in the baseline `mcp-runtime` tool registry. It lets an authenticated client locate text under a configured filesystem safe root without downloading every file, executing `grep`, evaluating a regular expression, or returning file contents.

## Input contract

The closed input object accepts only:

- `path`: an absolute directory inside one configured safe root;
- `query`: one non-empty, case-sensitive UTF-8 line of at most 256 bytes;
- `max_depth`: optional integer from 1 through 5, defaulting to 5.

Unknown properties, empty queries, NUL, CR/LF, oversized queries, and out-of-range depths are rejected before filesystem work. The query is always a literal. Regex syntax, glob characters, shell syntax, and command-like text have no special meaning.

## Descriptor boundary

The operation anchors the supplied path to the most specific configured root label, duplicates and identity-verifies that root's lifetime-pinned descriptor, and resolves every descendant directory and file relative to the retained duplicate with `NOFOLLOW`. Symlinks and non-regular file types are skipped. An opened file is checked again before reading, so exchanging a validated pathname for a symlink, FIFO, device, or outside-root directory cannot redirect the read.

Blocking enumeration and reads run outside the async executor. No subprocess, shell, Android API, network request, write, or temporary file is involved.

## Fixed resource ceilings

Callers cannot override these limits:

| Resource | Limit |
| --- | ---: |
| Traversal depth | 5 |
| Query size | 256 UTF-8 bytes |
| Directory entries examined | 8,192 |
| Files scanned | 4,096 |
| Bytes per file | 1 MiB |
| Aggregate bytes | 8 MiB |
| Published matches | 256 |
| Complete JSON-RPC response | 256 KiB |

Files beyond a byte budget, invalid UTF-8 files, unreadable entries, symlinks, and unsupported file types do not expose raw errors or content. The result reports bounded aggregate skip counts and sets `truncated` whenever a supported input could not be fully searched or the match/response budget omitted results.

## Output contract

Each match contains only:

- safe-rooted `path`;
- one-based `lineNumber`;
- one-based UTF-8 byte `columnByte`.

The query, matching line, surrounding context, raw file bytes, and operating-system errors are not echoed. Matches are sorted by path, line, and column before publication. The result also publishes the fixed limits and aggregate scanned/skipped counts so clients can distinguish a complete result from a bounded partial result.

## Audit and validation

Successful calls increment `search_text` with `safe_root_text_searched`. Invalid queries, arguments, depths, safe-root rejections, response-limit failures, and internal search failures use stable reason codes. Counters never retain the path, query, content, or match data.

Repository tests cover exact matches, multiple matches, depth, query-byte boundaries, deterministic ordering, invalid UTF-8 and oversized files, aggregate byte and match ceilings, response truncation, outside-root denial, symlink skipping, post-open directory exchange, closed MCP arguments, response size, and audit privacy. The release validator executes `search_text` against the exact native AArch64 Android `mcp-runtime` artifact in the pinned official Termux environment.

Tracked by #240.
