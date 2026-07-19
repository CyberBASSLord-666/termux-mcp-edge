//! Request-authorized, preview-first Android audio-volume control.
//!
//! The public surface is deliberately narrower than the upstream Termux:API
//! wrapper. Callers select one value from the documented six-stream enum and
//! one integer level. Execution always uses the fixed absolute `termux-volume`
//! program, a cleared environment, `/` as the working directory, null stdin,
//! bounded output, and the shared cancellation-safe process supervisor. The
//! public client surface is preview-only; live preparation and execution stay
//! crate-private behind the transport's request-grant boundary.

use std::{ffi::OsString, path::PathBuf, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{
    android_volume::{
        AndroidVolumeClient, AndroidVolumeError, AndroidVolumeStream, MAX_VOLUME_STDERR_BYTES,
        MAX_VOLUME_STDOUT_BYTES, TERMUX_VOLUME_PROGRAM,
    },
    bounded_process::{BoundedProcess, BoundedProcessError},
};

pub const VOLUME_CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
pub const VOLUME_CONTROL_CONCURRENCY: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AndroidVolumeStreamName {
    Alarm,
    Call,
    Music,
    Notification,
    Ring,
    System,
}

impl AndroidVolumeStreamName {
    pub const ALL: [Self; 6] = [
        Self::Alarm,
        Self::Call,
        Self::Music,
        Self::Notification,
        Self::Ring,
        Self::System,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Alarm => "alarm",
            Self::Call => "call",
            Self::Music => "music",
            Self::Notification => "notification",
            Self::Ring => "ring",
            Self::System => "system",
        }
    }

    pub const fn grant_code(self) -> u8 {
        match self {
            Self::Alarm => 1,
            Self::Call => 2,
            Self::Music => 3,
            Self::Notification => 4,
            Self::Ring => 5,
            Self::System => 6,
        }
    }

    pub const fn from_grant_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Alarm),
            2 => Some(Self::Call),
            3 => Some(Self::Music),
            4 => Some(Self::Notification),
            5 => Some(Self::Ring),
            6 => Some(Self::System),
            _ => None,
        }
    }
}

