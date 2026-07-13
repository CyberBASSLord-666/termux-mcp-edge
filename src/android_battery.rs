//! Bounded Termux:API battery telemetry for the opt-in Android platform gate.
//!
//! The production client invokes one fixed absolute program with no arguments,
//! no stdin, no inherited environment, and no shell interpolation. Process
//! output is read concurrently behind hard byte ceilings before a strict
//! allowlist parser constructs the public response.

use std::{io::ErrorKind, path::PathBuf, process::Stdio, time::Duration};

use serde::Serialize;
use serde_json::{Map, Value};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    task::JoinHandle,
    time::timeout,
};

pub const TERMUX_BATTERY_STATUS_PROGRAM: &str =
    "/data/data/com.termux/files/usr/bin/termux-battery-status";
pub const BATTERY_STATUS_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_BATTERY_STDOUT_BYTES: usize = 16 * 1024;
pub const MAX_BATTERY_STDERR_BYTES: usize = 4 * 1024;

#[cfg(test)]
pub(crate) static ANDROID_BATTERY_TEST_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

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
}

impl AndroidBatteryClient {
    pub fn termux() -> Self {
        Self {
            program: PathBuf::from(TERMUX_BATTERY_STATUS_PROGRAM),
            timeout: BATTERY_STATUS_TIMEOUT,
            max_stdout_bytes: MAX_BATTERY_STDOUT_BYTES,
            max_stderr_bytes: MAX_BATTERY_STDERR_BYTES,
        }
    }

    pub async fn collect(&self) -> Result<AndroidBatteryStatus, AndroidBatteryError> {
        let mut command = Command::new(&self.program);
        command
            .env_clear()
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                AndroidBatteryError::ApiUnavailable
            } else {
                AndroidBatteryError::SpawnFailed
            }
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or(AndroidBatteryError::SpawnFailed)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(AndroidBatteryError::SpawnFailed)?;
        let stdout_task = spawn_bounded_read(stdout, self.max_stdout_bytes);
        let stderr_task = spawn_bounded_read(stderr, self.max_stderr_bytes);

        let wait_result = timeout(self.timeout, child.wait()).await;
        let timed_out = wait_result.is_err();
        let wait_failed = matches!(&wait_result, Ok(Err(_)));
        if timed_out || wait_failed {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        let stdout = join_bounded_read(stdout_task).await?;
        let stderr = join_bounded_read(stderr_task).await?;

        if timed_out {
            return Err(AndroidBatteryError::TimedOut);
        }
        if wait_failed {
            return Err(AndroidBatteryError::WaitFailed);
        }
        if stdout.limit_exceeded {
            return Err(AndroidBatteryError::StdoutLimitExceeded);
        }
        if stderr.limit_exceeded {
            return Err(AndroidBatteryError::StderrLimitExceeded);
        }

        let status = wait_result
            .expect("timeout result checked")
            .map_err(|_| AndroidBatteryError::WaitFailed)?;
        if !status.success() {
            return Err(AndroidBatteryError::ApiFailed);
        }

        let stdout =
            String::from_utf8(stdout.bytes).map_err(|_| AndroidBatteryError::InvalidUtf8)?;
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
            program,
            timeout,
            max_stdout_bytes,
            max_stderr_bytes,
        }
    }
}

impl Default for AndroidBatteryClient {
    fn default() -> Self {
        Self::termux()
    }
}

struct BoundedRead {
    bytes: Vec<u8>,
    limit_exceeded: bool,
}

fn spawn_bounded_read(
    mut reader: impl AsyncRead + Unpin + Send + 'static,
    limit: usize,
) -> JoinHandle<Result<BoundedRead, AndroidBatteryError>> {
    tokio::spawn(async move {
        let mut bytes = Vec::with_capacity(limit);
        let mut chunk = [0_u8; 4 * 1024];
        let mut limit_exceeded = false;

        loop {
            let read = reader
                .read(&mut chunk)
                .await
                .map_err(|_| AndroidBatteryError::WaitFailed)?;
            if read == 0 {
                break;
            }

            let remaining = limit.saturating_sub(bytes.len());
            let retained = remaining.min(read);
            bytes.extend_from_slice(&chunk[..retained]);
            if retained < read {
                limit_exceeded = true;
            }
        }

        Ok(BoundedRead {
            bytes,
            limit_exceeded,
        })
    })
}

async fn join_bounded_read(
    task: JoinHandle<Result<BoundedRead, AndroidBatteryError>>,
) -> Result<BoundedRead, AndroidBatteryError> {
    task.await.map_err(|_| AndroidBatteryError::WaitFailed)?
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
    use std::{fs, os::unix::fs::PermissionsExt};

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    fn executable_script(script: &str, timeout: Duration) -> (TempDir, AndroidBatteryClient) {
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("battery-status");
        fs::write(&program, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let client = AndroidBatteryClient::with_program_and_limits(
            program,
            timeout,
            MAX_BATTERY_STDOUT_BYTES,
            MAX_BATTERY_STDERR_BYTES,
        );
        (directory, client)
    }

    fn output_script(stdout: &str, stderr: &str) -> String {
        assert!(!stdout.contains('\''));
        assert!(!stderr.contains('\''));
        format!("printf '%s' '{stdout}'\nprintf '%s' '{stderr}' >&2")
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
