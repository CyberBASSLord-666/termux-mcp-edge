# Shizuku/rish physical identity workflow

The `Android Rish Physical Identity` workflow is the protected, manual
development gate for the S2.5 Shizuku/rish identity probe. It proves only that
one exact same-repository pull-request commit can run the closed
`android_rish_status` probe through a pinned private `rish` DEX as Android
shell UID 2000 on a configured physical AArch64 device.

This is reviewed-code physical compatibility evidence. It is not an
adversarial-code sandbox, security attestation, same-UID persistence proof,
or claim that the candidate binary cannot misuse authority already available
to the Termux UID. Before the physical boundary is admitted, the resolver
requires a non-draft exact-head PR and at least two distinct current
exact-commit approvals from repository owners, members, or collaborators.
The PR author cannot supply either approval.

The evidence is deliberately narrow:

- `releaseEligible` is always `false`.
- `productionControlQualified` is always `false`.
- `qualificationClass` is
  `physical_shizuku_rish_identity_development_v1`.
- `scope` is `s2_5_uid_probe_only`.
- S3 attestation, typed Android reads, capability grants, device mutation,
  arbitrary shell execution, and production control are all outside this
  gate.
- `sameUidPersistenceExcluded` is always `false`; the gate makes no positive
  claim that one process sharing the Termux UID cannot persist state for a
  later process.
- `continuousNetworkIsolation` and `adversarialNetworkIsolation` are always
  `false`; the two bounded controller observations do not establish either
  broader property.

Passing evidence contains exactly these ordered physical scenarios:

1. `controller_offline_posture_pre_candidate`
2. `trusted_direct_rish_probe_pre_candidate`
3. `runtime_disabled_tool_absent`
4. `candidate_mcp_status_uid_2000`
5. `extra_arguments_rejected`
6. `unknown_shell_rejected`
7. `dex_tamper_rejected`
8. `dex_mode_rejected`
9. `dex_symlink_rejected`
10. `all_mutation_gates_disabled`
11. `trusted_direct_rish_probe_post_candidate`
12. `controller_offline_posture_post_candidate`
13. `bounded_test_fixture_cleanup`
14. `device_slot_quarantined_after_candidate`

The four pre/post controller validation booleans
`controllerOfflinePosturePreCandidate`,
`trustedDirectRishProbePreCandidate`,
`trustedDirectRishProbePostCandidate`, and
`controllerOfflinePosturePostCandidate` are all `true`, and cleanup is the
exact five-key object
`candidateProcessGroupStopped:true`, `portReleased:true`,
`deviceFixtureStateRemoved:true`, `controllerTransportRemoved:true`, and
`deviceSlotQuarantinedAfterCandidate:true`.

This workflow is independent of the release qualification workflows and does
not add an Android build posture to their governed inventories. It must first
land on `main`; it can then qualify an exact open pull-request head without
merging that head, moving `main`, or changing any governed release inventory.

## Trust boundaries

The workflow accepts exactly two caller inputs: a 40-character candidate
commit and its open pull-request number. It has four ordered boundaries:

1. **Hosted preflight and build.** A GitHub-hosted runner resolves the open
   non-draft same-repository PR head, verifies its two current trusted
   exact-head approvals, and requeries exact successful, first-attempt CI,
   Security, and Android Cross Compile runs. It checks out the candidate only
   on this hosted runner, builds the `android-rish` posture, and emits the
   closed three-file development bundle.
2. **Protected physical gate.** A dedicated self-hosted controller checks out
   only the workflow-definition commit from `main`, independently requeries
   the same PR and companion runs, downloads the exact hosted artifact by ID,
   and invokes only the trusted default-branch controller and device-gate
   scripts. The candidate binary is the only candidate-controlled file sent
   to the device; candidate scripts never run on the controller or device.
3. **Hosted evidence validation.** A fresh hosted runner downloads the
   sanitized evidence by artifact ID, requeries the candidate and companion
   runs, validates the closed policy/schema, and reconciles every committed
   identity.
4. **Protected final review.** A separate protected environment pauses the
   final hosted job. After approval, the job repeats all remote and local
   validation and uploads only
   `android-rish-physical-identity-evidence-v1.json`.

