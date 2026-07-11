# Command Profile Validation Runbook

Termux MCP Edge is built for developers, advanced Termux operators, and power users who understand local automation risk. This runbook defines the operator checks required before any future command-policy profile can be considered for enablement.

Command execution remains disabled in the staged runtime. This document does not enable a command-capable MCP tool, does not relax policy, and does not authorize shell access, arbitrary arguments, process control, Android platform control, or high-impact actions.

## Validation objective

A command profile is acceptable only when an operator can prove that it is narrow, deterministic, bounded, safe-rooted, auditable, and explicitly opted in. Failure to prove any required property is a block.

The validation output should be a short record containing:

- profile identifier;
- intended operational purpose;
- reviewer name or handle;
- validation date;
- source revision reviewed;
- explicit pass/fail decision;
- reason codes for any rejected profile.

Do not store command output, file contents, environment values, secrets, private host metadata, usernames, device identifiers, or filesystem paths outside the documented safe-root decision context in the validation record.

## Required preconditions

Before profile review starts, confirm that:

1. the command-execution gate is still disabled by default;
2. no command-capable MCP tool is exposed through `tools/list`;
3. the profile is defined in reviewed project configuration or source, not supplied dynamically by a caller;
4. the operator explicitly opted in to the reviewed profile set;
5. the profile has a documented owner and purpose;
6. the profile does not overlap with Android control, package management, service mutation, network mutation, credential access, device control, or other high-impact capability classes.

If any precondition fails, reject the profile with `profile_precondition_failed`.

## Fixed executable review

Each profile must name exactly one fixed executable.

Pass criteria:

- executable identity is static in project-owned configuration or source;
- executable is not a shell, shell wrapper, interpreter escape hatch, package manager, process-control utility, credential tool, network mutation tool, or Android control tool;
- executable purpose matches the documented profile purpose;
- executable location is not caller-controlled;
- executable resolution cannot be changed by caller-supplied `PATH` or environment values.

Reject the profile if the executable is any of:

- `sh`, `bash`, `zsh`, `fish`, `dash`, `busybox sh`, or equivalent shell surface;
- `su`, `sudo`, `doas`, `rish`, `adb`, or similar privilege/device bridge;
- `pkg`, `apt`, `dpkg`, `pm`, `cmd`, `am`, `settings`, `svc`, `termux-*` control commands, or package/platform mutation tools;
- `kill`, `pkill`, `reboot`, `mount`, `ip`, `iptables`, `nft`, `ssh`, `scp`, `curl`, `wget`, or broad system/network mutation tools;
- any interpreter intended to evaluate caller-supplied code.

Recommended rejection reason codes:

- `shell_executable_rejected`;
- `privilege_bridge_rejected`;
- `platform_control_rejected`;
- `package_or_service_mutation_rejected`;
- `network_mutation_rejected`;
- `dynamic_executable_rejected`.

## Fixed argv review

Each profile must define a fixed argv vector. Callers must not be able to add, remove, reorder, interpolate, template, or escape arguments.

Pass criteria:

- argv is fully enumerated in reviewed project-owned configuration or source;
- no argv element contains caller-controlled text;
- no argv element invokes shell evaluation, command substitution, glob expansion, redirection, pipes, background execution, or command chaining;
- no argv element expands access to filesystem locations outside the safe root;
- no argv element requests verbose secret-bearing output.

Reject the profile if it uses:

- free-form command strings;
- templated argv values;
- wildcard argv values that broaden filesystem or process scope;
- `--config`, `--script`, `--exec`, `--eval`, `-c`, or equivalent options that load caller-controlled code or commands;
- command separators such as `;`, `&&`, `||`, pipes, or redirection operators.

Recommended rejection reason codes:

- `dynamic_argv_rejected`;
- `argv_template_rejected`;
- `shell_syntax_rejected`;
- `unsafe_scope_expansion_rejected`;
- `caller_controlled_code_rejected`.

## Working-directory and safe-root review

Each profile must run from a safe-rooted working directory. The caller must not be able to select arbitrary paths.

Pass criteria:

- working directory is fixed or selected from a small project-owned allowlist;
- every allowed working directory resolves inside a configured safe root;
- symlinks cannot escape the safe root;
- relative paths are normalized before policy decisions;
- profile behavior does not depend on ambient current directory state;
- profile cannot read or write outside the safe root through arguments, config files, symlinks, archive extraction, or generated output paths.

Reject the profile if it permits:

