//! Attested, read-only Shizuku/rish availability probe.
//!
//! This module deliberately does not expose a general shell. Production uses
//! one fixed Android runtime, one pinned and digest-bound loader DEX, one fixed
//! loader class, and one fixed command which can only prove that the remote
//! principal is Android's non-root `shell` UID. The configured DEX descriptor
//! remains pinned for the client's lifetime. Each probe copies its revalidated
//! bytes into an execution snapshot and passes only that snapshot through
//! `/proc/self/fd` so pathname replacement of the operator source cannot change
//! the bytes `app_process64` loads. When the kernel allows it, the snapshot is a
//! sealed memfd. Android kernels that refuse to reopen memfd paths for ART fall
//! back to a private `O_TMPFILE` (or exclusive named) read-only copy; that
//! fallback still isolates the operator source, but it cannot claim sealed
//! same-UID immutability.

use std::{
    ffi::OsString,
    fmt,
    fs::{self, File, Metadata},
    os::{
        fd::AsRawFd,
        unix::ffi::OsStrExt,
        unix::fs::{DirBuilderExt, FileExt, MetadataExt},
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use rustix::{
    fs::{
        fcntl_add_seals, fcntl_get_seals, memfd_create, open, MemfdFlags, Mode, OFlags, SealFlags,
    },
    io::{fcntl_getfd, FdFlags},
    process::getuid,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::bounded_process::{
    BoundedChildContext, BoundedProcess, BoundedProcessConfigError, BoundedProcessError,
};

pub(crate) const RISH_APP_PROCESS_PROGRAM: &str = "/system/bin/app_process64";
pub(crate) const RISH_APPLICATION_ID: &str = "com.termux";
pub(crate) const RISH_STATUS_TIMEOUT: Duration = Duration::from_secs(5);
/// Per-probe timeout for allowlisted `cmd package has-feature` reads.
/// Ten sequential probes at the full status timeout would exceed typical
/// MCP client/server budgets and monopolize the single concurrency lane.
pub(crate) const RISH_FEATURE_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// Hard wall-clock budget for the full ten-probe system-features family.
pub(crate) const RISH_FEATURE_FAMILY_BUDGET: Duration = Duration::from_secs(12);
pub(crate) const RISH_STATUS_STDOUT_BYTES: usize = 1024;
pub(crate) const RISH_STATUS_STDERR_BYTES: usize = 4 * 1024;
pub(crate) const RISH_STATUS_CONCURRENCY: usize = 1;

/// Shared concurrency lane: each sequential child holds a clone through
/// cancellation-independent cleanup so the permit cannot be released while
/// a prior rish process group is still being reaped.
type RishLanePermit = Arc<OwnedSemaphorePermit>;

const MIN_RISH_DEX_BYTES: u64 = 1;
const MAX_RISH_DEX_BYTES: u64 = 16 * 1024 * 1024;
const RISH_LOADER_CLASS: &str = "rikka.shizuku.shell.ShizukuShellLoader";
/// Host process environment keys required for `app_process64`/ART to load the
/// pinned rish DEX. Values are copied from the server process at spawn time;
/// MCP callers cannot set or override them. This is not a general environment
/// passthrough: only this closed Android runtime allowlist is considered.
const RISH_ANDROID_RUNTIME_ENV_KEYS: &[&str] = &[
    "ANDROID_ART_ROOT",
    "ANDROID_ASSETS",
    "ANDROID_DATA",
    "ANDROID_I18N_ROOT",
    "ANDROID_ROOT",
    "ANDROID_RUNTIME_ROOT",
    "ANDROID_STORAGE",
    "ANDROID_TZDATA_ROOT",
    "ANDROID__BUILD_VERSION_SDK",
    "BOOTCLASSPATH",
    "DEX2OATBOOTCLASSPATH",
    "SYSTEMSERVERCLASSPATH",
];
/// S2.5 foundation command: prove exact Android shell UID only.
const RISH_FIXED_COMMAND: &str = "exec /system/bin/id -u";
/// S3 extended fixed commands. Each is a complete, non-shell argv payload for
/// the pinned rish loader (`-c <command>`). No caller input is interpolated.
const RISH_FIXED_COMMAND_GID: &str = "exec /system/bin/id -g";
const RISH_FIXED_COMMAND_GROUPS: &str = "exec /system/bin/id -G";
// Prefer /proc attr over `id -Z`: under rish some devices emit context on
// stderr, which would fail-closed the empty-stderr identity contract.
const RISH_FIXED_COMMAND_SELINUX: &str = "exec /system/bin/cat /proc/self/attr/current";
const RISH_FIXED_COMMAND_SDK: &str = "exec /system/bin/getprop ro.build.version.sdk";
const RISH_FIXED_COMMAND_FINGERPRINT: &str = "exec /system/bin/getprop ro.build.fingerprint";
const RISH_FIXED_COMMAND_BOOT_ID: &str = "exec /system/bin/cat /proc/sys/kernel/random/boot_id";
const ANDROID_SHELL_UID: u32 = 2000;
const EXPECTED_STDOUT: &[u8] = b"2000\n";
const MIN_SUPPORTED_SDK: u32 = 30;
const MAX_SUPPORTED_SDK: u32 = 36;
const MAX_SUPPLEMENTARY_GROUPS: usize = 64;
const MAX_SELINUX_CONTEXT_BYTES: usize = 128;
const MAX_FINGERPRINT_BYTES: usize = 256;
const EXPECTED_SHELL_SELINUX_PREFIX: &[u8] = b"u:r:shell:";
/// First typed-read family: one fixed `has-feature` probe per allowlisted name.
/// Full `list features` dumps exceed rish stream bounds on large OEM builds.
const SYSTEM_FEATURE_PROBES: &[&str] = &[
    "exec /system/bin/cmd package has-feature android.hardware.wifi",
    "exec /system/bin/cmd package has-feature android.hardware.bluetooth",
    "exec /system/bin/cmd package has-feature android.hardware.camera.any",
    "exec /system/bin/cmd package has-feature android.hardware.location",
    "exec /system/bin/cmd package has-feature android.hardware.microphone",
    "exec /system/bin/cmd package has-feature android.hardware.nfc",
    "exec /system/bin/cmd package has-feature android.hardware.fingerprint",
    "exec /system/bin/cmd package has-feature android.hardware.sensor.accelerometer",
    "exec /system/bin/cmd package has-feature android.hardware.touchscreen",
    "exec /system/bin/cmd package has-feature android.software.webview",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RishBackendConfigError {
    DexDigestFormatInvalid,
    DexPathInvalid,
    DexParentInvalid,
    DexFileInvalid,
    DexDigestMismatch,
    DexDescriptorInvalid,
    DexIdentityChanged,
    Internal,
}

impl RishBackendConfigError {
    pub(crate) const fn reason_code(self) -> &'static str {
        match self {
            Self::DexDigestFormatInvalid => "rish_dex_digest_format_invalid",
            Self::DexPathInvalid => "rish_dex_path_invalid",
            Self::DexParentInvalid => "rish_dex_parent_invalid",
            Self::DexFileInvalid => "rish_dex_file_invalid",
            Self::DexDigestMismatch => "rish_dex_digest_mismatch",
            Self::DexDescriptorInvalid => "rish_dex_descriptor_invalid",
            Self::DexIdentityChanged => "rish_dex_identity_changed",
            Self::Internal => "rish_backend_config_invalid",
        }
    }
}

impl fmt::Display for RishBackendConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason_code())
    }
}

impl std::error::Error for RishBackendConfigError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RishBackendError {
    DexDigestFormatInvalid,
    DexPathInvalid,
    DexParentInvalid,
    DexFileInvalid,
    DexDigestMismatch,
    DexDescriptorInvalid,
    DexIdentityChanged,
    DexSnapshotFailed,
    ConcurrencyLimitExceeded,
    ProgramUnavailable,
    SpawnFailed,
    WaitFailed,
    TimedOut,
    StdoutLimitExceeded,
    StderrLimitExceeded,
    ProgramFailed,
    IdentityOutputInvalid,
    AttestationOutputInvalid,
    WorkerFailed,
}

impl RishBackendError {
    pub(crate) const fn reason_code(self) -> &'static str {
        match self {
            Self::DexDigestFormatInvalid => "rish_dex_digest_format_invalid",
            Self::DexPathInvalid => "rish_dex_path_invalid",
            Self::DexParentInvalid => "rish_dex_parent_invalid",
            Self::DexFileInvalid => "rish_dex_file_invalid",
            Self::DexDigestMismatch => "rish_dex_digest_mismatch",
            Self::DexDescriptorInvalid => "rish_dex_descriptor_invalid",
            Self::DexIdentityChanged => "rish_dex_identity_changed",
            Self::DexSnapshotFailed => "rish_dex_snapshot_failed",
            Self::ConcurrencyLimitExceeded => "rish_probe_concurrency_limit",
            Self::ProgramUnavailable => "rish_app_process_unavailable",
            Self::SpawnFailed => "rish_probe_spawn_failed",
            Self::WaitFailed => "rish_probe_wait_failed",
            Self::TimedOut => "rish_probe_timeout",
            Self::StdoutLimitExceeded => "rish_probe_stdout_limit_exceeded",
            Self::StderrLimitExceeded => "rish_probe_stderr_limit_exceeded",
            Self::ProgramFailed => "rish_probe_failed",
            Self::IdentityOutputInvalid => "rish_probe_identity_invalid",
            Self::AttestationOutputInvalid => "rish_attestation_output_invalid",
            Self::WorkerFailed => "rish_probe_worker_failed",
        }
    }
}

impl fmt::Display for RishBackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason_code())
    }
}

impl std::error::Error for RishBackendError {}

