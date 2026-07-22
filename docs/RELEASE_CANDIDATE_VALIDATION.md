# Downloaded Release-Candidate Validation

`scripts/termux_release_validate.sh` validates downloaded Android release candidates without downloading anything or installing packages. It complements the source-building gate in [`DEVICE_PRODUCTION_GATE.md`](DEVICE_PRODUCTION_GATE.md):

- the device-production gate proves an exact source commit can build and survive a comprehensive isolated Termux lifecycle;
- the release-candidate validator proves the exact downloaded default, `mcp-runtime`, `android-volume-control`, and governed `full-suite` artifacts, their checksums, feature postures, fail-closed runtime gates, aggregate runtime behavior, and deployment behavior.

Neither gate creates a tag or GitHub Release. Publishing remains a separate maintainer decision under [`RELEASE_GOVERNANCE.md`](RELEASE_GOVERNANCE.md).

## Dependencies and non-goals

The validator requires Bash, `awk`, `curl`, `dd`, `file`, `grep`, `jq`, `ln`, `realpath`, `sha256sum`, `timeout`, GNU-compatible `stat`/`install`/`mv`, and the repository's `termux_deploy.sh` for deployment phases. It never invokes `pkg`, `git`, GitHub APIs, a browser, or an artifact download command.

Preflight does not start a listener, touch runit, modify the configured safe root, or alter deployment state. After digest and ELF verification, it executes each artifact only as a bounded five-second `--version` probe. It creates only its private temporary workspace and the requested mode-`0600` JSON report.

Runtime and deployment phases are unavailable unless their explicit command-line confirmations are present. The default deployment exercise uses unique project-owned test roots and a dedicated runsvdir. A canonical production-root action requires both a named single action and an action-specific confirmation value.

## No-clone operator setup

The validator does not require a repository clone. Fetch the two reviewed scripts from the exact candidate commit before running it:

```bash
EXPECTED_COMMIT=<full-40-character-main-commit>
TOOLS="$HOME/termux-mcp-release-tools-$EXPECTED_COMMIT"
mkdir -m 700 "$TOOLS"

for script in termux_release_validate.sh termux_deploy.sh; do
  curl --fail --location --proto '=https' --tlsv1.2 \
    "https://raw.githubusercontent.com/CyberBASSLord-666/termux-mcp-edge/$EXPECTED_COMMIT/scripts/$script" \
    --output "$TOOLS/$script"
  chmod 700 "$TOOLS/$script"
  bash -n "$TOOLS/$script"
done
```

Download the default, `mcp-runtime`, `android-volume-control`, and `full-suite` workflow artifacts from the recorded Android run and extract them into separate mode-`0700` directories. In each directory, run `sha256sum -c SHA256SUMS`, then set the extracted binary to mode `0700`. Downloading scripts or artifacts is deliberately outside the validator's authority and should finish before preflight begins.

## Private literal configuration

Create a private regular file in a canonical private directory. Do not source it:

