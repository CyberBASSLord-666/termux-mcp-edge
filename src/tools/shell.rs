//! Shell execution tools via rish (Shizuku) for elevated commands.

use rmcp::tool;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Clone, Default)]
pub struct ShellTools;

#[derive(Debug, Serialize, Deserialize)]
pub struct ShellResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[tool]
impl ShellTools {
    #[tool(description = "Execute a shell command via rish (requires Shizuku)")]
    pub async fn rish_exec(&self, command: String) -> Result<ShellResult, AppError> {
        let output = std::process::Command::new("rish")
            .arg("-c")
            .arg(&command)
            .output()
            .map_err(AppError::Io)?;

        Ok(ShellResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
}
