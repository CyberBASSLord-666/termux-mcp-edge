# Filesystem safe-root startup contract

The server opens no listener until every configured filesystem safe root has been anchored successfully.

## Required state before service start

Each safe root must:

- be an absolute path accepted by configuration validation;
- already exist;
- be accessible to the Termux application UID;
- resolve successfully through the current mount and symlink layout; and
- resolve to a directory rather than a regular file or device node.

Canonical aliases and duplicate entries are collapsed before the filesystem tools and readiness state are constructed. Readiness therefore reports the number of distinct anchored roots, not the number of repeated configuration values.

Live `copy_file` additionally retains both anchored root descriptors, the source file and parent, and the destination parent while it binds and lock-revalidates exact source identity/content/SHA-256 and destination absence. Its independent default-false grant contract is [`COPY_FILE_CAPABILITY_GRANTS.md`](COPY_FILE_CAPABILITY_GRANTS.md).

Every live create, copy, find, hash, list, metadata, binary-read, text-read, search, and write operation starts from one of these opened anchors and traverses descendants descriptor-relatively with no-follow semantics. `write_file` holds the selected parent descriptor and, for replacement, the exact old regular-file descriptor and device/inode identity through grant validation and publication; see [`WRITE_FILE_CAPABILITY_GRANTS.md`](WRITE_FILE_CAPABILITY_GRANTS.md). `find_paths` classifies and traverses only verified regular files/directories, skips links/special/invalid-UTF-8 objects, and returns at most 512 content-free matches after examining at most 8,192 entries; see [`SAFE_ROOT_PATH_DISCOVERY.md`](SAFE_ROOT_PATH_DISCOVERY.md). `hash_file` retains the exact regular-file descriptor after path identity verification and streams at most 16 MiB through SHA-256; see [`SAFE_ROOT_FILE_HASHING.md`](SAFE_ROOT_FILE_HASHING.md). `read_binary_file` applies the same verified nonblocking final-open boundary, reads at most 1 MiB with a max-plus-one ceiling, and returns canonical padded base64; see [`SAFE_ROOT_BINARY_READS.md`](SAFE_ROOT_BINARY_READS.md). `read_binary_range` retains that exact descriptor and initial size, reads at most 256 KiB from a file up to 64 MiB, and rejects a detected size change; see [`SAFE_ROOT_BINARY_RANGES.md`](SAFE_ROOT_BINARY_RANGES.md). `read_text_range` adds UTF-8 validation and code-point-boundary pagination to the same held-descriptor 64 MiB/256 KiB range envelope; see [`SAFE_ROOT_TEXT_RANGES.md`](SAFE_ROOT_TEXT_RANGES.md).

## Failure behavior

A missing, inaccessible, or non-directory root aborts startup before the TCP listener is bound. Startup errors identify only the one-based configuration position and a stable reason. They do not echo the configured path or operating-system error text.

This fail-closed behavior is intentional for Android and Termux. Shared-storage permissions, scoped-storage access, removable media, and mount state can change independently of a previously valid configuration. Operators must restore the directory and its access permissions before restarting the runit service; they must not substitute a broader root merely to make startup succeed.

## Deployment checks

Before an install, upgrade, or rollback:

```sh
for root in "$HOME" "$HOME/storage/shared"; do
  test -d "$root" && test -r "$root" && test -x "$root" || exit 1
done
```

Use the actual configured roots rather than copying the example paths. After deployment, confirm that the service remains up and that `/ready` reports the expected distinct safe-root count.

## Live-operation boundary

Startup anchoring prevents unresolved lexical roots from entering the production jail. Live filesystem work remains descriptor-relative with component-by-component no-follow resolution. `create_directory` consumes one principal/session/root/target-bound grant only after held-parent absence revalidation. `copy_file` is independently default-disabled, binds and revalidates both roots/paths plus exact source identity/bytes/SHA-256 and destination absence, then consumes its grant immediately before private staging. `write_file` is independently gated and consumes its content/disposition/identity-bound grant immediately before publication. Each target parent reserves `.termux-mcp-write-quarantine`, a mode-`0700` namespace hidden from every MCP filesystem surface. Copy and write-create move randomized mode-`0600` staging entries from that quarantine with atomic no-replace publication and retain no artifact. Write replacement performs one irreversible exchange and retains the displaced prior inode/content under a randomized quarantine name. The process-wide lock serializes cooperating create/copy/write publication. Binary reads retain one verified descriptor and never re-resolve the pathname after open; metadata, discovery, and search remain content-private. See [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md), [`COPY_FILE_CAPABILITY_GRANTS.md`](COPY_FILE_CAPABILITY_GRANTS.md), [`SAFE_ROOT_FILE_COPY.md`](SAFE_ROOT_FILE_COPY.md), and [`WRITE_FILE_CAPABILITY_GRANTS.md`](WRITE_FILE_CAPABILITY_GRANTS.md).

The quarantine is capped at 32 regular artifacts and 32 MiB per target parent. Its nonblocking advisory lock coordinates cooperating runtime mutations only. It is not a global disk limit or an isolation boundary from another process under the same Unix UID; such a peer can force contention or denial of service. Production mutation safe roots therefore require exclusive operational ownership by this service, with no independent writer modifying a configured root while any live create/copy/write gate is enabled. Quiesce the service and all same-UID writers before inspecting or manually removing selected artifacts.
