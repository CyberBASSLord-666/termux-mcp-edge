//! Configuration management with strong validation.

use std::path::PathBuf;

use anyhow::bail;
use serde::Deserialize;

const DEFAULT_FILE_SAFE_ROOT: &str = "/data/data/com.termux/files/home/termux-mcp-edge-files";

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
        let default_safe_roots = vec![String::from(DEFAULT_FILE_SAFE_ROOT)];
        let cfg = config::Config::builder()
            .set_default("server.host", "127.0.0.1")?
            .set_default("server.port", 8000)?
            .set_default("auth.static_token", None::<String>)?
            .set_default("auth.allow_unauthenticated_localhost_only", false)?
            .set_default("file.safe_roots", default_safe_roots)?
            .add_source(config::Environment::with_prefix("MCP").separator("__"))
            .build()?;

        let config: Self = cfg.try_deserialize()?;
        validate_file_safe_roots(&config.file)?;
        Ok(config)
    }
}

fn validate_file_safe_roots(file: &FileConfig) -> anyhow::Result<()> {
    if file.safe_roots.is_empty() {
        bail!("MCP__FILE__SAFE_ROOTS must contain at least one absolute safe root");
    }

    for root in &file.safe_roots {
        if root.as_os_str().is_empty() {
            bail!("MCP__FILE__SAFE_ROOTS contains an empty safe root");
        }

        if !root.is_absolute() {
            bail!(
                "MCP__FILE__SAFE_ROOTS contains a non-absolute safe root: {}",
                root.display()
            );
        }

        if root == std::path::Path::new("/") {
            bail!("MCP__FILE__SAFE_ROOTS must not include filesystem root /");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_safe_root_is_narrow_termux_home_directory() {
        let file = FileConfig {
            safe_roots: vec![PathBuf::from(DEFAULT_FILE_SAFE_ROOT)],
        };

        validate_file_safe_roots(&file).expect("default safe root should validate");
        assert_eq!(file.safe_roots, vec![PathBuf::from(DEFAULT_FILE_SAFE_ROOT)]);
        assert!(!file.safe_roots.contains(&PathBuf::from("/storage/emulated/0")));
        assert!(!file.safe_roots.contains(&PathBuf::from("/sdcard")));
    }

    #[test]
    fn empty_safe_roots_are_rejected() {
        let file = FileConfig { safe_roots: vec![] };

        let err = validate_file_safe_roots(&file).expect_err("empty safe roots must fail closed");
        assert!(err.to_string().contains("at least one absolute safe root"));
    }

    #[test]
    fn relative_safe_roots_are_rejected() {
        let file = FileConfig {
            safe_roots: vec![PathBuf::from("relative/path")],
        };

        let err = validate_file_safe_roots(&file).expect_err("relative safe roots must fail");
        assert!(err.to_string().contains("non-absolute safe root"));
    }

    #[test]
    fn filesystem_root_is_rejected() {
        let file = FileConfig {
            safe_roots: vec![PathBuf::from("/")],
        };

        let err = validate_file_safe_roots(&file).expect_err("filesystem root must fail");
        assert!(err.to_string().contains("must not include filesystem root"));
    }
}
