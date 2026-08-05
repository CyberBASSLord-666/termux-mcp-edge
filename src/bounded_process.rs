//! Shared bounded execution for fixed, project-owned process profiles.
//!
//! Callers select a reviewed executable, argv vector, and working directory at
//! construction time. Every invocation clears the environment, supplies null
//! stdin, bounds both output streams, isolates the process group, and delegates
//! cancellation-safe cleanup to an independently owned supervisor. This module
//! never invokes a shell and never reads caller or ambient environment values.

use std::{
    ffi::OsString, fs::File, io::ErrorKind, path::PathBuf, process::Stdio, sync::Arc,
    time::Duration,
};

use rustix::io::{fcntl_getfd, fcntl_setfd, FdFlags};
use rustix::process::{kill_process_group, Pid, Signal};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, ChildStderr, ChildStdout, Command},
    sync::oneshot,
    time::{sleep_until, timeout_at, Instant},
};

const MIN_PROCESS_CLEANUP_RESERVE: Duration = Duration::from_millis(1);
const MAX_PROCESS_CLEANUP_RESERVE: Duration = Duration::from_millis(250);
const MIN_PROVIDER_TIMEOUT: Duration = Duration::from_millis(4);
pub(crate) const MAX_BOUNDED_PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const MAX_BOUNDED_PROCESS_STDOUT_BYTES: usize = 16 * 1024;
pub(crate) const MAX_BOUNDED_PROCESS_STDERR_BYTES: usize = 4 * 1024;