```bash
CONFIG="$HOME/release-validation.env"
TOKEN_FILE="$HOME/release-validation.token"

umask 077
printf '%s' '<private-bearer-token>' >"$TOKEN_FILE"
chmod 600 "$TOKEN_FILE"

cat >"$CONFIG" <<EOF
EXPECTED_COMMIT=<full-40-character-main-commit>
EXPECTED_VERSION=0.6.0
DEFAULT_ARTIFACT=$HOME/artifacts/default/termux-mcp-server
DEFAULT_SHA256=<64-lowercase-hex>
DEFAULT_MANIFEST=$HOME/artifacts/default/artifact-manifest.json
MCP_ARTIFACT=$HOME/artifacts/mcp-runtime/termux-mcp-server
MCP_SHA256=<64-lowercase-hex>
MCP_MANIFEST=$HOME/artifacts/mcp-runtime/artifact-manifest.json
VOLUME_CONTROL_ARTIFACT=$HOME/artifacts/android-volume-control/termux-mcp-server
VOLUME_CONTROL_SHA256=<64-lowercase-hex>
VOLUME_CONTROL_MANIFEST=$HOME/artifacts/android-volume-control/artifact-manifest.json
FULL_SUITE_ARTIFACT=$HOME/artifacts/full-suite/termux-mcp-server
FULL_SUITE_SHA256=<64-lowercase-hex>
FULL_SUITE_MANIFEST=$HOME/artifacts/full-suite/artifact-manifest.json
BASELINE_ARTIFACT=$HOME/artifacts/termux-mcp-server-v0.5.1-aarch64-linux-android-mcp-runtime
BASELINE_VERSION=0.5.1
BASELINE_SHA256=<64-lowercase-hex>
AUTH_TOKEN_FILE=$TOKEN_FILE
SAFE_ROOT=$HOME/mcp-release-validation
BIND_HOST=127.0.0.1
PORT=18765
DEPLOY_SCRIPT=$TOOLS/termux_deploy.sh
CI_RUN_ID=<exact-main-ci-run-id>
SECURITY_RUN_ID=<exact-main-security-run-id>
ANDROID_RUN_ID=<exact-main-android-run-id>
SUSTAINED_OBSERVATION_STATUS=not_run
SUSTAINED_OBSERVATION_MINUTES=0
SUSTAINED_OBSERVATION_REASON_CODE=not_observed
EOF
chmod 600 "$CONFIG"
```

The parser accepts only these keys, rejects duplicates, carriage returns, oversized files, excessive line counts, and malformed lines, preserves every value literally, and never evaluates configuration as shell code. The configuration and token files must be non-symlink regular files with exact mode `0600`. The bearer token must be 1–4096 printable ASCII bytes with no whitespace or trailing newline.

`BIND_HOST` is intentionally restricted to `127.0.0.1`. `PORT` must be between 1024 and 65535. `SAFE_ROOT` must be an existing canonical non-symlink directory strictly beneath `HOME`. Runtime validation creates and later removes one unique child below that root.

The baseline fields are required only for the complete dedicated deployment cycle. The baseline version must differ from the candidate version.

## Phase 1: artifact preflight

Preflight is the default:

```bash
bash scripts/termux_release_validate.sh \
  --config "$CONFIG" \
  --report "$HOME/release-preflight.json" \
  --phase preflight
```

For all four downloaded artifacts it verifies:

- regular, executable, non-symlink state;
- nonzero size no greater than 64 MiB;
- exact supplied SHA-256 digest;
- pairwise-distinct default, `mcp-runtime`, `android-volume-control`, and `full-suite` files, manifests, and digests;
- AArch64 Android ELF identity from `file`;
- an exact workflow-generated manifest matching repository, commit, Android run ID, artifact name, posture, feature set, target, version, digest, size, and ELF classification;
- exact embedded `--version` output.

After checksum verification, the validator copies each artifact into a posture-specific private directory in its temporary workspace, preserving the executable basename as `termux-mcp-server`. It rechecks size, digest, and ELF identity there and uses only that pinned copy for the bounded version, runtime, and deployment phases. Preserving the basename is part of the fixed command-profile identity boundary; it does not weaken the no-follow device/inode checks. The version probe is limited to five seconds. The validator does not request listener or service startup during preflight, but `--version` still executes candidate code; treat the supplied digest and matching workflow manifest as the trust boundary for that execution.

The CI and Security run IDs remain operator-supplied provenance assertions. Each Android bundle includes `artifact-manifest.json` and `SHA256SUMS`; the validator requires the manifest's exact commit and Android run ID to match the private configuration and requires its digest/size to match the downloaded binary. The binary does not currently embed a Git commit, so retain the complete workflow bundle and independently confirm that the recorded run identifies the intended commit.

## Phase 2: isolated runtime validation

Runtime validation requires explicit mutation confirmation:

```bash
bash scripts/termux_release_validate.sh \
  --config "$CONFIG" \
  --report "$HOME/release-runtime.json" \
  --phase runtime \
  --confirm-runtime-mutation
```

