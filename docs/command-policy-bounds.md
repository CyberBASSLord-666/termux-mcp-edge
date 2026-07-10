# Command Policy Bounds Contract

The command policy remains a design-only preview. It does not expose an MCP tool, spawn a process, invoke a shell, read environment values, or enable command execution.

## Request bounds

Every accepted preview must satisfy all of the following before detailed allowlist comparison:

- argv contains at most 16 elements;
- environment-name metadata contains at most 8 elements;
- timeout is at least 1 second and no greater than the selected fixed profile limit;
- stdout and stderr caps are each at least 1 byte and no greater than the selected fixed profile limits;
- the working directory is already proven safe-rooted;
- argv exactly matches the fixed profile vector;
- every environment name is present in the fixed profile allowlist.

Zero does not mean unlimited. Zero timeout and zero output caps are rejected with stable lower-bound reason codes.

## Stable denial reasons

The policy emits non-sensitive reason codes:

- `argv_count_exceeds_limit`;
- `environment_name_count_exceeds_limit`;
- `timeout_below_minimum`;
- `timeout_exceeds_limit`;
- `stdout_cap_below_minimum`;
- `stdout_cap_exceeds_limit`;
- `stderr_cap_below_minimum`;
- `stderr_cap_exceeds_limit`.

Existing fixed-command, exact-argv, safe-root, environment-name allowlist, and disabled-execution reasons remain unchanged.

## Denial precedence

The policy first resolves the fixed command identifier and preserves the disabled-runtime gate for execution requests. It then rejects oversized argv or environment-name cardinality before inspecting individual elements. Exact argv matching, numeric lower and upper bounds, safe-root status, and environment allowlist comparison follow.

This ordering bounds work for malformed previews and makes the first denial deterministic. Tests cover zero, minimum, maximum, above-limit, cardinality-boundary, and competing-denial cases.

## Audit privacy

Audit events retain only stable reason codes, the fixed command ordinal when known, bounded numeric limits, and request-side counts. They never contain argv values, environment names or values, command output, secrets, filesystem paths, or caller-provided strings.

See `docs/command-profile-validation.md` for the broader operator review required before any future runtime enablement.