impl From<BoundedProcessError> for RishBackendError {
    fn from(error: BoundedProcessError) -> Self {
        match error {
            BoundedProcessError::ProgramUnavailable => Self::ProgramUnavailable,
            BoundedProcessError::SpawnFailed => Self::SpawnFailed,
            BoundedProcessError::WaitFailed => Self::WaitFailed,
            BoundedProcessError::TimedOut => Self::TimedOut,
            BoundedProcessError::StdoutLimitExceeded => Self::StdoutLimitExceeded,
            BoundedProcessError::StderrLimitExceeded => Self::StderrLimitExceeded,
            BoundedProcessError::ProgramFailed => Self::ProgramFailed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RishBackendStatus {
    pub(crate) available: bool,
    pub(crate) backend: &'static str,
    pub(crate) principal: &'static str,
    pub(crate) uid: u32,
    pub(crate) state: &'static str,
    pub(crate) root_accepted: bool,
    pub(crate) arbitrary_shell: bool,
    pub(crate) mutation_ready: bool,
}

/// First typed-read family: ten compile-time-allowlisted feature booleans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AndroidSystemFeaturesStatus {
    pub(crate) available: bool,
    pub(crate) backend: &'static str,
    pub(crate) state: &'static str,
    pub(crate) control_authority_proven: bool,
    pub(crate) arbitrary_shell: bool,
    pub(crate) mutation_ready: bool,
    pub(crate) features: AndroidSystemFeatureFlags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AndroidSystemFeatureFlags {
    pub(crate) wifi: bool,
    pub(crate) bluetooth: bool,
    pub(crate) camera_any: bool,
    pub(crate) location: bool,
    pub(crate) microphone: bool,
    pub(crate) nfc: bool,
    pub(crate) fingerprint: bool,
    pub(crate) accelerometer: bool,
    pub(crate) touchscreen: bool,
    pub(crate) webview: bool,
}

/// Private S3 attestation material. Never serialized to MCP responses, logs, or
/// runtime_status. A new epoch is minted only after a complete successful
/// multi-probe attestation; any later failure rotates the epoch away.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PrivateBackendEpoch {
    generation: u64,
    dex_sha256: [u8; 32],
    build_fingerprint_sha256: [u8; 32],
    boot_id_sha256: [u8; 32],
    selinux_context_sha256: [u8; 32],
    sdk: u32,
    gid: u32,
    groups: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    owner: u32,
    links: u64,
    bytes: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl FileIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            owner: metadata.uid(),
            links: metadata.nlink(),
            bytes: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct RishBackendClient {
    dex_path: Arc<PathBuf>,
    dex_file: Arc<File>,
    dex_identity: FileIdentity,
    parent_identity: FileIdentity,
    expected_sha256: [u8; 32],
    program: PathBuf,
    concurrency: Arc<Semaphore>,
    /// Monotonic private epoch generation counter. Public surfaces never see it.
    epoch_generation: Arc<AtomicU64>,
    /// Current S3 private epoch, if a complete attestation has succeeded.
    private_epoch: Arc<Mutex<Option<PrivateBackendEpoch>>>,
    #[cfg(test)]
    validation_delay: Duration,
}

impl fmt::Debug for RishBackendClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RishBackendClient")
            .field("dex_path", &"<redacted>")
            .field("dex_file", &"<redacted>")
            .field("expected_sha256", &"<redacted>")
            .field("program", &RISH_APP_PROCESS_PROGRAM)
            .field("concurrency_limit", &RISH_STATUS_CONCURRENCY)
            .field("private_epoch", &"<redacted>")
            .finish()
    }
}

impl RishBackendClient {
    // The binary target performs configuration and passes the constructed
    // client into the library transport; the library target never constructs
    // this security boundary from ambient configuration itself.
    #[allow(dead_code)]
    pub(crate) fn try_new(
        dex_path: PathBuf,
        expected_sha256: &str,
    ) -> Result<Self, RishBackendConfigError> {
        Self::new_inner(
            dex_path,
            expected_sha256,
            PathBuf::from(RISH_APP_PROCESS_PROGRAM),
        )
    }

    fn new_inner(
        dex_path: PathBuf,
        expected_sha256: &str,
        program: PathBuf,
    ) -> Result<Self, RishBackendConfigError> {
        let expected_sha256 = parse_sha256(expected_sha256).map_err(map_config_error)?;
        let parent_identity = validate_parent(&dex_path).map_err(map_config_error)?;
        let dex_file = open_pinned_dex(&dex_path).map_err(map_config_error)?;
        let dex_identity = validate_dex_identity(&dex_path, &dex_file, None, expected_sha256)
            .map_err(map_config_error)?;
        ensure_descriptor_close_on_exec(&dex_file).map_err(map_config_error)?;

        Ok(Self {
            dex_path: Arc::new(dex_path),
            dex_file: Arc::new(dex_file),
            dex_identity,
            parent_identity,
            expected_sha256,
            program,
            concurrency: Arc::new(Semaphore::new(RISH_STATUS_CONCURRENCY)),
            epoch_generation: Arc::new(AtomicU64::new(0)),
            private_epoch: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            validation_delay: Duration::ZERO,
        })
    }

    /// S2.5 foundation probe: exact Android shell UID `2000` only.
    ///
    /// Retained for development physical evidence lanes that claim only the
    /// UID-2000 contract. Public `android_rish_status` uses [`Self::attest_read_only`].
    #[allow(dead_code)] // retained for S2.5-only physical evidence callers/tests
    pub(crate) async fn probe(&self) -> Result<RishBackendStatus, RishBackendError> {
        let permit = Arc::clone(&self.concurrency)
            .try_acquire_owned()
            .map_err(|_| RishBackendError::ConcurrencyLimitExceeded)?;

        let dex_path = Arc::clone(&self.dex_path);
        let dex_file = Arc::clone(&self.dex_file);
        let expected_identity = self.dex_identity;
        let expected_parent = self.parent_identity;
        let expected_sha256 = self.expected_sha256;
        let program = self.program.clone();
        #[cfg(test)]
        let validation_delay = self.validation_delay;

        // Validation owns the permit even if its request waiter disappears.
        // On success the permit is transferred into the bounded process's
        // cancellation-independent cleanup supervisor.
        let (permit, dex_guard) = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            std::thread::sleep(validation_delay);
            let parent = validate_parent(&dex_path)?;
            if parent != expected_parent {
                return Err(RishBackendError::DexIdentityChanged);
            }
            validate_dex_identity(
                &dex_path,
                &dex_file,
                Some(expected_identity),
                expected_sha256,
            )?;
            let snapshot =
                create_execution_dex_snapshot(&dex_file, expected_identity.bytes, expected_sha256)?;
            // A source race during snapshot construction changes either the
            // byte digest or the pinned metadata and therefore fails before
            // the sealed copy can be executed.
            validate_dex_identity(
                &dex_path,
                &dex_file,
                Some(expected_identity),
                expected_sha256,
            )?;
            Ok::<(OwnedSemaphorePermit, Arc<File>), RishBackendError>((permit, Arc::new(snapshot)))
        })
        .await
        .map_err(|_| RishBackendError::WorkerFailed)??;

