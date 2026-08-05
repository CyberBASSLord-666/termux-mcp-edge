//! Diagnostic, read-only Shizuku/rish UID-token probe.
//!
//! This module deliberately does not expose a general shell. This diagnostic
//! backend uses one fixed Android runtime, one pinned and digest-bound loader
//! DEX, one fixed loader class, and one fixed command which evaluates an exact
//! UID-2000 predicate. Stock rish races two local readers over one remote output
//! pipe and does not expose a trustworthy remote exit status. The accepted token is
//! therefore only a local diagnostic observation; it does not prove remote
//! stream separation, orderly exit, Binder identity/lifecycle, or revocation.
//! The configured DEX descriptor
//! remains pinned for the client's lifetime. Each probe copies its revalidated
//! bytes into an execution snapshot and passes only that snapshot through
//! `/proc/self/fd` so pathname replacement of the operator source cannot change
//! the bytes `app_process64` loads. Only a fully sealed memfd snapshot is
//! executable. Android kernels that refuse to reopen that path for ART fail
//! closed: owner-private tmpfiles remain mutable by the same UID and cannot
//! satisfy this boundary.

use std::{
    ffi::OsString,
    fmt,
    fs::{self, File, Metadata},
    os::{
        fd::AsRawFd,
        unix::ffi::OsStrExt,
        unix::fs::{FileExt, MetadataExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
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
pub(crate) const RISH_STATUS_STDOUT_BYTES: usize = 1024;
pub(crate) const RISH_STATUS_STDERR_BYTES: usize = 4 * 1024;
pub(crate) const RISH_STATUS_CONCURRENCY: usize = 1;

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
/// S2.5 foundation command: evaluate the fixed UID-2000 predicate only.
const RISH_FIXED_COMMAND: &str = "exec /system/bin/id -u";
const ANDROID_SHELL_UID: u32 = 2000;
const EXPECTED_UID_TOKEN: &[u8] = b"2000\n";

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
    #[cfg(test)]
    validation_delay: Duration,
    #[cfg(test)]
    process_cleanup_delay: Duration,
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
            #[cfg(test)]
            validation_delay: Duration::ZERO,
            #[cfg(test)]
            process_cleanup_delay: Duration::ZERO,
        })
    }

    /// S2.5 foundation probe: exact UID-2000 predicate token only.
    ///
    /// The public state label remains `verified_shell_uid` for the existing v1
    /// evidence contract. Success means only that exact `2000\n` bytes were
    /// captured in one local reader while the other capture was empty. It does
    /// not qualify remote exit status, stream separation, or Binder lifecycle.
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
        #[cfg(test)]
        let process_cleanup_delay = self.process_cleanup_delay;

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

        run_fixed_probe(
            program,
            dex_guard,
            permit,
            #[cfg(test)]
            process_cleanup_delay,
        )
        .await
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