The phase starts each artifact directly on loopback, one at a time, using a private token and a unique validation child below `SAFE_ROOT`. Child environments are rebuilt from an explicit allowlist so ambient `MCP__*` settings cannot broaden the test. The validator creates a private random capability key, enables only the `mcp-runtime` candidate's four filesystem-mutation gates, invokes that exact binary's operation-specific offline issuers, and destroys grant material with its isolated workspace.

The default artifact must:

- become healthy and ready;
- report `mcp_runtime_enabled=false`;
- return HTTP 404 for `/mcp`.

The `mcp-runtime` artifact must prove:

- readiness with the configured request-body ceiling;
- unauthenticated HTTP 401 before body limiting;
- authentication before Host/Origin validation, plus rejected unexpected Host, missing Origin, and unexpected Origin values;
- stable `2025-11-25` initialize/initialized lifecycle;
- required protocol and session headers plus unknown-session rejection;
- the exact 17-tool allowlist, including grant-gated `create_directory`, bounded content-private `copy_file`, reversible recovery-retained `trash_file`, content-free bounded literal `find_paths`, bounded SHA-256 `hash_file`, canonical bounded `read_binary_file`, canonical bounded `read_binary_range`, code-point-safe bounded `read_text_range`, content-free `path_metadata`, and bounded literal `search_text`;
- command execution, Android control, and high-impact gates disabled;
- bounded read-only platform, Android, and project-service metadata with the project-service allowlist enforced;
- enabled directory-mutation discovery/status, missing-grant denial, wrong-target binding denial, dry-run non-consumption, one locally issued exact-target mode-`0700` creation, and replay denial;
- deterministic bounded listing and descriptor-relative path metadata;
- exact bounded SHA-256 hashing plus symlink/oversize rejection without path, content, or digest reflection;
- bounded safe-root read plus rejection of JSON expansion beyond the response ceiling;
- default-preview and explicit binary `copy_file`, fixed mode `0600`, exact content, existing-destination preservation, symlink denial, one-byte-over rejection, and content-free results;
- default-preview and exact-grant `trash_file`, identity/content mismatch and oversized-response preflight denial before mutation, exact 1 MiB acceptance and one-byte-over rejection, atomic no-replace movement of the exact inode into a mode-`0700` bounded recovery quarantine, namespace hiding, and path/content/artifact-free results and audits;
- disabled-posture write discovery and live-mutation denial; enabled-posture grant-free dry run; missing-grant and content-binding denial; exact mode-`0600` grant-authorized create and replace; `recoveryArtifactRetained:false` for preview/create and `true` for replacement; one bounded preserved recovery artifact; reserved-quarantine isolation/capacity; and replay denial;
- lexical out-of-root and in-root symlink-escape denial without path/content reflection;
- unavailable shell/high-impact invocation;
- authenticated HTTP 413 and unauthenticated-first HTTP 401 ordering;
- documented default JSON/GET-405 posture and default-disabled bounded SSE option;
- explicit session deletion.

Trash-grant replay and concurrent-replay denial remain required automated core/integration-test evidence. Validator v11 directly proves issuance, binding denial, preflight preservation, and successful recovery retention, but it does not reuse a consumed trash grant.

The `android-volume-control` artifact must additionally prove:

- the incompatible `mcp-runtime` artifact rejects `MCP__ANDROID__VOLUME_CONTROL_ENABLED=true` before listener startup;
- the control artifact starts with its runtime gate omitted and reports `androidVolumeControlCompiled=true`, `androidVolumeControlEnabled=false`, and no active grant requirement;
- `set_android_volume` is absent from the exact discovery allowlist while disabled;
- a direct call returns the stable `volume_control_runtime_disabled` result;
- no control grant is issued, `termux-volume` is never invoked, and device audio is never changed by the canonical validator.

The governed `full-suite` artifact is exercised in six runtime postures:

