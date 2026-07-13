//! Bounded Termux:API battery telemetry for the opt-in Android platform gate.
//!
//! The production client invokes one fixed absolute program with no arguments,
//! no stdin, no inherited environment, and no shell interpolation. Process
//! output is read concurrently behind hard byte ceilings before a strict
//! allowlist parser constructs the public response.

use std::{io::ErrorKind, path::PathBuf, process::Stdio, time::Duration};

#[cfg(test)]
use rustix::process::test_kill_process;
use rustix::process::{kill_process_group, Pid, Signal};
use serde::Serialize;
use serde_json::{Map, Value};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, ChildStderr, ChildStdout, Command},
    sync::oneshot,
    time::{sleep_until, timeout_at, Instant},
};

pub const TERMUX_BATTERY_STATUS_PROGRAM: &str =
    "/data/data/com.termux/files/usr/bin/termux-battery-status";
pub const BATTERY_STATUS_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_BATTERY_STDOUT_BYTES: usize = 16 * 1024;
pub const MAX_BATTERY_STDERR_BYTES: usize = 4 * 1024;
const MAX_PROCESS_CLEANUP_RESERVE: Duration = Duration::from_millis(250);

#[cfg(test)]
pub(crate) static ANDROID_BATTERY_TEST_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());
#[cfg(test)]
static ACTIVE_BATTERY_SUPERVISORS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

const MAX_STATUS_LABEL_BYTES: usize = 32;
const MIN_TEMPERATURE_CELSIUS: f64 = -100.0;
const MAX_TEMPERATURE_CELSIUS: f64 = 200.0;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AndroidBatteryStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub present: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugged: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_celsius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voltage_millivolts: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_microamps: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_average_microamps: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percentage: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_counter_microamp_hours: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_nanowatt_hours: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_count: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndroidBatteryError {
    ApiUnavailable,
    SpawnFailed,
    WaitFailed,
    TimedOut,
    StdoutLimitExceeded,
    StderrLimitExceeded,
    ApiFailed,
    InvalidUtf8,
    InvalidJson,
    InvalidField,
}

impl AndroidBatteryError {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::ApiUnavailable => "battery_api_unavailable",
            Self::SpawnFailed => "battery_api_spawn_failed",
            Self::WaitFailed => "battery_api_wait_failed",
            Self::TimedOut => "battery_api_timeout",
            Self::StdoutLimitExceeded => "battery_stdout_limit_exceeded",
            Self::StderrLimitExceeded => "battery_stderr_limit_exceeded",
            Self::ApiFailed => "battery_api_failed",
            Self::InvalidUtf8 => "battery_output_invalid_utf8",
            Self::InvalidJson => "battery_output_invalid_json",
            Self::InvalidField => "battery_output_invalid_field",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AndroidBatteryClient {
    program: PathBuf,
    timeout: Duration,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
    #[cfg(test)]
    forced_cleanup_delay: Duration,
}

impl AndroidBatteryClient {
    pub fn termux() -> Self {
        Self {
            program: PathBuf::from(TERMUX_BATTERY_STATUS_PROGRAM),
            timeout: BATTERY_STATUS_TIMEOUT,
            max_stdout_bytes: MAX_BATTERY_STDOUT_BYTES,
            max_stderr_bytes: MAX_BATTERY_STDERR_BYTES,
            #[cfg(test)]
            forced_cleanup_delay: Duration::ZERO,
        }
    }

