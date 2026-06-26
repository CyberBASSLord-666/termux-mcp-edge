# Changelog

## 2026-06-26 — Authentication Posture Hardening

- Changed startup authentication behavior to fail closed when `MCP__AUTH__STATIC_TOKEN` is missing.
- Rejected empty or whitespace-only `MCP__AUTH__STATIC_TOKEN` values at startup.
- Added explicit `MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true` local-development opt-in, constrained to loopback bind addresses.
- Allowed `localhost` as an explicit loopback hostname for local-only unauthenticated development.
- Documented the unauthenticated-mode constraints in `README.md` and `docs/SECURITY.md`.

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
