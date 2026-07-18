# Command profile validation runbook

This runbook governs changes to the implemented `run_command_profile` registry. The current registry is intentionally closed to three read-only diagnostics of the exact server binary. A proposed profile is denied unless every check below passes.

## Current approved profiles

| Identifier | Executable | Argv | Side effects |
|---|---|---|---|
| `server_version` | Current server | `--version` | None |
| `server_help` | Current server | `--help` | None |
| `execution_boundary` | Current server | `--self-check-command-boundary` | None |

These profiles have no parameters, placeholders, request-derived paths, environment input, stdin input, or configurable limits.

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
8. Are timeout and independent stdout/stderr ceilings finite and conservative?
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

Tests must prove that missing arguments, unknown identifiers, oversized identifiers, shell-shaped values, and each attempted override field fail before spawn. Runtime-disabled evaluation must not disclose whether a supplied identifier is known.

## Executable and argv review

The production client resolves only `std::env::current_exe()`. A proposal to launch any other executable is outside this gate.

Argv must be a complete immutable slice. Reject shell metacharacters, command substitution, redirection, pipes, globs, path expansion, configuration-file arguments, dynamic verbosity, and any option that can load or execute external content. Tests must assert the exact argv observed by a fixture executable.

## Working-directory review

The command client receives the first safe root only after startup has canonicalized it and verified that it is an existing directory. It rejects relative paths, `/`, and nonexistent directories. Callers cannot select or see the working directory.

Any profile whose behavior can escape the directory through its own fixed arguments or implicit config discovery is ineligible even though the child cwd is safe-rooted.

## Environment and stdin review

`env_clear()` is mandatory; there is no environment allowlist in this gate. Profiles that require any environment variable are ineligible. `stdin` is always `/dev/null`; profiles requiring interactive or static input are ineligible.

The native `execution_boundary` profile must continue to prove both properties without reflecting their values.

## Bounds and cleanup review

Every profile owns explicit timeout, stdout bytes, and stderr bytes. Four milliseconds is the implementation minimum because a real nonzero cleanup reserve is mandatory; production profiles use five seconds.

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
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo test --workspace --all-targets --all-features
bash tests/package_android_artifact_test.sh
```

The exact PR head must also pass:

- CI on all feature combinations;
- Security checks;
- six Android artifact builds;
- native ARM64 official-Termux execution of `termux_command_emulated_gate.sh`;
- evidence validation against `command-emulated-evidence-schema-v1.json`.

Do not substitute a long idle observation for these deterministic boundary tests. Physical observation, when release governance requires it for changed runtime inputs, is a separate release-qualification decision and does not replace command-policy evidence.
