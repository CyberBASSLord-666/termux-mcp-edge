//! Configuration management with strong validation.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub file: FileConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    /// Optional static token for simple deployments.
    /// For production, consider integrating with external IdP.
    pub static_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileConfig {
    /// Whitelisted root directories for file operations.
    /// All paths are resolved absolutely and checked against these roots.
    pub safe_roots: Vec<PathBuf>,
}

impl AppConfig {
    pub fn load() -> anyhow::Result<Self> {
        let cfg = config::Config::builder()
            .set_default("server.host", "127.0.0.1")?
            .set_default("server.port", 8000)?
            .set_default("auth.static_token", None::<String>)?
            .set_default(
                "file.safe_roots",
                vec![
                    "/storage/emulated/0".to_string(),
                    "/sdcard".to_string(),
                ],
            )?
            .add_source(config::Environment::with_prefix("MCP").separator("__"))
            .build()?;

        Ok(cfg.try_deserialize()?)
    }
}
