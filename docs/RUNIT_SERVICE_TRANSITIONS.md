# Atomic runit Service Transitions

The canonical Termux deployment manager owns only `$PREFIX/var/service/mcp_runtime`. Release links, the service definition, and runtime state are treated as one deployment transaction.

## Fail-closed shutdown

Upgrade, rollback, recovery, and uninstall must stop the canonical service before replacing release links, replacing the service `run` file, or deleting release state. A failed `sv down` request or failure to observe a confirmed `down:` status is a hard error. The operation must leave the prior links, release directories, and service definition intact.

The shutdown polling budget is controlled by `TERMUX_MCP_STOP_ATTEMPTS` and `TERMUX_MCP_STOP_DELAY_SECONDS`. Production defaults are bounded to avoid hanging a mobile deployment indefinitely.

## Atomic service publication

Initial installation builds a complete private service directory outside the runit service root. The staged directory contains:

- an executable, owner-only `run` file;
- an owner-only `down` marker that prevents automatic startup;
- no logs, tokens, or generated environment values.

The complete directory is then renamed into the service root. Runit cannot observe a partial `run` file or a service directory that is eligible for startup before activation is ready.

For an existing service, deployment first confirms the service is down, creates the `down` marker, writes a private temporary run file in the service directory, and atomically renames it over `run`.

The `down` marker is removed only when release links and the complete service definition are ready, immediately before the explicit `sv up` transition.

## Exact restoration

Before mutation, the manager snapshots whether the canonical service directory, `run` file, and `down` marker existed, together with the existing run file and directory mode. It also snapshots the exact `current` and `previous` targets.

On candidate failure or interruption, the manager:

1. attempts to stop the candidate service;
2. restores the exact prior release-link state;
3. restores or removes the service directory, run file, and down marker to match the snapshot;
4. removes the failed candidate release;
5. restarts and probes the prior runtime when one existed.

A failed initial installation therefore does not leave a newly visible service directory or run file behind.

## Uninstall safety

Uninstall acquires the deployment lock and confirms the canonical service is down before deleting the service directory or any release state. If shutdown cannot be requested or confirmed, uninstall fails without deleting the deployment, configuration, release links, or service definition.

Configuration is preserved unless `--purge-config` is explicitly supplied.

## Operator validation

Run the repository shell suite before device testing:

```bash
bash -n scripts/termux_deploy.sh
bash tests/termux_deploy_test.sh
```

On a Termux device with `termux-services` installed, validate:

1. initial install does not start before `current` and `run` are complete;
2. upgrade stops the old process, atomically switches releases, and reaches ready state;
3. an unhealthy candidate restores the prior service definition and release links;
4. rollback failure restores the original active runtime;
5. a forced shutdown failure leaves all release and service state untouched;
6. uninstall refuses to delete state while the service cannot be confirmed down;
7. successful uninstall removes only the project service and deployment roots.

Never bypass a shutdown failure by manually deleting the service directory or release tree. Resolve the runit/process state first, then rerun the canonical deployment command.