        run_fixed_probe(program, dex_guard, permit).await
    }

    /// S3 public read-only attestation used by `android_rish_status`.
    ///
    /// Runs the fixed multi-command identity suite under one concurrency lane,
    /// mints a private backend epoch on full success, and returns a public
    /// status with `state = attested_read_only`. Sensitive fields (epoch,
    /// fingerprints, boot id, SELinux context, group inventory, digests) never
    /// leave this module.
    pub(crate) async fn attest_read_only(&self) -> Result<RishBackendStatus, RishBackendError> {
        let permit = Arc::clone(&self.concurrency)
            .try_acquire_owned()
            .map_err(|_| RishBackendError::ConcurrencyLimitExceeded)?;
        let lane = Arc::new(permit);
        self.attest_read_only_with_lane(lane).await
    }

    /// S3 attestation under a pre-acquired concurrency lane.
    ///
    /// Callers that already hold the sole rish permit (typed-read families)
    /// must use this path so epoch establishment cannot TOCTOU against a
    /// second acquisition and so each child keeps a lane clone through cleanup.
    async fn attest_read_only_with_lane(
        &self,
        lane: RishLanePermit,
    ) -> Result<RishBackendStatus, RishBackendError> {
        let dex_path = Arc::clone(&self.dex_path);
        let dex_file = Arc::clone(&self.dex_file);
        let expected_identity = self.dex_identity;
        let expected_parent = self.parent_identity;
        let expected_sha256 = self.expected_sha256;
        let program = self.program.clone();
        #[cfg(test)]
        let validation_delay = self.validation_delay;

        let prepare = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            std::thread::sleep(validation_delay);
            let parent = validate_parent(&dex_path)?;
            if parent != expected_parent {
                return Err(RishBackendError::DexIdentityChanged);
            }
            validate_dex_identity(
                &dex_path,
                &dex_file,
                Some(expected_identity),
                expected_sha256,
            )?;
            let snapshot =
                create_execution_dex_snapshot(&dex_file, expected_identity.bytes, expected_sha256)?;
            validate_dex_identity(
                &dex_path,
                &dex_file,
                Some(expected_identity),
                expected_sha256,
            )?;
            Ok::<Arc<File>, RishBackendError>(Arc::new(snapshot))
        })
        .await
        .map_err(|_| RishBackendError::WorkerFailed)?;

        let dex_guard = match prepare {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.invalidate_private_epoch();
                return Err(error);
            }
        };

        let attestation = match run_s3_attestation_suite(
            program,
            Arc::clone(&dex_guard),
            Arc::clone(&lane),
        )
        .await
        {
            Ok(attestation) => attestation,
            Err(error) => {
                drop(dex_guard);
                drop(lane);
                self.invalidate_private_epoch();
                return Err(error);
            }
        };
        drop(dex_guard);
        drop(lane);

        let generation = self.epoch_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let epoch = PrivateBackendEpoch {
            generation,
            dex_sha256: expected_sha256,
            build_fingerprint_sha256: attestation.build_fingerprint_sha256,
            boot_id_sha256: attestation.boot_id_sha256,
            selinux_context_sha256: attestation.selinux_context_sha256,
            sdk: attestation.sdk,
            gid: attestation.gid,
            groups: attestation.groups,
        };
        {
            let mut slot = self
                .private_epoch
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *slot = Some(epoch);
        }

        Ok(RishBackendStatus {
            available: true,
            backend: "shizuku_rish",
            principal: "android_shell",
            uid: ANDROID_SHELL_UID,
            state: "attested_read_only",
            root_accepted: false,
            arbitrary_shell: false,
            mutation_ready: false,
        })
    }

    pub(crate) fn has_live_s3_epoch(&self) -> bool {
        self.private_epoch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    /// First typed-read family: ten allowlisted PackageManager feature flags.
    ///
    /// Acquires the sole rish concurrency lane first, then establishes a live
    /// S3 epoch under that same permit when absent (no epoch/permit TOCTOU).
    /// Uses only fixed `cmd package has-feature <name>` probes for the
    /// compile-time allowlist. Never returns raw OEM inventory text.
    pub(crate) async fn list_system_features(
        &self,
    ) -> Result<AndroidSystemFeaturesStatus, RishBackendError> {
        let permit = Arc::clone(&self.concurrency)
            .try_acquire_owned()
            .map_err(|_| RishBackendError::ConcurrencyLimitExceeded)?;
        let lane = Arc::new(permit);

        // Epoch precondition is established only while holding the lane so a
        // concurrent attestation failure cannot invalidate the epoch between
        // a pre-check and feature probe admission.
        if !self.has_live_s3_epoch() {
            self.attest_read_only_with_lane(Arc::clone(&lane)).await?;
        }

        let dex_path = Arc::clone(&self.dex_path);
        let dex_file = Arc::clone(&self.dex_file);
        let expected_identity = self.dex_identity;
        let expected_parent = self.parent_identity;
        let expected_sha256 = self.expected_sha256;
        let program = self.program.clone();

        let prepare = tokio::task::spawn_blocking(move || {
            let parent = validate_parent(&dex_path)?;
            if parent != expected_parent {
                return Err(RishBackendError::DexIdentityChanged);
            }
            validate_dex_identity(
                &dex_path,
                &dex_file,
                Some(expected_identity),
                expected_sha256,
            )?;
            let snapshot =
                create_execution_dex_snapshot(&dex_file, expected_identity.bytes, expected_sha256)?;
            validate_dex_identity(
                &dex_path,
                &dex_file,
                Some(expected_identity),
                expected_sha256,
            )?;
            Ok::<Arc<File>, RishBackendError>(Arc::new(snapshot))
        })
        .await
        .map_err(|_| RishBackendError::WorkerFailed)?;

        let dex_guard = match prepare {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.invalidate_private_epoch();
                return Err(error);
            }
        };

        let family_deadline = tokio::time::Instant::now() + RISH_FEATURE_FAMILY_BUDGET;
        let mut present = [false; SYSTEM_FEATURE_PROBES.len()];
        for (index, command) in SYSTEM_FEATURE_PROBES.iter().enumerate() {
            if tokio::time::Instant::now() >= family_deadline {
                drop(dex_guard);
                drop(lane);
                self.invalidate_private_epoch();
                return Err(RishBackendError::TimedOut);
            }
            let stdout = match run_fixed_command_output_with_bounds(
                &program,
                &dex_guard,
                command,
                &RISH_FEATURE_COMMAND_BOUNDS,
                Arc::clone(&lane),
            )
            .await
            {
                Ok(stdout) => stdout,
                Err(error) => {
                    drop(dex_guard);
                    drop(lane);
                    self.invalidate_private_epoch();
                    return Err(error);
                }
            };
            present[index] = match parse_has_feature_line(&stdout) {
                Ok(value) => value,
                Err(error) => {
                    drop(dex_guard);
                    drop(lane);
                    self.invalidate_private_epoch();
                    return Err(error);
                }
            };
        }
        drop(dex_guard);
        drop(lane);

        if !self.has_live_s3_epoch() {
            return Err(RishBackendError::AttestationOutputInvalid);
        }

        Ok(AndroidSystemFeaturesStatus {
            available: true,
            backend: "shizuku_rish",
            state: "attested_read_only",
            control_authority_proven: false,
            arbitrary_shell: false,
            mutation_ready: false,
            features: AndroidSystemFeatureFlags {
                wifi: present[0],
                bluetooth: present[1],
                camera_any: present[2],
                location: present[3],
                microphone: present[4],
                nfc: present[5],
                fingerprint: present[6],
                accelerometer: present[7],
                touchscreen: present[8],
                webview: present[9],
            },
        })
    }

    fn invalidate_private_epoch(&self) {
        let mut slot = self
            .private_epoch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.take().is_some() {
            // Bump generation so any future grant/epoch binding cannot reuse
            // the previous private identity after a fail-closed transition.
            self.epoch_generation.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_program(
        dex_path: PathBuf,
        expected_sha256: &str,
        program: PathBuf,
    ) -> Result<Self, RishBackendConfigError> {
        Self::new_inner(dex_path, expected_sha256, program)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct S3AttestationMaterial {
    gid: u32,
    groups: Vec<u32>,
    sdk: u32,
    build_fingerprint_sha256: [u8; 32],
    boot_id_sha256: [u8; 32],
    selinux_context_sha256: [u8; 32],
}

fn map_config_error(error: RishBackendError) -> RishBackendConfigError {
    match error {
        RishBackendError::DexDigestFormatInvalid => RishBackendConfigError::DexDigestFormatInvalid,
        RishBackendError::DexPathInvalid => RishBackendConfigError::DexPathInvalid,
        RishBackendError::DexParentInvalid => RishBackendConfigError::DexParentInvalid,
        RishBackendError::DexFileInvalid => RishBackendConfigError::DexFileInvalid,
        RishBackendError::DexDigestMismatch => RishBackendConfigError::DexDigestMismatch,
        RishBackendError::DexDescriptorInvalid => RishBackendConfigError::DexDescriptorInvalid,
        RishBackendError::DexIdentityChanged => RishBackendConfigError::DexIdentityChanged,
        RishBackendError::DexSnapshotFailed
        | RishBackendError::ConcurrencyLimitExceeded
        | RishBackendError::ProgramUnavailable
        | RishBackendError::SpawnFailed
        | RishBackendError::WaitFailed
        | RishBackendError::TimedOut
        | RishBackendError::StdoutLimitExceeded
        | RishBackendError::StderrLimitExceeded
        | RishBackendError::ProgramFailed
        | RishBackendError::IdentityOutputInvalid
        | RishBackendError::AttestationOutputInvalid
        | RishBackendError::WorkerFailed => RishBackendConfigError::Internal,
    }
}

fn rish_child_environment() -> Vec<(OsString, OsString)> {
    let mut environment = vec![
        (
            OsString::from("RISH_APPLICATION_ID"),
            OsString::from(RISH_APPLICATION_ID),
        ),
        (OsString::from("RISH_PRESERVE_ENV"), OsString::from("0")),
    ];
    for key in RISH_ANDROID_RUNTIME_ENV_KEYS {
        let Some(value) = std::env::var_os(key) else {
            continue;
        };
        // Reject empty or NUL-bearing values so ambient corruption cannot open
        // an injection channel into the fixed app_process child.
        if value.is_empty() || value.as_bytes().contains(&0) {
            continue;
        }
        environment.push((OsString::from(*key), value));
    }
    environment
}

#[allow(dead_code)] // used by probe() S2.5 path
async fn run_fixed_probe(
    program: PathBuf,
    dex_guard: Arc<File>,
    permit: OwnedSemaphorePermit,
) -> Result<RishBackendStatus, RishBackendError> {
    ensure_descriptor_close_on_exec(&dex_guard)?;
    let dex_argument = OsString::from(format!(
        "-Djava.class.path=/proc/self/fd/{}",
        dex_guard.as_raw_fd()
    ));
    let process = BoundedProcess::new_with_child_context(
        program,
        vec![
            dex_argument,
            OsString::from("/system/bin"),
            OsString::from("--nice-name=termux-mcp-rish"),
            OsString::from(RISH_LOADER_CLASS),
            OsString::from("-c"),
            OsString::from(RISH_FIXED_COMMAND),
        ],
        PathBuf::from("/"),
        RISH_STATUS_TIMEOUT,
        RISH_STATUS_STDOUT_BYTES,
        RISH_STATUS_STDERR_BYTES,
        BoundedChildContext::with_inherited_descriptor(
            rish_child_environment(),
            Arc::clone(&dex_guard),
        ),
    )
    .map_err(map_fixed_process_config)?;
    let output = process.run_with_completion_guard(permit).await?;
    drop(dex_guard);

    if output.stdout.as_slice() != EXPECTED_STDOUT || !output.stderr.is_empty() {
        return Err(RishBackendError::IdentityOutputInvalid);
    }

    Ok(RishBackendStatus {
        available: true,
        backend: "shizuku_rish",
        principal: "android_shell",
        uid: ANDROID_SHELL_UID,
        state: "verified_shell_uid",
        root_accepted: false,
        arbitrary_shell: false,
        mutation_ready: false,
    })
}

async fn run_s3_attestation_suite(
    program: PathBuf,
    dex_guard: Arc<File>,
    lane: RishLanePermit,
) -> Result<S3AttestationMaterial, RishBackendError> {
    // Order is intentional: prove shell UID before any broader identity claim.
    // Every fixed child retains a lane clone until process-group cleanup finishes.
    let uid_stdout =
        run_fixed_command_output(&program, &dex_guard, RISH_FIXED_COMMAND, Arc::clone(&lane))
            .await?;
    if uid_stdout.as_slice() != EXPECTED_STDOUT {
        return Err(RishBackendError::IdentityOutputInvalid);
    }

    let gid = parse_exact_u32_line(
        &run_fixed_command_output(
            &program,
            &dex_guard,
            RISH_FIXED_COMMAND_GID,
            Arc::clone(&lane),
        )
        .await?,
    )?;
    if gid != ANDROID_SHELL_UID {
        return Err(RishBackendError::AttestationOutputInvalid);
    }

    let groups = parse_group_list_line(
        &run_fixed_command_output(
            &program,
            &dex_guard,
            RISH_FIXED_COMMAND_GROUPS,
            Arc::clone(&lane),
        )
        .await?,
    )?;
    if !groups.contains(&ANDROID_SHELL_UID) {
        return Err(RishBackendError::AttestationOutputInvalid);
    }

    let selinux_raw = run_fixed_command_output(
        &program,
        &dex_guard,
        RISH_FIXED_COMMAND_SELINUX,
        Arc::clone(&lane),
    )
    .await?;
    let selinux = parse_shell_selinux_context(&selinux_raw)?;
    let selinux_context_sha256 = Sha256::digest(selinux).into();

    let sdk = parse_exact_u32_line(
        &run_fixed_command_output(
            &program,
            &dex_guard,
            RISH_FIXED_COMMAND_SDK,
            Arc::clone(&lane),
        )
        .await?,
    )?;
    if !(MIN_SUPPORTED_SDK..=MAX_SUPPORTED_SDK).contains(&sdk) {
        return Err(RishBackendError::AttestationOutputInvalid);
    }

    let fingerprint_raw = run_fixed_command_output(
        &program,
        &dex_guard,
        RISH_FIXED_COMMAND_FINGERPRINT,
        Arc::clone(&lane),
    )
    .await?;
    let fingerprint = parse_build_fingerprint(&fingerprint_raw)?;
    let build_fingerprint_sha256 = Sha256::digest(fingerprint).into();

    let boot_raw = run_fixed_command_output(
        &program,
        &dex_guard,
        RISH_FIXED_COMMAND_BOOT_ID,
        Arc::clone(&lane),
    )
    .await?;
    let boot_id = parse_boot_id(&boot_raw)?;
    let boot_id_sha256 = Sha256::digest(boot_id).into();

    Ok(S3AttestationMaterial {
        gid,
        groups,
        sdk,
        build_fingerprint_sha256,
        boot_id_sha256,
        selinux_context_sha256,
    })
}

struct FixedCommandBounds {
    timeout: Duration,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
    accept_nonzero_exit: bool,
}

const RISH_STATUS_COMMAND_BOUNDS: FixedCommandBounds = FixedCommandBounds {
    timeout: RISH_STATUS_TIMEOUT,
    max_stdout_bytes: RISH_STATUS_STDOUT_BYTES,
    max_stderr_bytes: RISH_STATUS_STDERR_BYTES,
    accept_nonzero_exit: false,
};

const RISH_FEATURE_COMMAND_BOUNDS: FixedCommandBounds = FixedCommandBounds {
    timeout: RISH_FEATURE_PROBE_TIMEOUT,
    max_stdout_bytes: RISH_STATUS_STDOUT_BYTES,
    max_stderr_bytes: RISH_STATUS_STDERR_BYTES,
    accept_nonzero_exit: true,
};

async fn run_fixed_command_output(
    program: &Path,
    dex_guard: &Arc<File>,
    fixed_command: &'static str,
    lane: RishLanePermit,
) -> Result<Vec<u8>, RishBackendError> {
    run_fixed_command_output_with_bounds(
        program,
        dex_guard,
        fixed_command,
        &RISH_STATUS_COMMAND_BOUNDS,
        lane,
    )
    .await
}

async fn run_fixed_command_output_with_bounds(
    program: &Path,
    dex_guard: &Arc<File>,
    fixed_command: &'static str,
    bounds: &FixedCommandBounds,
    lane: RishLanePermit,
) -> Result<Vec<u8>, RishBackendError> {
    ensure_descriptor_close_on_exec(dex_guard)?;
    let dex_argument = OsString::from(format!(
        "-Djava.class.path=/proc/self/fd/{}",
        dex_guard.as_raw_fd()
    ));
    let process = BoundedProcess::new_with_child_context(
        program.to_path_buf(),
        vec![
            dex_argument,
            OsString::from("/system/bin"),
            OsString::from("--nice-name=termux-mcp-rish"),
            OsString::from(RISH_LOADER_CLASS),
            OsString::from("-c"),
            OsString::from(fixed_command),
        ],
        PathBuf::from("/"),
        bounds.timeout,
        bounds.max_stdout_bytes,
        bounds.max_stderr_bytes,
        BoundedChildContext::with_inherited_descriptor(
            rish_child_environment(),
            Arc::clone(dex_guard),
        ),
    )
    .map_err(map_fixed_process_config)?
    .with_accept_nonzero_exit(bounds.accept_nonzero_exit);
    // Hold a lane clone in the cancellation-independent supervisor so a
    // dropped request future cannot open the concurrency lane before reaping.
    let output = process.run_with_completion_guard(lane).await?;
    // Under rish, a few Android utilities occasionally emit the sole payload on
    // stderr with empty stdout. Accept exactly one non-empty stream.
    match (output.stdout.is_empty(), output.stderr.is_empty()) {
        (false, true) => Ok(output.stdout),
        (true, false) => Ok(output.stderr),
        _ => Err(RishBackendError::AttestationOutputInvalid),
    }
}

fn parse_has_feature_line(stdout: &[u8]) -> Result<bool, RishBackendError> {
    match stdout {
        b"true\n" | b"true" => Ok(true),
        b"false\n" | b"false" => Ok(false),
        _ => Err(RishBackendError::AttestationOutputInvalid),
    }
}

#[allow(dead_code)]
fn parse_exact_u32_line(stdout: &[u8]) -> Result<u32, RishBackendError> {
    let Some(line) = stdout.strip_suffix(b"\n") else {
        return Err(RishBackendError::AttestationOutputInvalid);
    };
    if line.is_empty() || !line.iter().all(u8::is_ascii_digit) || line.len() > 10 {
        return Err(RishBackendError::AttestationOutputInvalid);
    }
    let text = std::str::from_utf8(line).map_err(|_| RishBackendError::AttestationOutputInvalid)?;
    text.parse::<u32>()
        .map_err(|_| RishBackendError::AttestationOutputInvalid)
}

#[allow(dead_code)]
fn parse_group_list_line(stdout: &[u8]) -> Result<Vec<u32>, RishBackendError> {
    let Some(line) = stdout.strip_suffix(b"\n") else {
        return Err(RishBackendError::AttestationOutputInvalid);
    };
    if line.is_empty() {
        return Err(RishBackendError::AttestationOutputInvalid);
    }
    let mut groups = Vec::new();
    for part in line.split(|byte| *byte == b' ') {
        if part.is_empty() || !part.iter().all(u8::is_ascii_digit) || part.len() > 10 {
            return Err(RishBackendError::AttestationOutputInvalid);
        }
        let text =
            std::str::from_utf8(part).map_err(|_| RishBackendError::AttestationOutputInvalid)?;
        let group = text
            .parse::<u32>()
            .map_err(|_| RishBackendError::AttestationOutputInvalid)?;
        groups.push(group);
        if groups.len() > MAX_SUPPLEMENTARY_GROUPS {
            return Err(RishBackendError::AttestationOutputInvalid);
        }
    }
    Ok(groups)
}

fn parse_shell_selinux_context(stdout: &[u8]) -> Result<&[u8], RishBackendError> {
    // `cat /proc/self/attr/current` is NUL-terminated; `id -Z` uses a newline.
    let line = stdout
        .strip_suffix(&[0])
        .or_else(|| stdout.strip_suffix(b"\n"))
        .unwrap_or(stdout);
    if line.is_empty()
        || line.len() > MAX_SELINUX_CONTEXT_BYTES
        || !line.starts_with(EXPECTED_SHELL_SELINUX_PREFIX)
    {
        return Err(RishBackendError::AttestationOutputInvalid);
    }
    // Allow only a tight charset so raw policy strings never become free text.
    if !line
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-' | b','))
    {
        return Err(RishBackendError::AttestationOutputInvalid);
    }
    Ok(line)
}

fn parse_build_fingerprint(stdout: &[u8]) -> Result<&[u8], RishBackendError> {
    // Accept optional trailing newline; reject empty and control characters.
    let line = stdout.strip_suffix(b"\n").unwrap_or(stdout);
    if line.is_empty()
        || line.len() > MAX_FINGERPRINT_BYTES
        || !line
            .iter()
            .all(|byte| byte.is_ascii_graphic() && *byte != b'\\')
    {
        return Err(RishBackendError::AttestationOutputInvalid);
    }
    Ok(line)
}

#[allow(dead_code)]
fn parse_boot_id(stdout: &[u8]) -> Result<&[u8], RishBackendError> {
    let Some(line) = stdout.strip_suffix(b"\n") else {
        return Err(RishBackendError::AttestationOutputInvalid);
    };
    // Canonical Linux boot_id: 8-4-4-4-12 lowercase hex with hyphens.
    if line.len() != 36 {
        return Err(RishBackendError::AttestationOutputInvalid);
    }
    for (index, byte) in line.iter().enumerate() {
        let ok = match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase(),
        };
        if !ok {
            return Err(RishBackendError::AttestationOutputInvalid);
        }
    }
    Ok(line)
}

fn map_fixed_process_config(_: BoundedProcessConfigError) -> RishBackendError {
    // Every bound above is a compile-time constant inside BoundedProcess's
    // accepted limits. A failure therefore represents an internal worker
    // configuration error, never caller input.
    RishBackendError::WorkerFailed
}

fn parse_sha256(value: &str) -> Result<[u8; 32], RishBackendError> {
    if value.len() != 64
        || !value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(RishBackendError::DexDigestFormatInvalid);
    }

    let mut digest = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]);
    }
    Ok(digest)
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("digest syntax is validated before decoding"),
    }
}

