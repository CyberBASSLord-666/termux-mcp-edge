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
    /// Static bearer token for simple deployments.
    /// For production, consider integrating with external IdP.
    pub static_token: Option<String>,
    /// Explicit unsafe/local-only opt-in for development without a bearer token.
    /// When true, startup still requires binding to localhost.
    pub allow_unauthenticated_localhost_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileConfig {
    /// Whitelisted root directories for file operations.
    /// All paths are resolved absolutely and checked against these roots.
    pub safe_roots: Vec<PathBuf>,
}

impl AppConfig {
    pub fn load() -> anyhow::Result<Self> {
        let default_safe_roots = vec![String::from("/storage/emulated/0"), String::from("/sdcard")];
        let cfg = config::Config::builder()
            .set_default("server.host", "127.0.0.1")?
            .set_default("server.port", 8000)?
            .set_default("auth.static_token", None::<String>)?
            .set_default("auth.allow_unauthenticated_localhost_only", false)?
            .set_default("file.safe_roots", default_safe_roots)?
            .add_source(config::Environment::with_prefix("MCP").separator("__"))
            .build()?;

        Ok(cfg.try_deserialize()?)
    }
}