    pub async fn collect(&self) -> Result<AndroidBatteryStatus, AndroidBatteryError> {
        let started_at = Instant::now();
        let final_deadline = started_at + self.timeout;
        let cleanup_reserve = (self.timeout / 4).min(MAX_PROCESS_CLEANUP_RESERVE);
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
                AndroidBatteryError::ApiUnavailable
            } else {
                AndroidBatteryError::SpawnFailed
            }
        })?;

        let process_group = child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .and_then(Pid::from_raw)
            .map(ProcessGroupGuard::new)
            .ok_or(AndroidBatteryError::SpawnFailed)?;

        let stdout = child
            .stdout
            .take()
            .ok_or(AndroidBatteryError::SpawnFailed)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(AndroidBatteryError::SpawnFailed)?;

        // The sender exists only to make dropping this `collect` future observable to
        // the independently owned supervisor. A cancelled caller therefore cannot
        // detach a provider process or leave a reader task behind.
        let (cancellation_sender, cancellation_receiver) = oneshot::channel();
        let supervisor = tokio::spawn(supervise_process(
            SpawnedBatteryProcess {
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
            .map_err(|_| AndroidBatteryError::WaitFailed)?;
        drop(cancellation_sender);
        result
    }

    #[cfg(test)]
    pub(crate) fn with_program_and_limits(
        program: PathBuf,
        timeout: Duration,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
    ) -> Self {
        Self {
            program,
            timeout,
            max_stdout_bytes,
            max_stderr_bytes,
            forced_cleanup_delay: Duration::ZERO,
        }
    }

    #[cfg(test)]
    fn with_forced_cleanup_delay(mut self, delay: Duration) -> Self {
        self.forced_cleanup_delay = delay;
        self
    }
}

impl Default for AndroidBatteryClient {
    fn default() -> Self {
        Self::termux()
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
        ACTIVE_BATTERY_SUPERVISORS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}

#[cfg(test)]
impl Drop for ActiveSupervisorGuard {
    fn drop(&mut self) {
        ACTIVE_BATTERY_SUPERVISORS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

enum BoundedRead {
    Complete(Vec<u8>),
    LimitExceeded,
}

enum SupervisorTerminal {
    Complete(Vec<u8>),
    Failure(AndroidBatteryError),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupOutcome {
    ReapedWithinDeadline,
    ReapedAfterDeadline,
    ReapFailed,
}

struct SpawnedBatteryProcess {
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
    process: SpawnedBatteryProcess,
    bounds: SupervisorBounds,
    mut cancellation: oneshot::Receiver<()>,
) -> Result<AndroidBatteryStatus, AndroidBatteryError> {
    #[cfg(test)]
    let _active_supervisor = ActiveSupervisorGuard::new();

    let SpawnedBatteryProcess {
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

            // Stable simultaneous-event precedence is cancellation, total-time
            // exhaustion, stdout, stderr, then direct-child completion. The
            // deadline comes before I/O so a continuously ready pipe cannot starve
            // the end-to-end wall-clock bound.
            tokio::select! {
                biased;

                _ = &mut cancellation => {
                    break SupervisorTerminal::Cancelled;
                }
                _ = &mut deadline => {
                    break SupervisorTerminal::Failure(AndroidBatteryError::TimedOut);
                }
                stdout = &mut stdout_read, if stdout_bytes.is_none() => {
                    match stdout {
                        Ok(BoundedRead::Complete(bytes)) => stdout_bytes = Some(bytes),
                        Ok(BoundedRead::LimitExceeded) => {
                            break SupervisorTerminal::Failure(
                                AndroidBatteryError::StdoutLimitExceeded,
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
                                AndroidBatteryError::StderrLimitExceeded,
                            );
                        }
                        Err(error) => break SupervisorTerminal::Failure(error),
                    }
                }
                status = &mut child_wait, if !child_succeeded => {
                    match status {
                        Ok(status) if status.success() => child_succeeded = true,
                        Ok(_) => {
                            break SupervisorTerminal::Failure(AndroidBatteryError::ApiFailed);
                        }
                        Err(_) => {
                            break SupervisorTerminal::Failure(AndroidBatteryError::WaitFailed);
                        }
                    }
                }
            }
        }
    };

    // Leaving the block above drops both reader futures and closes their pipes.
    // Kill the whole isolated process group and synchronously reap the direct
    // child. If the reserved cleanup window is exhausted, reaping takes priority
    // over returning the primary result; the supervisor reports a stable wait
    // failure only after the direct child is reaped (or wait itself fails).
    let cleanup_outcome =
        terminate_process_group_and_reap(&mut child, &mut process_group, &bounds).await;

    if cleanup_outcome != CleanupOutcome::ReapedWithinDeadline {
        return Err(AndroidBatteryError::WaitFailed);
    }

    match terminal {
        SupervisorTerminal::Complete(stdout) => {
            let stdout = String::from_utf8(stdout).map_err(|_| AndroidBatteryError::InvalidUtf8)?;
            parse_battery_status(&stdout)
        }
        SupervisorTerminal::Failure(error) => Err(error),
        SupervisorTerminal::Cancelled => Err(AndroidBatteryError::WaitFailed),
    }
}

async fn terminate_process_group_and_reap(
    child: &mut Child,
    process_group: &mut ProcessGroupGuard,
    bounds: &SupervisorBounds,
) -> CleanupOutcome {
    process_group.terminate();

    // Test-only delay injection happens after process-group termination so the
    // regression forces late reap confirmation without modeling delayed cleanup.
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
                    // The operation deadline is no longer authoritative once it
                    // conflicts with synchronous child reaping. SIGKILL has already
                    // been sent to the isolated process group, both pipes are closed,
                    // and this independently owned supervisor remains responsible for
                    // the direct child until wait confirms collection.
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
) -> Result<BoundedRead, AndroidBatteryError> {
    let mut bytes = Vec::with_capacity(limit);
    let mut chunk = [0_u8; 4 * 1024];

    loop {
        let read = reader
            .read(&mut chunk)
            .await
            .map_err(|_| AndroidBatteryError::WaitFailed)?;
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

fn parse_battery_status(input: &str) -> Result<AndroidBatteryStatus, AndroidBatteryError> {
    let value: Value = serde_json::from_str(input).map_err(|_| AndroidBatteryError::InvalidJson)?;
    let object = value.as_object().ok_or(AndroidBatteryError::InvalidJson)?;

    let mut recognized_fields = 0_usize;
    let present = optional_bool(object, "present", &mut recognized_fields)?;
    let health = optional_label(object, "health", &mut recognized_fields)?;
    let plugged = optional_label(object, "plugged", &mut recognized_fields)?;
    let status = optional_label(object, "status", &mut recognized_fields)?;
    let temperature_celsius = optional_float(
        object,
        "temperature",
        MIN_TEMPERATURE_CELSIUS,
        MAX_TEMPERATURE_CELSIUS,
        &mut recognized_fields,
    )?;
    let voltage_millivolts =
        optional_integer(object, "voltage", 0, 100_000, &mut recognized_fields)?;
    let current_microamps = optional_integer(
        object,
        "current",
        -1_000_000_000,
        1_000_000_000,
        &mut recognized_fields,
    )?;
    let current_average_microamps = optional_integer(
        object,
        "current_average",
        -1_000_000_000,
        1_000_000_000,
        &mut recognized_fields,
    )?;
    let percentage = optional_integer(object, "percentage", 0, 100, &mut recognized_fields)?;
    let level = optional_integer(object, "level", 0, 1_000_000, &mut recognized_fields)?;
    let scale = optional_integer(object, "scale", 1, 1_000_000, &mut recognized_fields)?;
    let charge_counter_microamp_hours = optional_integer(
        object,
        "charge_counter",
        -1_000_000_000_000,
        1_000_000_000_000,
        &mut recognized_fields,
    )?;
    let energy_nanowatt_hours = optional_integer(
        object,
        "energy",
        -1_000_000_000_000_000,
        1_000_000_000_000_000,
        &mut recognized_fields,
    )?;
    let cycle_count = optional_integer(object, "cycle", 0, 1_000_000, &mut recognized_fields)?;

    if recognized_fields == 0
        || matches!((level, scale), (Some(level), Some(scale)) if level > scale)
    {
        return Err(AndroidBatteryError::InvalidField);
    }

    Ok(AndroidBatteryStatus {
        present,
        health,
        plugged,
        status,
        temperature_celsius,
        voltage_millivolts,
        current_microamps,
        current_average_microamps,
        percentage,
        level,
        scale,
        charge_counter_microamp_hours,
        energy_nanowatt_hours,
        cycle_count,
    })
}

fn optional_bool(
    object: &Map<String, Value>,
    key: &str,
    recognized_fields: &mut usize,
) -> Result<Option<bool>, AndroidBatteryError> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    *recognized_fields = recognized_fields.saturating_add(1);
    value
        .as_bool()
        .map(Some)
        .ok_or(AndroidBatteryError::InvalidField)
}

fn optional_label(
    object: &Map<String, Value>,
    key: &str,
    recognized_fields: &mut usize,
) -> Result<Option<String>, AndroidBatteryError> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    *recognized_fields = recognized_fields.saturating_add(1);
    let value = value.as_str().ok_or(AndroidBatteryError::InvalidField)?;
    if value.is_empty()
        || value.len() > MAX_STATUS_LABEL_BYTES
        || !value.bytes().all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return Err(AndroidBatteryError::InvalidField);
    }
    Ok(Some(value.to_owned()))
}

fn optional_float(
    object: &Map<String, Value>,
    key: &str,
    minimum: f64,
    maximum: f64,
    recognized_fields: &mut usize,
) -> Result<Option<f64>, AndroidBatteryError> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    *recognized_fields = recognized_fields.saturating_add(1);
    let value = value.as_f64().ok_or(AndroidBatteryError::InvalidField)?;
    if !value.is_finite() || !(minimum..=maximum).contains(&value) {
        return Err(AndroidBatteryError::InvalidField);
    }
    Ok(Some(value))
}

fn optional_integer(
    object: &Map<String, Value>,
    key: &str,
    minimum: i64,
    maximum: i64,
    recognized_fields: &mut usize,
) -> Result<Option<i64>, AndroidBatteryError> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    *recognized_fields = recognized_fields.saturating_add(1);
    let value = value.as_i64().ok_or(AndroidBatteryError::InvalidField)?;
    if !(minimum..=maximum).contains(&value) {
        return Err(AndroidBatteryError::InvalidField);
    }
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    fn executable_script_in(
        directory: &TempDir,
        script: &str,
        timeout: Duration,
    ) -> AndroidBatteryClient {
        let program = directory.path().join("battery-status");
        fs::write(&program, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        AndroidBatteryClient::with_program_and_limits(
            program,
            timeout,
            MAX_BATTERY_STDOUT_BYTES,
            MAX_BATTERY_STDERR_BYTES,
        )
    }

    fn executable_script(script: &str, timeout: Duration) -> (TempDir, AndroidBatteryClient) {
        let directory = tempfile::tempdir().unwrap();
        let client = executable_script_in(&directory, script, timeout);
        (directory, client)
    }

    fn output_script(stdout: &str, stderr: &str) -> String {
        assert!(!stdout.contains('\''));
        assert!(!stderr.contains('\''));
        format!("printf '%s' '{stdout}'\nprintf '%s' '{stderr}' >&2")
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
        .expect("provider did not publish its process identifier")
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
        .unwrap_or_else(|_| panic!("provider process {pid} survived bounded cleanup"));
    }

    async fn assert_no_active_supervisors() {
        tokio::time::timeout(Duration::from_secs(2), async {
            while ACTIVE_BATTERY_SUPERVISORS.load(std::sync::atomic::Ordering::SeqCst) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("battery supervisor task did not finish bounded cleanup");
    }

    #[test]
    fn parser_returns_only_allowlisted_normalized_fields() {
        let status = parse_battery_status(
            r#"{
                "present":true,
                "technology":"vendor-private-value",
                "health":"GOOD",
                "plugged":"PLUGGED_USB",
                "status":"CHARGING",
                "temperature":31.2,
                "voltage":4210,
                "current":-123456,
                "current_average":-120000,
                "percentage":87,
                "level":87,
                "scale":100,
                "charge_counter":4100000,
                "energy":17000000,
                "cycle":234,
                "android_id":"must-not-be-reflected"
            }"#,
        )
        .unwrap();

        assert_eq!(status.percentage, Some(87));
        assert_eq!(status.temperature_celsius, Some(31.2));
        let value = serde_json::to_value(status).unwrap();
        assert_eq!(value["voltage_millivolts"], 4210);
        assert_eq!(value["current_microamps"], -123456);
        assert!(value.get("technology").is_none());
        assert!(value.get("android_id").is_none());
        assert!(!value.to_string().contains("vendor-private-value"));
        assert!(!value.to_string().contains("must-not-be-reflected"));
    }

    #[test]
    fn parser_rejects_non_objects_empty_known_shapes_and_bad_fields() {
        assert_eq!(
            parse_battery_status("{").unwrap_err(),
            AndroidBatteryError::InvalidJson
        );

        for value in [
            json!(null),
            json!([]),
            json!({}),
            json!({"technology":"Li-ion"}),
            json!({"percentage":101}),
            json!({"temperature":-1000}),
            json!({"status":"charging"}),
            json!({"health":"GOOD\nSECRET"}),
            json!({"present":"yes"}),
            json!({"level":101,"scale":100}),
            json!({"cycle":-1}),
        ] {
            assert!(parse_battery_status(&value.to_string()).is_err(), "{value}");
        }
    }

    #[tokio::test]
    async fn fixed_program_receives_no_arguments_and_returns_bounded_status() {
        let _test_guard = ANDROID_BATTERY_TEST_LOCK.lock().await;
        let script = r#"
            test "$#" -eq 0
            test "$PWD" = /
            printf '%s' '{"percentage":42,"status":"DISCHARGING"}'
        "#;
        let (_directory, client) = executable_script(script, Duration::from_secs(1));

        let status = client.collect().await.unwrap();
        assert_eq!(status.percentage, Some(42));
        assert_eq!(status.status.as_deref(), Some("DISCHARGING"));
    }

    #[tokio::test]
    async fn exact_stdout_and_stderr_limits_are_accepted() {
        let _test_guard = ANDROID_BATTERY_TEST_LOCK.lock().await;
        let mut stdout = String::from("{\"percentage\":50}");
        stdout.push_str(&" ".repeat(MAX_BATTERY_STDOUT_BYTES - stdout.len()));
        let stderr = "x".repeat(MAX_BATTERY_STDERR_BYTES);
        let script = output_script(&stdout, &stderr);
        let (_directory, client) = executable_script(&script, Duration::from_secs(1));

        assert_eq!(client.collect().await.unwrap().percentage, Some(50));
    }

    #[tokio::test]
    async fn stdout_and_stderr_overflow_fail_without_returning_output() {
        let _test_guard = ANDROID_BATTERY_TEST_LOCK.lock().await;
        let stdout = "x".repeat(MAX_BATTERY_STDOUT_BYTES + 1);
        let script = output_script(&stdout, "");
        let (_directory, client) = executable_script(&script, Duration::from_secs(1));
        assert_eq!(
            client.collect().await.unwrap_err(),
            AndroidBatteryError::StdoutLimitExceeded
        );

        let stderr = "x".repeat(MAX_BATTERY_STDERR_BYTES + 1);
        let script = output_script("{\"percentage\":50}", &stderr);
        let (_directory, client) = executable_script(&script, Duration::from_secs(1));
        assert_eq!(
            client.collect().await.unwrap_err(),
            AndroidBatteryError::StderrLimitExceeded
        );
    }

    #[tokio::test]
    async fn endless_output_fails_promptly_and_does_not_accumulate_supervisors() {
        let _test_guard = ANDROID_BATTERY_TEST_LOCK.lock().await;
        let cases = [
            (
                "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done",
                AndroidBatteryError::StdoutLimitExceeded,
            ),
            (
                "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx' >&2; done",
                AndroidBatteryError::StderrLimitExceeded,
            ),
        ];

        for (script, expected) in cases {
            for _ in 0..8 {
                let (_directory, client) = executable_script(script, Duration::from_secs(2));
                let started_at = Instant::now();
                assert_eq!(client.collect().await.unwrap_err(), expected);
                assert!(
                    started_at.elapsed() < Duration::from_secs(1),
                    "endless output reached the timeout instead of the byte ceiling"
                );
                assert_no_active_supervisors().await;
            }
        }
    }

    #[tokio::test]
    async fn pipe_holding_descendants_are_killed_without_unbounded_reader_joins() {
        let _test_guard = ANDROID_BATTERY_TEST_LOCK.lock().await;

        for held_stream in ["stdout", "stderr"] {
            for iteration in 0..4 {
                let directory = tempfile::tempdir().unwrap();
                let direct_pid_path = directory.path().join("direct-pid");
                let descendant_pid_path = directory.path().join("descendant-pid");
                let direct_pid_text = direct_pid_path.to_string_lossy();
                let descendant_pid_text = descendant_pid_path.to_string_lossy();
                assert!(!direct_pid_text.contains('\''));
                assert!(!descendant_pid_text.contains('\''));
                let redirection = if held_stream == "stdout" {
                    "2>/dev/null"
                } else {
                    ">/dev/null"
                };
                let script = format!(
                    "printf '%s\\n' \"$$\" >'{direct_pid_text}'\n\
                     /bin/sleep 30 {redirection} &\n\
                     printf '%s\\n' \"$!\" >'{descendant_pid_text}'\n\
                     printf '%s' '{{\"percentage\":50}}'\n\
                     exit 0"
                );
                let client = executable_script_in(&directory, &script, Duration::from_millis(400));
                let started_at = Instant::now();
                assert_eq!(
                    client.collect().await.unwrap_err(),
                    AndroidBatteryError::TimedOut,
                    "held stream {held_stream}, iteration {iteration}"
                );
                assert!(started_at.elapsed() < Duration::from_secs(1));

                let direct_pid = read_pid_file(&direct_pid_path).await;
                let descendant_pid = read_pid_file(&descendant_pid_path).await;
                assert_process_gone(direct_pid).await;
                assert_process_gone(descendant_pid).await;
                assert_no_active_supervisors().await;
            }
        }
    }

    #[tokio::test]
    async fn caller_cancellation_kills_and_reaps_each_process_group() {
        let _test_guard = ANDROID_BATTERY_TEST_LOCK.lock().await;

        for iteration in 0..8 {
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
            let client = executable_script_in(&directory, &script, Duration::from_secs(2));
            let task = tokio::spawn(async move { client.collect().await });
            let direct_pid = read_pid_file(&direct_pid_path).await;
            let descendant_pid = read_pid_file(&descendant_pid_path).await;

            task.abort();
            assert!(
                task.await.unwrap_err().is_cancelled(),
                "iteration {iteration}"
            );
            assert_process_gone(direct_pid).await;
            assert_process_gone(descendant_pid).await;
            assert_no_active_supervisors().await;
        }
    }

    #[tokio::test]
    async fn cleanup_reserve_exhaustion_overrides_each_primary_failure_after_reaping() {
        let _test_guard = ANDROID_BATTERY_TEST_LOCK.lock().await;

        for terminal in ["timeout", "stdout", "stderr"] {
            for iteration in 0..4 {
                let directory = tempfile::tempdir().unwrap();
                let direct_pid_path = directory.path().join("direct-pid");
                let direct_pid_text = direct_pid_path.to_string_lossy();
                assert!(!direct_pid_text.contains('\''));
                let body = match terminal {
                    "timeout" => "/bin/sleep 30",
                    "stdout" => "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done",
                    "stderr" => "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx' >&2; done",
                    _ => unreachable!(),
                };
                let script = format!(
                    "printf '%s\\n' \"$$\" >'{direct_pid_text}'\n\
                     {body}"
                );
                let client = executable_script_in(&directory, &script, Duration::from_millis(120))
                    .with_forced_cleanup_delay(Duration::from_millis(160));

                let started_at = Instant::now();
                assert_eq!(
                    client.collect().await.unwrap_err(),
                    AndroidBatteryError::WaitFailed,
                    "terminal {terminal}, iteration {iteration}"
                );
                assert!(
                    started_at.elapsed() >= Duration::from_millis(150),
                    "late cleanup was not exercised for {terminal}, iteration {iteration}"
                );

                let direct_pid = read_pid_file(&direct_pid_path).await;
                assert_process_gone(direct_pid).await;
                assert_no_active_supervisors().await;
            }
        }
    }

    #[tokio::test]
    async fn caller_cancellation_cannot_detach_cleanup_after_reserve_exhaustion() {
        let _test_guard = ANDROID_BATTERY_TEST_LOCK.lock().await;

        for iteration in 0..4 {
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
            let client = executable_script_in(&directory, &script, Duration::from_secs(1))
                .with_forced_cleanup_delay(Duration::from_millis(1_100));
            let task = tokio::spawn(async move { client.collect().await });
            let direct_pid = read_pid_file(&direct_pid_path).await;
            let descendant_pid = read_pid_file(&descendant_pid_path).await;

            let cancelled_at = Instant::now();
            task.abort();
            assert!(
                task.await.unwrap_err().is_cancelled(),
                "iteration {iteration}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(
                ACTIVE_BATTERY_SUPERVISORS.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "cancelled caller detached its supervisor in iteration {iteration}"
            );

            assert_process_gone(direct_pid).await;
            assert_process_gone(descendant_pid).await;
            assert_no_active_supervisors().await;
            assert!(
                cancelled_at.elapsed() >= Duration::from_millis(1_000),
                "forced late cancellation cleanup was not exercised in iteration {iteration}"
            );
        }
    }

    #[tokio::test]
    async fn timeout_nonzero_invalid_utf8_and_missing_program_are_stable() {
        let _test_guard = ANDROID_BATTERY_TEST_LOCK.lock().await;
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("battery-status");
        let late_marker = directory.path().join("must-not-exist");
        fs::write(
            &program,
            format!(
                "#!/bin/sh\nset -eu\n/bin/sleep 1\nprintf late >'{}'\n",
                late_marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let client = AndroidBatteryClient::with_program_and_limits(
            program,
            Duration::from_millis(25),
            MAX_BATTERY_STDOUT_BYTES,
            MAX_BATTERY_STDERR_BYTES,
        );
        assert_eq!(
            client.collect().await.unwrap_err(),
            AndroidBatteryError::TimedOut
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!late_marker.exists());

        let (_directory, client) = executable_script("exit 7", Duration::from_secs(1));
        assert_eq!(
            client.collect().await.unwrap_err(),
            AndroidBatteryError::ApiFailed
        );

        let (_directory, client) = executable_script("printf '\\377'", Duration::from_secs(1));
        assert_eq!(
            client.collect().await.unwrap_err(),
            AndroidBatteryError::InvalidUtf8
        );

        let directory = tempfile::tempdir().unwrap();
        let client = AndroidBatteryClient::with_program_and_limits(
            directory.path().join("missing"),
            Duration::from_secs(1),
            MAX_BATTERY_STDOUT_BYTES,
            MAX_BATTERY_STDERR_BYTES,
        );
        assert_eq!(
            client.collect().await.unwrap_err(),
            AndroidBatteryError::ApiUnavailable
        );
    }

    #[test]
    fn reason_codes_are_stable_and_non_sensitive() {
        let errors = [
            AndroidBatteryError::ApiUnavailable,
            AndroidBatteryError::SpawnFailed,
            AndroidBatteryError::WaitFailed,
            AndroidBatteryError::TimedOut,
            AndroidBatteryError::StdoutLimitExceeded,
            AndroidBatteryError::StderrLimitExceeded,
            AndroidBatteryError::ApiFailed,
            AndroidBatteryError::InvalidUtf8,
            AndroidBatteryError::InvalidJson,
            AndroidBatteryError::InvalidField,
        ];
        for error in errors {
            let reason = error.reason_code();
            assert!(reason
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'));
            assert!(!reason.contains('/'));
        }
    }
}
