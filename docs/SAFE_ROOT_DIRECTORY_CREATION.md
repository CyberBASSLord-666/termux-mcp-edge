# Safe-rooted directory creation contract

`create_directory` creates exactly one project-owned directory inside a configured filesystem safe root. It is part of the authenticated baseline `mcp-runtime` posture and does not enable shell access, recursive creation, deletion, rename, permission selection, or writes outside the existing safe-root authority.

## Input schema

The closed MCP argument object accepts:

- `path` — required absolute path inside one configured `MCP__FILE__SAFE_ROOTS` entry;
- `dry_run` — optional boolean. Omitted or `true` validates the request without mutation. Only explicit `false` authorizes creation.

Unknown fields, missing `path`, wrong JSON types, relative paths, NUL bytes, parent traversal, the safe root itself, and outside-root paths are rejected. The tool creates one directory only: every parent component must already exist.

## Descriptor and publication boundary

The runtime anchors the request to the most specific configured safe root, opens that root with no-follow semantics, and walks every existing parent component by descriptor. Each component is opened with `O_PATH | O_NOFOLLOW`, classified with `fstat`, and required to be a directory. The final writable parent is reopened from the held descriptor; no authorized pathname is later re-resolved for mutation.

Mutation uses this sequence:

1. Confirm the final name is absent with no-follow descriptor metadata.
2. Create one unpredictable temporary directory in the held parent descriptor.
3. Open that exact temporary directory without following links.
4. Force and verify mode `0700` on the opened descriptor.
5. Sync the directory descriptor.
6. Publish it to the requested final name with atomic `RENAME_NOREPLACE` semantics.
7. Sync the exact held parent descriptor before reporting success.

An existing file, directory, link, or concurrently inserted final object is never replaced. The temporary directory is removed on pre-publication failure. After publication, rollback cleanup compares the held directory identity with no-follow metadata before removing the empty object, so a concurrently substituted path is not deleted.

## Result and resource bound

A successful call returns exactly:

```json
{
  "path": "/absolute/configured-root/new-directory",
  "dryRun": true,
  "mode": "0700",
  "maxResponseBytes": 16384
}
```

The complete JSON-RPC/MCP response, including the caller-controlled request identifier, is capped at 16 KiB. Response eligibility is checked before any mutation, so an oversized envelope cannot create a directory and then report failure. If the full envelope cannot fit, the runtime returns a bounded HTTP 413 response with a null identifier rather than reflecting an oversized caller value.

The result does not contain inode/device numbers, UID/GID, raw mode bits, timestamps, parent contents, file content, host errors, temporary names, or rollback internals. `mode` is the fixed public contract string, not caller-selected or raw host metadata.

## Stable audit contract

Only aggregate in-memory counters are retained. Events use stable labels such as:

- tool: `create_directory`;
- gate: `filesystem_write`;
- mode: `dry_run` or `mutating`;
- allowed reasons: `dry_run_preview`, `safe_root_directory_created`;
- denied reasons: `missing_arguments`, `invalid_arguments`, `safe_root_rejected`, `filesystem_parent_not_found`, `filesystem_destination_exists`, `response_size_limit_exceeded`, or `filesystem_directory_create_failed`.

Paths, names, temporary identifiers, OS errors, and request identifiers are never stored in audit counters.

## Explicit non-goals

- Recursive or parent creation.
- Existing-directory success or idempotent overwrite semantics.
- File, directory, or tree deletion.
- Move, rename, copy, chmod, chown, or caller-selected permissions.
- Broad Android shared-storage authority.
- Shell, command, service, package, process, network, or Android control.