#[cfg(test)]
pub(crate) static BOUNDED_PROCESS_TEST_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());
#[cfg(test)]
static ACTIVE_PROCESS_SUPERVISORS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundedProcessError {
    ProgramUnavailable,
    SpawnFailed,
    WaitFailed,
    TimedOut,
    StdoutLimitExceeded,
    StderrLimitExceeded,
    ProgramFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundedProcessConfigError {
    TimeoutTooShort,
    TimeoutTooLong,
    StdoutLimitTooLarge,
    StderrLimitTooLarge,
}

#[derive(Debug, Clone)]
pub(crate) struct BoundedProcess {
    program: PathBuf,
    arguments: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    working_directory: PathBuf,
    timeout: Duration,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
    inherited_descriptor: Option<Arc<File>>,
    /// When true, a non-zero child exit still returns drained stdout/stderr.
    /// Used only by fixed has-feature probes that print `false` and exit 1.
    accept_nonzero_exit: bool,
    #[cfg(test)]
    forced_cleanup_delay: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoundedProcessOutput {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) duration: Duration,
}

pub(crate) struct BoundedChildContext {
    environment: Vec<(OsString, OsString)>,
    inherited_descriptor: Option<Arc<File>>,
}

impl BoundedChildContext {
    #[cfg(feature = "android-rish")]
    pub(crate) fn with_inherited_descriptor(
        environment: Vec<(OsString, OsString)>,
        inherited_descriptor: Arc<File>,
    ) -> Self {
        Self {
            environment,
            inherited_descriptor: Some(inherited_descriptor),
        }
    }
}

impl BoundedProcess {
    #[allow(
        dead_code,
        reason = "the isolated android-rish posture uses only the fixed-environment constructor"
    )]
    pub(crate) fn new(
        program: PathBuf,
        arguments: Vec<OsString>,
        working_directory: PathBuf,
        timeout: Duration,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
    ) -> Result<Self, BoundedProcessConfigError> {
        Self::new_with_fixed_environment(
            program,
            arguments,
            working_directory,
            timeout,
            max_stdout_bytes,
            max_stderr_bytes,
            Vec::new(),
        )
    }

    /// Construct a bounded process with one project-owned, fixed environment.
    ///
    /// This remains crate-private: request data must never select environment
    /// names or values. The child still starts from `env_clear`, so ambient
    /// Termux values cannot cross the process boundary.
    pub(crate) fn new_with_fixed_environment(
        program: PathBuf,
        arguments: Vec<OsString>,
        working_directory: PathBuf,
        timeout: Duration,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
        environment: Vec<(OsString, OsString)>,
    ) -> Result<Self, BoundedProcessConfigError> {
        Self::new_inner(
            program,
            arguments,
            working_directory,
            timeout,
            max_stdout_bytes,
            max_stderr_bytes,
            BoundedChildContext {
                environment,
                inherited_descriptor: None,
            },
        )
    }

    /// Construct one fixed child that alone inherits a caller-pinned descriptor.
    ///
    /// The parent descriptor must remain `CLOEXEC`. The pre-exec hook removes
    /// that flag only in this forked child, so unrelated provider processes
    /// cannot inherit the asset during concurrent execution.
    #[cfg(feature = "android-rish")]
    pub(crate) fn new_with_child_context(
        program: PathBuf,
        arguments: Vec<OsString>,
        working_directory: PathBuf,
        timeout: Duration,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
        context: BoundedChildContext,
    ) -> Result<Self, BoundedProcessConfigError> {
        Self::new_inner(
            program,
            arguments,
            working_directory,
            timeout,
            max_stdout_bytes,
            max_stderr_bytes,
            context,
        )
    }

    fn new_inner(
        program: PathBuf,
        arguments: Vec<OsString>,
        working_directory: PathBuf,
        timeout: Duration,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
        context: BoundedChildContext,
    ) -> Result<Self, BoundedProcessConfigError> {
        let BoundedChildContext {
            environment,
            inherited_descriptor,
        } = context;
        if timeout < MIN_PROVIDER_TIMEOUT {
            return Err(BoundedProcessConfigError::TimeoutTooShort);
        }
        if timeout > MAX_BOUNDED_PROCESS_TIMEOUT {
            return Err(BoundedProcessConfigError::TimeoutTooLong);
        }
        if max_stdout_bytes > MAX_BOUNDED_PROCESS_STDOUT_BYTES {
            return Err(BoundedProcessConfigError::StdoutLimitTooLarge);
        }
        if max_stderr_bytes > MAX_BOUNDED_PROCESS_STDERR_BYTES {
            return Err(BoundedProcessConfigError::StderrLimitTooLarge);
        }
        Ok(Self {
            program,
            arguments,
            environment,
            working_directory,
            timeout,
            max_stdout_bytes,
            max_stderr_bytes,
            inherited_descriptor,
            accept_nonzero_exit: false,
            #[cfg(test)]
            forced_cleanup_delay: Duration::ZERO,
        })
    }

    /// Accept non-zero exits after both output streams have been fully drained.
    ///
    /// Default remains fail-closed (`ProgramFailed`). The rish has-feature path
    /// enables this so Android's `cmd package has-feature` exit-1-for-false
    /// can still yield a boolean payload.
    #[cfg(feature = "android-rish")]
    pub(crate) fn with_accept_nonzero_exit(mut self, accept: bool) -> Self {
        self.accept_nonzero_exit = accept;
        self
    }

    #[cfg(any(
        feature = "android-battery-status",
        feature = "android-volume-status",
        feature = "command-execution",
        test
    ))]
    pub(crate) async fn run(&self) -> Result<BoundedProcessOutput, BoundedProcessError> {
        self.run_inner(None).await
    }

    /// Run while retaining an authority guard through cancellation-independent
    /// process-group cleanup and direct-child reaping.
    ///
    /// The rish lane uses this for its sole non-queueing semaphore permit. If
    /// the request future is dropped, the existing cancellation channel still
    /// terminates the child immediately, while the supervisor keeps the guard
    /// until that cleanup is authoritatively complete.
    #[cfg(feature = "android-rish")]
    pub(crate) async fn run_with_completion_guard<G>(
        &self,
        completion_guard: G,
    ) -> Result<BoundedProcessOutput, BoundedProcessError>
    where
        G: Send + 'static,
    {
        self.run_inner(Some(Box::new(completion_guard))).await
    }

    async fn run_inner(
        &self,
        completion_guard: Option<Box<dyn Send>>,
    ) -> Result<BoundedProcessOutput, BoundedProcessError> {
        let started_at = Instant::now();
        let final_deadline = started_at + self.timeout;
        // Construction rejects timeouts whose quarter-budget would round below
        // one millisecond, so cleanup always owns a real nonzero reserve.
        let cleanup_reserve =
            (self.timeout / 4).clamp(MIN_PROCESS_CLEANUP_RESERVE, MAX_PROCESS_CLEANUP_RESERVE);
        let operation_deadline = final_deadline - cleanup_reserve;

        let mut command = Command::new(&self.program);
        command
            .args(&self.arguments)
            .env_clear()
            .envs(self.environment.iter().cloned())
            .current_dir(&self.working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        if let Some(descriptor) = self.inherited_descriptor.clone() {
            // SAFETY: the hook performs only async-signal-safe `fcntl`
            // operations on an already-open descriptor and does not allocate,
            // lock, or access request-controlled data.
            unsafe {
                command.pre_exec(move || {
                    let mut flags =
                        fcntl_getfd(descriptor.as_ref()).map_err(std::io::Error::from)?;
                    flags.remove(FdFlags::CLOEXEC);
                    fcntl_setfd(descriptor.as_ref(), flags).map_err(std::io::Error::from)
                });
            }
        }

        let mut child = command.spawn().map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                BoundedProcessError::ProgramUnavailable
            } else {
                BoundedProcessError::SpawnFailed
            }
        })?;

        let process_group = child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .and_then(Pid::from_raw)
            .map(ProcessGroupGuard::new)
            .ok_or(BoundedProcessError::SpawnFailed)?;

        let stdout = child
            .stdout
            .take()
            .ok_or(BoundedProcessError::SpawnFailed)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(BoundedProcessError::SpawnFailed)?;

        // The sender exists only to make dropping this collection future visible
        // to the independently owned supervisor. A cancelled caller therefore
        // cannot detach a fixed process or reader future.
        let (cancellation_sender, cancellation_receiver) = oneshot::channel();
        let supervisor = tokio::spawn(supervise_process(
            SpawnedProcess {
                child,
                process_group,
                stdout,
                stderr,
            },
            SupervisorBounds {
                max_stdout_bytes: self.max_stdout_bytes,
                max_stderr_bytes: self.max_stderr_bytes,
                operation_deadline,
                final_deadline,
                accept_nonzero_exit: self.accept_nonzero_exit,
                #[cfg(test)]
                forced_cleanup_delay: self.forced_cleanup_delay,
            },
            cancellation_receiver,
            completion_guard,
        ));
        let result = supervisor
            .await
            .map_err(|_| BoundedProcessError::WaitFailed)?;
        drop(cancellation_sender);
        result.map(|mut output| {
            output.duration = started_at.elapsed();
            output
        })
    }

    #[cfg(test)]
    pub(crate) fn with_forced_cleanup_delay(mut self, delay: Duration) -> Self {
        self.forced_cleanup_delay = delay;
        self
    }
}