The workflow uses `workflow_dispatch` only. It has no
`pull_request_target`, scheduled, push, or pull-request trigger. The dispatch
must target `main`, must be the first run attempt, and must execute the exact
workflow definition checked out by every job.

## Required repository environments

Create both environments before dispatching the workflow. Restrict both to
the `main` branch, disable administrator bypass, prevent self-review, and
require named reviewers. Do not place credentials in either environment.

### `android-rish-physical-development`

This environment protects the dedicated self-hosted controller job. Its
reviewer must be independent of the workflow initiator and candidate author
and must compare the exact candidate head before approval. Neither the
initiator nor candidate author may approve their own run. Configure these
environment-scoped variables:

| Variable | Required value |
| --- | --- |
| `ANDROID_RISH_PHYSICAL_DEVELOPMENT_PROTECTED` | `reviewed-v1` |
| `ANDROID_RISH_PHYSICAL_DEVICE_SLOT` | An administrator-chosen slot name matching `^[a-z0-9][a-z0-9_-]{0,31}$` |

Do not define either variable at repository or organization scope. The
controller resolves the slot through its trusted local inventory; no device
serial, SSH identity, DEX path, or command is accepted from workflow input.

### `android-rish-physical-final-review`

This environment protects the final hosted validation and evidence upload.
Its required reviewer set must be disjoint from the physical-development
reviewer set. The final reviewer must repeat the exact-head comparison and
must not be the workflow initiator or candidate author. Configure this
environment-scoped variable:

| Variable | Required value |
| --- | --- |
| `ANDROID_RISH_PHYSICAL_FINAL_REVIEW_PROTECTED` | `reviewed-v1` |

## Controller host and device

Use a dedicated, ephemeral controller registered with all of these runner
labels:

- `self-hosted`
- `linux`
- `x64`
- `termux-rish-controller`

Never use a personal workstation or a general-purpose self-hosted runner.
GitHub documents that self-hosted runners can be persistently compromised by
untrusted repository code; this design avoids executing the candidate tree on
the controller, but the controller and physical test device must still be
network-isolated, disposable, and free of personal or production data.

The trusted controller configuration root defaults to
`/etc/termux-mcp-edge/physical-devices`. Each environment-selected slot maps
to one private JSON file named `<slot>.json`. The root directory and every
controller private working/evidence directory must be canonical, mode `0700`,
and owned by the dedicated runner service account. The slot file, SSH
identity, and known-hosts file must be canonical regular files, mode `0600`,
and owned by that same account.

The configuration root and quarantine inventory must be on durable,
non-memory-backed storage that survives runner-process and controller-host
restart. Ephemeral compute may mount that inventory, but deleting or replacing
the controller does not clear a device quarantine. A replacement controller
must treat a missing local marker as unknown state and must not enroll or reuse
the device until an administrator has reset or reprovisioned it and reconciled
the external slot inventory.

The slot JSON is closed and has exactly these fields:

| Field | Contract |
| --- | --- |
| `schemaVersion` | Integer `1` |
| `slot` | Exact environment-selected slot |
| `adbSerial` | Fixed controller inventory value; never workflow input |
| `sshUser` | Fixed Termux SSH account |
| `sshIdentityFile` | Absolute canonical controller file, mode `0600` |
| `sshKnownHostsFile` | Absolute canonical controller file, mode `0600` |
| `sshDevicePort` | Fixed integer from 1024 through 65535 |
| `privateEvidenceRoot` | Absolute canonical runner-local mode-`0700` directory owned by the dedicated service account |
| `rishDexPath` | Fixed private canonical Termux-home path |
| `rishDexSha256` | Lowercase SHA-256 of the pinned DEX |
| `deviceProfileCommitment` | Lowercase SHA-256 administrator commitment |
| `termuxVersion` | Pinned official Termux version |
| `termuxSignerSha256` | Lowercase SHA-256 signer commitment |
| `shizukuVersion` | Pinned Shizuku version |
| `shizukuSignerSha256` | Lowercase SHA-256 signer commitment |

A shape-only example, containing no usable identifiers or credentials, is:

