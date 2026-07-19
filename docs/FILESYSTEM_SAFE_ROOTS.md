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

Every live create, copy, find, hash, list, metadata, binary-read, text-read, search, and write operation starts from one of these opened anchors and traverses descendants descriptor-relatively with no-follow semantics. Authorized `write_file` retains the exact root, parent, staging, and replacement descriptors; binds the grant to normalized target, content digest, and create/replace posture; publishes fixed mode `0600`; and removes only identities owned by the operation. See [`SAFE_ROOT_FILE_WRITES.md`](SAFE_ROOT_FILE_WRITES.md) and [`WRITE_FILE_CAPABILITY_GRANTS.md`](WRITE_FILE_CAPABILITY_GRANTS.md). `find_paths` classifies and traverses only verified regular files/directories, skips links/special/invalid-UTF-8 objects, and returns at most 512 content-free matches after examining at most 8,192 entries; see [`SAFE_ROOT_PATH_DISCOVERY.md`](SAFE_ROOT_PATH_DISCOVERY.md). `hash_file` retains the exact regular-file descriptor after path identity verification and streams at most 16 MiB through SHA-256; see [`SAFE_ROOT_FILE_HASHING.md`](SAFE_ROOT_FILE_HASHING.md). `read_binary_file` applies the same verified nonblocking final-open boundary, reads at most 1 MiB with a max-plus-one ceiling, and returns canonical padded base64; see [`SAFE_ROOT_BINARY_READS.md`](SAFE_ROOT_BINARY_READS.md). `read_binary_range` retains that exact descriptor and initial size, reads at most 256 KiB from a file up to 64 MiB, and rejects a detected size change; see [`SAFE_ROOT_BINARY_RANGES.md`](SAFE_ROOT_BINARY_RANGES.md). `read_text_range` adds UTF-8 validation and code-point-boundary pagination to the same held-descriptor 64 MiB/256 KiB range envelope; see [`SAFE_ROOT_TEXT_RANGES.md`](SAFE_ROOT_TEXT_RANGES.md).

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

Startup anchoring prevents unresolved lexical roots from entering the production jail. Live directory creation, file copy, path discovery, binary and text reads, listings, bounded literal searches, metadata lookup, temporary-object creation, cleanup, replacement, and parent sync additionally remain descriptor-relative after anchoring, with component-by-component no-follow resolution. `create_directory` stages one fixed-mode directory and atomically publishes it without replacement; mutation is independently default-disabled and consumes one principal/session/root/target-bound grant only after the held parent and absent child are proven. `copy_file` reads from one verified held regular-file descriptor and stages/publishes under one held destination-parent descriptor with fixed mode, no replacement, and identity verification; its full contract is in [`SAFE_ROOT_FILE_COPY.md`](SAFE_ROOT_FILE_COPY.md). Binary reads retain one verified descriptor and never re-resolve the pathname after open. Metadata, path discovery, and text search remain content-private. The #200 regression suite plus #240 search, #242 metadata, #244 directory creation, #247 file-copy exchange/cleanup, #248 request authorization, #261 hashing, #262 whole-file binary reads, #264 binary range reads, and #266 path discovery exercise boundaries before and after descriptors are opened; future filesystem changes must preserve those capability boundaries rather than reintroducing pathname re-resolution. See [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md) for the authorization layer that is additive to confinement.