struct ProcessGroupGuard {
    process_group: Pid,
    armed: bool,
}

impl ProcessGroupGuard {
    fn new(process_group: Pid) -> Self {
        Self {
            process_group,
            armed: true,
        }
    }

    fn terminate(&self) {
        let _ = kill_process_group(self.process_group, Signal::KILL);
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if self.armed {
            self.terminate();
        }
    }
}

#[cfg(test)]
struct ActiveSupervisorGuard;

#[cfg(test)]
impl ActiveSupervisorGuard {
    fn new() -> Self {
        ACTIVE_PROCESS_SUPERVISORS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}

#[cfg(test)]
impl Drop for ActiveSupervisorGuard {
    fn drop(&mut self) {
        ACTIVE_PROCESS_SUPERVISORS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) fn active_supervisor_count() -> usize {
    ACTIVE_PROCESS_SUPERVISORS.load(std::sync::atomic::Ordering::SeqCst)
}

enum BoundedRead {
    Complete(Vec<u8>),
    LimitExceeded,
}

enum SupervisorTerminal {
    Complete { stdout: Vec<u8>, stderr: Vec<u8> },
    Failure(BoundedProcessError),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupOutcome {
    ReapedWithinDeadline,
    ReapedAfterDeadline,
    ReapFailed,
}

struct SpawnedProcess {
    child: Child,
    process_group: ProcessGroupGuard,
    stdout: ChildStdout,
    stderr: ChildStderr,
}

struct SupervisorBounds {
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
    operation_deadline: Instant,
    final_deadline: Instant,
    accept_nonzero_exit: bool,
    #[cfg(test)]
    forced_cleanup_delay: Duration,
}

async fn supervise_process(
    process: SpawnedProcess,
    bounds: SupervisorBounds,
    mut cancellation: oneshot::Receiver<()>,
    completion_guard: Option<Box<dyn Send>>,
) -> Result<BoundedProcessOutput, BoundedProcessError> {
    #[cfg(test)]
    let _active_supervisor = ActiveSupervisorGuard::new();

    let SpawnedProcess {
        mut child,
        mut process_group,
        stdout,
        stderr,
    } = process;

    let terminal = {
        let stdout_read = read_bounded(stdout, bounds.max_stdout_bytes);
        let stderr_read = read_bounded(stderr, bounds.max_stderr_bytes);
        let child_wait = child.wait();
        let deadline = sleep_until(bounds.operation_deadline);
        tokio::pin!(stdout_read, stderr_read, child_wait, deadline);

        let mut stdout_bytes = None;
        let mut stderr_bytes = None;
        let mut child_finished = false;
        let mut child_succeeded = false;

        loop {
            // Wait for both streams after any finished wait so non-zero exits
            // can still yield a drained boolean payload when allowed.
            if child_finished && stdout_bytes.is_some() && stderr_bytes.is_some() {
                if child_succeeded || bounds.accept_nonzero_exit {
                    break SupervisorTerminal::Complete {
                        stdout: stdout_bytes
                            .take()
                            .expect("stdout completion checked before extraction"),
                        stderr: stderr_bytes
                            .take()
                            .expect("stderr completion checked before extraction"),
                    };
                }
                break SupervisorTerminal::Failure(BoundedProcessError::ProgramFailed);
            }

            // Stable simultaneous-event precedence is cancellation, normal
            // operation exhaustion, stdout, stderr, then child completion.
            tokio::select! {
                biased;

                _ = &mut cancellation => {
                    break SupervisorTerminal::Cancelled;
                }
                _ = &mut deadline => {
                    break SupervisorTerminal::Failure(BoundedProcessError::TimedOut);
                }
                stdout = &mut stdout_read, if stdout_bytes.is_none() => {
                    match stdout {
                        Ok(BoundedRead::Complete(bytes)) => stdout_bytes = Some(bytes),
                        Ok(BoundedRead::LimitExceeded) => {
                            break SupervisorTerminal::Failure(
                                BoundedProcessError::StdoutLimitExceeded,
                            );
                        }
                        Err(error) => break SupervisorTerminal::Failure(error),
                    }
                }
                stderr = &mut stderr_read, if stderr_bytes.is_none() => {
                    match stderr {
                        Ok(BoundedRead::Complete(bytes)) => stderr_bytes = Some(bytes),
                        Ok(BoundedRead::LimitExceeded) => {
                            break SupervisorTerminal::Failure(
                                BoundedProcessError::StderrLimitExceeded,
                            );
                        }
                        Err(error) => break SupervisorTerminal::Failure(error),
                    }
                }
                status = &mut child_wait, if !child_finished => {
                    match status {
                        Ok(status) if status.success() => {
                            child_finished = true;
                            child_succeeded = true;
                        }
                        Ok(_) => {
                            child_finished = true;
                            child_succeeded = false;
                        }
                        Err(_) => {
                            break SupervisorTerminal::Failure(BoundedProcessError::WaitFailed);
                        }
                    }
                }
            }
        }
    };

    // Dropping the reader futures closes both pipes. Cleanup is authoritative
    // for every terminal path and remains owned by this task after caller drop.
    let cleanup_outcome =
        terminate_process_group_and_reap(&mut child, &mut process_group, &bounds).await;
    if cleanup_outcome != CleanupOutcome::ReapedWithinDeadline {
        return Err(BoundedProcessError::WaitFailed);
    }

    let result = match terminal {
        SupervisorTerminal::Complete { stdout, stderr } => Ok(BoundedProcessOutput {
            stdout,
            stderr,
            duration: Duration::ZERO,
        }),
        SupervisorTerminal::Failure(error) => Err(error),
        SupervisorTerminal::Cancelled => Err(BoundedProcessError::WaitFailed),
    };
    drop(completion_guard);
    result
}

async fn terminate_process_group_and_reap(
    child: &mut Child,
    process_group: &mut ProcessGroupGuard,
    bounds: &SupervisorBounds,
) -> CleanupOutcome {
    process_group.terminate();

    // Delay only reap confirmation in tests; process-group termination remains
    // immediate so the hook cannot weaken cancellation behavior.
    #[cfg(test)]
    if !bounds.forced_cleanup_delay.is_zero() {
        tokio::time::sleep(bounds.forced_cleanup_delay).await;
    }

    let final_deadline = bounds.final_deadline;
    let mut within_deadline = Instant::now() <= final_deadline;
    let reaped = match child.try_wait() {
        Ok(Some(_)) => true,
        Ok(None) | Err(_) => {
            let _ = child.start_kill();
            match timeout_at(final_deadline, child.wait()).await {
                Ok(Ok(_)) => true,
                Ok(Err(_)) => false,
                Err(_) => {
                    within_deadline = false;
                    // Once latency conflicts with synchronous reaping, cleanup
                    // remains authoritative until wait confirms collection.
                    child.wait().await.is_ok()
                }
            }
        }
    };

    if reaped {
        process_group.disarm();
        if within_deadline && Instant::now() <= final_deadline {
            CleanupOutcome::ReapedWithinDeadline
        } else {
            CleanupOutcome::ReapedAfterDeadline
        }
    } else {
        CleanupOutcome::ReapFailed
    }
}

async fn read_bounded(
    mut reader: impl AsyncRead + Unpin + Send + 'static,
    limit: usize,
) -> Result<BoundedRead, BoundedProcessError> {
    // Capacity follows bytes actually read, never the selected ceiling. This
    // avoids even attempting an excessive up-front allocation if an internal
    // caller supplies a forged limit.
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4 * 1024];