fn validate_parent(dex_path: &Path) -> Result<FileIdentity, RishBackendError> {
    if !dex_path.is_absolute() {
        return Err(RishBackendError::DexPathInvalid);
    }
    let canonical_dex = fs::canonicalize(dex_path).map_err(|_| RishBackendError::DexPathInvalid)?;
    if canonical_dex.as_os_str().as_bytes() != dex_path.as_os_str().as_bytes() {
        return Err(RishBackendError::DexPathInvalid);
    }

    let parent = dex_path
        .parent()
        .ok_or(RishBackendError::DexParentInvalid)?;
    let canonical_parent =
        fs::canonicalize(parent).map_err(|_| RishBackendError::DexParentInvalid)?;
    if canonical_parent.as_os_str().as_bytes() != parent.as_os_str().as_bytes() {
        return Err(RishBackendError::DexParentInvalid);
    }
    let metadata = fs::symlink_metadata(parent).map_err(|_| RishBackendError::DexParentInvalid)?;
    let identity = FileIdentity::from_metadata(&metadata);
    let permissions = identity.mode & 0o7777;
    if !metadata.file_type().is_dir()
        || identity.owner != getuid().as_raw()
        || permissions & 0o077 != 0
        || permissions & 0o500 != 0o500
    {
        return Err(RishBackendError::DexParentInvalid);
    }
    Ok(identity)
}

