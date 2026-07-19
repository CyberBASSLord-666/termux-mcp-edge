//! Bounded execution client for reviewed read-only command profiles.
//!
//! Production profiles invoke only the kernel-pinned currently running server
//! image with project-owned argv. The caller cannot select a program, argument,
//! working directory, environment value, timeout, or output limit.

use std::{
    ffi::{OsStr, OsString},
    os::fd::{AsRawFd, OwnedFd},
    path::PathBuf,
    sync::Arc,
};

use rustix::fs::{fstat, open, FileType, Mode, OFlags};
use serde::Serialize;
use tokio::sync::Semaphore;

use crate::{
    bounded_process::{BoundedProcess, BoundedProcessConfigError, BoundedProcessError},
    command_policy::CommandProfile,
};

pub const MAX_CONCURRENT_COMMAND_PROFILES: usize = 2;
const EXPECTED_SERVER_EXECUTABLE_NAME: &str = env!("CARGO_PKG_NAME");
const PINNED_CURRENT_EXECUTABLE: &str = "/proc/self/exe";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandClientConfigError {
    CurrentExecutableUnavailable,
    CurrentExecutableIdentityMismatch,
    ProgramMustBeAbsolute,
    WorkingDirectoryMustBeSafeRooted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandExecutionError {
    ProgramUnavailable,
    SpawnFailed,
    WaitFailed,
    TimedOut,
    StdoutLimitExceeded,
    StderrLimitExceeded,
    ProgramFailed,
    InvalidUtf8,
    ConcurrencyLimitExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandExecutionResult {
    pub profile: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub duration_milliseconds: u64,
}

impl From<BoundedProcessError> for CommandExecutionError {
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

#[derive(Clone)]
pub(crate) struct CommandExecutionClient {
    program: PathBuf,
    working_directory: PathBuf,
    working_directory_guard: Arc<OwnedFd>,
    concurrency: Arc<Semaphore>,
}

impl std::fmt::Debug for CommandExecutionClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommandExecutionClient")
            .field("program", &"<redacted>")
            .field("working_directory", &"<redacted>")
            .field("working_directory_guard", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl CommandExecutionClient {
    pub(crate) fn current_server(
        working_directory: OwnedFd,
    ) -> Result<Self, CommandClientConfigError> {
        let program = std::env::current_exe()
            .map_err(|_| CommandClientConfigError::CurrentExecutableUnavailable)?;
        Self::exact_server_program(program, working_directory)
    }

    fn exact_server_program(
        program: PathBuf,
        working_directory: OwnedFd,
    ) -> Result<Self, CommandClientConfigError> {
        if !program.is_absolute()
            || program.file_name() != Some(OsStr::new(EXPECTED_SERVER_EXECUTABLE_NAME))
        {
            return Err(CommandClientConfigError::CurrentExecutableIdentityMismatch);
        }
        let loaded = open(
            PINNED_CURRENT_EXECUTABLE,
            OFlags::PATH | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| CommandClientConfigError::CurrentExecutableUnavailable)?;
        let loaded =
            fstat(&loaded).map_err(|_| CommandClientConfigError::CurrentExecutableUnavailable)?;
        let candidate = open(
            &program,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| CommandClientConfigError::CurrentExecutableIdentityMismatch)?;
        let candidate = fstat(&candidate)
            .map_err(|_| CommandClientConfigError::CurrentExecutableIdentityMismatch)?;
        if !FileType::from_raw_mode(candidate.st_mode).is_file()
            || !FileType::from_raw_mode(loaded.st_mode).is_file()
            || candidate.st_mode & 0o111 == 0
            || loaded.st_mode & 0o111 == 0
            || candidate.st_dev != loaded.st_dev
            || candidate.st_ino != loaded.st_ino
        {
            return Err(CommandClientConfigError::CurrentExecutableIdentityMismatch);
        }
        // Retain no reopenable installation path. The descriptor comparison
        // proves that the exact-name candidate is the already-loaded image;
        // every later spawn remains bound to that inode through /proc/self/exe.
        Self::new(PathBuf::from(PINNED_CURRENT_EXECUTABLE), working_directory)
    }

    fn new(
        program: PathBuf,
        working_directory_guard: OwnedFd,
    ) -> Result<Self, CommandClientConfigError> {
        if !program.is_absolute() {
            return Err(CommandClientConfigError::ProgramMustBeAbsolute);
        }
        let working_directory_identity = fstat(&working_directory_guard)
            .map_err(|_| CommandClientConfigError::WorkingDirectoryMustBeSafeRooted)?;
        let filesystem_root = open(
            "/",
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| CommandClientConfigError::WorkingDirectoryMustBeSafeRooted)?;
        let filesystem_root_identity = fstat(&filesystem_root)
            .map_err(|_| CommandClientConfigError::WorkingDirectoryMustBeSafeRooted)?;
        if !FileType::from_raw_mode(working_directory_identity.st_mode).is_dir()
            || (working_directory_identity.st_dev == filesystem_root_identity.st_dev
                && working_directory_identity.st_ino == filesystem_root_identity.st_ino)
        {
            return Err(CommandClientConfigError::WorkingDirectoryMustBeSafeRooted);
        }

        let working_directory = PathBuf::from(format!(
            "/proc/self/fd/{}",
            working_directory_guard.as_raw_fd()
        ));
        let reopened = open(
            &working_directory,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| CommandClientConfigError::WorkingDirectoryMustBeSafeRooted)?;
        let reopened = fstat(&reopened)
            .map_err(|_| CommandClientConfigError::WorkingDirectoryMustBeSafeRooted)?;
        if reopened.st_dev != working_directory_identity.st_dev
            || reopened.st_ino != working_directory_identity.st_ino
        {
            return Err(CommandClientConfigError::WorkingDirectoryMustBeSafeRooted);
        }

        Ok(Self {
            program,
            working_directory,
            working_directory_guard: Arc::new(working_directory_guard),
            concurrency: Arc::new(Semaphore::new(MAX_CONCURRENT_COMMAND_PROFILES)),
        })
    }

    pub(crate) async fn execute(
        &self,
        profile: &'static CommandProfile,
    ) -> Result<CommandExecutionResult, CommandExecutionError> {
        let _permit = self
            .concurrency
            .clone()
            .try_acquire_owned()
            .map_err(|_| CommandExecutionError::ConcurrencyLimitExceeded)?;
        // Keep a distinct strong reference through spawn and supervision so the
        // /proc/self/fd working-directory handle cannot close or be reused.
        let working_directory_guard = Arc::clone(&self.working_directory_guard);
        let process = BoundedProcess::new(
            self.program.clone(),
            profile.argv().iter().map(OsString::from).collect(),
            self.working_directory.clone(),
            profile.timeout(),
            profile.max_stdout_bytes(),
            profile.max_stderr_bytes(),
        )
        .map_err(|error| match error {
            BoundedProcessConfigError::TimeoutTooShort
            | BoundedProcessConfigError::TimeoutTooLong
            | BoundedProcessConfigError::StdoutLimitTooLarge
            | BoundedProcessConfigError::StderrLimitTooLarge => CommandExecutionError::WaitFailed,
        })?;
        let output = process.run().await;
        drop(working_directory_guard);
        let output = output?;
        let stdout_bytes = output.stdout.len();
        let stderr_bytes = output.stderr.len();
        let stdout =
            String::from_utf8(output.stdout).map_err(|_| CommandExecutionError::InvalidUtf8)?;
        let stderr =
            String::from_utf8(output.stderr).map_err(|_| CommandExecutionError::InvalidUtf8)?;
        let duration_milliseconds = u64::try_from(output.duration.as_millis()).unwrap_or(u64::MAX);

        Ok(CommandExecutionResult {
            profile: profile.id().to_owned(),
            exit_code: 0,
            stdout,
            stderr,
            stdout_bytes,
            stderr_bytes,
            duration_milliseconds,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_program_and_concurrency(
        program: PathBuf,
        working_directory: PathBuf,
        concurrency_limit: usize,
    ) -> Result<Self, CommandClientConfigError> {
        let mut client = Self::new_for_test(program, working_directory)?;
        client.concurrency = Arc::new(Semaphore::new(concurrency_limit));
        Ok(client)
    }

    #[cfg(test)]
    fn new_for_test(
        program: PathBuf,
        working_directory: PathBuf,
    ) -> Result<Self, CommandClientConfigError> {
        let working_directory = open_test_working_directory(&working_directory)?;
        Self::new(program, working_directory)
    }

    #[cfg(test)]
    fn exact_server_program_for_test(
        program: PathBuf,
        working_directory: PathBuf,
    ) -> Result<Self, CommandClientConfigError> {
        let working_directory = open_test_working_directory(&working_directory)?;
        Self::exact_server_program(program, working_directory)
    }
}

#[cfg(test)]
fn open_test_working_directory(
    path: &std::path::Path,
) -> Result<OwnedFd, CommandClientConfigError> {
    if !path.is_absolute() {
        return Err(CommandClientConfigError::WorkingDirectoryMustBeSafeRooted);
    }
    open(
        path,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| CommandClientConfigError::WorkingDirectoryMustBeSafeRooted)
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        bounded_process::{
            BOUNDED_PROCESS_TEST_LOCK, MAX_BOUNDED_PROCESS_STDERR_BYTES,
            MAX_BOUNDED_PROCESS_STDOUT_BYTES, MAX_BOUNDED_PROCESS_TIMEOUT,
        },
        command_policy::{
            command_profile, command_profile_ids, test_command_profile, CommandProfile,
        },
        tools::FileSystemTools,
    };

    fn executable(script: &str) -> (TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("fixed-program");
        fs::write(&program, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        (directory, program)
    }

    fn profile_with_bounds(
        argv: &'static [&'static str],
        timeout: Duration,
        stdout: usize,
        stderr: usize,
    ) -> &'static CommandProfile {
        test_command_profile(argv, timeout, stdout, stderr)
    }

    #[test]
    fn current_server_candidate_requires_exact_regular_executable_identity() {
        let safe_root = tempfile::tempdir().unwrap();
        let program_root = tempfile::tempdir().unwrap();
        let wrong_name = program_root.path().join("embedding-binary");
        fs::write(&wrong_name, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&wrong_name, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            CommandExecutionClient::exact_server_program_for_test(
                wrong_name,
                safe_root.path().to_path_buf(),
            )
            .unwrap_err(),
            CommandClientConfigError::CurrentExecutableIdentityMismatch
        );

        let non_regular_root = tempfile::tempdir().unwrap();
        let exact_directory = non_regular_root
            .path()
            .join(EXPECTED_SERVER_EXECUTABLE_NAME);
        fs::create_dir(&exact_directory).unwrap();
        assert_eq!(
            CommandExecutionClient::exact_server_program_for_test(
                exact_directory,
                safe_root.path().to_path_buf(),
            )
            .unwrap_err(),
            CommandClientConfigError::CurrentExecutableIdentityMismatch
        );

        let symlink_root = tempfile::tempdir().unwrap();
        let symlink_target = symlink_root.path().join("target");
        fs::write(&symlink_target, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&symlink_target, fs::Permissions::from_mode(0o700)).unwrap();
        let exact_symlink = symlink_root.path().join(EXPECTED_SERVER_EXECUTABLE_NAME);
        std::os::unix::fs::symlink(&symlink_target, &exact_symlink).unwrap();
        assert_eq!(
            CommandExecutionClient::exact_server_program_for_test(
                exact_symlink,
                safe_root.path().to_path_buf(),
            )
            .unwrap_err(),
            CommandClientConfigError::CurrentExecutableIdentityMismatch
        );

        let exact = program_root.path().join(EXPECTED_SERVER_EXECUTABLE_NAME);
        fs::write(&exact, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&exact, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            CommandExecutionClient::exact_server_program_for_test(
                exact.clone(),
                safe_root.path().to_path_buf(),
            )
            .unwrap_err(),
            CommandClientConfigError::CurrentExecutableIdentityMismatch
        );

        fs::set_permissions(&exact, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            CommandExecutionClient::exact_server_program_for_test(
                exact,
                safe_root.path().to_path_buf(),
            )
            .unwrap_err(),
            CommandClientConfigError::CurrentExecutableIdentityMismatch
        );

        let current_executable = std::env::current_exe().unwrap();
        let hard_link_root = tempfile::tempdir_in(current_executable.parent().unwrap()).unwrap();
        let loaded_identity = hard_link_root.path().join(EXPECTED_SERVER_EXECUTABLE_NAME);
        fs::hard_link(&current_executable, &loaded_identity).unwrap();
        let client = CommandExecutionClient::exact_server_program_for_test(
            loaded_identity,
            safe_root.path().to_path_buf(),
        )
        .unwrap();
        assert_eq!(client.program, PathBuf::from(PINNED_CURRENT_EXECUTABLE));
        assert!(client
            .working_directory
            .starts_with(std::path::Path::new("/proc/self/fd")));
    }

    #[test]
    fn every_policy_profile_fits_the_supervisor_hard_maxima() {
        for profile_id in command_profile_ids() {
            let profile = command_profile(profile_id).expect("registry IDs resolve");
            assert!(profile.timeout() <= MAX_BOUNDED_PROCESS_TIMEOUT);
            assert!(profile.max_stdout_bytes() <= MAX_BOUNDED_PROCESS_STDOUT_BYTES);
            assert!(profile.max_stderr_bytes() <= MAX_BOUNDED_PROCESS_STDERR_BYTES);
        }
    }

    #[tokio::test]
    async fn forged_profile_bounds_fail_before_program_spawn() {
        let safe_root = tempfile::tempdir().unwrap();
        let marker = safe_root.path().join("must-not-exist");
        let script = format!("touch '{}'", marker.display());
        let (_program_root, program) = executable(&script);
        let client =
            CommandExecutionClient::new_for_test(program, safe_root.path().to_path_buf()).unwrap();

        for profile in [
            profile_with_bounds(
                &[],
                MAX_BOUNDED_PROCESS_TIMEOUT + Duration::from_millis(1),
                MAX_BOUNDED_PROCESS_STDOUT_BYTES,
                MAX_BOUNDED_PROCESS_STDERR_BYTES,
            ),
            profile_with_bounds(
                &[],
                MAX_BOUNDED_PROCESS_TIMEOUT,
                MAX_BOUNDED_PROCESS_STDOUT_BYTES + 1,
                MAX_BOUNDED_PROCESS_STDERR_BYTES,
            ),
            profile_with_bounds(
                &[],
                MAX_BOUNDED_PROCESS_TIMEOUT,
                MAX_BOUNDED_PROCESS_STDOUT_BYTES,
                MAX_BOUNDED_PROCESS_STDERR_BYTES + 1,
            ),
        ] {
            assert_eq!(
                client.execute(profile).await.unwrap_err(),
                CommandExecutionError::WaitFailed
            );
        }
        assert!(!marker.exists());
    }

    #[test]
    fn client_requires_absolute_program_and_narrow_existing_working_directory() {
        let safe_root = tempfile::tempdir().unwrap();
        assert_eq!(
            CommandExecutionClient::new_for_test(
                PathBuf::from("relative"),
                safe_root.path().to_path_buf()
            )
            .unwrap_err(),
            CommandClientConfigError::ProgramMustBeAbsolute
        );
        assert_eq!(
            CommandExecutionClient::new_for_test(
                PathBuf::from("/program"),
                PathBuf::from("relative")
            )
            .unwrap_err(),
            CommandClientConfigError::WorkingDirectoryMustBeSafeRooted
        );
        assert_eq!(
            CommandExecutionClient::new_for_test(PathBuf::from("/program"), PathBuf::from("/"))
                .unwrap_err(),
            CommandClientConfigError::WorkingDirectoryMustBeSafeRooted
        );
        assert_eq!(
            CommandExecutionClient::new_for_test(
                PathBuf::from("/program"),
                PathBuf::from("/tmp/.."),
            )
            .unwrap_err(),
            CommandClientConfigError::WorkingDirectoryMustBeSafeRooted
        );

        let link_root = tempfile::tempdir().unwrap();
        let linked_root = link_root.path().join("linked-safe-root");
        std::os::unix::fs::symlink(safe_root.path(), &linked_root).unwrap();
        assert_eq!(
            CommandExecutionClient::new_for_test(PathBuf::from("/program"), linked_root)
                .unwrap_err(),
            CommandClientConfigError::WorkingDirectoryMustBeSafeRooted
        );
    }

    #[tokio::test]
    async fn working_directory_descriptor_survives_path_replacement() {
        let _guard = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let root = tempfile::tempdir().unwrap();
        let safe_root = root.path().join("safe-root");
        let retained_root = root.path().join("retained-safe-root");
        fs::create_dir(&safe_root).unwrap();
        fs::write(safe_root.join("retained-marker"), b"retained").unwrap();

        let (_program_root, program) = executable(
            "test -f retained-marker\n\
             test ! -e replacement-marker\n\
             printf '%s' 'descriptor-anchored'",
        );
        let file_tools = FileSystemTools::try_new(vec![safe_root.clone()]).unwrap();
        let client = CommandExecutionClient::new(
            program,
            file_tools.duplicate_safe_root_descriptor(0).unwrap(),
        )
        .unwrap();
        let debug = format!("{client:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(safe_root.to_string_lossy().as_ref()));
        assert!(!debug.contains("/proc/self/fd/"));

        let retained_client = client.clone();
        drop(client);
        fs::rename(&safe_root, &retained_root).unwrap();
        fs::write(&safe_root, b"replacement-file").unwrap();

        let result = retained_client
            .execute(profile_with_bounds(&[], Duration::from_secs(1), 64, 64))
            .await
            .unwrap();
        assert_eq!(result.stdout, "descriptor-anchored");
        assert!(retained_root.join("retained-marker").is_file());
        assert_eq!(fs::read(&safe_root).unwrap(), b"replacement-file");
    }

    #[tokio::test]
    async fn fixed_profile_clears_environment_nulls_stdin_and_uses_safe_root_cwd() {
        let _guard = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let safe_root = tempfile::tempdir().unwrap();
        let expected_cwd = safe_root.path().to_string_lossy();
        let script = format!(
            "test \"$#\" -eq 1\n\
             test \"$1\" = --version\n\
             test \"$PWD\" = '{expected_cwd}'\n\
             test \"$(/usr/bin/readlink /proc/self/fd/0)\" = /dev/null\n\
             test -z \"${{TERMUX_MCP_COMMAND_TEST_SECRET+x}}\"\n\
             printf '%s' 'termux-mcp-server 0.6.0'\n\
             printf '%s' 'diagnostic' >&2"
        );
        let (_program_root, program) = executable(&script);
        std::env::set_var("TERMUX_MCP_COMMAND_TEST_SECRET", "must-not-be-inherited");
        let client =
            CommandExecutionClient::new_for_test(program, safe_root.path().to_path_buf()).unwrap();
        let result = client
            .execute(command_profile("server_version").unwrap())
            .await;
        std::env::remove_var("TERMUX_MCP_COMMAND_TEST_SECRET");

        let result = result.unwrap();
        assert_eq!(result.profile, "server_version");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "termux-mcp-server 0.6.0");
        assert_eq!(result.stderr, "diagnostic");
        assert_eq!(result.stdout_bytes, result.stdout.len());
        assert_eq!(result.stderr_bytes, result.stderr.len());
    }

    #[tokio::test]
    async fn timeout_and_output_caps_fail_closed() {
        let _guard = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let safe_root = tempfile::tempdir().unwrap();

        let (_root, program) = executable("/bin/sleep 30");
        let client =
            CommandExecutionClient::new_for_test(program, safe_root.path().to_path_buf()).unwrap();
        assert_eq!(
            client
                .execute(profile_with_bounds(&[], Duration::from_millis(80), 8, 8))
                .await
                .unwrap_err(),
            CommandExecutionError::TimedOut
        );

        let (_root, program) = executable("printf '12345'");
        let client =
            CommandExecutionClient::new_for_test(program, safe_root.path().to_path_buf()).unwrap();
        assert_eq!(
            client
                .execute(profile_with_bounds(&[], Duration::from_secs(1), 4, 4))
                .await
                .unwrap_err(),
            CommandExecutionError::StdoutLimitExceeded
        );

        let (_root, program) = executable("printf '12345' >&2");
        let client =
            CommandExecutionClient::new_for_test(program, safe_root.path().to_path_buf()).unwrap();
        assert_eq!(
            client
                .execute(profile_with_bounds(&[], Duration::from_secs(1), 4, 4))
                .await
                .unwrap_err(),
            CommandExecutionError::StderrLimitExceeded
        );
    }

    #[tokio::test]
    async fn nonzero_and_invalid_utf8_outputs_are_not_reflected() {
        let _guard = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let safe_root = tempfile::tempdir().unwrap();

        let (_root, program) = executable("printf 'private failure' >&2; exit 9");
        let client =
            CommandExecutionClient::new_for_test(program, safe_root.path().to_path_buf()).unwrap();
        assert_eq!(
            client
                .execute(profile_with_bounds(&[], Duration::from_secs(1), 64, 64))
                .await
                .unwrap_err(),
            CommandExecutionError::ProgramFailed
        );

        let (_root, program) = executable("printf '\\377'");
        let client =
            CommandExecutionClient::new_for_test(program, safe_root.path().to_path_buf()).unwrap();
        assert_eq!(
            client
                .execute(profile_with_bounds(&[], Duration::from_secs(1), 64, 64))
                .await
                .unwrap_err(),
            CommandExecutionError::InvalidUtf8
        );
    }

    #[tokio::test]
    async fn command_specific_concurrency_limit_rejects_without_queueing() {
        let _guard = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let safe_root = tempfile::tempdir().unwrap();
        let (_root, program) = executable("/bin/sleep 1; printf done");
        let client = CommandExecutionClient::with_program_and_concurrency(
            program,
            safe_root.path().to_path_buf(),
            1,
        )
        .unwrap();
        let profile = profile_with_bounds(&[], Duration::from_secs(2), 64, 64);
        let running_client = client.clone();
        let running = tokio::spawn(async move { running_client.execute(profile).await });
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(
            client.execute(profile).await.unwrap_err(),
            CommandExecutionError::ConcurrencyLimitExceeded
        );
        assert_eq!(running.await.unwrap().unwrap().stdout, "done");
    }
}
