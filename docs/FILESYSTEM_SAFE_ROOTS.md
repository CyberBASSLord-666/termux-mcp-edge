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

Startup anchoring prevents unresolved lexical roots from entering the production jail. Live reads, listings, bounded literal searches, metadata lookup, temporary-file creation, cleanup, replacement, and parent sync additionally remain descriptor-relative after anchoring, with component-by-component no-follow resolution. `path_metadata` classifies the exact opened final descriptor and rejects links or unsupported types without returning host identifiers; its full contract is in [`SAFE_ROOT_PATH_METADATA.md`](SAFE_ROOT_PATH_METADATA.md). Search rechecks opened file type and skips symlinks and unsupported entries; its full contract is in [`SAFE_ROOT_TEXT_SEARCH.md`](SAFE_ROOT_TEXT_SEARCH.md). The #200 regression suite plus #240 search and #242 metadata exchange coverage exercise boundaries before and after descriptors are opened; future filesystem changes must preserve that capability boundary rather than reintroducing pathname re-resolution.
