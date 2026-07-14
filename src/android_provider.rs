//! Shared bounded execution for fixed, zero-argument Termux:API providers.
//!
//! Callers select one reviewed absolute executable at construction time. Every
//! invocation clears the environment, fixes the working directory, supplies no
//! arguments or stdin, bounds both output streams, isolates the process group,
//! and delegates cancellation-safe cleanup to an independently owned supervisor.

use std::{io::ErrorKind, path::PathBuf, process::Stdio, time::Duration};

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

#[cfg(test)]
pub(crate) static ANDROID_PROVIDER_TEST_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());
#[cfg(test)]
static ACTIVE_PROVIDER_SUPERVISORS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AndroidProviderError {
    ProgramUnavailable,
    SpawnFailed,
    WaitFailed,
    TimedOut,
    StdoutLimitExceeded,
    StderrLimitExceeded,
    ProgramFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AndroidProviderConfigError {
    TimeoutTooShort,
}

#[derive(Debug, Clone)]
pub(crate) struct BoundedAndroidProvider {
    program: PathBuf,
    timeout: Duration,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
    #[cfg(test)]
    forced_cleanup_delay: Duration,
}

impl BoundedAndroidProvider {
    pub(crate) fn new(
        program: PathBuf,
        timeout: Duration,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
    ) -> Result<Self, AndroidProviderConfigError> {
        if timeout < MIN_PROVIDER_TIMEOUT {
            return Err(AndroidProviderConfigError::TimeoutTooShort);
        }

        Ok(Self {
            program,
            timeout,
            max_stdout_bytes,
            max_stderr_bytes,
            #[cfg(test)]
            forced_cleanup_delay: Duration::ZERO,
        })
    }

    pub(crate) async fn collect_stdout(&self) -> Result<Vec<u8>, AndroidProviderError> {
        let started_at = Instant::now();
        let final_deadline = started_at + self.timeout;
        // Construction rejects timeouts whose quarter-budget would round below
        // one millisecond, so cleanup always owns a real nonzero reserve.
        let cleanup_reserve = (self.timeout / 4)
            .clamp(MIN_PROCESS_CLEANUP_RESERVE, MAX_PROCESS_CLEANUP_RESERVE);
        let operation_deadline = final_deadline - cleanup_reserve;

        let mut command = Command::new(&self.program);
        command
            .env_clear()
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                AndroidProviderError::ProgramUnavailable
            } else {
                AndroidProviderError::SpawnFailed
            }
        })?;

        let process_group = child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .and_then(Pid::from_raw)
            .map(ProcessGroupGuard::new)
            .ok_or(AndroidProviderError::SpawnFailed)?;

        let stdout = child
            .stdout
            .take()
            .ok_or(AndroidProviderError::SpawnFailed)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(AndroidProviderError::SpawnFailed)?;

        // The sender exists only to make dropping this collection future visible
        // to the independently owned supervisor. A cancelled caller therefore
        // cannot detach a provider process or reader future.
        let (cancellation_sender, cancellation_receiver) = oneshot::channel();
        let supervisor = tokio::spawn(supervise_process(
            SpawnedProviderProcess {
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
                #[cfg(test)]
                forced_cleanup_delay: self.forced_cleanup_delay,
            },
            cancellation_receiver,
        ));
        let result = supervisor
            .await
            .map_err(|_| AndroidProviderError::WaitFailed)?;
        drop(cancellation_sender);
        result
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
        ACTIVE_PROVIDER_SUPERVISORS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}

