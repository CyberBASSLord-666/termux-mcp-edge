//! Bounded Termux:API battery telemetry for the opt-in Android platform gate.
//!
//! The production client invokes one fixed absolute program with no arguments,
//! no stdin, no inherited environment, and no shell interpolation. Process
//! output is read concurrently behind hard byte ceilings before a strict
//! allowlist parser constructs the public response.

use std::{path::PathBuf, time::Duration};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::android_provider::{AndroidProviderError, BoundedAndroidProvider};

#[cfg(test)]
pub(crate) use crate::android_provider::ANDROID_PROVIDER_TEST_LOCK as ANDROID_BATTERY_TEST_LOCK;

pub const TERMUX_BATTERY_STATUS_PROGRAM: &str =
    "/data/data/com.termux/files/usr/bin/termux-battery-status";
pub const BATTERY_STATUS_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_BATTERY_STDOUT_BYTES: usize = 16 * 1024;
pub const MAX_BATTERY_STDERR_BYTES: usize = 4 * 1024;

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

impl From<AndroidProviderError> for AndroidBatteryError {
    fn from(error: AndroidProviderError) -> Self {
        match error {
            AndroidProviderError::ProgramUnavailable => Self::ApiUnavailable,
            AndroidProviderError::SpawnFailed => Self::SpawnFailed,
            AndroidProviderError::WaitFailed => Self::WaitFailed,
            AndroidProviderError::TimedOut => Self::TimedOut,
            AndroidProviderError::StdoutLimitExceeded => Self::StdoutLimitExceeded,
            AndroidProviderError::StderrLimitExceeded => Self::StderrLimitExceeded,
            AndroidProviderError::ProgramFailed => Self::ApiFailed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AndroidBatteryClient {
    provider: BoundedAndroidProvider,
}

impl AndroidBatteryClient {
    pub fn termux() -> Self {
        Self {
            provider: BoundedAndroidProvider::new(
                PathBuf::from(TERMUX_BATTERY_STATUS_PROGRAM),
                BATTERY_STATUS_TIMEOUT,
                MAX_BATTERY_STDOUT_BYTES,
                MAX_BATTERY_STDERR_BYTES,
            ),
        }
    }

    pub async fn collect(&self) -> Result<AndroidBatteryStatus, AndroidBatteryError> {
        let stdout = self.provider.collect_stdout().await?;
        let stdout = String::from_utf8(stdout).map_err(|_| AndroidBatteryError::InvalidUtf8)?;
        parse_battery_status(&stdout)
    }

    #[cfg(test)]
    pub(crate) fn with_program_and_limits(
        program: PathBuf,
        timeout: Duration,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
    ) -> Self {
        Self {
            provider: BoundedAndroidProvider::new(
                program,
                timeout,
                max_stdout_bytes,
                max_stderr_bytes,
            ),
        }
    }

    #[cfg(test)]
    fn with_forced_cleanup_delay(mut self, delay: Duration) -> Self {
        self.provider = self.provider.with_forced_cleanup_delay(delay);
        self
    }
}

impl Default for AndroidBatteryClient {
    fn default() -> Self {
        Self::termux()
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

    use rustix::process::{test_kill_process, Pid};
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::time::Instant;

    use super::*;
    use crate::android_provider::active_supervisor_count;

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
            while active_supervisor_count() != 0 {
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
                active_supervisor_count(),
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
