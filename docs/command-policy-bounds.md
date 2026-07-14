# Fixed command policy bounds contract

`run_command_profile` accepts exactly one public value: a profile identifier no longer than 64 bytes. The current enum is `server_version`, `server_help`, and `execution_boundary`. No command, program, argv, working directory, environment, stdin, timeout, output-limit, or concurrency value is request-configurable.

## Fixed bounds

| Profile | Timeout | stdout | stderr |
|---|---:|---:|---:|
| `server_version` | 5 s | 4,096 bytes | 1,024 bytes |
| `server_help` | 5 s | 16,384 bytes | 1,024 bytes |
| `execution_boundary` | 5 s | 1,024 bytes | 1,024 bytes |

All profiles use the exact current executable, the first canonical configured safe root, empty environment, null stdin, and a two-permit non-queueing semaphore.

## Deterministic denial order

1. Missing or structurally invalid arguments are rejected before policy evaluation.
2. An uncompiled build returns `command_feature_not_compiled`.
3. A compiled but runtime-disabled build returns `command_runtime_disabled` without resolving the identifier.
4. An enabled build rejects oversized or unknown identifiers with `command_profile_not_allowlisted`.
5. An allowlisted profile requires an available safe root.
6. Execution then enforces concurrency, process creation, deadline, stream limits, zero exit, valid UTF-8, and cleanup confirmation.

No failure returns partial child output.

## Cleanup budget

The process timeout is divided into an operation deadline and a reserved cleanup window. The reserve is one quarter of the timeout, clamped from 1 ms through 250 ms. Construction rejects timeouts below 4 ms. Cleanup-reserve exhaustion overrides the primary outcome with `command_wait_failed`, while the independent supervisor remains responsible until reaping completes.

## Audit privacy

Audit metadata contains only the numeric profile ordinal when an allowlisted profile was resolved. Requested identifiers, executable paths, argv, cwd, environment, output, limits, and caller text are never stored. See [`command-execution-gate.md`](command-execution-gate.md) for the full runtime contract and [`command-profile-validation.md`](command-profile-validation.md) for profile-change review.
