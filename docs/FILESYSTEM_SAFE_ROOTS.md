# Filesystem safe-root startup contract

The server opens no listener until fallible construction has validated and lifetime-pinned every configured filesystem safe root.

## Required state before service start

Configuration must contain between one and 64 entries. Each entry must:

- be an absolute path accepted by configuration validation;
- normalize without parent traversal and not name filesystem root `/`;
- already exist;
- be reachable through its ancestors by the Termux application UID;
- contain no symbolic-link component in the root or any ancestor; and
- resolve to a directory rather than a regular file or device node.

Normalized entries are sorted and lexically deduplicated before the filesystem tools and readiness state are constructed. Symlink aliases are rejected rather than canonicalized. Readiness therefore reports the number of distinct normalized labels and corresponding pins, not the number of repeated configuration values. The configured-path count is bounded before deduplication, so even a repeated configuration may not contain more than 64 entries. Different normalized labels are not deduplicated merely because they resolve to equal device/inode identities, as can occur with bind mounts.

## Lifetime-pinned authority

Startup opens each root one component at a time with directory, path-descriptor, close-on-exec, and no-follow semantics. It retains the final descriptor and its device/inode identity for the lifetime of the runtime. Every `FileSystemTools` clone shares that same pinned set. A filesystem operation selects a normalized root label, duplicates the corresponding retained descriptor, verifies the duplicate still has the pinned identity, and resolves every descendant from that descriptor with component-by-component no-follow operations. The runtime never reopens a configured root pathname as its authority.

Root labels are configuration metadata used for deterministic request matching; they are not authority. Renaming a configured root or replacing either the root or one of its ancestors cannot redirect an already-running process. Existing operations continue against the originally pinned directory, and a replacement object now visible at the configured pathname is untouched. A controlled service restart intentionally validates and pins the objects then present at the configured paths.

Offline capability-grant issuers use the same fallible constructor and bind grants to the device/inode identity of their pinned root. Runtime target preparation and grant consumption use the running process's retained identity. If the configured pathname was replaced after the runtime started, a grant issued against that replacement cannot authorize an operation against the runtime's original pinned directory; the root binding fails closed. Create, copy, and write issuance and consumption therefore share the same descriptor-identity contract as descendant filesystem operations.

Live `copy_file` additionally retains both anchored root descriptors, the source file and parent, and the destination parent while it binds and lock-revalidates exact source identity/content/SHA-256 and destination absence. Its independent default-false grant contract is [`COPY_FILE_CAPABILITY_GRANTS.md`](COPY_FILE_CAPABILITY_GRANTS.md).

Live `trash_file` retains the anchored root, target parent, and exact single-link regular-file descriptor while it repeatedly verifies identity, size, high-resolution ctime, link count, and SHA-256 content. Its independent default-false recovery-retaining grant contract is [`TRASH_FILE_CAPABILITY_GRANTS.md`](TRASH_FILE_CAPABILITY_GRANTS.md).

Every live create, copy, trash, find, hash, list, metadata, binary-read, text-read, search, and write operation starts from a duplicate of one of these lifetime-pinned anchors and traverses descendants descriptor-relatively with no-follow semantics. `trash_file` and replacement `write_file` retain the exact existing descriptor and identity through grant validation and publication; see [`TRASH_FILE_CAPABILITY_GRANTS.md`](TRASH_FILE_CAPABILITY_GRANTS.md) and [`WRITE_FILE_CAPABILITY_GRANTS.md`](WRITE_FILE_CAPABILITY_GRANTS.md). `find_paths` classifies and traverses only verified regular files/directories, skips links/special/invalid-UTF-8 objects, and returns at most 512 content-free matches after examining at most 8,192 entries; see [`SAFE_ROOT_PATH_DISCOVERY.md`](SAFE_ROOT_PATH_DISCOVERY.md). `hash_file` retains the exact regular-file descriptor after path identity verification and streams at most 16 MiB through SHA-256; see [`SAFE_ROOT_FILE_HASHING.md`](SAFE_ROOT_FILE_HASHING.md). `read_binary_file` applies the same verified nonblocking final-open boundary, reads at most 1 MiB with a max-plus-one ceiling, and returns canonical padded base64; see [`SAFE_ROOT_BINARY_READS.md`](SAFE_ROOT_BINARY_READS.md). `read_binary_range` retains that exact descriptor and initial size, reads at most 256 KiB from a file up to 64 MiB, and rejects a detected size change; see [`SAFE_ROOT_BINARY_RANGES.md`](SAFE_ROOT_BINARY_RANGES.md). `read_text_range` adds UTF-8 validation and code-point-boundary pagination to the same held-descriptor 64 MiB/256 KiB range envelope; see [`SAFE_ROOT_TEXT_RANGES.md`](SAFE_ROOT_TEXT_RANGES.md).

