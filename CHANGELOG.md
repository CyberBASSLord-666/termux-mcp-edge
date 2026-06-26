# Changelog

## 2026-06-26 — Automated Filesystem Safety Pass

- Reworked `FileSystemTools::sanitize` into a public, testable safe-root guard that rejects relative paths, NUL bytes, explicit parent-directory components, and paths escaping configured roots.
- Replaced recursive async directory traversal with bounded iterative breadth-first traversal to avoid Rust async recursion compile failures and reduce stack/future-size risk.
- Added list traversal bounds for maximum depth and maximum entries, plus metrics for truncated and unsafe skipped entries.
- Hardened atomic writes by staging temporary files in the destination directory and cleaning up failed temporary writes or renames.
- Added real integration tests for dry-run writes, atomic writes, reads, directory listing, and traversal rejection.
- Added property tests for safe-root path acceptance and rejection behavior.
- Added `proptest` and `tempfile` dev-dependencies and corrected crate repository metadata.
- Added `docs/VALIDATION.md` with validation commands and current run limitations.

## 2026-06-25 — Repository Baseline

- Created canonical Git repository layout for the Termux MCP Rust/Android edge server.
- Added repository hygiene files: `.gitignore`, `.gitattributes`, `.editorconfig`, license, contribution guide, GitHub Actions, Dependabot, and operational documentation.
- Preserved the uploaded Rust MCP server source tree as the initial baseline.
- Added validation and deployment instructions for desktop Rust validation and Android cross-compilation.
