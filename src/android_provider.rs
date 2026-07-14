//! Fixed zero-argument adapter for bounded Termux:API providers.
//!
//! Battery and volume collection share the generic bounded process supervisor,
//! while this adapter preserves their narrower fixed-program, zero-argument,
//! root-working-directory contract and stable provider error taxonomy.

use std::{path::PathBuf, time::Duration};

use crate::bounded_process::{BoundedProcess, BoundedProcessConfigError, BoundedProcessError};

#[cfg(test)]
pub(crate) use crate::bounded_process::active_supervisor_count;
#[cfg(test)]
pub(crate) use crate::bounded_process::BOUNDED_PROCESS_TEST_LOCK as ANDROID_PROVIDER_TEST_LOCK;

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

impl From<BoundedProcessError> for AndroidProviderError {
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

#[derive(Debug, Clone)]
pub(crate) struct BoundedAndroidProvider {
    process: BoundedProcess,
}

impl BoundedAndroidProvider {
    pub(crate) fn new(
        program: PathBuf,
        timeout: Duration,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
    ) -> Result<Self, AndroidProviderConfigError> {
        let process = BoundedProcess::new(
            program,
            Vec::new(),
            PathBuf::from("/"),
            timeout,
            max_stdout_bytes,
            max_stderr_bytes,
        )
        .map_err(|error| match error {
            BoundedProcessConfigError::TimeoutTooShort => {
                AndroidProviderConfigError::TimeoutTooShort
            }
        })?;
        Ok(Self { process })
    }

    pub(crate) async fn collect_stdout(&self) -> Result<Vec<u8>, AndroidProviderError> {
        self.process
            .run()
            .await
            .map(|output| output.stdout)
            .map_err(AndroidProviderError::from)
    }

    #[cfg(test)]
    pub(crate) fn with_forced_cleanup_delay(mut self, delay: Duration) -> Self {
        self.process = self.process.with_forced_cleanup_delay(delay);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_rejects_timeouts_without_a_nonzero_cleanup_reserve() {
        for timeout in [
            Duration::ZERO,
            Duration::from_millis(1),
            Duration::from_millis(2),
            Duration::from_millis(3),
        ] {
            assert_eq!(
                BoundedAndroidProvider::new(PathBuf::from("/provider"), timeout, 1, 1).unwrap_err(),
                AndroidProviderConfigError::TimeoutTooShort,
            );
        }

        assert!(BoundedAndroidProvider::new(
            PathBuf::from("/provider"),
            Duration::from_millis(4),
            1,
            1,
        )
        .is_ok());
    }
}
