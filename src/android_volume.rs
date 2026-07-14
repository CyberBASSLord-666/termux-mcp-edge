//! Bounded read-only Android audio-volume telemetry through Termux:API.
//!
//! The client calls only the fixed `termux-volume` executable with zero
//! arguments. The shared Android provider supervisor enforces cancellation,
//! process-group, deadline, reaping, and output guarantees before this module
//! accepts a strict six-stream JSON contract.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use serde::Serialize;
use serde_json::Value;

use crate::android_provider::{AndroidProviderError, BoundedAndroidProvider};

pub const TERMUX_VOLUME_PROGRAM: &str = "/data/data/com.termux/files/usr/bin/termux-volume";
pub const VOLUME_STATUS_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_VOLUME_STDOUT_BYTES: usize = 8 * 1024;
pub const MAX_VOLUME_STDERR_BYTES: usize = 4 * 1024;
const MAX_VOLUME_INDEX: i64 = 10_000;
const VOLUME_STREAM_NAMES: [&str; 6] = ["alarm", "call", "music", "notification", "ring", "system"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AndroidVolumeStatus {
    pub streams: Vec<AndroidVolumeStream>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidVolumeStream {
    pub stream: String,
    pub volume: i64,
    pub max_volume: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndroidVolumeError {
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

impl AndroidVolumeError {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::ApiUnavailable => "volume_api_unavailable",
            Self::SpawnFailed => "volume_api_spawn_failed",
            Self::WaitFailed => "volume_api_wait_failed",
            Self::TimedOut => "volume_api_timeout",
            Self::StdoutLimitExceeded => "volume_stdout_limit_exceeded",
            Self::StderrLimitExceeded => "volume_stderr_limit_exceeded",
            Self::ApiFailed => "volume_api_failed",
            Self::InvalidUtf8 => "volume_output_invalid_utf8",
            Self::InvalidJson => "volume_output_invalid_json",
            Self::InvalidField => "volume_output_invalid_field",
        }
    }
}

impl From<AndroidProviderError> for AndroidVolumeError {
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
pub struct AndroidVolumeClient {
    provider: BoundedAndroidProvider,
}

impl AndroidVolumeClient {
    pub fn termux() -> Self {
        Self {
            provider: BoundedAndroidProvider::new(
                PathBuf::from(TERMUX_VOLUME_PROGRAM),
                VOLUME_STATUS_TIMEOUT,
                MAX_VOLUME_STDOUT_BYTES,
                MAX_VOLUME_STDERR_BYTES,
            )
            .expect("fixed volume provider timeout must reserve cleanup time"),
        }
    }

    pub async fn collect(&self) -> Result<AndroidVolumeStatus, AndroidVolumeError> {
        let stdout = self.provider.collect_stdout().await?;
        let stdout = String::from_utf8(stdout).map_err(|_| AndroidVolumeError::InvalidUtf8)?;
        parse_volume_status(&stdout)
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
            )
            .expect("test volume provider timeout must reserve cleanup time"),
        }
    }
}

impl Default for AndroidVolumeClient {
    fn default() -> Self {
        Self::termux()
    }
}

fn parse_volume_status(input: &str) -> Result<AndroidVolumeStatus, AndroidVolumeError> {
    let value: Value = serde_json::from_str(input).map_err(|_| AndroidVolumeError::InvalidJson)?;
    let entries = value.as_array().ok_or(AndroidVolumeError::InvalidJson)?;
    if entries.len() != VOLUME_STREAM_NAMES.len() {
        return Err(AndroidVolumeError::InvalidField);
    }

    let mut streams = BTreeMap::new();
    for entry in entries {
        let object = entry.as_object().ok_or(AndroidVolumeError::InvalidField)?;
        if object.len() != 3
            || object
                .keys()
                .any(|key| !matches!(key.as_str(), "stream" | "volume" | "max_volume"))
        {
            return Err(AndroidVolumeError::InvalidField);
        }

        let stream = object
            .get("stream")
            .and_then(Value::as_str)
            .filter(|stream| VOLUME_STREAM_NAMES.contains(stream))
            .ok_or(AndroidVolumeError::InvalidField)?;
        let volume = object
            .get("volume")
            .and_then(Value::as_i64)
            .ok_or(AndroidVolumeError::InvalidField)?;
        let max_volume = object
            .get("max_volume")
            .and_then(Value::as_i64)
            .ok_or(AndroidVolumeError::InvalidField)?;
        if !(1..=MAX_VOLUME_INDEX).contains(&max_volume) || !(0..=max_volume).contains(&volume) {
            return Err(AndroidVolumeError::InvalidField);
        }
        if streams
            .insert(stream.to_owned(), (volume, max_volume))
            .is_some()
        {
            return Err(AndroidVolumeError::InvalidField);
        }
    }

    if streams.len() != VOLUME_STREAM_NAMES.len() {
        return Err(AndroidVolumeError::InvalidField);
    }

    let streams = VOLUME_STREAM_NAMES
        .into_iter()
        .map(|stream| {
            let (volume, max_volume) = streams
                .remove(stream)
                .expect("all canonical volume streams checked before normalization");
            AndroidVolumeStream {
                stream: stream.to_owned(),
                volume,
                max_volume,
            }
        })
        .collect();

    Ok(AndroidVolumeStatus { streams })
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::android_provider::ANDROID_PROVIDER_TEST_LOCK;

    fn valid_status() -> Value {
        json!([
            {"stream":"system","volume":2,"max_volume":7},
            {"stream":"notification","volume":3,"max_volume":7},
            {"stream":"alarm","volume":4,"max_volume":7},
            {"stream":"music","volume":5,"max_volume":15},
            {"stream":"call","volume":1,"max_volume":5},
            {"stream":"ring","volume":6,"max_volume":7}
        ])
    }

    fn executable_script(script: &str, timeout: Duration) -> (TempDir, AndroidVolumeClient) {
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("volume-status");
        fs::write(&program, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let client = AndroidVolumeClient::with_program_and_limits(
            program,
            timeout,
            MAX_VOLUME_STDOUT_BYTES,
            MAX_VOLUME_STDERR_BYTES,
        );
        (directory, client)
    }

    #[test]
    fn parser_requires_exact_stream_set_and_returns_canonical_public_shape() {
        let status = parse_volume_status(&valid_status().to_string()).unwrap();
        assert_eq!(
            status
                .streams
                .iter()
                .map(|entry| entry.stream.as_str())
                .collect::<Vec<_>>(),
            VOLUME_STREAM_NAMES
        );
        assert_eq!(status.streams[0].volume, 4);

        let value = serde_json::to_value(status).unwrap();
        assert_eq!(value["streams"][0]["stream"], "alarm");
        assert_eq!(value["streams"][0]["maxVolume"], 7);
        assert!(!value.to_string().contains("max_volume"));
    }

    #[test]
    fn parser_rejects_malformed_missing_duplicate_unknown_and_out_of_policy_values() {
        assert_eq!(
            parse_volume_status("{").unwrap_err(),
            AndroidVolumeError::InvalidJson
        );
        assert_eq!(
            parse_volume_status("{}").unwrap_err(),
            AndroidVolumeError::InvalidJson
        );

        let mut cases = vec![json!([]), json!([null])];

        let mut missing = valid_status().as_array().unwrap().clone();
        missing.pop();
        cases.push(Value::Array(missing));

        let mut duplicate = valid_status().as_array().unwrap().clone();
        duplicate[5]["stream"] = json!("alarm");
        cases.push(Value::Array(duplicate));

        for (field, value) in [
            ("stream", json!("vendor-private")),
            ("volume", json!(-1)),
            ("volume", json!(8)),
            ("volume", json!(1.5)),
            ("max_volume", json!(0)),
            ("max_volume", json!(10_001)),
            ("max_volume", json!("7")),
        ] {
            let mut invalid = valid_status();
            invalid[0][field] = value;
            cases.push(invalid);
        }

        let mut extra_field = valid_status();
        extra_field[0]["device_id"] = json!("must-not-be-reflected");
        cases.push(extra_field);

        for value in cases {
            assert_eq!(
                parse_volume_status(&value.to_string()).unwrap_err(),
                AndroidVolumeError::InvalidField,
                "{value}"
            );
        }
    }

    #[tokio::test]
    async fn fixed_provider_receives_no_arguments_or_environment() {
        let _test_guard = ANDROID_PROVIDER_TEST_LOCK.lock().await;
        let status = valid_status().to_string();
        assert!(!status.contains('\''));
        let script = format!(
            "test \"$#\" -eq 0\n\
             test \"$PWD\" = /\n\
             test \"$(/usr/bin/readlink /proc/self/fd/0)\" = /dev/null\n\
             test -z \"${{TERMUX_MCP_VOLUME_TEST_SECRET+x}}\"\n\
             printf '%s' '{status}'"
        );
        std::env::set_var("TERMUX_MCP_VOLUME_TEST_SECRET", "must-not-be-inherited");
        let (_directory, client) = executable_script(&script, Duration::from_secs(1));
        let result = client.collect().await;
        std::env::remove_var("TERMUX_MCP_VOLUME_TEST_SECRET");

        let result = result.unwrap();
        assert_eq!(result.streams.len(), 6);
        assert_eq!(result.streams[2].stream, "music");
    }

    #[tokio::test]
    async fn output_limits_and_provider_failures_use_stable_errors() {
        let _test_guard = ANDROID_PROVIDER_TEST_LOCK.lock().await;

        let stdout = "x".repeat(MAX_VOLUME_STDOUT_BYTES + 1);
        let script = format!("printf '%s' '{stdout}'");
        let (_directory, client) = executable_script(&script, Duration::from_secs(1));
        assert_eq!(
            client.collect().await.unwrap_err(),
            AndroidVolumeError::StdoutLimitExceeded
        );

        let stderr = "x".repeat(MAX_VOLUME_STDERR_BYTES + 1);
        let script = format!("printf '%s' '{stderr}' >&2");
        let (_directory, client) = executable_script(&script, Duration::from_secs(1));
        assert_eq!(
            client.collect().await.unwrap_err(),
            AndroidVolumeError::StderrLimitExceeded
        );

        let (_directory, client) =
            executable_script("exec /bin/sleep 30", Duration::from_millis(30));
        assert_eq!(
            client.collect().await.unwrap_err(),
            AndroidVolumeError::TimedOut
        );

        let (_directory, client) = executable_script("exit 7", Duration::from_secs(1));
        assert_eq!(
            client.collect().await.unwrap_err(),
            AndroidVolumeError::ApiFailed
        );

        let (_directory, client) = executable_script("printf '\\377'", Duration::from_secs(1));
        assert_eq!(
            client.collect().await.unwrap_err(),
            AndroidVolumeError::InvalidUtf8
        );
    }

    #[test]
    fn reason_codes_are_stable_and_non_sensitive() {
        let cases = [
            (AndroidVolumeError::ApiUnavailable, "volume_api_unavailable"),
            (AndroidVolumeError::SpawnFailed, "volume_api_spawn_failed"),
            (AndroidVolumeError::WaitFailed, "volume_api_wait_failed"),
            (AndroidVolumeError::TimedOut, "volume_api_timeout"),
            (
                AndroidVolumeError::StdoutLimitExceeded,
                "volume_stdout_limit_exceeded",
            ),
            (
                AndroidVolumeError::StderrLimitExceeded,
                "volume_stderr_limit_exceeded",
            ),
            (AndroidVolumeError::ApiFailed, "volume_api_failed"),
            (
                AndroidVolumeError::InvalidUtf8,
                "volume_output_invalid_utf8",
            ),
            (
                AndroidVolumeError::InvalidJson,
                "volume_output_invalid_json",
            ),
            (
                AndroidVolumeError::InvalidField,
                "volume_output_invalid_field",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.reason_code(), expected);
            assert!(error.reason_code().is_ascii());
            assert!(!error.reason_code().contains('/'));
        }
    }
}
