# Safe-rooted directory creation contract

`create_directory` validates exactly one project-owned directory target inside a configured filesystem safe root. Mutation is separately default-disabled and requires one request-scoped, single-use capability grant. It does not enable shell access, recursive creation, deletion, rename, permission selection, or writes outside the existing safe-root authority. The complete grant format, issuance workflow, denial contract, and rotation procedure are defined in [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md).

## Input schema

The closed MCP argument object accepts:

- `path` — required absolute path inside one configured `MCP__FILE__SAFE_ROOTS` entry;
- `dry_run` — optional boolean. Omitted or `true` validates the request without mutation. Explicit `false` selects the mutating posture but does not authorize it by itself.

Unknown fields, missing `path`, wrong JSON types, relative paths, NUL bytes, parent traversal, the safe root itself, and outside-root paths are rejected. The tool creates one directory only: every parent component must already exist.

With `MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=false` (the default), discovery constrains `dry_run` to `true` and dispatch denies explicit mutation. When the gate is enabled, explicit mutation additionally requires exactly one valid `MCP-Capability-Grant` header bound to the authenticated static principal, active MCP session, safe-root identity, normalized target, and mutating posture. Grant material is never accepted in the argument object.

## Authorization boundary

Confinement and response-size validation complete before grant matching. The runtime then atomically consumes the grant immediately before the first `mkdirat` attempt. Dry runs, invalid paths, target mismatches, and wrong-context header use do not consume a valid grant. Once consumed, a grant stays consumed after any later staging, verification, sync, publication, response, or cleanup failure. Reuse and concurrent replay fail closed.

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
- denied reasons: `missing_arguments`, `invalid_arguments`, `safe_root_rejected`, `filesystem_parent_not_found`, `filesystem_destination_exists`, `response_size_limit_exceeded`, `create_directory_mutation_disabled`, stable `capability_*` authorization reasons, or `filesystem_directory_create_failed`.

Paths, names, temporary identifiers, OS errors, request identifiers, grants, keys, sessions, JTIs, and target digests are never stored in audit counters.

## Explicit non-goals

- Recursive or parent creation.
- Existing-directory success or idempotent overwrite semantics.
- File, directory, or tree deletion.
- Move, rename, copy, chmod, chown, or caller-selected permissions.
- Broad Android shared-storage authority.
- Shell, command, service, package, process, network, or Android control.