#[cfg(test)]
impl Drop for ActiveSupervisorGuard {
    fn drop(&mut self) {
        ACTIVE_PROVIDER_SUPERVISORS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) fn active_supervisor_count() -> usize {
    ACTIVE_PROVIDER_SUPERVISORS.load(std::sync::atomic::Ordering::SeqCst)
}

enum BoundedRead {
    Complete(Vec<u8>),
    LimitExceeded,
}

enum SupervisorTerminal {
    Complete(Vec<u8>),
    Failure(AndroidProviderError),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupOutcome {
    ReapedWithinDeadline,
    ReapedAfterDeadline,
    ReapFailed,
}

struct SpawnedProviderProcess {
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
    #[cfg(test)]
    forced_cleanup_delay: Duration,
}

async fn supervise_process(
    process: SpawnedProviderProcess,
    bounds: SupervisorBounds,
    mut cancellation: oneshot::Receiver<()>,
) -> Result<Vec<u8>, AndroidProviderError> {
    #[cfg(test)]
    let _active_supervisor = ActiveSupervisorGuard::new();

    let SpawnedProviderProcess {
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
        let mut stderr_complete = false;
        let mut child_succeeded = false;

        loop {
            if child_succeeded && stdout_bytes.is_some() && stderr_complete {
                break SupervisorTerminal::Complete(
                    stdout_bytes
                        .take()
                        .expect("stdout completion checked before extraction"),
                );
            }

            // Stable simultaneous-event precedence is cancellation, normal
            // operation exhaustion, stdout, stderr, then child completion.
            tokio::select! {
                biased;

                _ = &mut cancellation => {
                    break SupervisorTerminal::Cancelled;
                }
                _ = &mut deadline => {
                    break SupervisorTerminal::Failure(AndroidProviderError::TimedOut);
                }
                stdout = &mut stdout_read, if stdout_bytes.is_none() => {
                    match stdout {
                        Ok(BoundedRead::Complete(bytes)) => stdout_bytes = Some(bytes),
                        Ok(BoundedRead::LimitExceeded) => {
                            break SupervisorTerminal::Failure(
                                AndroidProviderError::StdoutLimitExceeded,
                            );
                        }
                        Err(error) => break SupervisorTerminal::Failure(error),
                    }
                }
                stderr = &mut stderr_read, if !stderr_complete => {
                    match stderr {
                        Ok(BoundedRead::Complete(_)) => stderr_complete = true,
                        Ok(BoundedRead::LimitExceeded) => {
                            break SupervisorTerminal::Failure(
                                AndroidProviderError::StderrLimitExceeded,
                            );
                        }
                        Err(error) => break SupervisorTerminal::Failure(error),
                    }
                }
                status = &mut child_wait, if !child_succeeded => {
                    match status {
                        Ok(status) if status.success() => child_succeeded = true,
                        Ok(_) => {
                            break SupervisorTerminal::Failure(
                                AndroidProviderError::ProgramFailed,
                            );
                        }
                        Err(_) => {
                            break SupervisorTerminal::Failure(AndroidProviderError::WaitFailed);
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
        return Err(AndroidProviderError::WaitFailed);
    }

    match terminal {
        SupervisorTerminal::Complete(stdout) => Ok(stdout),
        SupervisorTerminal::Failure(error) => Err(error),
        SupervisorTerminal::Cancelled => Err(AndroidProviderError::WaitFailed),
    }
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
) -> Result<BoundedRead, AndroidProviderError> {
    let mut bytes = Vec::with_capacity(limit);
    let mut chunk = [0_u8; 4 * 1024];

    loop {
        let read = reader
            .read(&mut chunk)
            .await
            .map_err(|_| AndroidProviderError::WaitFailed)?;
        if read == 0 {
            return Ok(BoundedRead::Complete(bytes));
        }

        let remaining = limit.saturating_sub(bytes.len());
        if read > remaining {
            return Ok(BoundedRead::LimitExceeded);
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use tempfile::TempDir;

    use super::*;

    fn executable_provider(script: &str, timeout: Duration) -> (TempDir, BoundedAndroidProvider) {
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("provider");
        fs::write(&program, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        (
            directory,
            BoundedAndroidProvider::new(program, timeout, 1024, 1024).unwrap(),
        )
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
                BoundedAndroidProvider::new(PathBuf::from("/provider"), timeout, 1, 1)
                    .unwrap_err(),
                AndroidProviderConfigError::TimeoutTooShort,
            );
        }

        assert!(BoundedAndroidProvider::new(
            PathBuf::from("/provider"),
            MIN_PROVIDER_TIMEOUT,
            1,
            1,
        )
        .is_ok());
    }

    async fn wait_for_supervisor_count(expected: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while active_supervisor_count() != expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("provider supervisor count did not converge");
    }

    #[tokio::test]
    async fn late_reaping_is_authoritative_for_shared_provider_clients() {
        let _test_guard = ANDROID_PROVIDER_TEST_LOCK.lock().await;
        let (_directory, provider) =
            executable_provider("/bin/sleep 30", Duration::from_millis(120));
        let provider = provider.with_forced_cleanup_delay(Duration::from_millis(160));
        let started_at = Instant::now();

        assert_eq!(
            provider.collect_stdout().await.unwrap_err(),
            AndroidProviderError::WaitFailed
        );
        assert!(started_at.elapsed() >= Duration::from_millis(150));
        assert_eq!(active_supervisor_count(), 0);
    }

    #[tokio::test]
    async fn caller_drop_cannot_detach_shared_provider_cleanup() {
        let _test_guard = ANDROID_PROVIDER_TEST_LOCK.lock().await;
        let (_directory, provider) = executable_provider("/bin/sleep 30", Duration::from_secs(1));
        let provider = provider.with_forced_cleanup_delay(Duration::from_millis(1_100));
        let task = tokio::spawn(async move { provider.collect_stdout().await });
        wait_for_supervisor_count(1).await;

        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(active_supervisor_count(), 1);
        wait_for_supervisor_count(0).await;
    }
}