1. With all optional runtime flags omitted, it must expose exactly the 17 baseline tools in deterministic order. Runtime status must report battery status, volume status/control, and fixed command execution as compiled but disabled. Direct calls must return their stable runtime-disabled results, and all four filesystem mutation gates and grant requirements must remain false.
2. Four isolated postures enable battery, volume status, volume control, or fixed command execution one at a time. Each must expose exactly the 17 baseline tools plus only its selected tool, report only that runtime gate active, and complete the selected bounded provider/profile call.
3. With `MCP__ANDROID__BATTERY_STATUS_ENABLED`, `MCP__ANDROID__VOLUME_STATUS_ENABLED`, `MCP__ANDROID__VOLUME_CONTROL_ENABLED`, and `MCP__COMMAND__ENABLED` explicitly enabled, it must expose exactly 21 tools in deterministic order. The validator reads normalized battery and six-stream volume status, requests a distinct in-range non-mutating volume preview, proves a distinct live request is rejected without its exact grant, re-reads the provider after both calls, and runs only the fixed `server_version` command profile. It directly dispatches create, copy, reversible trash, and write with `dry_run:false`, requires each stable disabled reason, and proves source identity/content, target absence, and quarantine state remain unchanged. A defective volume candidate triggers fixed-path restoration and provider re-verification before the validator fails.

The fully enabled phase requires the real Termux:API battery and volume providers to succeed. It never issues a volume-control grant and never changes device audio. Fixed command execution succeeds only from the basename-preserving pinned artifact and remains bound to the already-loaded executable inode.

Response bodies, safe-root paths, test file contents, bearer tokens, capability keys/grants, and session identifiers stay in the private temporary workspace and are deleted. They are never copied into JSON evidence. A passing validator-v11 runtime result includes `request_scoped_single_use_grant_enforced`, `request_scoped_single_use_copy_grant_enforced`, `request_scoped_trash_grant_enforced`, `trash_identity_content_binding_enforced`, `trash_recovery_quarantine_verified`, `request_scoped_single_use_write_grant_enforced`, `safe_root_path_discovery_verified`, `safe_root_file_hash_verified`, `safe_root_binary_read_verified`, `safe_root_binary_range_read_verified`, `incompatible_volume_control_artifact_rejected`, `volume_control_hidden_while_disabled`, `volume_control_disabled_call_rejected`, `full_suite_default_disabled_17_tool_posture_verified`, four `full_suite_*_runtime_gate_independence_verified` results, `full_suite_enabled_21_tool_posture_verified`, `full_suite_optional_provider_success_verified`, `full_suite_volume_preview_and_grant_boundary_verified`, `full_suite_command_basename_and_profile_verified`, and `full_suite_filesystem_mutations_independently_disabled`.

## Phase 3: deployment validation

The dedicated deployment cycle requires explicit confirmation:

```bash
bash scripts/termux_release_validate.sh \
  --config "$CONFIG" \
  --report "$HOME/release-deployment.json" \
  --phase deployment \
  --confirm-deployment-mutation
```

It uses the canonical `termux_deploy.sh` manager with unique deployment, configuration, service, and safe-root paths. On a real Termux device it starts a dedicated `runsvdir` and first installs, upgrades to, rolls back from, and uninstalls the default-posture artifact. It then verifies the governed `full-suite` recovery cycle:

1. baseline install;
2. forced candidate-readiness failure and prior-runtime recovery;
3. candidate upgrade;
4. forced rollback-readiness failure and original-candidate recovery;
5. successful rollback;
6. uninstall with configuration-preservation behavior;
7. confirmed service shutdown and isolated cleanup.

Use `--phase all` with both confirmation flags to run preflight, runtime, and the dedicated deployment cycle in one evidence document.

### Canonical production-root actions

Production-root mutation is deliberately one action per invocation. It is never selected by `--phase all`. The operator must provide:

- `--phase deployment`;
- `--confirm-deployment-mutation`;
- `--production-action install|upgrade|upgrade-failure|rollback|uninstall`;
- `--confirm-production-roots termux-mcp-edge-production-<action>`.

For example, an explicitly approved upgrade:

```bash
bash scripts/termux_release_validate.sh \
  --config "$CONFIG" \
  --report "$HOME/release-production-upgrade.json" \
  --phase deployment \
  --confirm-deployment-mutation \
  --production-action upgrade \
  --confirm-production-roots termux-mcp-edge-production-upgrade
```

