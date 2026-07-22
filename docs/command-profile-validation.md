# Command profile validation runbook

This runbook governs changes to the implemented `run_command_profile` registry. The current registry is intentionally closed to three read-only diagnostics of the server-owned running image. A proposed profile is denied unless every check below passes.

## Current approved profiles

| Identifier | Executable | Argv | Side effects |
|---|---|---|---|
| `server_version` | Server-owned running image | `--version` | None |
| `server_help` | Server-owned running image | `--help` | None |
| `execution_boundary` | Server-owned running image | `--self-check-command-boundary` | None |

These profiles have no parameters, placeholders, request-derived paths, environment input, stdin input, or configurable limits. Their fields, resolved decision handle, lookup function, execution client, and raw request/result types are crate-private. Command enablement is structurally confined to the binary target: `src/main.rs` compiles the module graph in the binary crate and alone can call the crate-private command switch on `McpRouterBuilder`, while the single public builder defaults the lane disabled and exposes no enabling method. No mintable command-authority token exists.

## Review record

Record the proposed identifier, exact source revision, purpose, exact executable identity, complete argv, timeout, stdout/stderr ceilings, expected output class, reviewer, and pass/fail decision. Do not put raw output, environment data, credentials, private paths, device identifiers, or caller payloads in the record.

## Eligibility checklist

A profile is eligible only when all answers are yes:

1. Is the purpose narrow, read-only, and diagnostic?
2. Is the program the exact running Termux MCP Edge binary?
3. Is every argv element fixed in source with no placeholder or caller data?
4. Does the program path avoid `PATH` lookup?
5. Does the profile run in an already-anchored configured safe root?
6. Is the inherited environment completely empty?
7. Is stdin always null?
8. Are timeout and independent stdout/stderr ceilings finite, conservative, and no greater than the immutable 5-second, 16 KiB stdout, and 4 KiB stderr supervisor maxima?
9. Does any nonzero exit, timeout, overflow, invalid UTF-8, or cleanup failure suppress all output?
10. Is the result useful without credentials, host paths, device identifiers, broad process state, or private file contents?
11. Does cancellation retain cleanup ownership until the direct child is reaped?
12. Are discovery, runtime-disabled, invalid-input, allowed, and failure paths audited without caller text?
13. Do exact-head CI, Security, Android cross-compile, and native ARM64 Termux gates pass?
14. Does the change preserve `arbitraryCommandExecution=false` and `highImpactTools=false`?

Any no answer blocks the profile.

## Automatic rejection classes

Reject profiles involving any of the following:

- shells, interpreters, `eval`, `-c`, scripts, plugins, config loading, or code evaluation;
- caller-selected programs, argv, paths, environment, stdin, timeout, output limits, or concurrency;
- package managers, privilege bridges, Android control commands, process-control tools, service mutation, network clients, or network configuration;
- recursive filesystem inspection, broad host roots, shared-storage roots, `/proc`, `/sys`, `/dev`, or arbitrary current directories;
- credentials, authentication material, cookies, keys, account data, identifiers, messages, contacts, notifications, location, camera, microphone, or accessibility data;
- writes, deletes, renames, permission changes, package/service changes, external requests, or any other side effect;
- unbounded, binary, secret-bearing, path-bearing, device-bearing, or nondeterministic output.

Such work belongs in a separate threat-modeled capability gate. It must not be disguised as a diagnostic profile.

## Identifier and schema review

Profile identifiers must be short stable ASCII-style names no longer than 64 bytes. They must not contain path syntax, whitespace, NUL, shell tokens, or command text. The public schema remains a one-property closed object whose enum is derived from the canonical registry.

Tests must prove that missing arguments, unknown identifiers, oversized identifiers, shell-shaped values, and each attempted override field fail before spawn. Compile/API coverage must first build a valid consumer of `McpRouterBuilder` and then prove that ordinary dependencies and selected workspace members cannot import or construct `CommandProfile`, inspect the resolved handle, reach the raw execution client, recover removed authority symbols, call the binary-only builder switch, or restore legacy router constructors and option/authority bundle types. Runtime-disabled evaluation must not disclose whether a supplied identifier is known.

## Executable and argv review

At initialization, the production client resolves an absolute `std::env::current_exe()` path with exact basename `termux-mcp-server`. It opens the candidate without following its final component, opens `/proc/self/exe` independently, and requires an executable regular candidate, a regular loaded image, and equal device/inode identity. Any failure makes the effective command posture unavailable before spawn. The client discards the reopenable installation path and launches only `/proc/self/exe`, binding execution to the already-running inode across later rename or replacement. A proposal to launch any other executable, or to weaken any identity check, is outside this gate.

