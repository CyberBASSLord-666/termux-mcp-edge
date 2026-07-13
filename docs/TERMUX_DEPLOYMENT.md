# Termux Deployment, Upgrade, and Recovery

This guide installs a validated `termux-mcp-server` binary into a project-owned, versioned layout and manages only the `mcp_runtime` runit service.

## Layout and boundaries

```text
~/.local/share/termux-mcp-edge/
  releases/<version>/termux-mcp-server
  releases/<version>/VERSION
  current  -> releases/<active-version>
  previous -> releases/<rollback-version>
~/.config/termux-mcp-edge/runtime.env
$PREFIX/var/service/mcp_runtime/run
```

Deployment and configuration roots must remain below `HOME`. The service root and service interpreter must remain below `PREFIX`. Deployment and configuration roots may not overlap. Configuration and bearer material remain outside versioned releases.

Install, upgrade, rollback, and uninstall are serialized by a project deployment lock. Temporary staging directories, link files, and owned stale locks are cleaned automatically.

## Prerequisites

```bash
pkg update
pkg install bash coreutils curl file termux-services
chmod 700 scripts/termux_deploy.sh
```

The deployment manager requires the standard Termux implementations of `realpath`, `stat`, `sha256sum`, `timeout`, `file`, `uname`, `install`, and `readlink`.

## Runtime configuration

Create a private configuration file before installation:

```bash
install -d -m 700 "$HOME/.config/termux-mcp-edge"
umask 077
cat >"$HOME/.config/termux-mcp-edge/runtime.env" <<'EOF'
MCP__AUTH__STATIC_TOKEN=replace-with-a-strong-random-token
MCP__SERVER__HOST=127.0.0.1
MCP__SERVER__PORT=8000
MCP__TRANSPORT__ALLOWED_HOSTS=localhost:8000,127.0.0.1:8000
MCP__TRANSPORT__ALLOWED_ORIGINS=http://localhost:8000,http://127.0.0.1:8000
MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4
MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30
MCP__TRANSPORT__MAX_BODY_BYTES=2097152
RUST_LOG=termux_mcp_server=info
EOF
chmod 600 "$HOME/.config/termux-mcp-edge/runtime.env"
```

The file must be a regular non-symlink file, owner-readable, and inaccessible to group and other users. Blank lines and comments are allowed. Entries use literal `NAME=value` syntax and are limited to `MCP__*`, `RUST_LOG`, and `RUST_BACKTRACE`.

An artifact built with `--features android-battery-status` may opt into its read-only battery tool by adding this literal entry after the official Termux:API prerequisites are installed:

```text
MCP__ANDROID__BATTERY_STATUS_ENABLED=true
```

Do not add that setting to a default or `mcp-runtime`-only artifact: startup intentionally fails when the runtime flag is true but the compile-time battery feature is absent. See [`ANDROID_BATTERY_STATUS.md`](ANDROID_BATTERY_STATUS.md).

Static-token mode requires a non-empty token without whitespace. A tokenless configuration is valid only for explicit localhost-only development with a loopback server host.

## Validate the candidate

```bash
ARTIFACT="target/aarch64-linux-android/release/termux-mcp-server"
file "$ARTIFACT"
"$ARTIFACT" --version
ARTIFACT_SHA256="$(sha256sum "$ARTIFACT" | awk '{print $1}')"
```

The candidate must be a non-empty regular executable, must not be a symbolic link, must remain below the configured artifact-size ceiling, and must report the requested version through `--version`. Outside test mode it must be an ELF executable matching the device architecture.

SHA-256 verification is required by default. An advanced operator may explicitly select the documented unverified-local-artifact option after independent validation; that option does not disable version, executable, architecture, size, root, configuration, or readiness checks.

## Initial install

```bash
scripts/termux_deploy.sh install \
  --artifact "$ARTIFACT" \
  --version 0.6.0 \
  --sha256 "$ARTIFACT_SHA256"
```

Initial install requires no active release. The manager validates all inputs, acquires the lock, stages the release, writes the fixed project service, atomically activates `current`, starts the service, and verifies `/health` and `/ready`.

If readiness fails, the failed candidate and active link are removed while persistent configuration is preserved.

## Upgrade