Canonical actions are accepted only on AArch64 Termux when `HOME` and `PREFIX` match the application's canonical private directories. Install, upgrade, and forced-upgrade-failure actions select the exact verified `full-suite` artifact; rollback and uninstall remain manager actions against existing production state. They use the production manager's default roots and existing private production configuration. The validator does not create or replace production `runtime.env`. Ordinary release validation should use the dedicated cycle; production actions are for an already approved release operation. The forced-failure action also snapshots the canonical `current`/`previous` links and requires exact restoration plus candidate removal before it passes.

## Versioned sanitized evidence

Validator-v11 reports conform to [`release-evidence-schema-v2.json`](release-evidence-schema-v2.json). Historical validator reports remain governed by the unchanged [`release-evidence-schema-v1.json`](release-evidence-schema-v1.json); v1 cannot represent or qualify the new full-suite build input. The v2 report contains:

- schema and validator versions;
- pass/fail status and one stable failure code;
- exact expected commit, package version, and workflow run IDs;
- architecture and bounded tool-version strings;
- exact default, `mcp-runtime`, `android-volume-control`, full-suite, and optional baseline digest, size, version, and ELF classifications;
- `deploymentCandidate.posture:"full-suite"` plus the bounded production action name when one was explicitly requested;
- requested phase and per-phase state;
- fixed check names, outcomes, and reason codes;
- operator-supplied sustained-observation state.

It intentionally excludes:

- artifact, configuration, token, report, safe-root, deployment, and service paths;
- bearer tokens, capability keys/grants, and environment values;
- request/response bodies and file contents;
- MCP session identifiers, grant identifiers/bindings, and replay state;
- PIDs, hostnames, usernames, and persistent device identifiers;
- arbitrary operator notes.

Reports are created atomically as mode `0600` and existing reports are never overwritten.

## Sustained observation

Automated endpoint success cannot establish battery, thermal, network, or Android process-restriction stability. The minimum production observation window is 60 minutes.

Before the observation:

```text
SUSTAINED_OBSERVATION_STATUS=not_run
SUSTAINED_OBSERVATION_MINUTES=0
SUSTAINED_OBSERVATION_REASON_CODE=not_observed
```

A passing operator observation requires at least 60 minutes:

```text
SUSTAINED_OBSERVATION_STATUS=pass
SUSTAINED_OBSERVATION_MINUTES=60
SUSTAINED_OBSERVATION_REASON_CODE=stable
```

A failed observation uses `fail`, a positive observed duration, and one bounded reason code: `battery_limit`, `thermal_limit`, `process_restriction`, `network_instability`, `operator_abort`, or `other`.

A supplied failed observation makes the validator exit nonzero with `sustained_observation_failed`, even when every automated phase passed. `not_run` may accompany a successful automated report but can never make it release-eligible.

`releaseEligible` becomes true only for a non-fixture validator-v11 `--phase all` report with every phase passing, the exact full-suite artifact bound into schema-v2 evidence, the dedicated full-suite deployment candidate selected, and a valid passing sustained observation. This field is evidence for maintainer review; it does not publish or authorize a release.

The canonical report intentionally models only a direct observation. The historical v0.5.1 observation and its bridge do not bind the full-suite digest and cannot qualify this changed build input; the first full-suite candidate requires a fresh direct physical AArch64 Termux observation. Any future inherited-observation route must explicitly bind the source report, bridge artifact, and full-suite digest under its then-current versioned contract. It cannot alter or relabel a `not_run` canonical report.

## Failure and cleanup

Any missing confirmation, digest mismatch, wrong architecture or version, feature-posture mismatch, failed runtime assertion, deployment recovery failure, interruption, or unconfirmed cleanup produces a nonzero exit and a fixed failure code.

Dedicated runtime and deployment state is removed only through the validator's owned cleanup paths. An unconfirmed service shutdown changes the result to `cleanup_unconfirmed`. Production-root actions are never automatically reversed beyond the recovery behavior already implemented by `termux_deploy.sh`.