## Failure behavior

An empty set, excessive set, relative path, filesystem root, parent traversal, missing object, non-directory, or root/ancestor symlink aborts startup after the exact TCP listener is bound but before a router exists or any request is served. Invalid roots never enter runtime state. Startup uses path descriptors, so successful pinning does not claim that every later read or mutation permission is available; each operation still fails closed if its required access is denied. Startup errors provide only a stable path-free reason. They do not echo the configured path, retained descriptor, device/inode identity, or operating-system error text. Debug and audit surfaces likewise expose only bounded counts and stable classifications, never root paths or descriptor metadata.

This fail-closed behavior is intentional for Android and Termux. Shared-storage permissions, scoped-storage access, removable media, and mount state can change independently of a previously valid configuration. Operators must restore the directory and its access permissions before restarting the runit service; they must not substitute a broader root merely to make startup succeed.

## Deployment checks

Before an install, upgrade, or rollback, a basic permission preflight is useful:

```sh
for root in "$HOME" "$HOME/storage/shared"; do
  test -d "$root" && test -r "$root" && test -x "$root" || exit 1
done
```

Use the actual configured roots rather than copying the example paths. This shell check is not proof of the complete contract: it does not establish the 64-entry ceiling, component-level no-follow posture, deduplication, or retained descriptor identity. Exact-binary startup is authoritative and must fail before listening when any requirement is not met. After deployment, confirm that the service remains up and that `/ready` reports the expected distinct normalized-label/pin count.

Do not rename, replace, remount, or hot-swap a configured storage hierarchy as a way to redirect a live runtime. Stop the service, make the storage change, restart it so the new objects are validated and pinned, then verify runit state, health, readiness, and representative filesystem calls.

## Live-operation boundary

Startup pinning prevents invalid or replaceable pathname authority from entering the production jail. Live filesystem work remains descriptor-relative with component-by-component no-follow resolution from the same retained root identity. `create_directory` consumes one principal/session/root/target-bound grant only after held-parent absence revalidation. `copy_file` is independently default-disabled, binds and revalidates both pinned roots/paths plus exact source identity/bytes/SHA-256 and destination absence, then consumes its grant immediately before private staging. `trash_file` binds one exact target identity/content and moves it only into `.termux-mcp-trash-quarantine`. `write_file` consumes its pinned-root/content/disposition/identity-bound grant immediately before publication. Each target parent reserves separate mode-`0700` write and trash quarantine namespaces hidden from every MCP filesystem surface. Copy and write-create move randomized mode-`0600` staging entries from the write quarantine with atomic no-replace publication and retain no artifact. Write replacement performs one irreversible exchange and retains the displaced prior inode/content; trash moves the exact original inode with `NOREPLACE` into its recovery quarantine. The process-wide lock serializes cooperating create/copy/trash/write publication. Binary reads retain one verified descriptor and never re-resolve the pathname after open; metadata, discovery, and search remain content-private. See [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md), [`COPY_FILE_CAPABILITY_GRANTS.md`](COPY_FILE_CAPABILITY_GRANTS.md), [`TRASH_FILE_CAPABILITY_GRANTS.md`](TRASH_FILE_CAPABILITY_GRANTS.md), [`SAFE_ROOT_FILE_COPY.md`](SAFE_ROOT_FILE_COPY.md), and [`WRITE_FILE_CAPABILITY_GRANTS.md`](WRITE_FILE_CAPABILITY_GRANTS.md).

Each recovery quarantine is capped at 32 regular artifacts and 32 MiB per target parent. Its nonblocking advisory lock coordinates cooperating runtime mutations only. It is not a global disk limit or an isolation boundary from another process under the same Unix UID; such a peer can force contention or denial of service. Production mutation safe roots therefore require exclusive operational ownership by this service, with no independent writer modifying a configured root while any live create/copy/trash/write gate is enabled. Quiesce the service and all same-UID writers before inspecting, restoring, or manually removing selected artifacts.
