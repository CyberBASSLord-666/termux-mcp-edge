//! Bounded, privacy-preserving Termux:API battery telemetry.
//!
//! This module deliberately exposes one fixed read-only program. Callers cannot
//! provide an executable, arguments, environment, stdin, working directory, or
//! shell text. Raw command output and operating-system errors never cross the
//! public error boundary.

use std::{collections::BTreeMap, path::Path, process::Stdio, time::Duration};

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;
use tokio::{io::AsyncReadExt, process::Command, time::timeout};

const TERMUX_BATTERY_STATUS: &str =
    "/data/data/com.termux/files/usr/bin/termux-battery-status";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_STDOUT_BYTES: usize = 16 * 1024;
const MAX_STDERR_BYTES: usize = 4 * 1024;
const MAX_LABEL_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum BatteryValue {
    Boolean(bool),
    Integer(i64),
    Decimal(f64),
    Label(String),
}

pub type BatteryStatus = BTreeMap<String, BatteryValue>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BatteryStatusError {
    #[error("android_battery_command_unavailable")]
    CommandUnavailable,
    #[error("android_battery_command_failed")]
    CommandFailed,
    #[error("android_battery_timeout")]
    Timeout,
    #[error("android_battery_stdout_limit_exceeded")]
    StdoutLimitExceeded,
    #[error("android_battery_stderr_limit_exceeded")]
    StderrLimitExceeded,
    #[error("android_battery_invalid_utf8")]
    InvalidUtf8,
    #[error("android_battery_invalid_json")]
    InvalidJson,
    #[error("android_battery_invalid_shape")]
    InvalidShape,
    #[error("android_battery_invalid_field")]
    InvalidField,
    #[error("android_battery_internal_failure")]
    InternalFailure,
}

/// Execute the single reviewed Termux:API battery command and return only
/// allowlisted, range-checked fields.
pub async fn collect_battery_status() -> Result<BatteryStatus, BatteryStatusError> {
    collect_from(Path::new(TERMUX_BATTERY_STATUS)).await
}

async fn collect_from(program: &Path) -> Result<BatteryStatus, BatteryStatusError> {
    let mut command = Command::new(program);
    command
        .args([] as [&str; 0])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            BatteryStatusError::CommandUnavailable
        } else {
            BatteryStatusError::InternalFailure
        }
    })?;

    let stdout = child
        .stdout
        .take()
        .ok_or(BatteryStatusError::InternalFailure)?;
    let stderr = child
        .stderr
        .take()
        .ok_or(BatteryStatusError::InternalFailure)?;

    let stdout_task = tokio::spawn(read_bounded(stdout, MAX_STDOUT_BYTES));
    let stderr_task = tokio::spawn(read_bounded(stderr, MAX_STDERR_BYTES));

    let status = match timeout(COMMAND_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => return Err(BatteryStatusError::InternalFailure),
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(BatteryStatusError::Timeout);
        }
    };

    let stdout = stdout_task
        .await
        .map_err(|_| BatteryStatusError::InternalFailure)?
        .map_err(|error| match error {
            ReadError::Limit => BatteryStatusError::StdoutLimitExceeded,
            ReadError::Io => BatteryStatusError::InternalFailure,
        })?;
    let _stderr = stderr_task
        .await
        .map_err(|_| BatteryStatusError::InternalFailure)?
        .map_err(|error| match error {
            ReadError::Limit => BatteryStatusError::StderrLimitExceeded,
            ReadError::Io => BatteryStatusError::InternalFailure,
        })?;

    if !status.success() {
        return Err(BatteryStatusError::CommandFailed);
    }

    parse_battery_status(&stdout)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadError {
    Limit,
    Io,
}

async fn read_bounded(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    limit: usize,
) -> Result<Vec<u8>, ReadError> {
    let mut output = Vec::with_capacity(limit.min(4096));
    let mut chunk = [0_u8; 1024];
    loop {
        let count = reader.read(&mut chunk).await.map_err(|_| ReadError::Io)?;
        if count == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(count) > limit {
            return Err(ReadError::Limit);
        }
        output.extend_from_slice(&chunk[..count]);
    }
}

fn parse_battery_status(bytes: &[u8]) -> Result<BatteryStatus, BatteryStatusError> {
    let text = std::str::from_utf8(bytes).map_err(|_| BatteryStatusError::InvalidUtf8)?;
    let value: Value = serde_json::from_str(text).map_err(|_| BatteryStatusError::InvalidJson)?;
    let object = value
        .as_object()
        .ok_or(BatteryStatusError::InvalidShape)?;

    let mut response = BatteryStatus::new();
    copy_bool(object, &mut response, "present")?;
    for key in ["health", "plugged", "status"] {
        copy_label(object, &mut response, key)?;
    }
    for (key, min, max) in [
        ("percentage", 0, 100),
        ("level", 0, 1_000_000),
        ("scale", 1, 1_000_000),
        ("voltage", -1, 100_000_000),
        ("current", -100_000_000, 100_000_000),
        ("current_average", -100_000_000, 100_000_000),
        ("charge_counter", -1, 1_000_000_000),
        ("energy", -1, i64::MAX),
        ("cycle", -1, 1_000_000),
    ] {
        copy_integer(object, &mut response, key, min, max)?;
    }
    copy_decimal(object, &mut response, "temperature", -100.0, 200.0)?;
    Ok(response)
}