async fn run_fixed_probe(
    program: PathBuf,
    dex_guard: Arc<File>,
    permit: OwnedSemaphorePermit,
    #[cfg(test)] process_cleanup_delay: Duration,
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
    #[cfg(test)]
    let process = process.with_forced_cleanup_delay(process_cleanup_delay);
    let output = process.run_with_completion_guard(permit).await?;
    drop(dex_guard);

    if !uid_token_is_exactly_one_capture(&output.stdout, &output.stderr) {
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

fn uid_token_is_exactly_one_capture(stdout: &[u8], stderr: &[u8]) -> bool {
    (stdout == EXPECTED_UID_TOKEN && stderr.is_empty())
        || (stderr == EXPECTED_UID_TOKEN && stdout.is_empty())
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
    let snapshot = try_create_sealed_memfd_snapshot(source, bytes, expected_sha256)?;
    ensure_execution_snapshot_qualified(&snapshot, bytes, expected_sha256)?;
    Ok(snapshot)
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

    fcntl_add_seals(&snapshot, required_snapshot_seals())
        .map_err(|_| RishBackendError::DexSnapshotFailed)?;
    let observed_seals =
        fcntl_get_seals(&snapshot).map_err(|_| RishBackendError::DexSnapshotFailed)?;
    if !observed_seals.contains(required_snapshot_seals()) {
        return Err(RishBackendError::DexSnapshotFailed);
    }
    if hash_exact_file(&snapshot, bytes)? != expected_sha256 {
        return Err(RishBackendError::DexSnapshotFailed);
    }
    Ok(snapshot)
}

fn ensure_execution_snapshot_qualified(
    snapshot: &File,
    bytes: u64,
    expected_sha256: [u8; 32],
) -> Result<(), RishBackendError> {
    if !snapshot_is_fully_sealed(snapshot)
        || !snapshot_classpath_path_is_usable(snapshot, bytes, expected_sha256)
    {
        return Err(RishBackendError::DexSnapshotFailed);
    }
    Ok(())
}

fn required_snapshot_seals() -> SealFlags {
    SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE | SealFlags::SEAL
}

fn snapshot_is_fully_sealed(snapshot: &File) -> bool {
    fcntl_get_seals(snapshot).is_ok_and(|seals| seals.contains(required_snapshot_seals()))
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
    use crate::bounded_process::{active_supervisor_count, BOUNDED_PROCESS_TEST_LOCK};

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
# S2.5 exposes only the fixed UID predicate.
[ "$6" = "exec /system/bin/id -u" ] || exit 40
printf '2000\n'
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

    async fn wait_for_valid_pid(path: &Path) -> u32 {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match fs::read_to_string(path)
                    .ok()
                    .and_then(|contents| contents.trim().parse::<u32>().ok())
                {
                    Some(pid) if pid > 0 => return pid,
                    _ => tokio::task::yield_now().await,
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("PID fixture did not become valid: {}", path.display()))
    }

    #[tokio::test]
    async fn fixed_probe_observes_environment_arguments_descriptor_and_uid_token() {
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
    async fn uid_token_is_accepted_from_exactly_one_stock_rish_capture() {
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let stdout = Fixture::new("#!/bin/sh\nprintf '2000\\n'\n");
        assert!(stdout.client().unwrap().probe().await.is_ok());

        let stderr = Fixture::new("#!/bin/sh\nprintf '2000\\n' >&2\n");
        assert!(stderr.client().unwrap().probe().await.is_ok());
    }

    #[tokio::test]
    async fn malformed_split_duplicated_or_injected_uid_tokens_fail_closed() {
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        for output in [
            "",
            "0\n",
            "1000\n",
            "2001\n",
            "2000",
            "2000\r\n",
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

        for script in [
            "#!/bin/sh\nprintf 'warning\\n' >&2\nprintf '2000\\n'\n",
            "#!/bin/sh\nprintf '2000\\n' >&2\nprintf '2000\\n'\n",
            "#!/bin/sh\nprintf '20'\nprintf '00\\n' >&2\n",
            "#!/bin/sh\nprintf '2000\\nextra\\n' >&2\n",
        ] {
            let fixture = Fixture::new(script);
            assert_eq!(
                fixture.client().unwrap().probe().await.unwrap_err(),
                RishBackendError::IdentityOutputInvalid
            );
        }

        let nonzero = Fixture::new("#!/bin/sh\nprintf '2000\\n'\nexit 73\n");
        assert_eq!(
            nonzero.client().unwrap().probe().await.unwrap_err(),
            RishBackendError::ProgramFailed
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
    fn owner_private_regular_snapshot_is_rejected_as_mutable() {
        let fixture = Fixture::new(VALID_SCRIPT);
        let path = fixture._directory.path().join("owner-private-snapshot.dex");
        fs::write(&path, b"fixed-test-rish-dex").unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o400)).unwrap();
        let ordinary = File::open(&path).unwrap();

        assert!(!snapshot_is_fully_sealed(&ordinary));
        assert_eq!(
            ensure_execution_snapshot_qualified(
                &ordinary,
                b"fixed-test-rish-dex".len() as u64,
                Sha256::digest(b"fixed-test-rish-dex").into(),
            ),
            Err(RishBackendError::DexSnapshotFailed)
        );

        // Mode 0400 is not immutability: the same owner can restore write
        // permission and replace the bytes. Such files must never reach ART.
        fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, b"same-uid-replacement").unwrap();
    }

    #[test]
    fn execution_snapshot_is_fully_sealed_and_art_reopenable() {
        let fixture = Fixture::new(VALID_SCRIPT);
        let client = fixture.client().unwrap();
        let snapshot = create_execution_dex_snapshot(
            client.dex_file.as_ref(),
            client.dex_identity.bytes,
            client.expected_sha256,
        )
        .unwrap();
        assert!(snapshot_is_fully_sealed(&snapshot));
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

    #[tokio::test]
    async fn cancelled_probe_retains_lane_until_child_cleanup_finishes() {
        // This covers only the local bounded-process supervisor: its direct and
        // descendant PIDs are gone before the permit returns. Stock rish still
        // supplies no authoritative remote Binder lifecycle or revocation fact.
        let _test_lock = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let fixture = Fixture::new(
            r#"#!/bin/sh
root="${0%/*}"
printf '%s\n' "$$" >"$root/direct.pid"
/bin/sleep 30 &
descendant=$!
printf '%s\n' "$descendant" >"$root/descendant.pid"
wait "$descendant"
printf '2000\n'
"#,
        );
        let mut client = fixture.client().unwrap();
        client.process_cleanup_delay = Duration::from_millis(500);

        let active = client.clone();
        let waiter = tokio::spawn(async move { active.probe().await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while active_supervisor_count() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("rish child supervisor did not start");
        let direct_pid_path = fixture._directory.path().join("direct.pid");
        let descendant_pid_path = fixture._directory.path().join("descendant.pid");
        let (direct_pid, descendant_pid) = tokio::join!(
            wait_for_valid_pid(&direct_pid_path),
            wait_for_valid_pid(&descendant_pid_path),
        );

        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert_eq!(client.concurrency.available_permits(), 0);
        assert_eq!(
            client.probe().await.unwrap_err(),
            RishBackendError::ConcurrencyLimitExceeded
        );

        tokio::time::timeout(Duration::from_secs(2), async {
            while active_supervisor_count() != 0
                || client.concurrency.available_permits() != RISH_STATUS_CONCURRENCY
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("rish child cleanup did not release the operation lane");
        tokio::time::timeout(Duration::from_secs(2), async {
            while Path::new(&format!("/proc/{direct_pid}")).exists()
                || Path::new(&format!("/proc/{descendant_pid}")).exists()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("rish direct or descendant child survived cleanup");
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
            (RishBackendError::WorkerFailed, "rish_probe_worker_failed"),
        ] {
            assert_eq!(error.reason_code(), code);
            assert_eq!(error.to_string(), code);
            assert_eq!(format!("{error:?}"), format!("{error:?}"));
            assert!(!code.contains('/'));
        }
    }
}