```bash
NEW_ARTIFACT="/path/to/termux-mcp-server"
NEW_SHA256="$(sha256sum "$NEW_ARTIFACT" | awk '{print $1}')"
scripts/termux_deploy.sh upgrade \
  --artifact "$NEW_ARTIFACT" \
  --version 0.6.0 \
  --sha256 "$NEW_SHA256"
```

Upgrade requires an active release. The exact prior `current` and `previous` link state is captured before activation. If the candidate fails readiness, the prior state is restored, the failed release is removed, and the prior active runtime is restarted and probed. The upgrade still exits non-zero after successful recovery.

## Rollback

```bash
scripts/termux_deploy.sh rollback
```

Rollback accepts only complete release targets below the project releases root. If the selected rollback target fails readiness, the original exact link state is restored and the original active runtime is restarted and probed. The command exits non-zero when rollback validation fails.

## Status

```bash
scripts/termux_deploy.sh status
sv status "$PREFIX/var/service/mcp_runtime"
curl -fsS http://127.0.0.1:8000/health
curl -fsS http://127.0.0.1:8000/ready
```

Status reports only the deployment root, validated current and previous targets, and the fixed service name. Invalid or escaping release links produce a non-zero result. Configuration and token values are never printed.

## Dry run

```bash
scripts/termux_deploy.sh upgrade \
  --artifact "$NEW_ARTIFACT" \
  --version 0.6.0 \
  --sha256 "$NEW_SHA256" \
  --dry-run
```

Dry run validates the requested operation and prints planned mutations without creating releases, links, services, or locks.

## Uninstall

Preserve configuration:

```bash
scripts/termux_deploy.sh uninstall
```

Remove configuration explicitly:

```bash
scripts/termux_deploy.sh uninstall --purge-config
```

Both operations target only the configured project deployment root, the fixed `mcp_runtime` service directory, and—when explicitly requested—the project configuration root.

## Interrupted-operation recovery

1. Run `scripts/termux_deploy.sh status`.
2. Inspect only the project deployment root and its `.deploy-lock` sibling.
3. Re-run the intended operation; an abandoned lock whose owner is no longer active is recovered automatically.
4. Do not manually point `current` or `previous` outside the releases root.
5. Preserve `runtime.env` during ordinary recovery.

## CI validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
bash tests/termux_deploy_test.sh
```

The test suite covers the binary CLI contract, verified install, invalid operation modes, checksum and version failures, active and stale locks, dry-run immutability, literal configuration handling, failed-candidate recovery, failed-rollback recovery, invalid links, unsafe roots, configuration preservation, and explicit purge.

## On-device production gate

Use [`DEVICE_PRODUCTION_GATE.md`](DEVICE_PRODUCTION_GATE.md) and `scripts/termux_device_smoke.sh` for the canonical automated no-clone exercise. It pins the fetched source to a required full commit SHA and covers the checks below in isolated real runit state. Preserve its mode-`0600` report with the exact CI and Android evidence.

Use [`RELEASE_CANDIDATE_VALIDATION.md`](RELEASE_CANDIDATE_VALIDATION.md) for downloaded default/`mcp-runtime` artifacts. Its deployment phase calls this same manager in unique test roots by default, requires explicit mutation confirmation, exercises failed-candidate and failed-rollback recovery, and emits a sanitized JSON result. Canonical production-root actions require a separate action-specific confirmation and never replace production configuration.

1. Confirm the artifact corresponds to the intended exact commit or release.
2. Verify its SHA-256 digest.
3. Verify AArch64 Android-compatible ELF metadata.
4. Confirm `--version` exactly matches the release version.
5. Confirm `runtime.env` is private and contains the intended authentication and transport settings.
6. Install or upgrade through `termux_deploy.sh`.
7. Confirm deployment status, runit status, `/health`, and `/ready`.
8. Run authenticated MCP discovery and representative allowed and denied calls.
9. Exercise rollback and restoration behavior.
10. Preserve the prior known-good release until sustained device validation is complete under realistic battery, thermal, and process-restriction conditions.

For the first governed public release, v0.6.0 has no authoritative prior public release to roll back to. The exact-main v0.5.1 candidate recorded in [`V0.6.0_RELEASE_CANDIDATE.md`](V0.6.0_RELEASE_CANDIDATE.md) is internal upgrade/recovery validation evidence, not a published installation source. A clean v0.6.0 installation can be uninstalled, but it cannot use `rollback` until a second complete release has been installed through this manager.