Argv must be a complete immutable slice. Reject shell metacharacters, command substitution, redirection, pipes, globs, path expansion, configuration-file arguments, dynamic verbosity, and any option that can load or execute external content. Tests must assert the exact argv observed by a fixture executable.

## Working-directory review

The command client opens the first canonical safe root once with a no-follow directory descriptor. It rejects relative paths, final-component symlinks, nonexistent/non-directory paths, and every filesystem-root alias by comparing device/inode identity with an independently opened `/`. The descriptor remains owned for the client lifetime, and each child uses `/proc/self/fd/<fd>` while a cloned guard stays alive through execution. Renaming the directory and replacing its former pathname therefore cannot redirect cwd. Callers cannot select or see the working directory.

Any profile whose behavior can escape the directory through its own fixed arguments or implicit config discovery is ineligible even though the child cwd is safe-rooted.

## Environment and stdin review

`env_clear()` is mandatory; there is no environment allowlist in this gate. Profiles that require any environment variable are ineligible. `stdin` is always `/dev/null`; profiles requiring interactive or static input are ineligible.

The native `execution_boundary` profile must continue to prove both properties without reflecting their values.

## Bounds and cleanup review

Every profile owns explicit timeout, stdout bytes, and stderr bytes. Four milliseconds is the implementation minimum because a real nonzero cleanup reserve is mandatory; production profiles use five seconds. Independently, the supervisor rejects any timeout above 5 seconds, stdout limit above 16 KiB, or stderr limit above 4 KiB before spawning. Pipe buffers grow through checked reservation only for bytes actually read, never from the selected ceiling; allocation failure must return a stable wait failure and must never panic or attempt an attacker-selected capacity.

The shared supervisor must preserve:

- independent concurrent pipe draining;
- immediate overflow recognition;
- isolated process-group kill;
- cleanup on success, failure, timeout, overflow, and caller cancellation;
- authoritative reap after the response future is cancelled;
- cleanup-deadline failure precedence.

The command-specific semaphore remains non-queueing and bounded at two concurrent profiles unless a separately reviewed resource analysis changes it.

## Output and response review

Only zero-exit valid-UTF-8 output may be returned. A response includes the stable profile identifier, zero exit code, each bounded stream once, exact byte counts, and bounded duration. Error responses contain stable reason codes only.

Review output for secrets, private paths, hostnames, usernames, Android identifiers, full environment, global process state, and dependency details that materially aid exploitation. If output cannot be proven safe and stable, reject the profile.

## Audit review

Allowed and denied events may retain only tool name, gate name, read-only mode, decision, stable reason code, and resolved numeric profile ordinal. They must not retain the requested identifier, executable path, argv, cwd, streams, environment, token, session ID, or arbitrary caller value.

## Required validation

Run:

```bash
cargo metadata --locked --all-features --format-version 1 --no-deps >/dev/null
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked --workspace --all-targets --features full-suite -- -D warnings
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets
cargo test --locked --workspace --all-targets --features full-suite
cargo test --locked --workspace --all-targets --all-features
bash tests/package_android_artifact_test.sh
```

The exact PR head must also pass:

- CI on all feature combinations;
- Security checks;
- all seven governed Android artifact builds, including `full-suite`;
- native ARM64 official-Termux execution of `termux_command_emulated_gate.sh`;
- evidence validation against `command-emulated-evidence-schema-v2.json`;
- ordinary-dependency and selected-workspace API compile failures for removed authority symbols, the private command switch, raw types, legacy constructors, and former public option/authority bundle types;
- runtime pre-spawn rejection for wrong name, wrong inode, symlink/non-regular/non-executable candidates, forged/raw input shapes, root/cwd aliases, and every hard-limit maximum-plus-one case;
- strict native v2 evidence with exactly 29 MCP requests plus a separate wrong-name construction-failure phase; the combined phase proves `/proc/self/exe` continues to execute the already-running image and the retained safe-root descriptor survives pathname rename/replacement, while the separate phase proves typed rejection before request serving without sensitive diagnostics.

Do not substitute a long idle observation for these deterministic boundary tests. Physical observation, when release governance requires it for changed runtime inputs, is a separate release-qualification decision and does not replace command-policy evidence.