- `/`, `/data`, `/proc`, `/sys`, `/dev`, `/sdcard`, `/storage`, `$HOME`, or other broad host roots as working directories;
- caller-provided absolute paths;
- path traversal segments that survive normalization;
- following symlinks outside the safe root;
- archive or build output that can write outside the safe root.

Recommended rejection reason codes:

- `safe_root_required`;
- `path_traversal_rejected`;
- `symlink_escape_rejected`;
- `global_path_scope_rejected`;
- `unsafe_output_path_rejected`.

## Environment review

Profiles may allow only environment variable names from an explicit allowlist. Environment values must not be captured in audit records or surfaced through MCP responses.

Pass criteria:

- allowed environment variable names are explicitly enumerated;
- names are necessary for the fixed executable and fixed argv to work;
- values are supplied by the local runtime or operator configuration, not by arbitrary MCP caller input;
- secrets are not required for profile execution unless a later dedicated credential gate is designed and approved;
- audit records include at most counts or allowlist names, never values.

Reject the profile if it allows:

- wildcard environment forwarding;
- full ambient environment inheritance;
- caller-supplied variable names;
- caller-supplied variable values;
- secret-bearing variables such as tokens, passwords, keys, cookies, authorization headers, or cloud credentials.

Recommended rejection reason codes:

- `environment_wildcard_rejected`;
- `ambient_environment_rejected`;
- `caller_environment_rejected`;
- `secret_environment_rejected`.

## Bounds review

Every profile must have explicit runtime and output bounds.

Pass criteria:

- timeout is finite and appropriate for the command purpose;
- stdout byte limit is finite;
- stderr byte limit is finite;
- combined output behavior is deterministic when limits are exceeded;
- failure mode is structured and does not include unbounded command output;
- retry behavior is absent or explicitly bounded.

Reject the profile if it has:

- no timeout;
- no output limits;
- unbounded streaming output;
- unbounded retries;
- failure responses that include raw, unlimited stdout or stderr.

Recommended rejection reason codes:

- `timeout_required`;
- `stdout_bound_required`;
- `stderr_bound_required`;
- `unbounded_retry_rejected`;
- `unbounded_output_rejected`.

## Dry-run or preview review

Where the executable can mutate files, state, services, network settings, packages, Android state, or external systems, a command profile is not eligible under the command-execution gate alone.

A profile with any high-impact or mutation behavior must remain disabled until the relevant higher-risk gate defines:

- dry-run or preview semantics;
- confirmation requirements;
- capability-token requirements;
- rollback expectations;
- audit requirements;
- concurrency and idempotency behavior.

Reject such profiles with `higher_risk_gate_required`.

## Audit review

Every accepted profile must produce a non-sensitive audit event for allowed and denied decisions.

Pass criteria:

- audit event contains stable tool name, gate name, mode, decision, and reason code;
- metadata contains only bounded counts, limit values, or stable profile identifiers;
- audit event does not include raw command output, argv supplied by callers, file contents, environment values, secrets, private host metadata, usernames, Android identifiers, or broad filesystem paths;
- denied decisions are auditable before execution;
- timeout and output-limit failures are auditable without leaking raw output.

Recommended reason codes for accepted profiles:

- `profile_allowlisted`;
- `fixed_argv_verified`;
- `safe_root_verified`;
- `bounded_execution_verified`;
- `audit_verified`.

## Final approval checklist

A profile may be approved only if all answers below are `yes`:

1. Is command execution still disabled unless the operator explicitly opts in?
2. Is the profile defined in reviewed project-owned configuration or source?
3. Is the executable fixed and non-shell?
4. Is argv fixed and non-templated?
5. Is the working directory safe-rooted?
6. Are path traversal and symlink escapes blocked?
7. Is environment exposure name-allowlisted and value-suppressed?
8. Are timeout, stdout, and stderr bounds finite?
9. Are allowed and denied decisions auditable with non-sensitive metadata only?
10. Does the profile avoid Android control, package management, service mutation, process control, network mutation, credential access, and other high-impact actions?
11. Is the failure mode structured and bounded?
12. Is the profile purpose narrow enough that a future reviewer can reason about it independently?

Any `no` answer blocks approval.

## Runtime enablement boundary

This runbook is a validation prerequisite, not an enablement mechanism. A future PR that enables any command-capable MCP surface must still include implementation, tests, documentation, threat review, and explicit opt-in behavior in that PR. Until then, command execution remains unavailable.
