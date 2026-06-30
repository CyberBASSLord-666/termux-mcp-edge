//! Configuration management with strong validation.

use std::{net::IpAddr, path::PathBuf};

use anyhow::bail;
use serde::Deserialize;

const DEFAULT_FILE_SAFE_ROOT: &str = "/data/data/com.termux/files/home/mcp-files";

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

pub fn validate_runtime_auth_posture(config: &AppConfig) -> anyhow::Result<AuthPosture> {
    if let Some(ref token) = config.auth.static_token {
        if token.trim().is_empty() {
            bail!(
                "MCP__AUTH__STATIC_TOKEN is configured but empty; please provide a non-empty token or use localhost-only unauthenticated mode",
            );
        }

        return Ok(AuthPosture::StaticTokenConfigured);
    }

    if !config.auth.allow_unauthenticated_localhost_only {
        bail!(
            "MCP__AUTH__STATIC_TOKEN is required unless MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true is explicitly set for local-only development",
        );
    }

    if !is_loopback_host(&config.server.host) {
        bail!(
            "Unauthenticated mode is only allowed on localhost; set MCP__AUTH__STATIC_TOKEN or bind MCP__SERVER__HOST to localhost, 127.0.0.1, or ::1",
        );
    }

    Ok(AuthPosture::UnauthenticatedLocalhostOnly)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPosture {
    StaticTokenConfigured,
    UnauthenticatedLocalhostOnly,
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
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

    fn app_config(
        host: &str,
        static_token: Option<&str>,
        allow_localhost_only: bool,
    ) -> AppConfig {
        AppConfig {
            server: ServerConfig {
                host: host.to_owned(),
                port: 8000,
            },
            auth: AuthConfig {
                static_token: static_token.map(str::to_owned),
                allow_unauthenticated_localhost_only: allow_localhost_only,
            },
            file: FileConfig {
                safe_roots: vec![PathBuf::from(DEFAULT_FILE_SAFE_ROOT)],
            },
        }
    }

    #[test]
    fn default_file_safe_root_is_narrow_termux_home_directory() {
        let file = FileConfig {
            safe_roots: vec![PathBuf::from(DEFAULT_FILE_SAFE_ROOT)],
        };
        let broad_storage = PathBuf::from("/storage/emulated/0");
        let sdcard = PathBuf::from("/sdcard");

        validate_file_safe_roots(&file).expect("default safe root should validate");
        assert_eq!(file.safe_roots, vec![PathBuf::from(DEFAULT_FILE_SAFE_ROOT)]);
        assert!(!file.safe_roots.contains(&broad_storage));
        assert!(!file.safe_roots.contains(&sdcard));
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

    #[test]
    fn static_token_auth_posture_is_accepted_for_non_loopback_hosts() {
        let config = app_config("0.0.0.0", Some("configured-token"), false);

        let posture = validate_runtime_auth_posture(&config).expect("token auth should validate");

        assert_eq!(posture, AuthPosture::StaticTokenConfigured);
    }

    #[test]
    fn empty_static_token_is_rejected() {
        let config = app_config("127.0.0.1", Some("   "), true);

        let err = validate_runtime_auth_posture(&config).expect_err("empty token must fail closed");

        assert!(err.to_string().contains("configured but empty"));
    }

    #[test]
    fn missing_token_requires_explicit_localhost_only_opt_in() {
        let config = app_config("127.0.0.1", None, false);

        let err = validate_runtime_auth_posture(&config)
            .expect_err("missing token must fail closed by default");

        assert!(err.to_string().contains("MCP__AUTH__STATIC_TOKEN is required"));
    }

    #[test]
    fn unauthenticated_localhost_only_mode_accepts_loopback_hosts() {
        for host in ["localhost", "127.0.0.1", "::1"] {
            let config = app_config(host, None, true);

            let posture = validate_runtime_auth_posture(&config)
                .expect("loopback development mode should validate");

            assert_eq!(posture, AuthPosture::UnauthenticatedLocalhostOnly);
        }
    }

    #[test]
    fn unauthenticated_localhost_only_mode_rejects_non_loopback_hosts() {
        for host in ["0.0.0.0", "192.168.1.10", "example.com"] {
            let config = app_config(host, None, true);

            let err = validate_runtime_auth_posture(&config)
                .expect_err("non-loopback unauthenticated listener must fail closed");

            assert!(err.to_string().contains("only allowed on localhost"));
        }
    }
}