fn copy_bool(
    source: &Map<String, Value>,
    target: &mut BatteryStatus,
    key: &str,
) -> Result<(), BatteryStatusError> {
    let Some(value) = source.get(key) else {
        return Ok(());
    };
    let value = value.as_bool().ok_or(BatteryStatusError::InvalidField)?;
    target.insert(key.to_owned(), BatteryValue::Boolean(value));
    Ok(())
}

fn copy_label(
    source: &Map<String, Value>,
    target: &mut BatteryStatus,
    key: &str,
) -> Result<(), BatteryStatusError> {
    let Some(value) = source.get(key) else {
        return Ok(());
    };
    let label = value.as_str().ok_or(BatteryStatusError::InvalidField)?;
    if label.is_empty()
        || label.len() > MAX_LABEL_BYTES
        || !label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b' '))
    {
        return Err(BatteryStatusError::InvalidField);
    }
    target.insert(key.to_owned(), BatteryValue::Label(label.to_owned()));
    Ok(())
}

fn copy_integer(
    source: &Map<String, Value>,
    target: &mut BatteryStatus,
    key: &str,
    min: i64,
    max: i64,
) -> Result<(), BatteryStatusError> {
    let Some(value) = source.get(key) else {
        return Ok(());
    };
    let number = value.as_i64().ok_or(BatteryStatusError::InvalidField)?;
    if !(min..=max).contains(&number) {
        return Err(BatteryStatusError::InvalidField);
    }
    target.insert(key.to_owned(), BatteryValue::Integer(number));
    Ok(())
}

fn copy_decimal(
    source: &Map<String, Value>,
    target: &mut BatteryStatus,
    key: &str,
    min: f64,
    max: f64,
) -> Result<(), BatteryStatusError> {
    let Some(value) = source.get(key) else {
        return Ok(());
    };
    let number = value.as_f64().ok_or(BatteryStatusError::InvalidField)?;
    if !number.is_finite() || !(min..=max).contains(&number) {
        return Err(BatteryStatusError::InvalidField);
    }
    target.insert(key.to_owned(), BatteryValue::Decimal(number));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_returns_only_reviewed_fields() {
        let parsed = parse_battery_status(
            br#"{
                "present": true,
                "health": "GOOD",
                "plugged": "UNPLUGGED",
                "status": "DISCHARGING",
                "temperature": 31.2,
                "voltage": 4187,
                "current": -420,
                "percentage": 74,
                "cycle": 143,
                "technology": "sensitive-vendor-text",
                "device_id": "must-not-escape"
            }"#,
        )
        .unwrap();

        assert_eq!(parsed.len(), 9);
        assert!(!parsed.contains_key("technology"));
        assert!(!parsed.contains_key("device_id"));
    }

    #[test]
    fn parser_rejects_non_object_and_invalid_fields() {
        assert_eq!(
            parse_battery_status(br#"[]"#),
            Err(BatteryStatusError::InvalidShape)
        );
        assert_eq!(
            parse_battery_status(br#"{"percentage":101}"#),
            Err(BatteryStatusError::InvalidField)
        );
        assert_eq!(
            parse_battery_status(br#"{"status":"BAD\nLABEL"}"#),
            Err(BatteryStatusError::InvalidField)
        );
    }

    #[test]
    fn parser_distinguishes_utf8_and_json_failures() {
        assert_eq!(
            parse_battery_status(&[0xff]),
            Err(BatteryStatusError::InvalidUtf8)
        );
        assert_eq!(
            parse_battery_status(b"not-json"),
            Err(BatteryStatusError::InvalidJson)
        );
    }

    #[tokio::test]
    async fn bounded_reader_accepts_exact_limit_and_rejects_over_limit() {
        assert_eq!(
            read_bounded(&b"abcd"[..], 4).await.unwrap(),
            b"abcd".to_vec()
        );
        assert_eq!(read_bounded(&b"abcde"[..], 4).await, Err(ReadError::Limit));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_program_proves_no_arguments_and_cleared_environment() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("battery-status");
        std::fs::write(
            &program,
            "#!/bin/sh\n[ \"$#\" -eq 0 ] || exit 10\n[ -z \"${PATH+x}\" ] || exit 11\nprintf '%s' '{\"present\":true,\"percentage\":50}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();

        let result = collect_from(&program).await.unwrap();
        assert_eq!(result.get("percentage"), Some(&BatteryValue::Integer(50)));
    }
}