fn open_pinned_dex(dex_path: &Path) -> Result<File, RishBackendError> {
    open(
        dex_path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| RishBackendError::DexFileInvalid)
}

fn validate_dex_identity(
    dex_path: &Path,
    dex_file: &File,
    expected_identity: Option<FileIdentity>,
    expected_sha256: [u8; 32],
) -> Result<FileIdentity, RishBackendError> {
    let descriptor_metadata = dex_file
        .metadata()
        .map_err(|_| RishBackendError::DexDescriptorInvalid)?;
    let descriptor_identity = FileIdentity::from_metadata(&descriptor_metadata);
    if expected_identity.is_some_and(|expected| expected != descriptor_identity) {
        return Err(RishBackendError::DexIdentityChanged);
    }
    validate_dex_metadata(&descriptor_metadata, descriptor_identity)?;

    let path_metadata =
        fs::symlink_metadata(dex_path).map_err(|_| RishBackendError::DexIdentityChanged)?;
    let path_identity = FileIdentity::from_metadata(&path_metadata);
    validate_dex_metadata(&path_metadata, path_identity)?;
    if descriptor_identity != path_identity {
        return Err(RishBackendError::DexIdentityChanged);
    }
    let actual_sha256 = hash_exact_file(dex_file, descriptor_identity.bytes)?;
    let after = dex_file
        .metadata()
        .map_err(|_| RishBackendError::DexDescriptorInvalid)?;
    if FileIdentity::from_metadata(&after) != descriptor_identity {
        return Err(RishBackendError::DexIdentityChanged);
    }
    if actual_sha256 != expected_sha256 {
        return Err(RishBackendError::DexDigestMismatch);
    }
    Ok(descriptor_identity)
}

fn validate_dex_metadata(
    metadata: &Metadata,
    identity: FileIdentity,
) -> Result<(), RishBackendError> {
    if !metadata.file_type().is_file()
        || identity.owner != getuid().as_raw()
        || identity.links != 1
        || identity.mode & 0o7777 != 0o400
        || !(MIN_RISH_DEX_BYTES..=MAX_RISH_DEX_BYTES).contains(&identity.bytes)
    {
        return Err(RishBackendError::DexFileInvalid);
    }
    Ok(())
}

fn hash_exact_file(file: &File, bytes: u64) -> Result<[u8; 32], RishBackendError> {
    let mut hasher = Sha256::new();
    let mut offset = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    while offset < bytes {
        let remaining = usize::try_from(bytes - offset).unwrap_or(usize::MAX);
        let requested = remaining.min(buffer.len());
        let read = file
            .read_at(&mut buffer[..requested], offset)
            .map_err(|_| RishBackendError::DexDescriptorInvalid)?;
        if read == 0 {
            return Err(RishBackendError::DexIdentityChanged);
        }
        hasher.update(&buffer[..read]);
        offset += u64::try_from(read).map_err(|_| RishBackendError::DexDescriptorInvalid)?;
    }
    Ok(hasher.finalize().into())
}

fn create_execution_dex_snapshot(
    source: &File,
    bytes: u64,
    expected_sha256: [u8; 32],
) -> Result<File, RishBackendError> {
    // Prefer a sealed memfd when the child can reopen it through
    // `/proc/self/fd` for ART's classpath. Android kernels commonly allow
    // sealed memfd creation but deny path reopen, which would make real
    // `app_process64` DEX loading fail closed after a false host-side success.
    if let Ok(snapshot) = try_create_sealed_memfd_snapshot(source, bytes, expected_sha256) {
        if snapshot_classpath_path_is_usable(&snapshot, bytes, expected_sha256) {
            return Ok(snapshot);
        }
    }
    create_private_tmpfile_snapshot(source, bytes, expected_sha256)
}

fn try_create_sealed_memfd_snapshot(
    source: &File,
    bytes: u64,
    expected_sha256: [u8; 32],
) -> Result<File, RishBackendError> {
    let base_flags = MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING;
    // Newer kernels can require an explicit non-executable memfd posture.
    // DEX input is data, not a native executable, so prefer NOEXEC_SEAL and
    // fall back only when an older API-30-compatible kernel rejects the flag.
    let descriptor = match memfd_create("termux-mcp-rish-dex", base_flags | MemfdFlags::NOEXEC_SEAL)
    {
        Ok(descriptor) => descriptor,
        Err(rustix::io::Errno::INVAL) => memfd_create("termux-mcp-rish-dex", base_flags)
            .map_err(|_| RishBackendError::DexSnapshotFailed)?,
        Err(_) => return Err(RishBackendError::DexSnapshotFailed),
    };
    let snapshot = File::from(descriptor);
    ensure_descriptor_close_on_exec(&snapshot).map_err(|_| RishBackendError::DexSnapshotFailed)?;
    finalize_writable_snapshot(source, &snapshot, bytes, expected_sha256)?;

    let required_seals = SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE | SealFlags::SEAL;
    fcntl_add_seals(&snapshot, required_seals).map_err(|_| RishBackendError::DexSnapshotFailed)?;
    let observed_seals =
        fcntl_get_seals(&snapshot).map_err(|_| RishBackendError::DexSnapshotFailed)?;
    if !observed_seals.contains(required_seals) {
        return Err(RishBackendError::DexSnapshotFailed);
    }
    if hash_exact_file(&snapshot, bytes)? != expected_sha256 {
        return Err(RishBackendError::DexSnapshotFailed);
    }
    Ok(snapshot)
}

fn snapshot_classpath_path_is_usable(
    snapshot: &File,
    bytes: u64,
    expected_sha256: [u8; 32],
) -> bool {
    // ART opens the classpath path; inherited-fd reads alone are insufficient.
    // Require a fresh open of `/proc/self/fd/N` to return the exact digest.
    let path = format!("/proc/self/fd/{}", snapshot.as_raw_fd());
    let Ok(reopened) = open(
        path.as_str(),
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    ) else {
        return false;
    };
    let reopened = File::from(reopened);
    hash_exact_file(&reopened, bytes).is_ok_and(|digest| digest == expected_sha256)
}

fn create_private_tmpfile_snapshot(
    source: &File,
    bytes: u64,
    expected_sha256: [u8; 32],
) -> Result<File, RishBackendError> {
    let snapshot_dir = private_snapshot_directory()?;
    let writable = match open(
        &snapshot_dir,
        OFlags::TMPFILE | OFlags::RDWR | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    ) {
        Ok(descriptor) => File::from(descriptor),
        Err(_) => create_named_private_snapshot_file(&snapshot_dir)?,
    };
    ensure_descriptor_close_on_exec(&writable).map_err(|_| RishBackendError::DexSnapshotFailed)?;
    finalize_writable_snapshot(source, &writable, bytes, expected_sha256)?;
    rustix::fs::fchmod(&writable, Mode::from_raw_mode(0o400))
        .map_err(|_| RishBackendError::DexSnapshotFailed)?;

    // Re-open read-only through the classpath path so the inherited descriptor
    // matches what ART will open and so mode 0400 is enforced on that open.
    let path = format!("/proc/self/fd/{}", writable.as_raw_fd());
    let readonly = open(
        path.as_str(),
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| RishBackendError::DexSnapshotFailed)?;
    ensure_descriptor_close_on_exec(&readonly).map_err(|_| RishBackendError::DexSnapshotFailed)?;
    if hash_exact_file(&readonly, bytes)? != expected_sha256 {
        return Err(RishBackendError::DexSnapshotFailed);
    }
    drop(writable);
    Ok(readonly)
}

