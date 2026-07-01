//! Shell execution tools via rish (Shizuku) for elevated commands.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::{process::Command, time::timeout};

use crate::error::AppError;

const RISH_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Default)]
pub struct ShellTools;

#[derive(Debug, Serialize, Deserialize)]
pub struct ShellResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl ShellTools {
    pub async fn rish_exec(&self, command: String) -> Result<ShellResult, AppError> {
        let mut process = Command::new("rish");
        process.kill_on_drop(true).arg("-c").arg(&command);
        let output = timeout(RISH_COMMAND_TIMEOUT, process.output())
            .await
            .map_err(|_| AppError::CommandTimeout {
                command: "rish".to_string(),
                timeout_seconds: RISH_COMMAND_TIMEOUT.as_secs(),
            })?
            .map_err(AppError::Io)?;

        if !output.status.success() {
            return Err(AppError::CommandFailed {
                command: "rish".to_string(),
                exit_code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }

        Ok(ShellResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
}