impl std::str::FromStr for AndroidVolumeStreamName {
    type Err = AndroidVolumeControlError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|stream| stream.as_str() == value)
            .ok_or(AndroidVolumeControlError::InvalidStream)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidVolumeControlResult {
    pub stream: AndroidVolumeStreamName,
    pub previous_level: i64,
    pub requested_level: i64,
    pub max_volume: i64,
    pub dry_run: bool,
    pub changed: bool,
    pub verified: bool,
    pub outcome: &'static str,
    pub rollback: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndroidVolumeControlError {
    InvalidStream,
    LevelOutOfRange,
    ConcurrencyLimitExceeded,
    StatusUnavailable(AndroidVolumeError),
    SetFailedRollbackConfirmed,
    SetFailedRollbackUnconfirmed,
    VerificationFailedRollbackConfirmed,
    VerificationFailedRollbackUnconfirmed,
    WorkerFailed,
}

impl AndroidVolumeControlError {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidStream => "volume_control_stream_invalid",
            Self::LevelOutOfRange => "volume_control_level_out_of_range",
            Self::ConcurrencyLimitExceeded => "volume_control_concurrency_limit",
            Self::StatusUnavailable(error) => error.reason_code(),
            Self::SetFailedRollbackConfirmed => "volume_control_set_failed_rollback_confirmed",
            Self::SetFailedRollbackUnconfirmed => "volume_control_set_failed_rollback_unconfirmed",
            Self::VerificationFailedRollbackConfirmed => {
                "volume_control_verification_failed_rollback_confirmed"
            }
            Self::VerificationFailedRollbackUnconfirmed => {
                "volume_control_verification_failed_rollback_unconfirmed"
            }
            Self::WorkerFailed => "volume_control_worker_failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AndroidVolumeControlClient {
    status: AndroidVolumeClient,
    program: PathBuf,
    timeout: Duration,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
    mutation_lane: Arc<Semaphore>,
}

impl AndroidVolumeControlClient {
    pub fn termux() -> Self {
        Self {
            status: AndroidVolumeClient::termux(),
            program: PathBuf::from(TERMUX_VOLUME_PROGRAM),
            timeout: VOLUME_CONTROL_TIMEOUT,
            max_stdout_bytes: MAX_VOLUME_STDOUT_BYTES,
            max_stderr_bytes: MAX_VOLUME_STDERR_BYTES,
            mutation_lane: Arc::new(Semaphore::new(VOLUME_CONTROL_CONCURRENCY)),
        }
    }

    pub async fn preview(
        &self,
        stream: AndroidVolumeStreamName,
        requested_level: i64,
    ) -> Result<AndroidVolumeControlResult, AndroidVolumeControlError> {
        let before = self.read_stream(stream).await?;
        validate_level(requested_level, before.max_volume)?;
        Ok(AndroidVolumeControlResult {
            stream,
            previous_level: before.volume,
            requested_level,
            max_volume: before.max_volume,
            dry_run: true,
            changed: false,
            verified: false,
            outcome: "preview",
            rollback: "not_required",
        })
    }

    /// Reserve the one non-queueing mutation lane and capture fresh status.
    ///
    /// Authorization is intentionally not accepted here. The caller validates
    /// and consumes the request grant only after this preparation succeeds and
    /// immediately before spawning [`PreparedAndroidVolumeMutation::execute`].
    pub(crate) async fn prepare_mutation(
        &self,
        stream: AndroidVolumeStreamName,
        requested_level: i64,
    ) -> Result<PreparedAndroidVolumeMutation, AndroidVolumeControlError> {
        let permit = Arc::clone(&self.mutation_lane)
            .try_acquire_owned()
            .map_err(|_| AndroidVolumeControlError::ConcurrencyLimitExceeded)?;
        let before = self.read_stream(stream).await?;
        validate_level(requested_level, before.max_volume)?;
        Ok(PreparedAndroidVolumeMutation {
            client: self.clone(),
            _permit: permit,
            stream,
            requested_level,
            before,
        })
    }

    async fn read_stream(
        &self,
        stream: AndroidVolumeStreamName,
    ) -> Result<AndroidVolumeStream, AndroidVolumeControlError> {
        self.status
            .collect()
            .await
            .map_err(AndroidVolumeControlError::StatusUnavailable)?
            .streams
            .into_iter()
            .find(|entry| entry.stream == stream.as_str())
            .ok_or(AndroidVolumeControlError::StatusUnavailable(
                AndroidVolumeError::InvalidField,
            ))
    }

    async fn set_level(
        &self,
        stream: AndroidVolumeStreamName,
        level: i64,
    ) -> Result<(), BoundedProcessError> {
        let process = BoundedProcess::new(
            self.program.clone(),
            vec![
                OsString::from(stream.as_str()),
                OsString::from(level.to_string()),
            ],
            PathBuf::from("/"),
            self.timeout,
            self.max_stdout_bytes,
            self.max_stderr_bytes,
        )
        .expect("validated volume control timeout must reserve cleanup time");
        process.run().await.map(|_| ())
    }

    #[cfg(test)]
    pub(crate) fn with_program_and_limits(
        program: PathBuf,
        timeout: Duration,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
    ) -> Self {
        Self {
            status: AndroidVolumeClient::with_program_and_limits(
                program.clone(),
                timeout,
                max_stdout_bytes,
                max_stderr_bytes,
            ),
            program,
            timeout,
            max_stdout_bytes,
            max_stderr_bytes,
            mutation_lane: Arc::new(Semaphore::new(VOLUME_CONTROL_CONCURRENCY)),
        }
    }
}

impl Default for AndroidVolumeControlClient {
    fn default() -> Self {
        Self::termux()
    }
}

#[derive(Debug)]
pub(crate) struct PreparedAndroidVolumeMutation {
    client: AndroidVolumeControlClient,
    _permit: OwnedSemaphorePermit,
    stream: AndroidVolumeStreamName,
    requested_level: i64,
    before: AndroidVolumeStream,
}

impl PreparedAndroidVolumeMutation {
    /// Execute, verify, and recover one already-authorized mutation.
    ///
    /// This value is intended to be moved into an independently owned Tokio
    /// task. Dropping the HTTP request future then detaches, rather than
    /// cancels, the recovery sequence.
    pub(crate) async fn execute(
        self,
    ) -> Result<AndroidVolumeControlResult, AndroidVolumeControlError> {
        if self
            .client
            .set_level(self.stream, self.requested_level)
            .await
            .is_err()
        {
            return Err(self.recover(true).await);
        }

        let verified = self
            .client
            .read_stream(self.stream)
            .await
            .is_ok_and(|after| after.volume == self.requested_level);
        if !verified {
            return Err(self.recover(false).await);
        }

        Ok(AndroidVolumeControlResult {
            stream: self.stream,
            previous_level: self.before.volume,
            requested_level: self.requested_level,
            max_volume: self.before.max_volume,
            dry_run: false,
            changed: self.before.volume != self.requested_level,
            verified: true,
            outcome: "mutation_verified",
            rollback: "not_required",
        })
    }

    async fn recover(&self, command_failed: bool) -> AndroidVolumeControlError {
        let restored = self
            .client
            .set_level(self.stream, self.before.volume)
            .await
            .is_ok()
            && self
                .client
                .read_stream(self.stream)
                .await
                .is_ok_and(|status| status.volume == self.before.volume);

        match (command_failed, restored) {
            (true, true) => AndroidVolumeControlError::SetFailedRollbackConfirmed,
            (true, false) => AndroidVolumeControlError::SetFailedRollbackUnconfirmed,
            (false, true) => AndroidVolumeControlError::VerificationFailedRollbackConfirmed,
            (false, false) => AndroidVolumeControlError::VerificationFailedRollbackUnconfirmed,
        }
    }
}

fn validate_level(level: i64, max_volume: i64) -> Result<(), AndroidVolumeControlError> {
    if (0..=max_volume).contains(&level) {
        Ok(())
    } else {
        Err(AndroidVolumeControlError::LevelOutOfRange)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use tempfile::TempDir;

    use super::*;
    use crate::android_provider::ANDROID_PROVIDER_TEST_LOCK;

    fn fixture(script_body: &str) -> (TempDir, PathBuf, AndroidVolumeControlClient) {
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("termux-volume");
        fs::write(&program, format!("#!/bin/sh\nset -eu\n{script_body}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let client = AndroidVolumeControlClient::with_program_and_limits(
            program,
            Duration::from_secs(2),
            MAX_VOLUME_STDOUT_BYTES,
            MAX_VOLUME_STDERR_BYTES,
        );
        let root_path = root.path().to_path_buf();
        (root, root_path, client)
    }

    fn stateful_script(root: &std::path::Path) -> String {
        let state = root.join("level");
        let log = root.join("calls");
        fs::write(&state, "5\n").unwrap();
        format!(
            r#"
state='{state}'
log='{log}'
if [ "$#" -eq 0 ]; then
  IFS= read -r music <"$state"
  printf '[{{"stream":"alarm","volume":1,"max_volume":7}},{{"stream":"call","volume":1,"max_volume":5}},{{"stream":"music","volume":%s,"max_volume":15}},{{"stream":"notification","volume":2,"max_volume":7}},{{"stream":"ring","volume":3,"max_volume":7}},{{"stream":"system","volume":2,"max_volume":7}}]' "$music"
  exit 0
fi
printf '%s:%s:%s:%s\n' "$#" "$1" "$2" "$PWD" >>"$log"
printf '%s\n' "$2" >"$state"
"#,
            state = state.display(),
            log = log.display(),
        )
    }

    #[test]
    fn stream_contract_is_exact_and_canonical() {
        assert_eq!(
            AndroidVolumeStreamName::ALL.map(AndroidVolumeStreamName::as_str),
            ["alarm", "call", "music", "notification", "ring", "system"]
        );
        for stream in AndroidVolumeStreamName::ALL {
            assert_eq!(
                AndroidVolumeStreamName::from_grant_code(stream.grant_code()),
                Some(stream)
            );
            assert_eq!(stream.as_str().parse(), Ok(stream));
            assert_eq!(
                serde_json::to_string(&stream).unwrap(),
                format!("\"{}\"", stream.as_str())
            );
        }
        assert_eq!(
            "media".parse::<AndroidVolumeStreamName>(),
            Err(AndroidVolumeControlError::InvalidStream)
        );
    }

    #[tokio::test]
    async fn preview_is_bounded_and_never_invokes_mutation_mode() {
        let _lock = ANDROID_PROVIDER_TEST_LOCK.lock().await;
        let root = tempfile::tempdir().unwrap();
        let script = stateful_script(root.path());
        let (_program_root, _, client) = fixture(&script);
        let result = client
            .preview(AndroidVolumeStreamName::Music, 9)
            .await
            .unwrap();
        assert!(result.dry_run);
        assert!(!result.changed);
        assert!(!result.verified);
        assert_eq!(result.previous_level, 5);
        assert_eq!(result.requested_level, 9);
        assert!(!root.path().join("calls").exists());
        assert_eq!(
            client.preview(AndroidVolumeStreamName::Music, 16).await,
            Err(AndroidVolumeControlError::LevelOutOfRange)
        );
    }

    #[tokio::test]
    async fn mutation_uses_exact_two_arguments_fixed_cwd_and_verifies() {
        let _lock = ANDROID_PROVIDER_TEST_LOCK.lock().await;
        let root = tempfile::tempdir().unwrap();
        let script = stateful_script(root.path());
        let (_program_root, _, client) = fixture(&script);
        let prepared = client
            .prepare_mutation(AndroidVolumeStreamName::Music, 9)
            .await
            .unwrap();
        let result = prepared.execute().await.unwrap();
        assert_eq!(result.outcome, "mutation_verified");
        assert!(result.changed);
        assert!(result.verified);
        assert_eq!(
            fs::read_to_string(root.path().join("level")).unwrap(),
            "9\n"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("calls")).unwrap(),
            "2:music:9:/\n"
        );
    }

    #[tokio::test]
    async fn same_level_still_executes_the_authorized_setter() {
        let _lock = ANDROID_PROVIDER_TEST_LOCK.lock().await;
        let root = tempfile::tempdir().unwrap();
        let script = stateful_script(root.path());
        let (_program_root, _, client) = fixture(&script);
        let result = client
            .prepare_mutation(AndroidVolumeStreamName::Music, 5)
            .await
            .unwrap()
            .execute()
            .await
            .unwrap();
        assert!(!result.changed);
        assert_eq!(
            fs::read_to_string(root.path().join("calls")).unwrap(),
            "2:music:5:/\n"
        );
    }

    #[tokio::test]
    async fn mutation_lane_is_non_queueing() {
        let _lock = ANDROID_PROVIDER_TEST_LOCK.lock().await;
        let root = tempfile::tempdir().unwrap();
        let script = stateful_script(root.path());
        let (_program_root, _, client) = fixture(&script);
        let first = client
            .prepare_mutation(AndroidVolumeStreamName::Music, 6)
            .await
            .unwrap();
        assert_eq!(
            client
                .prepare_mutation(AndroidVolumeStreamName::Ring, 4)
                .await
                .unwrap_err(),
            AndroidVolumeControlError::ConcurrencyLimitExceeded
        );
        drop(first);
        assert!(client
            .prepare_mutation(AndroidVolumeStreamName::Ring, 4)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn verification_failure_restores_and_confirms_the_prior_level() {
        let _lock = ANDROID_PROVIDER_TEST_LOCK.lock().await;
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("level");
        let calls = root.path().join("calls");
        fs::write(&state, "5\n").unwrap();
        let script = format!(
            r#"
state='{state}'
calls='{calls}'
if [ "$#" -eq 0 ]; then
  IFS= read -r music <"$state"
  printf '[{{"stream":"alarm","volume":1,"max_volume":7}},{{"stream":"call","volume":1,"max_volume":5}},{{"stream":"music","volume":%s,"max_volume":15}},{{"stream":"notification","volume":2,"max_volume":7}},{{"stream":"ring","volume":3,"max_volume":7}},{{"stream":"system","volume":2,"max_volume":7}}]' "$music"
  exit 0
fi
printf '%s\n' "$2" >>"$calls"
if [ "$2" = 5 ]; then printf '5\n' >"$state"; fi
"#,
            state = state.display(),
            calls = calls.display(),
        );
        let (_program_root, _, client) = fixture(&script);
        let error = client
            .prepare_mutation(AndroidVolumeStreamName::Music, 9)
            .await
            .unwrap()
            .execute()
            .await
            .unwrap_err();
        assert_eq!(
            error,
            AndroidVolumeControlError::VerificationFailedRollbackConfirmed
        );
        assert_eq!(fs::read_to_string(calls).unwrap(), "9\n5\n");
    }
}