    loop {
        let read = reader
            .read(&mut chunk)
            .await
            .map_err(|_| BoundedProcessError::WaitFailed)?;
        if read == 0 {
            return Ok(BoundedRead::Complete(bytes));
        }

        let remaining = limit.saturating_sub(bytes.len());
        if read > remaining {
            return Ok(BoundedRead::LimitExceeded);
        }
        bytes
            .try_reserve_exact(read)
            .map_err(|_| BoundedProcessError::WaitFailed)?;
        bytes.extend_from_slice(&chunk[..read]);
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    #[cfg(feature = "android-rish")]
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use rustix::process::test_kill_process;
    use tempfile::TempDir;

    use super::*;

    fn executable_process(script: &str, timeout: Duration) -> (TempDir, BoundedProcess) {
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("process");
        fs::write(&program, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        (
            directory,
            BoundedProcess::new(program, Vec::new(), PathBuf::from("/"), timeout, 1024, 1024)
                .unwrap(),
        )
    }

    #[test]
    fn hard_resource_maxima_match_the_reviewed_command_contract() {
        assert_eq!(MAX_BOUNDED_PROCESS_TIMEOUT, Duration::from_secs(5));
        assert_eq!(MAX_BOUNDED_PROCESS_STDOUT_BYTES, 16 * 1024);
        assert_eq!(MAX_BOUNDED_PROCESS_STDERR_BYTES, 4 * 1024);
    }

    #[test]
    fn construction_rejects_timeouts_without_a_nonzero_cleanup_reserve() {
        for timeout in [
            Duration::ZERO,
            Duration::from_millis(1),
            Duration::from_millis(2),
            Duration::from_millis(3),
        ] {
            assert_eq!(
                BoundedProcess::new(
                    PathBuf::from("/process"),
                    Vec::new(),
                    PathBuf::from("/"),
                    timeout,
                    1,
                    1,
                )
                .unwrap_err(),
                BoundedProcessConfigError::TimeoutTooShort,
            );
        }

        assert!(BoundedProcess::new(
            PathBuf::from("/process"),
            Vec::new(),
            PathBuf::from("/"),
            MIN_PROVIDER_TIMEOUT,
            1,
            1,
        )
        .is_ok());
    }

    #[test]
    fn construction_enforces_hard_resource_maxima_before_spawn() {
        let program = PathBuf::from("/definitely/not/a/program");
        let valid = || {
            (
                program.clone(),
                Vec::new(),
                PathBuf::from("/"),
                MAX_BOUNDED_PROCESS_TIMEOUT,
                MAX_BOUNDED_PROCESS_STDOUT_BYTES,
                MAX_BOUNDED_PROCESS_STDERR_BYTES,
            )
        };

        let (program, arguments, working_directory, _, stdout, stderr) = valid();
        assert_eq!(
            BoundedProcess::new(
                program,
                arguments,
                working_directory,
                MAX_BOUNDED_PROCESS_TIMEOUT + Duration::from_millis(1),
                stdout,
                stderr,
            )
            .unwrap_err(),
            BoundedProcessConfigError::TimeoutTooLong
        );

        let (program, arguments, working_directory, timeout, _, stderr) = valid();
        assert_eq!(
            BoundedProcess::new(
                program,
                arguments,
                working_directory,
                timeout,
                MAX_BOUNDED_PROCESS_STDOUT_BYTES + 1,
                stderr,
            )
            .unwrap_err(),
            BoundedProcessConfigError::StdoutLimitTooLarge
        );

        let (program, arguments, working_directory, timeout, stdout, _) = valid();
        assert_eq!(
            BoundedProcess::new(
                program,
                arguments,
                working_directory,
                timeout,
                stdout,
                MAX_BOUNDED_PROCESS_STDERR_BYTES + 1,
            )
            .unwrap_err(),
            BoundedProcessConfigError::StderrLimitTooLarge
        );

        let (program, arguments, working_directory, timeout, stdout, stderr) = valid();
        assert!(BoundedProcess::new(
            program,
            arguments,
            working_directory,
            timeout,
            stdout,
            stderr,
        )
        .is_ok());
    }

    #[tokio::test]
    async fn bounded_reader_never_preallocates_a_selected_capacity() {
        match read_bounded(tokio::io::empty(), usize::MAX).await.unwrap() {
            BoundedRead::Complete(bytes) => assert!(bytes.is_empty()),
            BoundedRead::LimitExceeded => panic!("empty input cannot exceed any limit"),
        }

        match read_bounded(&b"1234"[..], 4).await.unwrap() {
            BoundedRead::Complete(bytes) => assert_eq!(bytes, b"1234"),
            BoundedRead::LimitExceeded => panic!("exact-limit input must succeed"),
        }
        assert!(matches!(
            read_bounded(&b"1234"[..], 3).await.unwrap(),
            BoundedRead::LimitExceeded
        ));
    }

    async fn wait_for_supervisor_count(expected: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while active_supervisor_count() != expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("process supervisor count did not converge");
    }

    async fn read_pid_file(path: &Path) -> u32 {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(value) = fs::read_to_string(path) {
                    if let Ok(pid) = value.trim().parse() {
                        break pid;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("bounded process did not publish its process identifier")
    }

    fn process_exists(pid: u32) -> bool {
        let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
            return false;
        };
        match test_kill_process(pid) {
            Ok(()) => true,
            Err(error) => error != rustix::io::Errno::SRCH,
        }
    }

    async fn assert_process_gone(pid: u32) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while process_exists(pid) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("bounded process {pid} survived cleanup"));
    }

    #[tokio::test]
    async fn pipe_holding_descendant_is_terminated_with_its_process_group() {
        let _test_guard = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let directory = tempfile::tempdir().unwrap();
        let direct_pid_path = directory.path().join("direct-pid");
        let descendant_pid_path = directory.path().join("descendant-pid");
        let direct_pid_text = direct_pid_path.to_string_lossy();
        let descendant_pid_text = descendant_pid_path.to_string_lossy();
        assert!(!direct_pid_text.contains('\''));
        assert!(!descendant_pid_text.contains('\''));
        let script = format!(
            "printf '%s\\n' \"$$\" >'{direct_pid_text}'\n\
             /bin/sleep 30 &\n\
             printf '%s\\n' \"$!\" >'{descendant_pid_text}'\n\
             printf done\n\
             exit 0"
        );
        let program = directory.path().join("process");
        fs::write(&program, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let process = BoundedProcess::new(
            program,
            Vec::new(),
            PathBuf::from("/"),
            Duration::from_millis(300),
            1024,
            1024,
        )
        .unwrap();

        assert_eq!(
            process.run().await.unwrap_err(),
            BoundedProcessError::TimedOut
        );
        let direct_pid = read_pid_file(&direct_pid_path).await;
        let descendant_pid = read_pid_file(&descendant_pid_path).await;
        assert_process_gone(direct_pid).await;
        assert_process_gone(descendant_pid).await;
        assert_eq!(active_supervisor_count(), 0);
    }

    #[tokio::test]
    async fn caller_cancellation_terminates_direct_child_and_descendant() {
        let _test_guard = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let directory = tempfile::tempdir().unwrap();
        let direct_pid_path = directory.path().join("direct-pid");
        let descendant_pid_path = directory.path().join("descendant-pid");
        let direct_pid_text = direct_pid_path.to_string_lossy();
        let descendant_pid_text = descendant_pid_path.to_string_lossy();
        assert!(!direct_pid_text.contains('\''));
        assert!(!descendant_pid_text.contains('\''));
        let script = format!(
            "printf '%s\\n' \"$$\" >'{direct_pid_text}'\n\
             /bin/sleep 30 >/dev/null 2>&1 &\n\
             printf '%s\\n' \"$!\" >'{descendant_pid_text}'\n\
             wait"
        );
        let program = directory.path().join("process");
        fs::write(&program, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let process = BoundedProcess::new(
            program,
            Vec::new(),
            PathBuf::from("/"),
            Duration::from_secs(2),
            1024,
            1024,
        )
        .unwrap();
        let task = tokio::spawn(async move { process.run().await });
        let direct_pid = read_pid_file(&direct_pid_path).await;
        let descendant_pid = read_pid_file(&descendant_pid_path).await;

        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_process_gone(direct_pid).await;
        assert_process_gone(descendant_pid).await;
        wait_for_supervisor_count(0).await;
    }

    #[tokio::test]
    async fn late_reaping_is_authoritative_for_shared_process_clients() {
        let _test_guard = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let (_directory, process) = executable_process("/bin/sleep 30", Duration::from_millis(120));
        let process = process.with_forced_cleanup_delay(Duration::from_millis(160));
        let started_at = Instant::now();

        assert_eq!(
            process.run().await.unwrap_err(),
            BoundedProcessError::WaitFailed
        );
        assert!(started_at.elapsed() >= Duration::from_millis(150));
        assert_eq!(active_supervisor_count(), 0);
    }

    #[tokio::test]
    async fn caller_drop_cannot_detach_shared_process_cleanup() {
        let _test_guard = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let (_directory, process) = executable_process("/bin/sleep 30", Duration::from_secs(1));
        let process = process.with_forced_cleanup_delay(Duration::from_millis(1_100));
        let task = tokio::spawn(async move { process.run().await });
        wait_for_supervisor_count(1).await;

        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(active_supervisor_count(), 1);
        wait_for_supervisor_count(0).await;
    }

    #[cfg(feature = "android-rish")]
    #[tokio::test]
    async fn rish_completion_guard_survives_waiter_drop_until_cleanup_finishes() {
        struct DropFlag(Arc<AtomicBool>);

        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let _test_guard = BOUNDED_PROCESS_TEST_LOCK.lock().await;
        let (_directory, process) = executable_process("/bin/sleep 30", Duration::from_secs(1));
        let process = process.with_forced_cleanup_delay(Duration::from_millis(500));
        let dropped = Arc::new(AtomicBool::new(false));
        let completion_guard = DropFlag(Arc::clone(&dropped));
        let task =
            tokio::spawn(async move { process.run_with_completion_guard(completion_guard).await });
        wait_for_supervisor_count(1).await;

        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(active_supervisor_count(), 1);
        assert!(!dropped.load(Ordering::SeqCst));
        wait_for_supervisor_count(0).await;
        assert!(dropped.load(Ordering::SeqCst));
    }
}