fn private_snapshot_directory() -> Result<PathBuf, RishBackendError> {
    let mut snapshot_dir = std::env::temp_dir();
    // Unique per call so concurrent probes never share a named-fallback
    // directory or race on mkdir mode verification.
    snapshot_dir.push(format!(
        "termux-mcp-rish-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    // Create mode 0700 at mkdir time (not chmod-after) so umask cannot leave a
    // briefly world-accessible directory under temp_dir().
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(&snapshot_dir)
        .map_err(|_| RishBackendError::DexSnapshotFailed)?;
    let metadata =
        fs::symlink_metadata(&snapshot_dir).map_err(|_| RishBackendError::DexSnapshotFailed)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != getuid().as_raw()
        || metadata.mode() & 0o777 != 0o700
    {
        let _ = fs::remove_dir(&snapshot_dir);
        return Err(RishBackendError::DexSnapshotFailed);
    }
    Ok(snapshot_dir)
}

fn create_named_private_snapshot_file(snapshot_dir: &Path) -> Result<File, RishBackendError> {
    // Exclusive named fallback when O_TMPFILE is unavailable. The name is
    // unlinked immediately after open so only the descriptor remains. Unlink
    // failure is fatal: leaving a named 0600 DEX copy on disk changes the
    // security posture of the fallback.
    let mut name = snapshot_dir.to_path_buf();
    name.push(format!(
        "snap-{}-{}.dex",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    let file = open(
        &name,
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    )
    .map(File::from)
    .map_err(|_| RishBackendError::DexSnapshotFailed)?;
    if fs::remove_file(&name).is_err() {
        drop(file);
        return Err(RishBackendError::DexSnapshotFailed);
    }
    Ok(file)
}

fn finalize_writable_snapshot(
    source: &File,
    snapshot: &File,
    bytes: u64,
    expected_sha256: [u8; 32],
) -> Result<(), RishBackendError> {
    let copied_sha256 = copy_and_hash_exact_file(source, snapshot, bytes)?;
    if copied_sha256 != expected_sha256 {
        return Err(RishBackendError::DexDigestMismatch);
    }
    let metadata = snapshot
        .metadata()
        .map_err(|_| RishBackendError::DexSnapshotFailed)?;
    if !metadata.file_type().is_file() || metadata.len() != bytes {
        return Err(RishBackendError::DexSnapshotFailed);
    }
    if hash_exact_file(snapshot, bytes)? != expected_sha256 {
        return Err(RishBackendError::DexSnapshotFailed);
    }
    Ok(())
}

fn copy_and_hash_exact_file(
    source: &File,
    destination: &File,
    bytes: u64,
) -> Result<[u8; 32], RishBackendError> {
    let mut hasher = Sha256::new();
    let mut offset = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    while offset < bytes {
        let remaining = usize::try_from(bytes - offset).unwrap_or(usize::MAX);
        let requested = remaining.min(buffer.len());
        let read = source
            .read_at(&mut buffer[..requested], offset)
            .map_err(|_| RishBackendError::DexDescriptorInvalid)?;
        if read == 0 {
            return Err(RishBackendError::DexIdentityChanged);
        }
        hasher.update(&buffer[..read]);

        let mut written = 0_usize;
        while written < read {
            let destination_offset = offset
                .checked_add(
                    u64::try_from(written).map_err(|_| RishBackendError::DexSnapshotFailed)?,
                )
                .ok_or(RishBackendError::DexSnapshotFailed)?;
            let count = destination
                .write_at(&buffer[written..read], destination_offset)
                .map_err(|_| RishBackendError::DexSnapshotFailed)?;
            if count == 0 {
                return Err(RishBackendError::DexSnapshotFailed);
            }
            written = written
                .checked_add(count)
                .ok_or(RishBackendError::DexSnapshotFailed)?;
        }
        offset = offset
            .checked_add(u64::try_from(read).map_err(|_| RishBackendError::DexSnapshotFailed)?)
            .ok_or(RishBackendError::DexSnapshotFailed)?;
    }
    Ok(hasher.finalize().into())
}

fn ensure_descriptor_close_on_exec(file: &File) -> Result<(), RishBackendError> {
    let flags = fcntl_getfd(file).map_err(|_| RishBackendError::DexDescriptorInvalid)?;
    if !flags.contains(FdFlags::CLOEXEC) {
        return Err(RishBackendError::DexDescriptorInvalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs::Permissions,
        os::unix::fs::{symlink, PermissionsExt},
    };

    use tempfile::TempDir;

    use super::*;
    use crate::bounded_process::BOUNDED_PROCESS_TEST_LOCK;

    const VALID_SCRIPT: &str = r#"#!/bin/sh
[ "$RISH_APPLICATION_ID" = "com.termux" ] || exit 31
[ "$RISH_PRESERVE_ENV" = "0" ] || exit 32
[ "$#" -eq 6 ] || exit 33
case "$1" in
  -Djava.class.path=/proc/self/fd/*) ;;
  *) exit 34 ;;
esac
dex="${1#-Djava.class.path=}"
# ART opens the classpath path; `[ -r ]` alone is insufficient on Android
# where memfd path reopen is denied while the test builtin may still pass.
content=$(cat "$dex" 2>/dev/null) || exit 35
[ "$content" = "fixed-test-rish-dex" ] || exit 35
[ "$2" = "/system/bin" ] || exit 36
[ "$3" = "--nice-name=termux-mcp-rish" ] || exit 37
[ "$4" = "rikka.shizuku.shell.ShizukuShellLoader" ] || exit 38
[ "$5" = "-c" ] || exit 39
[ -z "$HOME" ] || exit 41
# S2.5 fixtures accept only the UID probe. S3 fixtures dispatch on $6.
[ "$6" = "exec /system/bin/id -u" ] || exit 40
printf '2000\n'
"#;

    const S3_VALID_SCRIPT: &str = r#"#!/bin/sh
[ "$RISH_APPLICATION_ID" = "com.termux" ] || exit 31
[ "$RISH_PRESERVE_ENV" = "0" ] || exit 32
[ "$#" -eq 6 ] || exit 33
case "$1" in
  -Djava.class.path=/proc/self/fd/*) ;;
  *) exit 34 ;;
esac
dex="${1#-Djava.class.path=}"
content=$(cat "$dex" 2>/dev/null) || exit 35
[ "$content" = "fixed-test-rish-dex" ] || exit 35
[ "$2" = "/system/bin" ] || exit 36
[ "$3" = "--nice-name=termux-mcp-rish" ] || exit 37
[ "$4" = "rikka.shizuku.shell.ShizukuShellLoader" ] || exit 38
[ "$5" = "-c" ] || exit 39
[ -z "$HOME" ] || exit 41
case "$6" in
  "exec /system/bin/id -u") printf '2000\n' ;;
  "exec /system/bin/id -g") printf '2000\n' ;;
  "exec /system/bin/id -G") printf '2000 1004 1007 3003\n' ;;
  "exec /system/bin/cat /proc/self/attr/current") printf 'u:r:shell:s0\0' ;;
  "exec /system/bin/getprop ro.build.version.sdk") printf '34\n' ;;
  "exec /system/bin/getprop ro.build.fingerprint")
    printf 'google/test/test:14/UQ1A.000000.000/0000000:user/release-keys\n'
    ;;
  "exec /system/bin/cat /proc/sys/kernel/random/boot_id")
    printf '01234567-89ab-cdef-0123-456789abcdef\n'
    ;;
  "exec /system/bin/cmd package has-feature android.hardware.wifi") printf 'true\n' ;;
  "exec /system/bin/cmd package has-feature android.hardware.bluetooth") printf 'true\n' ;;
  "exec /system/bin/cmd package has-feature android.hardware.camera.any") printf 'true\n' ;;
  "exec /system/bin/cmd package has-feature android.hardware.location") printf 'true\n' ;;
  "exec /system/bin/cmd package has-feature android.hardware.microphone") printf 'true\n' ;;
  # Mirror AOSP: has-feature prints the boolean and exits 1 when false.
  "exec /system/bin/cmd package has-feature android.hardware.nfc")
    printf 'false\n'
    exit 1
    ;;
  "exec /system/bin/cmd package has-feature android.hardware.fingerprint") printf 'true\n' ;;
  "exec /system/bin/cmd package has-feature android.hardware.sensor.accelerometer") printf 'true\n' ;;
  "exec /system/bin/cmd package has-feature android.hardware.touchscreen") printf 'true\n' ;;
  "exec /system/bin/cmd package has-feature android.software.webview") printf 'true\n' ;;
  *) exit 40 ;;
esac
"#;

    struct Fixture {
        _directory: TempDir,
        dex_path: PathBuf,
        digest: String,
        program: PathBuf,
    }

    impl Fixture {
        fn new(script: &str) -> Self {
            let directory = tempfile::tempdir().unwrap();
            fs::set_permissions(directory.path(), Permissions::from_mode(0o700)).unwrap();
            let dex_path = directory.path().join("rish_shizuku.dex");
            fs::write(&dex_path, b"fixed-test-rish-dex").unwrap();
            fs::set_permissions(&dex_path, Permissions::from_mode(0o400)).unwrap();
            let digest = Sha256::digest(b"fixed-test-rish-dex")
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let program = directory.path().join("fake-app-process64");
            fs::write(&program, script).unwrap();
            fs::set_permissions(&program, Permissions::from_mode(0o700)).unwrap();
            Self {
                _directory: directory,
                dex_path,
                digest,
                program,
            }
        }

        fn client(&self) -> Result<RishBackendClient, RishBackendConfigError> {
            RishBackendClient::with_test_program(
                self.dex_path.clone(),
                &self.digest,
                self.program.clone(),
            )
        }
    }

    #[tokio::test]
    async fn fixed_probe_attests_environment_arguments_descriptor_and_shell_uid() {
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let fixture = Fixture::new(VALID_SCRIPT);
        let client = fixture.client().unwrap();
        let status = client.probe().await.unwrap();
        assert_eq!(
            status,
            RishBackendStatus {
                available: true,
                backend: "shizuku_rish",
                principal: "android_shell",
                uid: 2000,
                state: "verified_shell_uid",
                root_accepted: false,
                arbitrary_shell: false,
                mutation_ready: false,
            }
        );
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            serde_json::json!({
                "available": true,
                "backend": "shizuku_rish",
                "principal": "android_shell",
                "uid": 2000,
                "state": "verified_shell_uid",
                "rootAccepted": false,
                "arbitraryShell": false,
                "mutationReady": false,
            })
        );
        assert!(fcntl_getfd(client.dex_file.as_ref())
            .unwrap()
            .contains(FdFlags::CLOEXEC));
    }

    #[test]
    fn digest_parser_requires_exact_lowercase_sha256() {
        let valid = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(parse_sha256(valid).is_ok());
        for invalid in [
            "",
            "0",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0",
            "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF",
            "g123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ] {
            assert_eq!(
                parse_sha256(invalid),
                Err(RishBackendError::DexDigestFormatInvalid)
            );
        }
    }

    #[test]
    fn constructor_rejects_relative_noncanonical_and_symlink_paths() {
        let fixture = Fixture::new(VALID_SCRIPT);
        assert_eq!(
            RishBackendClient::with_test_program(
                PathBuf::from("rish_shizuku.dex"),
                &fixture.digest,
                fixture.program.clone(),
            )
            .unwrap_err(),
            RishBackendConfigError::DexPathInvalid
        );

        let noncanonical = fixture.dex_path.parent().unwrap().join(".").join(
            fixture
                .dex_path
                .file_name()
                .expect("fixture DEX has a filename"),
        );
        assert_eq!(
            RishBackendClient::with_test_program(
                noncanonical,
                &fixture.digest,
                fixture.program.clone(),
            )
            .unwrap_err(),
            RishBackendConfigError::DexPathInvalid
        );

        let link = fixture.dex_path.parent().unwrap().join("linked.dex");
        symlink(&fixture.dex_path, &link).unwrap();
        assert_eq!(
            RishBackendClient::with_test_program(link, &fixture.digest, fixture.program.clone())
                .unwrap_err(),
            RishBackendConfigError::DexPathInvalid
        );
    }

    #[test]
    fn constructor_rejects_non_private_parent_and_wrong_owner_file_mode() {
        let fixture = Fixture::new(VALID_SCRIPT);
        fs::set_permissions(
            fixture.dex_path.parent().unwrap(),
            Permissions::from_mode(0o750),
        )
        .unwrap();
        assert_eq!(
            fixture.client().unwrap_err(),
            RishBackendConfigError::DexParentInvalid
        );

        fs::set_permissions(
            fixture.dex_path.parent().unwrap(),
            Permissions::from_mode(0o700),
        )
        .unwrap();
        for mode in [0o000, 0o200, 0o440, 0o600, 0o700] {
            fs::set_permissions(&fixture.dex_path, Permissions::from_mode(mode)).unwrap();
            assert_eq!(
                fixture.client().unwrap_err(),
                RishBackendConfigError::DexFileInvalid
            );
        }
    }

    #[test]
    fn constructor_rejects_empty_oversize_hardlinked_and_digest_mismatched_dex() {
        let fixture = Fixture::new(VALID_SCRIPT);
        fs::set_permissions(&fixture.dex_path, Permissions::from_mode(0o600)).unwrap();
        fs::write(&fixture.dex_path, []).unwrap();
        fs::set_permissions(&fixture.dex_path, Permissions::from_mode(0o400)).unwrap();
        assert_eq!(
            fixture.client().unwrap_err(),
            RishBackendConfigError::DexFileInvalid
        );

        fs::set_permissions(&fixture.dex_path, Permissions::from_mode(0o600)).unwrap();
        let oversized = File::options().write(true).open(&fixture.dex_path).unwrap();
        oversized.set_len(MAX_RISH_DEX_BYTES + 1).unwrap();
        drop(oversized);
        fs::set_permissions(&fixture.dex_path, Permissions::from_mode(0o400)).unwrap();
        assert_eq!(
            fixture.client().unwrap_err(),
            RishBackendConfigError::DexFileInvalid
        );

        fs::set_permissions(&fixture.dex_path, Permissions::from_mode(0o600)).unwrap();
        fs::write(&fixture.dex_path, b"fixed-test-rish-dex").unwrap();
        fs::set_permissions(&fixture.dex_path, Permissions::from_mode(0o400)).unwrap();
        let hardlink = fixture.dex_path.parent().unwrap().join("hardlink.dex");
        match fs::hard_link(&fixture.dex_path, &hardlink) {
            Ok(()) => {
                assert_eq!(
                    fixture.client().unwrap_err(),
                    RishBackendConfigError::DexFileInvalid
                );
                fs::remove_file(&hardlink).unwrap();
            }
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                // Android app data commonly denies hard links. The production
                // nlink != 1 check remains; this host cannot exercise it.
            }
            Err(error) => panic!("unexpected hard_link failure: {error}"),
        }

        assert_eq!(
            RishBackendClient::with_test_program(
                fixture.dex_path.clone(),
                &"0".repeat(64),
                fixture.program.clone(),
            )
            .unwrap_err(),
            RishBackendConfigError::DexDigestMismatch
        );
    }

    #[tokio::test]
    async fn every_non_shell_or_noncanonical_output_fails_closed() {
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        for output in [
            "0\n",
            "1000\n",
            "2001\n",
            "2000",
            "2000\nextra\n",
            " 2000\n",
            "not-a-uid\n",
            "\n2000\n",
        ] {
            let fixture = Fixture::new(&format!("#!/bin/sh\nprintf '{output}'\n"));
            assert_eq!(
                fixture.client().unwrap().probe().await.unwrap_err(),
                RishBackendError::IdentityOutputInvalid,
                "output {output:?} must fail closed"
            );
        }

        let fixture = Fixture::new("#!/bin/sh\nprintf 'warning\\n' >&2\nprintf '2000\\n'\n");
        assert_eq!(
            fixture.client().unwrap().probe().await.unwrap_err(),
            RishBackendError::IdentityOutputInvalid
        );
    }

    #[tokio::test]
    async fn dex_is_rehashed_and_revalidated_before_each_probe() {
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let fixture = Fixture::new(VALID_SCRIPT);
        let client = fixture.client().unwrap();
        assert!(client.probe().await.is_ok());

        fs::set_permissions(&fixture.dex_path, Permissions::from_mode(0o600)).unwrap();
        fs::write(&fixture.dex_path, b"changed-test-rish-dex").unwrap();
        fs::set_permissions(&fixture.dex_path, Permissions::from_mode(0o400)).unwrap();
        assert!(matches!(
            client.probe().await,
            Err(RishBackendError::DexIdentityChanged) | Err(RishBackendError::DexDigestMismatch)
        ));
    }

    #[tokio::test]
    async fn pinned_descriptor_rejects_path_replacement_before_execution() {
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let fixture = Fixture::new(VALID_SCRIPT);
        let client = fixture.client().unwrap();
        fs::remove_file(&fixture.dex_path).unwrap();
        fs::write(&fixture.dex_path, b"fixed-test-rish-dex").unwrap();
        fs::set_permissions(&fixture.dex_path, Permissions::from_mode(0o400)).unwrap();
        assert_eq!(
            client.probe().await.unwrap_err(),
            RishBackendError::DexIdentityChanged
        );
    }

    #[test]
    fn sealed_snapshot_rejects_same_uid_writes_and_resizing() {
        let fixture = Fixture::new(VALID_SCRIPT);
        let client = fixture.client().unwrap();
        let snapshot = try_create_sealed_memfd_snapshot(
            client.dex_file.as_ref(),
            client.dex_identity.bytes,
            client.expected_sha256,
        )
        .unwrap();
        let required_seals =
            SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE | SealFlags::SEAL;
        assert!(fcntl_get_seals(&snapshot).unwrap().contains(required_seals));
        assert!(fcntl_getfd(&snapshot).unwrap().contains(FdFlags::CLOEXEC));
        assert!(snapshot.write_at(b"attacker", 0).is_err());
        assert!(snapshot.set_len(client.dex_identity.bytes + 1).is_err());
        assert!(snapshot.set_len(client.dex_identity.bytes - 1).is_err());
        assert_eq!(
            hash_exact_file(&snapshot, client.dex_identity.bytes).unwrap(),
            client.expected_sha256
        );
    }

    #[test]
    fn private_tmpfile_snapshot_is_art_openable_and_digest_matched() {
        // Explicitly exercises the O_TMPFILE / named fallback path used when
        // sealed memfd path-reopen is unusable (common on Android kernels).
        let fixture = Fixture::new(VALID_SCRIPT);
        let client = fixture.client().unwrap();
        let snapshot = create_private_tmpfile_snapshot(
            client.dex_file.as_ref(),
            client.dex_identity.bytes,
            client.expected_sha256,
        )
        .unwrap();
        assert!(fcntl_getfd(&snapshot).unwrap().contains(FdFlags::CLOEXEC));
        assert!(snapshot_classpath_path_is_usable(
            &snapshot,
            client.dex_identity.bytes,
            client.expected_sha256,
        ));
        assert_eq!(
            hash_exact_file(&snapshot, client.dex_identity.bytes).unwrap(),
            client.expected_sha256
        );
        // Mode 0400 on the reopen path: same-UID writes through the classpath
        // path must fail after finalize.
        let path = format!("/proc/self/fd/{}", snapshot.as_raw_fd());
        assert!(File::options().write(true).open(&path).is_err());
    }

    #[test]
    fn private_snapshot_directory_is_owner_private_at_creation() {
        let dir = private_snapshot_directory().unwrap();
        let metadata = fs::symlink_metadata(&dir).unwrap();
        assert!(metadata.file_type().is_dir());
        assert_eq!(metadata.uid(), getuid().as_raw());
        assert_eq!(metadata.mode() & 0o777, 0o700);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn execution_snapshot_prefers_usable_classpath_path() {
        let fixture = Fixture::new(VALID_SCRIPT);
        let client = fixture.client().unwrap();
        let snapshot = create_execution_dex_snapshot(
            client.dex_file.as_ref(),
            client.dex_identity.bytes,
            client.expected_sha256,
        )
        .unwrap();
        assert!(snapshot_classpath_path_is_usable(
            &snapshot,
            client.dex_identity.bytes,
            client.expected_sha256,
        ));
    }

    #[tokio::test]
    async fn child_executes_sealed_bytes_when_same_uid_source_is_rewritten() {
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let fixture = Fixture::new(
            r#"#!/bin/sh
dex="${1#-Djava.class.path=}"
original="${0%/*}/rish_shizuku.dex"
# Rewrite the operator source after the parent has already copied a snapshot.
chmod 600 "$original" || exit 51
printf 'same-uid-attacker-bytes' >"$original" || exit 52
chmod 400 "$original" || exit 53
# ART opens the classpath path. The child must still observe the original
# digest-matched snapshot bytes even after the source rewrite.
content=$(cat "$dex" 2>/dev/null) || exit 55
[ "$content" = "fixed-test-rish-dex" ] || exit 56
printf '2000\n'
"#,
        );
        let client = fixture.client().unwrap();
        let first = client.probe().await;
        assert!(first.is_ok(), "first probe failed: {first:?}");
        assert!(matches!(
            client.probe().await,
            Err(RishBackendError::DexIdentityChanged) | Err(RishBackendError::DexDigestMismatch)
        ));
    }

    #[tokio::test]
    async fn a_second_probe_does_not_queue_behind_the_single_lane() {
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let fixture = Fixture::new("#!/bin/sh\n/bin/sleep 1\nprintf '2000\\n'\n");
        let client = fixture.client().unwrap();
        let active = client.clone();
        let first = tokio::spawn(async move { active.probe().await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            client.probe().await.unwrap_err(),
            RishBackendError::ConcurrencyLimitExceeded
        );
        assert!(first.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn cancelled_waiter_cannot_release_the_active_probe_lane() {
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let fixture = Fixture::new(VALID_SCRIPT);
        let mut client = fixture.client().unwrap();
        client.validation_delay = Duration::from_millis(500);
        let active = client.clone();
        let waiter = tokio::spawn(async move { active.probe().await });
        for _ in 0..100 {
            if client.concurrency.available_permits() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(client.concurrency.available_permits(), 0);
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());

        assert_eq!(
            client.probe().await.unwrap_err(),
            RishBackendError::ConcurrencyLimitExceeded
        );
        for _ in 0..300 {
            if client.concurrency.available_permits() == RISH_STATUS_CONCURRENCY {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            client.concurrency.available_permits(),
            RISH_STATUS_CONCURRENCY
        );
        assert!(client.probe().await.is_ok());
    }

    #[test]
    fn debug_and_errors_never_disclose_path_descriptor_or_digest() {
        let fixture = Fixture::new(VALID_SCRIPT);
        let client = fixture.client().unwrap();
        let debug = format!("{client:?}");
        assert!(!debug.contains(fixture.dex_path.to_string_lossy().as_ref()));
        assert!(!debug.contains(&fixture.digest));
        assert!(debug.contains("<redacted>"));

        for (error, code) in [
            (
                RishBackendError::DexDigestFormatInvalid,
                "rish_dex_digest_format_invalid",
            ),
            (RishBackendError::DexPathInvalid, "rish_dex_path_invalid"),
            (
                RishBackendError::DexParentInvalid,
                "rish_dex_parent_invalid",
            ),
            (RishBackendError::DexFileInvalid, "rish_dex_file_invalid"),
            (
                RishBackendError::DexDigestMismatch,
                "rish_dex_digest_mismatch",
            ),
            (
                RishBackendError::DexDescriptorInvalid,
                "rish_dex_descriptor_invalid",
            ),
            (
                RishBackendError::DexIdentityChanged,
                "rish_dex_identity_changed",
            ),
            (
                RishBackendError::DexSnapshotFailed,
                "rish_dex_snapshot_failed",
            ),
            (
                RishBackendError::ConcurrencyLimitExceeded,
                "rish_probe_concurrency_limit",
            ),
            (
                RishBackendError::ProgramUnavailable,
                "rish_app_process_unavailable",
            ),
            (RishBackendError::SpawnFailed, "rish_probe_spawn_failed"),
            (RishBackendError::WaitFailed, "rish_probe_wait_failed"),
            (RishBackendError::TimedOut, "rish_probe_timeout"),
            (
                RishBackendError::StdoutLimitExceeded,
                "rish_probe_stdout_limit_exceeded",
            ),
            (
                RishBackendError::StderrLimitExceeded,
                "rish_probe_stderr_limit_exceeded",
            ),
            (RishBackendError::ProgramFailed, "rish_probe_failed"),
            (
                RishBackendError::IdentityOutputInvalid,
                "rish_probe_identity_invalid",
            ),
            (
                RishBackendError::AttestationOutputInvalid,
                "rish_attestation_output_invalid",
            ),
            (RishBackendError::WorkerFailed, "rish_probe_worker_failed"),
        ] {
            assert_eq!(error.reason_code(), code);
            assert_eq!(error.to_string(), code);
            assert_eq!(format!("{error:?}"), format!("{error:?}"));
            assert!(!code.contains('/'));
        }
    }

    #[tokio::test]
    async fn s3_attestation_mints_private_epoch_without_public_secrets() {
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let fixture = Fixture::new(S3_VALID_SCRIPT);
        let client = fixture.client().unwrap();
        assert!(!client.has_live_s3_epoch());
        let status = client.attest_read_only().await.unwrap();
        assert_eq!(
            status,
            RishBackendStatus {
                available: true,
                backend: "shizuku_rish",
                principal: "android_shell",
                uid: 2000,
                state: "attested_read_only",
                root_accepted: false,
                arbitrary_shell: false,
                mutation_ready: false,
            }
        );
        assert!(client.has_live_s3_epoch());
        let public = serde_json::to_value(status).unwrap();
        assert_eq!(public["state"], "attested_read_only");
        assert!(public.get("backendEpoch").is_none());
        assert!(public.get("buildFingerprint").is_none());
        assert!(public.get("bootId").is_none());
        assert!(public.get("selinuxContext").is_none());
        assert!(public.get("groups").is_none());
        let debug = format!("{client:?}");
        assert!(!debug.contains("01234567-89ab-cdef-0123-456789abcdef"));
        assert!(!debug.contains("google/test/test"));
        assert!(!debug.contains("u:r:shell:s0"));
    }

    #[tokio::test]
    async fn s3_attestation_rejects_non_shell_gid_and_invalidates_epoch() {
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let fixture = Fixture::new(
            r#"#!/bin/sh
case "$6" in
  "exec /system/bin/id -u") printf '2000\n' ;;
  "exec /system/bin/id -g") printf '1000\n' ;;
  *) printf '2000\n' ;;
esac
"#,
        );
        let client = fixture.client().unwrap();
        // Seed a live epoch, then prove failure rotates it away.
        {
            let mut slot = client
                .private_epoch
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *slot = Some(PrivateBackendEpoch {
                generation: 1,
                dex_sha256: [0; 32],
                build_fingerprint_sha256: [0; 32],
                boot_id_sha256: [0; 32],
                selinux_context_sha256: [0; 32],
                sdk: 34,
                gid: 2000,
                groups: vec![2000],
            });
        }
        assert!(client.has_live_s3_epoch());
        assert_eq!(
            client.attest_read_only().await.unwrap_err(),
            RishBackendError::AttestationOutputInvalid
        );
        assert!(!client.has_live_s3_epoch());
    }

    #[test]
    fn s3_parsers_fail_closed_on_malformed_identity_material() {
        assert_eq!(parse_exact_u32_line(b"2000\n").unwrap(), 2000);
        assert!(parse_exact_u32_line(b"2000").is_err());
        assert!(parse_exact_u32_line(b" 2000\n").is_err());
        assert!(parse_exact_u32_line(b"2000\nextra\n").is_err());
        assert!(parse_exact_u32_line(b"0x2000\n").is_err());

        assert_eq!(
            parse_group_list_line(b"2000 1004 3003\n").unwrap(),
            vec![2000, 1004, 3003]
        );
        assert!(parse_group_list_line(b"2000\n").is_ok());
        assert!(parse_group_list_line(b"2000  \n").is_err());
        assert!(parse_group_list_line(b"2000,-1\n").is_err());

        assert_eq!(
            parse_shell_selinux_context(b"u:r:shell:s0\n").unwrap(),
            b"u:r:shell:s0"
        );
        assert_eq!(
            parse_shell_selinux_context(b"u:r:shell:s0\0").unwrap(),
            b"u:r:shell:s0"
        );
        assert!(parse_shell_selinux_context(b"u:r:system_server:s0\n").is_err());
        assert!(parse_shell_selinux_context(b"u:r:shell:s0;id\n").is_err());

        assert!(parse_build_fingerprint(
            b"google/test/test:14/UQ1A.000000.000/0000000:user/release-keys\n"
        )
        .is_ok());
        assert!(parse_build_fingerprint(
            b"google/test/test:14/UQ1A.000000.000/0000000:user/release-keys"
        )
        .is_ok());
        assert!(parse_build_fingerprint(b"\n").is_err());
        assert!(parse_build_fingerprint(b"has space\n").is_err());

        assert!(parse_boot_id(b"01234567-89ab-cdef-0123-456789abcdef\n").is_ok());
        assert!(parse_boot_id(b"01234567-89AB-CDEF-0123-456789ABCDEF\n").is_err());
        assert!(parse_boot_id(b"not-a-uuid\n").is_err());
    }

    #[tokio::test]
    async fn list_system_features_returns_allowlisted_booleans() {
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let fixture = Fixture::new(S3_VALID_SCRIPT);
        let client = fixture.client().unwrap();
        let status = client.list_system_features().await.unwrap();
        assert_eq!(status.state, "attested_read_only");
        assert!(!status.control_authority_proven);
        assert!(!status.mutation_ready);
        assert!(status.features.wifi);
        assert!(status.features.webview);
        assert!(!status.features.nfc); // not in S3_VALID_SCRIPT fixture list
        assert!(client.has_live_s3_epoch());
    }
}
