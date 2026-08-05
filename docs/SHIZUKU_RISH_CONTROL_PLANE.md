# Shizuku/rish device-control plane

Status: implementation and release contract. The repository implements only the isolated, default-disabled `android-rish` backend foundation and the no-argument `android_rish_status` identity probe described below. Every typed control family and every rish-backed mutation remains a blocked roadmap item.

This document defines the maximum production target for local Android control when Termux MCP Edge uses an adb-started Shizuku/rish backend. The target identity is the Android shell user, UID `2000`. Root, Sui, device-owner enrollment, silent accessibility enablement, and a caller-programmable shell are outside this target.

“Complete control” in this project means the complete, explicitly documented set of operations that the exact device grants to shell UID `2000`. It does not mean omnipotent control. Android service permission checks, SELinux, scoped storage, user-consent surfaces, device policy, secure windows, hardware trust boundaries, OS-version changes, and OEM changes remain authoritative.

The source boundary is explicit:

- AOSP defines `AID_SHELL` as UID `2000`, distinct from `AID_SYSTEM` (`1000`) and root (`0`) in the [Android filesystem identity registry](https://android.googlesource.com/platform/system/core/+/master/libcutils/include/private/android_filesystem_config.h).
- Shizuku documents that a non-root UserService started through adb runs as shell UID `2000`, while a rooted backend runs as UID `0`; it also documents binder lifecycle and UserService limitations in the [official Shizuku API guide](https://github.com/RikkaApps/Shizuku-API/blob/master/README.md).
- `rish` delegates to the high-privilege daemon and normally passes arguments to a remote shell, as described by the [official rish README](https://github.com/RikkaApps/Shizuku-API/blob/master/rish/README.md). This project therefore treats rish as a privileged transport, not as an MCP tool.
- The [Shizuku project](https://github.com/RikkaApps/Shizuku) warns that adb permissions are limited and vary across Android versions. AOSP’s current [Shell package manifest](https://android.googlesource.com/platform/frameworks/base/+/master/packages/Shell/AndroidManifest.xml) is useful evidence, but it is not a promise that an OEM build grants or honors every listed permission.

Normative words such as **must**, **must not**, **required**, and **blocked** are release requirements.

## Non-negotiable properties

The control plane must preserve all of these properties:

1. Root is neither required nor accepted. A backend reporting effective UID `0`, `1000`, or anything other than `2000` is unavailable.
2. No MCP request can supply a command, shell fragment, executable, service-manager name, raw argv vector, environment, stdin, working directory, timeout, or output limit.
3. Every public operation has a closed typed schema and a server-owned mapping to one reviewed backend operation.
4. Compile-time inclusion, runtime enablement, family enablement, authenticated transport, backend state, target policy, preview, request authorization, durable audit readiness, concurrency admission, and fresh state preconditions remain independent gates.
5. Backend or device support is measured. Command presence, an Android API level, a Shizuku version, or an AOSP permission declaration is never sufficient proof.
6. Read, preview, and mutation authority are separate. No read or preview result is itself a mutation credential.
7. Every mutation defaults to preview and requires `dry_run:false`, a single-use grant v2, a durable pre-intent record, a fresh backend attestation, and a verified terminal outcome.
8. Root-only, device-owner-only, profile-owner-only, accessibility-only, user-consent-only, carrier-only, system-app-only, and OEM-private operations are reported as such. They are not emulated, bypassed, or silently downgraded.
9. Existing MCP authentication, Host/Origin validation, resource bounds, protocol separation, safe-root authority, and legacy grant behavior cannot be weakened to add Android control.
10. High-impact tools remain absent from discovery and fail closed on direct invocation until their exact family is production-qualified.

## Runtime state model

The effective backend has exactly one state. State transitions are monotonic only within an individual request; any failed check can immediately return the process to a less-authoritative state.

| State | Stable name | Meaning | Public authority |
|---|---|---|---|
| S0 | `not_compiled` | The rish backend feature is absent. A runtime request to enable it is a startup error. | None; only existing non-rish tools remain. |
| S1 | `compiled_disabled` | The feature exists, but the explicit runtime gate is false or absent. No rish asset is opened and no backend process is started. | None. |
| S2 | `enabled_unavailable` | The runtime gate is true, but configuration, asset attestation, rish connectivity, effective identity, API support, or a health probe failed. | Bounded backend status only; no family tools. |
| S2.5 | `verified_shell_uid` | The implemented foundation has pinned the configured DEX and one live fixed probe returned exactly UID `2000`. It has not yet established the extended device/build/backend epoch required for S3. | The bounded `android_rish_status` result only. No Android action family and no mutation. |
| S3 | `attested_read_only` | The exact backend assets are pinned, a fresh probe proves UID/GID `2000`, and the current backend epoch is healthy. | Qualified status, bounded reads, and deterministic previews for independently enabled families. |
| S4 | `mutation_ready` | S3 holds and grant v2, the durable audit/replay store, emergency stop, protected-target resolution, and at least one independently qualified mutation family are ready. | Only the specifically enabled family/action/target authorized by one fresh grant. S4 is not global mutation authority. |

`runtime_status` may expose the stable state and non-sensitive booleans. It must not expose the rish path, DEX path or digest, Shizuku application ID, backend epoch, boot identifier, build fingerprint, package inventory, raw probe output, OS error, or grant/audit material.

An S3 or S4 process must transition to S2 and rotate its backend epoch when any of the following occurs:

- rish exits unexpectedly, times out, or produces malformed output;
- the Shizuku binder dies or permission is revoked;
- an effective-identity, boot, SDK, asset identity, or asset digest check changes;
- an operation reports an authority failure inconsistent with its qualified probe;
- the audit/replay store becomes unavailable, inconsistent, full, or tampered with;
- emergency stop is activated;
- the server restarts.

Read operations can resume at S3 only after a complete successful re-attestation. Mutation readiness can resume only after recovery reconciliation and a new S4 transition. Old grants never survive an epoch change.

## Backend architecture: typed operations, never an MCP shell

The native Termux server uses rish because it is not itself a conventional Android APK capable of hosting `ShizukuProvider`. The long-term backend interface is an internal typed adapter:

```text
MCP typed request
  -> closed schema and policy
  -> typed Android operation enum
  -> fixed operation registry
  -> fixed app_process runtime and pinned rish DEX
  -> rish "exec"
  -> fixed absolute Android program and fixed subcommand
```

The registry must be compile-time/static data. Variable fields are parsed into bounded types and passed as individual argv entries only where the reviewed operation permits them. They are never interpolated into a command string. The registry must not expose `/system/bin/sh`, `su`, `app_process`, `dalvikvm`, an interpreter, a multi-call escape, raw `service call`, an unrestricted `cmd`, or an operator-configurable executable.

It is acceptable for the pinned rish loader to use its own implementation machinery. It is not acceptable for an MCP caller to select that machinery. Every backend invocation must:

- use the explicit absolute Android `app_process64` runtime and a pinned DEX bundle installed in the Termux application’s private storage;
- reject missing, relative, symlinked, non-regular, unexpectedly writable, or digest-mismatched assets;
- pin parent directories and exact asset identities, and verify configured SHA-256 digests;
- require the Android 14+ rish DEX to be non-writable, consistent with the [rish launcher’s documented Android 14 constraint](https://github.com/RikkaApps/Shizuku-API/blob/master/rish/README.md);
- clear inherited environment and set only the minimum fixed rish variables plus a closed Android ART runtime allowlist copied from the server process (`BOOTCLASSPATH`, ART apex roots, and related keys), with `RISH_PRESERVE_ENV=0`;
- use null stdin, bounded independent stdout/stderr, a hard deadline, a dedicated non-queueing semaphore, process-group cleanup, and authoritative child reaping;
- accept only exact UTF-8 output for that operation’s versioned parser and suppress raw output on every failure;
- remeasure the backend before a mutation grant is consumed.

The same-UID residual boundary must be documented: another untrusted Termux process under the same application UID can interfere with application-private files and processes. Production mutation posture therefore requires exclusive operator control of the Termux app UID and no untrusted plugins, sessions, or same-UID processes.

## Implemented foundation: exact status probe

The implemented rish foundation adds only backend construction, DEX attestation, and one read-only status tool. It does not add package, settings, intent, process, input, capture, storage, network, or other control operations.

### Gates

- Compile-time feature: `android-rish`.
- Runtime gate: `MCP__ANDROID__RISH_ENABLED=true`.
- A runtime true value without the compile feature aborts startup.
- Enabling configuration must include `MCP__ANDROID__RISH_DEX_PATH`, `MCP__ANDROID__RISH_DEX_SHA256`, and a valid static bearer token. The DEX path and digest must also be configured together when the runtime gate is false. Invalid, empty, ambiguous, or partial configuration aborts startup before an MCP router can serve.
- The DEX path must be canonical and absolute, outside every MCP filesystem safe root, inside an owner-only directory, and identify one current-UID-owned, single-link, exact-mode `0400` regular file between 1 byte and 16 MiB. The configured digest is exactly 64 lowercase hexadecimal characters.
- The implementation bypasses the mutable launcher script. It invokes the fixed Android system runtime `/system/bin/app_process64`, the fixed loader class `rikka.shizuku.shell.ShizukuShellLoader`, the fixed Termux application identity `com.termux`, and a private execution snapshot of the pinned DEX through its inherited `/proc/self/fd` descriptor. When the kernel allows ART to reopen a sealed memfd path, that snapshot is sealed; otherwise the server falls back to a private `O_TMPFILE` or exclusive named read-only copy that still isolates the operator source.
- Root/Sui mode is rejected even if it would provide more authority.

### Fixed probe

The adapter invokes exactly one server-owned command through the rish loader: `exec /system/bin/id -u`. An MCP caller cannot change the executable, command, arguments, environment, stdin, working directory, timeout, or output limits.

Each invocation has a five-second total deadline, 1 KiB stdout ceiling, 4 KiB stderr ceiling, null stdin, a cleared environment containing only fixed rish variables, process-group cleanup, and one non-queueing probe permit. Success requires stdout to be the exact bytes `2000\n` and stderr to be empty. Root (`0`), system (`1000`), every other UID, extra whitespace, extra lines, missing newline, warnings, invalid UTF-8, timeout, overflow, abnormal exit, and process-supervision failure all fail closed. The permit is acquired before blocking validation and moves into the independently owned process supervisor; request cancellation terminates the process group promptly, and the permit is not released until direct-child reaping is authoritatively complete.

Before every probe the server rechecks the pinned descriptor and pathname identity, parent identity, ownership, mode, link count, size, timestamps, and configured SHA-256. It copies exactly the bounded, digest-matched bytes into a `CLOEXEC` execution snapshot and verifies that a fresh open of `/proc/self/fd/N` returns those bytes—the same open path ART uses for the classpath. Prefer a sealed memfd when that path reopen works. On Android kernels that create sealed memfds but deny memfd path reopen, fall back to a private `O_TMPFILE` or exclusive named file at mode `0400`; that fallback still isolates the operator source from pathname replacement, but it cannot claim sealed same-UID immutability. The source is revalidated after the copy and before launch. Only the intended rish child clears `CLOEXEC` on the snapshot. Path replacement, a race during copying, permission change, or digest drift blocks execution. Raw process output, the DEX path, digest, and file descriptor are never returned or logged.

The extended UID/GID/group/SELinux/SDK/boot attestation and private backend epoch described by S3 are still required before the first typed read family. They are intentionally not inferred from the public UID-only foundation.

### S3 private attestation (development)

The backend implements a private `attest_read_only` multi-probe suite that, under one concurrency lane and one DEX execution snapshot, runs only these fixed loader commands (never caller-selected):

1. `exec /system/bin/id -u` — exact `2000`
2. `exec /system/bin/id -g` — exact `2000`
3. `exec /system/bin/id -G` — space-separated GIDs including `2000`
4. `exec /system/bin/cat /proc/self/attr/current` — shell SELinux domain prefix `u:r:shell:` (preferred over `id -Z`, which can emit the context on stderr under rish)
5. `exec /system/bin/getprop ro.build.version.sdk` — integer in the supported API band
6. `exec /system/bin/getprop ro.build.fingerprint` — bounded graphic fingerprint (hashed privately)
7. `exec /system/bin/cat /proc/sys/kernel/random/boot_id` — canonical UUID (hashed privately)

On full success it mints a private restart-sensitive backend epoch binding DEX digest, fingerprint hash, boot-id hash, SELinux context hash, SDK, GID, and groups. Epoch material, fingerprints, boot ids, SELinux strings, and group inventories never appear in MCP responses, logs, or `Debug` output. Failure invalidates any prior live epoch.

### Public tool

`android_rish_status` accepts no arguments and runs the S3 multi-probe suite. Its complete success payload is:

```json
{
  "available": true,
  "backend": "shizuku_rish",
  "principal": "android_shell",
  "uid": 2000,
  "state": "attested_read_only",
  "rootAccepted": false,
  "arbitraryShell": false,
  "mutationReady": false
}
```

Sensitive S3 material (backend epoch, fingerprints, boot id, SELinux context, group inventory, digests) is never returned.

### First typed read: `android_system_features`

Separately default-disabled via `MCP__ANDROID__SYSTEM_FEATURES_ENABLED=true` (requires rish enabled). Accepts no arguments. After a live S3 epoch exists (establishes one if needed), runs only fixed `exec /system/bin/cmd package has-feature <allowlisted-name>` probes for ten compile-time feature names. Returns only those booleans plus:

```json
{
  "available": true,
  "backend": "shizuku_rish",
  "state": "attested_read_only",
  "controlAuthorityProven": false,
  "arbitraryShell": false,
  "mutationReady": false,
  "features": {
    "wifi": true,
    "bluetooth": true,
    "cameraAny": true,
    "location": true,
    "microphone": true,
    "nfc": true,
    "fingerprint": true,
    "accelerometer": true,
    "touchscreen": true,
    "webview": true
  }
}
```

Never returns raw OEM lines, feature versions, or non-allowlisted names.

When configured but unavailable, each tool returns an MCP tool error containing only its stable unavailable token (`android_rish_status_unavailable` or `android_system_features_unavailable`) plus one stable low-cardinality reason code. It never reflects raw stderr or configuration details. When the runtime gate is false, the tool is absent from discovery and direct invocation fails closed.

The feature has dedicated host Clippy, tests, and release-build checks, and raw `--all-features` validation. It is intentionally excluded from the governed `full-suite` alias and existing Android release-artifact inventory: the official Termux container reports no Android framework and cannot qualify a real Shizuku Binder lifecycle. Physical-device evidence remains mandatory before this feature can join release qualification or support any broader production claim.

## Capability classification

The matrix uses these types:

- **R** — bounded read/status candidate.
- **P** — deterministic preview candidate that returns a typed proposed delta and state precondition without mutating.
- **M** — mutation candidate, eligible only at S4 with grant v2 and all family prerequisites.
- **C** — requires a separate Android companion component and an explicit platform/user consent or enrollment flow.
- **D** — device-, release-, user-, or OEM-dependent; availability must be probed and certified on the exact build.
- **X** — outside the non-root shell-UID target.

An operation marked M is not promised to work on every device. It means a typed implementation can be considered after a direct authority probe and physical qualification. A denied service call, SELinux denial, missing command, unsupported option, user restriction, locked-user state, or OEM divergence makes that operation unavailable without fallback.

## Typed capability matrix

| Family | R / P surface | M surface at the shell-UID ceiling | Required limits and hard boundary |
|---|---|---|---|
| Packages | **R:** allowlisted package identity, version, enabled/suspended state, installer identity, requested permissions, and bounded install-session status. **P:** exact install/update/uninstall/enable/disable/clear delta. | **M/D:** create/write/commit a verified APK install session; uninstall for one user; enable/disable/suspend/unsuspend; clear app data. AOSP’s reviewed command surface is visible in [`PackageManagerShellCommand`](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/services/core/java/com/android/server/pm/PackageManagerShellCommand.java). | APK must originate from a pinned safe root and bind digest, package name, version, signer digest, user, install flags, and expected installed state. No APEX, staged-installer bypass, verification bypass, downgrade, test-only override, arbitrary flags, protected packages, or silent claim of OEM support. |
| Permissions and app ops | **R:** declared runtime permissions, current grant flags, and allowlisted app-op modes. **P:** one exact grant/revoke or mode transition. | **M/D:** grant/revoke only runtime/changeable permissions; set/reset one allowlisted app op for one unprotected package/user. | No signature, privileged, role, policy, restricted-permission allowlist, cross-user, sensor-privacy, special-access, or unknown operation bypass. Runtime permission behavior follows the [Android permission model](https://developer.android.com/guide/topics/permissions/overview). |
| Settings | **R:** named allowlisted `system`, `secure`, or `global` key with typed/redacted value. **P:** exact old/new typed value. | **M/D:** put/delete only an independently reviewed key/value pair proven writable by shell on that device. | No caller-selected namespace/key, `device_config`, ADB/debugging state, Shizuku state, accessibility enablement, input-method selection, credential/lock settings, identifiers, location bypass, verifier policy, emergency controls, or protected-target policy. AOSP’s shell permission is evidence, not authority; device-owner setting APIs remain separately constrained by [`DevicePolicyManager`](https://developer.android.com/reference/android/app/admin/DevicePolicyManager). |
| Intents and broadcasts | **R:** resolve an explicit allowlisted component/action for one user. **P:** normalized explicit destination, typed extras, flags, and expected visibility. | **M/D:** start one explicit activity; send one allowlisted explicit broadcast; start/stop one allowlisted application component where the platform permits it. | No implicit broadcast fanout, raw URI, arbitrary MIME type, selector, clip data, file-descriptor grant, parcelable/blob extras, background-start bypass, protected action, shell option, or cross-user default. Android background launch rules remain authoritative; the candidate AOSP surface is documented in [`ActivityManagerShellCommand`](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/services/core/java/com/android/server/am/ActivityManagerShellCommand.java). |
| Processes, services, and jobs | **R:** bounded state for an allowlisted package/component/job ID. **P:** force-stop, kill-background, job run/cancel, or component lifecycle delta. | **M/D:** force-stop or kill background processes for one unprotected app; run/cancel one known job; start/stop one explicit app service when platform rules permit. | No global process inventory, raw PID control, arbitrary signal, persistent/system process, native daemon, init service, binder service-manager transaction, debugger attach, or server/Shizuku/Termux termination. Android’s [app power management](https://source.android.com/docs/core/power/app_mgmt) remains in force. |
| Notifications | **R/D:** bounded package/channel/policy state without notification content. **P:** post/cancel a server-owned test notification, package notification policy, or shade action. | **M/D:** only fixed server-owned test notifications, cancellation of an explicitly allowlisted app’s notifications, and reviewed notification-policy changes. | No reading notification text or actions, listener enablement, reply/pending-intent execution, DND bypass, safety/emergency notification suppression, or protected-package cancellation. Reading third-party notifications requires a separately user-enabled notification-listener companion, not rish inference. |
| UI and input | **R/D:** display size/orientation, interactive/locked state, focused-package identity, and a separately protected/redacted hierarchy snapshot if physically qualified. **P:** one bounded key, tap, swipe, text, rotation, or navigation action with display and focus preconditions. | **M/D:** inject one allowlisted key; tap/swipe within current measured bounds; bounded UTF-8 text; rotate; wake; expand/collapse a reviewed system surface. | No raw input script, indefinite gesture, hidden-display target, lock-credential entry, biometric bypass, secure-key action, overlay bypass, arbitrary hierarchy dump, or operation while focus/lock/display epoch differs. Shell input is not an accessibility service. |
| Accessibility | **R/C:** companion reports whether the user has explicitly enabled its service and which declared capabilities are active. **P/C:** user-visible proposed accessibility action. | **M/C:** only through a separately installed, explicitly user-enabled service whose declared purpose and behavior satisfy platform policy. | The shell plane must not enable, select, or impersonate an accessibility service. Android states that [`AccessibilityService`](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService) is for assisting users with disabilities. No hidden enablement, settings write, consent automation, credential capture, or secure-window bypass. |
| Capture | **R/D:** display/capture availability and bounds, never pixels. **P:** capture type, display, dimensions, duration, byte ceiling, and destination classification. | **M/D:** one bounded screenshot or short screen recording into a new private/safe-root file; optional **C** MediaProjection path with per-session user consent. | No streaming by default, audio capture, indefinite recording, caller path, overwrite, secure-surface bypass, or capture while locked unless explicitly certified. `FLAG_SECURE` is authoritative and [prevents screenshots/non-secure display](https://developer.android.com/security/fraud-prevention/activities). A companion MediaProjection must obtain [user consent for every session](https://developer.android.com/media/grow/media-projection). |
| Clipboard | **R/C/D:** only presence/type or companion-owned clip state by default; content read requires a foreground/user-mediated companion posture. **P/C:** bounded MIME/text replacement or clear. | **M/C/D:** set/clear a bounded clip only where a supported API and user-visible companion context are proven. | No universal background clipboard promise. Android 10+ limits reads to the focused app or default IME, as documented in [Android 10 privacy changes](https://developer.android.com/about/versions/10/privacy/changes), and Android 12+ provides access notifications. No password/OTP collection or accessibility workaround. |
| Storage, content, and media | **R:** existing safe-root tools; allowlisted provider/collection metadata with column and row limits. **P:** exact provider row or media/file delta. | **M/D:** fixed-schema query/insert/update/delete for an allowlisted provider; bounded MediaStore changes; user-selected document operations through **C**. | No raw authority/URI, SQL selection, projection, sort order, bundle, file descriptor, arbitrary provider, `/data/data` access, Android credential stores, shared-storage sweep, or safe-root bypass. SAF access remains [user-selected](https://developer.android.com/training/data-storage/shared/documents-files); MediaStore/scoped-storage rules remain authoritative under the [shared-media contract](https://developer.android.com/training/data-storage/shared/media). |
| Network, radios, and VPN | **R/D:** bounded connectivity, Wi-Fi, Bluetooth, airplane, tethering, DNS/proxy, and VPN status with identifiers redacted. **P:** exact proposed radio/policy transition. | **M/D:** only device-qualified Wi-Fi/Bluetooth/radio or network-policy actions with fixed options. VPN configuration/control is **C** unless an already-consented app exposes a typed local interface. | No packet capture, credential/SSID disclosure, arbitrary proxy/DNS, firewall/routing shell, cellular/carrier mutation, eSIM, SIM, IMS, hotspot credential extraction, or consent bypass. Modern apps cannot generally toggle [Wi-Fi](https://developer.android.com/reference/android/net/wifi/WifiManager) or [Bluetooth](https://developer.android.com/reference/android/bluetooth/BluetoothAdapter); always-on VPN policy belongs to device/profile owners, while ordinary [`VpnService`](https://developer.android.com/reference/android/net/VpnService) requires platform consent. |
| Power and Doze | **R:** battery, charging, saver, interactive, idle, standby-bucket, and allowlist status. **P:** wake/sleep, saver, idle/standby, temporary allowlist, or reboot proposal. | **M/D:** wake/sleep, reviewed saver/idle/standby changes, bounded temporary allowlisting, and separately gated reboot where the exact device grants shell authority. | No shutdown/reboot loop, factory reset, thermal bypass, charging-control hardware writes, permanent Doze exemption, battery-stat fabrication in production, or protected package restriction. Reboot is its own highest-impact action and requires a physical recovery plan. |
| Users and profiles | **R/D:** bounded current-user and unlocked/running state. **P:** start/stop/switch an existing allowlisted secondary user. | **M/D:** only explicitly qualified existing-user lifecycle actions. User creation/removal and restriction changes remain disabled until separately reviewed and may be **X/C** under device policy. | No default cross-user action, owner/profile enrollment, work-profile boundary bypass, credential handling, guest deletion, user removal, or system-user mutation. Device/profile-owner authority follows Android’s [device-management model](https://developer.android.com/work/dpc/device-management), not shell inference. |
| Diagnostics | **R/D:** typed, redacted subsets of build, battery, storage, memory, service health, package state, and bounded logs owned by this service. **P:** diagnostic collection plan and byte/time budget. | No general diagnostic mutation. A separately consented bounded bugreport export may be considered **M/D** because it writes sensitive evidence. | No raw `dumpsys`, `logcat`, `bugreport`, `/proc`, tombstone, account, telephony, identifier, binder, or filesystem inventory over MCP. Every parser is versioned and suppresses unknown fields and raw output. |

The matrix is a ceiling and backlog, not a single release. Each row can produce multiple independent capability families when risk or rollback differs.

## Boundaries that cannot be erased by Shizuku/rish

The following must remain explicit in discovery, runtime status, operator documentation, and release claims.

| Boundary | Required classification |
|---|---|
| Root/kernel | Mounting protected filesystems, kernel/module control, raw block/hardware device access, SELinux policy changes, reading arbitrary app-private data, modifying verified system partitions, and bypassing hardware-backed security are **X**. |
| System UID | UID `2000` is not UID `1000`. Binder services may make caller-UID, permission, user, SELinux, build-type, and device-policy checks independently. There is no “act as system” fallback. |
| Device/profile owner | Kiosk/lock-task policy, factory reset policy, credential policy, silent enterprise enrollment, always-on VPN policy, broad user restrictions, and many modern Wi-Fi/Bluetooth controls require owner or role authority. They are **C/X**, not shell promises. |
| Accessibility | A user must install and explicitly enable the companion accessibility service. The rish backend cannot toggle it through settings or automate the consent UI. |
| Screen capture | Secure windows/protected buffers remain uncapturable. MediaProjection consent is per session. A blank/denied secure region is a valid platform outcome, not a backend fault to bypass. |
| Clipboard | Background access is not reliable or universally permitted. Foreground/default-IME rules and user notifications remain intact. |
| Storage | Scoped storage, provider permissions, SAF grants, media permissions, file ownership, and SELinux remain authoritative. Shell authority is not a license to expose the entire shared or private filesystem. |
| Network/carrier | Carrier privileges, telephony roles, eSIM/SIM provisioning, VPN consent, radio HALs, and OEM services remain outside generic shell authority. |
| Multi-user/work profile | User unlock state, cross-user permissions, profile policy, quiet mode, and owner restrictions are rechecked per request. |
| OEM drift | AOSP source describes a reference implementation. An OEM can remove commands, change options/output, add service checks, or deny an operation. Unsupported devices fail closed. |
| Reboot lifecycle | On non-root devices Shizuku normally must be restarted after boot; Android 11+ can use wireless debugging, while older devices require adb from a computer, as documented by the [Shizuku API guide](https://github.com/RikkaApps/Shizuku-API/blob/master/README.md). No control plane may claim unattended boot persistence unless physical evidence proves the operator’s separate startup procedure. |

## Grant v2: required before any device mutation

The existing narrow grant families are not sufficient for general Android state. Grant v2 must land as a focused authorization foundation before the first S4 family.

Grant v2 remains offline-issued by the exact local binary. There is no MCP, HTTP, Android intent, or backend operation that issues or renews a grant. Raw grant material is accepted only in one bounded singleton `MCP-Capability-Grant` header and is never accepted in JSON, query strings, URLs, logs, responses, previews, or audit labels.

Each opaque signed grant must bind:

- grant format/version and a globally unique capability-family code;
- exact typed action and canonical schema version;
- authenticated principal;
- protocol era and canonical request context, including the legacy session when one exists;
- user/profile identifier;
- canonical typed argument digest;
- target package/component/resource classification;
- expected current-state/precondition digest returned by a fresh preview;
- backend epoch and exact device/build measurement digest;
- `dry_run:false`;
- protected-target policy generation and emergency-stop generation;
- issue time, expiry no more than 60 seconds later, and a random 256-bit one-use identifier.

The signed binding may include private values, but the serialized envelope, public errors, and MCP status must not reveal them. Keys are domain-separated per family, loaded from fixed-mode private files, locked from accidental debug formatting, and rotatable without accepting an old key indefinitely.

Replay state must be durable and process-independent for device mutation. Atomic grant consumption is persisted and synchronized immediately before the first state change. Restart, crash, multi-process execution, or clock rollback cannot make a consumed grant reusable. An unavailable, full, poisoned, rolled-back, or inconsistent replay store blocks S4.

Modern stateless support requires its own explicit transport change. It must not reinterpret a legacy session grant, invent a session, or accept v1. Legacy and modern evaluators remain distinct even if they share the v2 envelope.

## Duplicate-key and ambiguity rejection

High-impact authorization cannot use last-value-wins parsing.

- JSON object keys must be unique at every nesting depth, including the JSON-RPC envelope, `_meta`, `params`, tool arguments, typed extras, and embedded policy objects.
- Singleton HTTP headers—including authorization, protocol/routing, session, replay, and capability-grant headers—must have exactly one canonical occurrence. Comma joining must not turn duplicated singleton fields into one value.
- Closed schemas reject unknown fields, aliases, case variants, Unicode confusables, duplicate set/list entries, and values with multiple canonical encodings.
- Package names, component names, permission names, app-op names, setting keys, intent actions, authorities, URIs, user IDs, job IDs, display IDs, and enum values use dedicated parsers. They are not generic strings after validation.
- Backend output parsers require one exact versioned shape. Duplicate labels, extra lines, unknown fields, truncated records, invalid UTF-8, or mixed stdout/stderr are failures, never partial success.
- Preview and live execution use the same canonical typed representation. A second parser or string reserialization cannot change the authorized operation.

The duplicate-key preflight must happen before concurrency admission, audit correlation allocation, preview calculation, grant evaluation, backend access, or state inspection.

## Durable fail-closed audit

In-memory counters remain useful for public low-cardinality status, but they are not sufficient for S4. Device mutation requires a local, operator-private, durable audit journal.

The journal must:

- live outside MCP-readable and MCP-writable safe roots in a descriptor-pinned mode-`0700` directory with mode-`0600` files;
- use bounded versioned records, a monotonic sequence, cryptographic chaining/MAC, key rotation records, and durable checkpoints;
- record only stable operation metadata: local correlation ID, family/action, schema version, user classification, protected-target class, backend/policy/emergency generations, decision, mode, stable reason, and phase;
- never record raw grants, keys, tokens, command/argv text, stdout/stderr, paths, URIs, intent extras, clipboard/screen content, notification content, package inventories, identifiers, settings values, or provider rows;
- write and synchronize a `prepared` record before grant consumption;
- durably consume the grant, then execute;
- write exactly one terminal `verified`, `recovered`, `denied`, or `outcome_unknown` record after the worker owns completion;
- block the affected family after a crash leaves a prepared/consumed operation without a terminal record until a local reconciliation command verifies actual state and appends a recovery record;
- rotate only through a durable checkpoint; never silently overwrite or discard unexported records;
- make disk-full, fsync, chain, key, ownership, permission, identity, rollback, or corruption failure block S4 before mutation.

The MCP surface exposes only aggregate counters and whether the durable journal is healthy. Journal export, verification, repair, acknowledgement, and retention are local CLI operations with the server stopped. They are not MCP tools.

## Mutation transaction

Every live operation follows the same fail-closed order:

1. Authenticate and validate transport, protocol, body, duplicate keys, closed schema, and response ceiling.
2. Require the compile, runtime, family, and action gates.
3. Resolve current user, target, protected-target generation, and emergency-stop generation.
4. Admit a family-specific non-queueing permit.
5. Perform a fresh S3 backend attestation.
6. Read and classify the current state through the typed adapter.
7. Recompute the preview and require its precondition digest to match grant v2.
8. Preflight rollback/recovery where the operation supports it. Irreversible operations must declare that fact in preview and require their own action class.
9. Append and fsync the durable `prepared` audit record.
10. Atomically validate and durably consume the grant immediately before the first state change.
11. Recheck emergency stop, backend epoch, focus/display/user state where relevant, and protected-target policy.
12. Execute one fixed typed backend operation.
13. Read fresh state and verify the exact authorized postcondition.
14. Attempt bounded recovery only when a reviewed recovery action is defined and cannot broaden authority.
15. Append and fsync exactly one terminal record.

Cancellation, HTTP timeout, disconnect, or waiter loss after worker ownership does not cancel cleanup, verification, recovery, replay consumption, or terminal audit. A crash can leave an uncertain device state; it must never leave the grant reusable or silently report success.

## Emergency stop

Emergency stop is a local authority above every MCP grant.

- Activation must be possible by a local exact-binary CLI command and by a fixed private sentinel while the MCP listener is unreachable.
- Activation increments a durable emergency generation, rotates the backend epoch, blocks new reads that could expose sensitive device data, rejects all previews/mutations, terminates in-flight backend process groups, and stops mutation-ready service admission.
- Every mutation checks the generation during preflight and again immediately before grant consumption/state change.
- All outstanding grants become invalid after activation.
- The service must remain in S2 until the operator stops it, reconciles every consumed/incomplete operation, verifies the audit chain, restores Shizuku intentionally, clears the sentinel through a local interactive command, rotates capability keys, and restarts.
- MCP cannot clear, acknowledge, weaken, postpone, or override emergency stop.
- Revoking Shizuku permission, stopping Shizuku, or stopping the Termux runit service remains an independent operator kill path.

Emergency activation is best effort for a backend operation already committed inside Android system_server. The production claim is bounded admission, process termination, grant invalidation, and reconciliation—not transactional rollback of Android itself.

## Protected targets

Protected-target policy is deny-first and generation-bound. It combines immutable built-ins, dynamically resolved safety roles, and operator additions.

The immutable/dynamic set includes:

- UID `0`, UID `1000`, UID `2000`, the current Termux application UID, and packages sharing those identities;
- Android framework, System UI, Settings, Shell, package installer/verifier, permission controller, device-policy controller/role holder, current launcher, current input method, current VPN, current accessibility services, emergency/telephony services, and user/profile management components;
- Shizuku, its manager/provider/backend package, the rish asset bundle, Termux, this server/service, the audit/replay store, emergency-stop controls, and all credential/key stores;
- any package/component required to recover connectivity, input, display, authentication, package installation, or the operator’s selected control path;
- operator-specified package, component, permission, app-op, setting, URI authority, user, job, notification, network, and filesystem exclusions.

Protected targets cannot be mutated, force-stopped, uninstalled, disabled, suspended, cleared, permission/app-op changed, notification-suppressed, data-modified, or used as an intent/input target through ordinary grants. An exceptional maintenance workflow, if ever added, is a separate offline local operation with the server stopped, physical confirmation, recovery evidence, and a different capability family. It is not an “override” boolean.

Dynamic target resolution is refreshed before every preview and mutation. Any ambiguity, package replacement, shared UID, role change, signer change, user change, or resolution failure denies the action. Operator additions are append-only while the service runs. Removing protection requires a stopped-service local policy update and generation rotation.

## Rollout families

Development must remain reviewable and reversible. The minimum order is:

1. **R0 — attested backend status:** S0–S3, fixed UID/GID/SDK probe, lifecycle/epoch handling, no Android action.
2. **R1 — S4 safety foundation:** grant v2, durable replay/audit, emergency stop, protected targets, duplicate-key rejection, local reconciliation commands; still no device mutation.
3. **R2 — read-only inventory:** qualified package, permission/app-op, settings, power, connectivity, user, and diagnostics subsets. No broad dumps.
4. **R3 — package lifecycle:** split install/update, uninstall, enable/disable/suspend, and clear-data into independently gated actions.
5. **R4 — permission/app-op and settings:** separate allowlists and action families; no arbitrary permission, op, namespace, or key.
6. **R5 — intents, process/job, and notifications:** explicit components/targets only; process and notification content remain private.
7. **R6 — power, network, and radio controls:** one action per proven service contract; reboot and VPN remain separate.
8. **R7 — storage/content/media:** preserve safe-root confinement and add only fixed provider/collection schemas or user-selected companion grants.
9. **R8 — UI/input:** display/focus/lock preconditions, bounded gestures, protected flows, and dedicated physical evidence.
10. **R9 — capture and companion consent surfaces:** screenshots/recording, clipboard, MediaProjection, notification listener, or accessibility only under their distinct platform/user-consent contracts.

Each rollout item can require several PRs. No PR should introduce more than one independently mutating family or combine a capability change with unrelated dependency, transport, release, or workflow work.

## Physical-device qualification matrix

Emulators, AOSP source review, cross-compilation, and an official Termux container remain necessary but cannot certify Shizuku lifecycle, OEM service policy, UI, radio, user, capture, or storage behavior.

The supported matrix for the first production control-plane release must include:

- every declared supported Android major, initially Android 11 through Android 16 (API 30 through 36);
- at least one AOSP/Pixel-class device on every supported major;
- current physical devices from at least three materially different OEM families, including Samsung and a Xiaomi/HyperOS-class device;
- at least two chipset families;
- a primary user, a secondary user where supported, and a managed/work profile for any cross-user/profile claim;
- locked and unlocked screen, light/dark UI, portrait/landscape, and at least two display densities for UI/input claims;
- scoped internal storage, removable storage where supported, MediaStore, SAF, and provider-denial cases for storage claims;
- Wi-Fi, Bluetooth, cellular-present/cellular-absent, airplane, tethering, and VPN consent states for network claims;
- secure-window, ordinary-window, capture-denied, user-cancelled MediaProjection, and interruption cases for capture claims.

Every capability/action/device tuple must record one of:

- `verified_supported`;
- `verified_denied_by_platform`;
- `verified_unavailable`;
- `not_tested`.

Only `verified_supported` tuples may be enabled. `not_tested` never means compatible.

The automated physical suite must cover:

- fresh Shizuku start, permission grant, permission denial, binder death, restart, backend identity mismatch, and reboot;
- exact app-process/DEX availability plus DEX tamper, replacement, symlink, mode, digest, and Android 14+ writable-DEX rejection;
- disabled compile/runtime/family truth tables and direct-call denial;
- duplicate keys/headers, malformed Unicode, oversized values, unknown fields, and injection-shaped inputs;
- every preview/live precondition mismatch and protected target;
- expired, mismatched, replayed, concurrently replayed, cross-family, cross-user, cross-device, cross-epoch, and post-restart grants;
- audit disk-full, fsync failure, truncation, rollback, chain corruption, key loss, rotation, crash recovery, and incomplete-operation reconciliation;
- emergency stop before admission, before consumption, after consumption, during backend execution, and after Android commit;
- timeout, output overflow, invalid UTF-8, process cancellation, child/grandchild cleanup, and semaphore exhaustion;
- positive postcondition verification, expected platform denial, recovery success/failure, cancellation independence, and no sensitive output in errors/evidence;
- regression of legacy and stateless MCP read-only behavior, current filesystem grants, fixed command diagnostics, Android volume control, release packaging, and native Termux gates.

Evidence must bind the exact commit, source version, Cargo lockfile, Android artifacts, asset digests, device model, OS build fingerprint digest, API level, security patch level, Shizuku version/start mode, rish asset digests, backend UID, family/action/schema version, and sanitized result. Raw identifiers, content, settings, package inventory, notification/clipboard/screen data, command output, tokens, grants, or keys must not enter public CI artifacts.

## Production release blockers

A device-control family cannot be called production-ready until all of these are true:

- its public schema, fixed backend mapping, state machine, limits, stable reasons, audit privacy, protected targets, preview, grant binding, verification, recovery/irreversibility, and operator runbook are documented;
- safe public Rust embeddings cannot enable raw rish or construct backend operations;
- default, MCP-runtime, full-suite, and all-feature host lanes pass under locked dependencies;
- all governed Android artifacts build and exact native official-Termux tests pass;
- the physical matrix has no unclassified supported tuple;
- Security/CodeQL/RustSec and a focused confused-deputy/threat review pass;
- no root, arbitrary shell, raw service-manager, generic command, unrestricted URI/provider, global inventory, or consent bypass is reachable;
- durable audit/replay crash recovery and emergency stop are exercised on the exact candidate;
- the release notes state the exact verified control ceiling and all unavailable/device-dependent boundaries;
- qualification is regenerated after every source, dependency, workflow, artifact, policy, or documentation change that affects the claim.

The project must not advertise “complete device control” without the qualifier “maximum verified shell-UID control on the listed devices and builds.” Unsupported authority is a stable denial, not a hidden backlog item or a reason to fall back to root.

## Primary source map

- [Shizuku overview](https://shizuku.rikka.app/)
- [Shizuku implementation and adb-permission limitations](https://github.com/RikkaApps/Shizuku)
- [Shizuku API, binder lifecycle, and shell-UID UserService](https://github.com/RikkaApps/Shizuku-API/blob/master/README.md)
- [rish execution behavior and environment](https://github.com/RikkaApps/Shizuku-API/blob/master/rish/README.md)
- [AOSP shell package permissions](https://android.googlesource.com/platform/frameworks/base/+/master/packages/Shell/AndroidManifest.xml)
- [AOSP shell UID definition](https://android.googlesource.com/platform/system/core/+/master/libcutils/include/private/android_filesystem_config.h)
- [AOSP package-manager shell implementation](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/services/core/java/com/android/server/pm/PackageManagerShellCommand.java)
- [AOSP activity-manager shell implementation](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/services/core/java/com/android/server/am/ActivityManagerShellCommand.java)
- [Android permission model](https://developer.android.com/guide/topics/permissions/overview)
- [Android enterprise device control](https://developer.android.com/work/dpc/device-management)
- [DevicePolicyManager authority](https://developer.android.com/reference/android/app/admin/DevicePolicyManager)
- [AccessibilityService user/purpose boundary](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService)
- [MediaProjection user-consent and lifecycle boundary](https://developer.android.com/media/grow/media-projection)
- [Secure-window capture boundary](https://developer.android.com/security/fraud-prevention/activities)
- [VpnService consent and owner-policy boundary](https://developer.android.com/reference/android/net/VpnService)
- [Wi-Fi mutation restrictions](https://developer.android.com/reference/android/net/wifi/WifiManager)
- [Bluetooth mutation restrictions](https://developer.android.com/reference/android/bluetooth/BluetoothAdapter)
- [Clipboard privacy restrictions](https://developer.android.com/about/versions/10/privacy/changes)
- [Storage Access Framework](https://developer.android.com/training/data-storage/shared/documents-files)
- [MediaStore and scoped shared-media access](https://developer.android.com/training/data-storage/shared/media)
- [Android app power management](https://source.android.com/docs/core/power/app_mgmt)
