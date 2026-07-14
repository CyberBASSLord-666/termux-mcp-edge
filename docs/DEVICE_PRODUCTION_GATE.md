# Exact-Commit Termux Device Production Gate

`scripts/termux_device_smoke.sh` is the canonical no-clone device gate for an AArch64 Termux release candidate. It bootstraps the required packages, fetches one Git ref, refuses to proceed unless that ref resolves to the required full commit SHA, builds the native `mcp-runtime` posture, and exercises the release through an isolated real `runsvdir`.

Passing host CI or Android cross-compilation does not replace this gate. Passing this gate also does not replace a sustained operational soak under the target device's real battery, thermal, network, and process-restriction conditions.

## Safety and effects

The harness:

- uses `set -Eeuo pipefail`, a private umask, and no shell tracing;
- requires a lowercase 40-character expected commit rather than trusting a mutable branch;
- checks out the fetched source detached and verifies the exact Git head before building;
- creates isolated deployment, configuration, safe-root, and service roots containing the commit prefix and a unique run ID;
- starts a dedicated `runsvdir` for only the isolated service root;
- generates a private bearer token without printing it;
- cleans the isolated live deployment, configuration, safe-root, and service state after confirmed shutdown on success, failure, or interruption;
- fails and preserves isolated state for manual recovery if service shutdown cannot be confirmed;
- preserves the report, source checkout, artifacts, and detailed logs under `HOME` for review.

By default, package bootstrap runs `pkg update`, `pkg upgrade`, and installs the required build and test packages. Set `TERMUX_MCP_SMOKE_SKIP_PACKAGE_BOOTSTRAP=true` only when the required packages are already installed and their state is understood. Set `TERMUX_MCP_SMOKE_UPGRADE_PACKAGES=false` to install missing requirements without a full package upgrade.

The harness does not touch the canonical production deployment root or canonical `$PREFIX/var/service/mcp_runtime` directory. It requires the isolated service root and `HOME` to be on the same filesystem because the deployment manager's atomic-publication contract depends on same-filesystem rename.

## Run without a local clone

Choose the exact commit from the release-candidate evidence. Download the harness from that immutable commit, record its checksum, and execute it:

```bash
EXPECTED_HEAD='<full-40-character-main-commit-sha>'
HARNESS="$HOME/termux-mcp-device-smoke-$EXPECTED_HEAD.sh"

curl -fL \
  "https://raw.githubusercontent.com/CyberBASSLord-666/termux-mcp-edge/$EXPECTED_HEAD/scripts/termux_device_smoke.sh" \
  -o "$HARNESS"
chmod 700 "$HARNESS"
sha256sum "$HARNESS"

TERMUX_MCP_SMOKE_EXPECTED_HEAD="$EXPECTED_HEAD" \
TERMUX_MCP_SMOKE_FETCH_REF=main \
TERMUX_MCP_SMOKE_CI_EVIDENCE='<exact-head-ci-run-url>' \
  bash "$HARNESS"
```

Using `main` as the fetch ref is safe only because the fetched head must still equal `EXPECTED_HEAD`; a concurrent main update causes a fail-closed mismatch. To validate an unmerged pull request, set `TERMUX_MCP_SMOKE_FETCH_REF=pull/<number>/head` and use that PR's exact head SHA. If the exact commit is directly fetchable, `TERMUX_MCP_SMOKE_FETCH_REF` may be omitted.

At least 1.5 GiB of free space is required. Reusing a prior Cargo target directory can materially reduce rebuild time:

```bash
TERMUX_MCP_SMOKE_EXPECTED_HEAD="$EXPECTED_HEAD" \
TERMUX_MCP_SMOKE_FETCH_REF=main \
TERMUX_MCP_SMOKE_CARGO_TARGET_DIR="$HOME/termux-mcp-cargo-target" \
  bash "$HARNESS"
```

The target directory must resolve beneath `HOME`.

## Automated evidence

The harness records and verifies:

1. exact fetched and built commit SHA;
2. Termux architecture and native Rust/Clang versions;
3. candidate package version and `--version` output;
4. AArch64 Android ELF identity and SHA-256 digest;
5. private runtime configuration mode;
6. same-filesystem atomic-publication prerequisite;
7. initial versioned install under a real isolated `runsvdir`;
8. injected candidate-readiness failure with prior-runtime restoration;
9. successful upgrade and exact `current`/`previous` links;
10. health and readiness;
11. unauthenticated MCP rejection;
12. stable `2025-11-25` initialization, notification, and session deletion;
13. the exact ten-tool discovery allowlist, including dry-run-first `create_directory` and bounded `path_metadata` and `search_text`;
14. disabled command, Android-control, and high-impact gates;
15. default-dry-run and explicit mode-`0700` directory creation, safe-rooted listing, content-free path metadata, and UTF-8 read;
16. default dry-run file write and explicit mutation with final mode `0600`;
17. out-of-root read denial without content reflection;
18. unavailable shell/high-impact invocation;
19. authenticated request-body limiting and unauthenticated-before-limit ordering;
20. rollback dry-run immutability;
21. injected rollback-readiness failure with original-candidate restoration;
22. successful rollback;
23. uninstall with configuration-preservation behavior;
24. isolated-state cleanup.

The final report must contain all of:

```text
exact_head=<expected-sha>
candidate_sha256=<artifact-sha256>
TERMUX_MCP_DEVICE_RESULT=PASS
cleanup_complete=true
final_status=PASS
```

Any `FAIL`, missing final marker, unexpected exit, interrupt, exact-head mismatch, or waived assertion means the candidate did not pass.

## Evidence handling

The report is created as mode `0600` in `HOME`. Detailed package, fetch, build, deployment, and protocol files remain under the reported work root. The bearer token and file contents used for denied-access testing are not printed to the report.

Before sharing a report, review it for device-local metadata you do not want to disclose, including installed tool versions, storage availability, PIDs, generated paths, and filesystem device numbers. Retain the exact CI, Security when applicable, and Android workflow URLs alongside the device report.

## Manual completion

After the automated pass:

1. deploy the retained candidate through the canonical production paths only after explicit release approval;
2. keep the previous known-good release available;
3. observe health, readiness, memory, battery, temperature, network behavior, and runit stability for the release's defined soak interval;
4. repeat authenticated representative calls against the production configuration without broadening safe roots;
5. roll back on any unexplained restart, resource growth, thermal instability, authentication anomaly, or filesystem-policy mismatch.

Do not label the exact candidate production-ready until its report, workflow evidence, and required sustained-device observation all refer to the same release candidate.