```json
{
  "schemaVersion": 1,
  "slot": "lab_device_1",
  "adbSerial": "ADMINISTRATOR_FIXED_SERIAL",
  "sshUser": "termux-gate",
  "sshIdentityFile": "/etc/termux-mcp-edge/keys/lab-device-1",
  "sshKnownHostsFile": "/etc/termux-mcp-edge/keys/lab-device-1.known-hosts",
  "sshDevicePort": 8022,
  "privateEvidenceRoot": "/var/lib/termux-mcp-edge/private-rish-evidence",
  "rishDexPath": "/data/data/com.termux/files/home/.private-rish/rish_shizuku.dex",
  "rishDexSha256": "REPLACE_WITH_64_LOWERCASE_HEX",
  "deviceProfileCommitment": "REPLACE_WITH_64_LOWERCASE_HEX",
  "termuxVersion": "REPLACE_WITH_PINNED_VERSION",
  "termuxSignerSha256": "REPLACE_WITH_64_LOWERCASE_HEX",
  "shizukuVersion": "REPLACE_WITH_PINNED_VERSION",
  "shizukuSignerSha256": "REPLACE_WITH_64_LOWERCASE_HEX"
}
```

The configured device must be:

- a non-personal physical AArch64 Android device on API 30 through 36;
- enrolled only for this development gate;
- running official Termux and Shizuku installations whose versions and
  signer commitments match the controller inventory;
- using Shizuku started through ADB and a private pinned `rish` DEX;
- storing that DEX as one canonical, single-link, Termux-UID-owned mode-`0400`
  regular file below a mode-`0700` private parent;
- reachable only through the controller's fixed transport;
- in the controller-verified offline posture before and after the probe:
  airplane mode on; Wi-Fi, mobile data, and Bluetooth settings off; and no
  default route reported by the fixed `/system/bin/ip route show default`
  query;
- returning Android shell UID 2000 through trusted direct `rish` probes both
  before and after the candidate process executes;
- reprovisioned or reset after every candidate execution, then manually
  inspected before any later use.

The controller quarantines the selected slot after every candidate execution,
including a successful run. Reuse is prohibited until an administrator
performs the out-of-band reset or reprovision, verifies the offline posture
and cleanup, and manually clears the quarantine marker. If cleanup cannot be
confirmed, retire the device and controller slot rather than clearing the
marker.

The controller generates a fresh 32-byte challenge for every run. Only its
SHA-256 digest is committed to evidence. Raw device output, device
identifiers, paths, package inventories, settings, keys, and the private
controller report are never uploaded.

The controller retains the exact private raw report as a unique mode-`0600`
file below `privateEvidenceRoot`; only its SHA-256 commitment enters public
evidence. Limit that directory to the dedicated service account, exclude it
from ordinary backups and log collection, review it only through an approved
out-of-band process, and destroy it under the operator's evidence-retention
policy after the final review or failed-run investigation. A retention or
cleanup failure leaves the device slot quarantined.

## Dispatch and review

From the Actions page, select `Android Rish Physical Identity`, choose
`main`, and provide:

- `expected_commit`: the exact candidate PR head SHA;
- `pull_request_number`: the matching open same-repository PR number.

Before each protected approval, compare the displayed candidate SHA, PR, and
companion run identities with the intended review. Reject the run if the
candidate PR is draft, has moved, closed, changed repositories, changed its
base from `main`, lacks two distinct current trusted exact-head approvals, or
if any companion run is missing, retried, cancelled, or no longer successful.
Do not approve a run you initiated or a candidate you authored.

The repeated GitHub reads are latest-observed checks, not an atomic lock on PR
or review state. The final job therefore requeries the PR, current approvals,
and all three companion runs once after approval and again immediately before
upload. A run observed to have moved fails closed; no workflow can prevent a
repository administrator from changing state in the interval after the last
read.

The public candidate and evidence artifacts are integrity records, not secret
containers. They must contain only their closed inventories. A successful
run is development evidence for the S2.5 identity probe and cannot be cited
as release evidence or as authorization for broader Android control.
It also cannot establish persistence isolation between processes sharing the
Termux UID; that remains outside this gate's claim boundary.
