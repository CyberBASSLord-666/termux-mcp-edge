//! Configuration management with strong validation.

use std::{env, net::IpAddr, path::PathBuf};

use anyhow::{anyhow, bail};

const DEFAULT_FILE_SAFE_ROOT: &str = "/data/data/com.termux/files/home/mcp-files";
const EMPTY_STATIC_TOKEN_ERROR: &str =
    "MCP__AUTH__STATIC_TOKEN is configured but empty; please provide a non-empty token or use localhost-only unauthenticated mode";
const MISSING_STATIC_TOKEN_ERROR: &str =
    "MCP__AUTH__STATIC_TOKEN is required unless MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true is explicitly set for local-only development";
const REMOTE_UNAUTHENTICATED_ERROR: &str =
    "Unauthenticated mode is only allowed on localhost; set MCP__AUTH__STATIC_TOKEN or bind MCP__SERVER__HOST to localhost, 127.0.0.1, or ::1";

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub file: FileConfig,
    pub transport: TransportConfig,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Static bearer token for simple deployments.
    /// For production, consider integrating with external IdP.
    pub static_token: Option<String>,
    /// Explicit unsafe/local-only opt-in for development without a bearer token.
    /// When true, startup still requires binding to localhost.
    pub allow_unauthenticated_localhost_only: bool,
}

#[derive(Debug, Clone)]
pub struct FileConfig {
    /// Whitelisted root directories for file operations.
    /// All paths are resolved absolutely and checked against these roots.
    pub safe_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// Allowed HTTP Host header values for future MCP transport routes.
    pub allowed_hosts: Vec<String>,
    /// Allowed browser Origin header values for future MCP transport routes.
    pub allowed_origins: Vec<String>,
    /// Explicit compatibility switch for non-browser clients that omit Origin.
    pub allow_missing_origin: bool,
}

impl AppConfig {
    pub fn load() -> anyhow::Result<Self> {
        let config = Self {
            server: ServerConfig {
                host: env_string("MCP__SERVER__HOST", "127.0.0.1"),
                port: env_u16("MCP__SERVER__PORT", 8000)?,
            },
            auth: AuthConfig {
                static_token: env::var("MCP__AUTH__STATIC_TOKEN").ok(),
                allow_unauthenticated_localhost_only: env_bool(
                    "MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY",
                    false,
                )?,
            },
            file: FileConfig {
                safe_roots: env_path_list("MCP__FILE__SAFE_ROOTS", &[DEFAULT_FILE_SAFE_ROOT]),
            },
            transport: TransportConfig {
                allowed_hosts: env_string_list(
                    "MCP__TRANSPORT__ALLOWED_HOSTS",
                    &["localhost:8000", "127.0.0.1:8000", "[::1]:8000"],
                ),
                allowed_origins: env_string_list(
                    "MCP__TRANSPORT__ALLOWED_ORIGINS",
                    &[
                        "http://localhost:8000",
                        "http://127.0.0.1:8000",
                        "http://[::1]:8000",
                    ],
                ),
                allow_missing_origin: env_bool("MCP__TRANSPORT__ALLOW_MISSING_ORIGIN", false)?,
            },
        };

        validate_file_safe_roots(&config.file)?;
        validate_transport_security(&config.transport)?;
        Ok(config)
    }
}

fn env_string(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn env_u16(name: &str, default: u16) -> anyhow::Result<u16> {
    match env::var(name) {
        Ok(value) => value
            .parse::<u16>()
            .map_err(|source| anyhow!("{name} must be an integer between 0 and 65535: {source}")),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(source) => Err(anyhow!("{name} could not be read: {source}")),
    }
}

fn env_bool(name: &str, default: bool) -> anyhow::Result<bool> {
    match env::var(name) {
        Ok(value) => parse_bool(name, &value),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(source) => Err(anyhow!("{name} could not be read: {source}")),
    }
}

fn parse_bool(name: &str, value: &str) -> anyhow::Result<bool> {
    let value = value.trim();

    if ["true", "1", "yes", "on"]
        .iter()
        .any(|accepted| value.eq_ignore_ascii_case(accepted))
    {
        return Ok(true);
    }

    if ["false", "0", "no", "off"]
        .iter()
        .any(|accepted| value.eq_ignore_ascii_case(accepted))
    {
        return Ok(false);
    }

    bail!("{name} must be a boolean value: true/false, 1/0, yes/no, or on/off")
}

fn env_string_list(name: &str, defaults: &[&str]) -> Vec<String> {
    match env::var(name) {
        Ok(value) => split_env_list(&value),
        Err(_) => defaults.iter().copied().map(str::to_owned).collect(),
    }
}

fn env_path_list(name: &str, defaults: &[&str]) -> Vec<PathBuf> {
    env_string_list(name, defaults)
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

fn split_env_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn validate_runtime_auth_posture(config: &AppConfig) -> anyhow::Result<AuthPosture> {
    if let Some(ref token) = config.auth.static_token {
        if token.trim().is_empty() {
            bail!(EMPTY_STATIC_TOKEN_ERROR);
        }

        return Ok(AuthPosture::StaticTokenConfigured);
    }

    if !config.auth.allow_unauthenticated_localhost_only {
        bail!(MISSING_STATIC_TOKEN_ERROR);
    }

    if !is_loopback_host(&config.server.host) {
        bail!(REMOTE_UNAUTHENTICATED_ERROR);
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

fn validate_transport_security(transport: &TransportConfig) -> anyhow::Result<()> {
    if transport.allowed_hosts.is_empty() {
        bail!("MCP__TRANSPORT__ALLOWED_HOSTS must contain at least one exact host");
    }

    if transport.allowed_origins.is_empty() {
        bail!("MCP__TRANSPORT__ALLOWED_ORIGINS must contain at least one exact origin");
    }

    for host in &transport.allowed_hosts {
        let host = host.trim();
        if host.is_empty() || host == "*" || host.contains('/') || host.contains(' ') {
            bail!("MCP__TRANSPORT__ALLOWED_HOSTS contains an invalid host: {host}");
        }
    }

    for origin in &transport.allowed_origins {
        let origin = origin.trim();
        if !is_exact_http_origin(origin) {
            bail!("MCP__TRANSPORT__ALLOWED_ORIGINS contains an invalid origin: {origin}");
        }
    }

    Ok(())
}

fn is_exact_http_origin(origin: &str) -> bool {
    let authority = if let Some(authority) = origin.strip_prefix("http://") {
        authority
    } else if let Some(authority) = origin.strip_prefix("https://") {
        authority
    } else {
        return false;
    };

    !authority.is_empty()
        && !authority.contains('*')
        && !authority.contains(' ')
        && !authority.contains('/')
        && !authority.contains('?')
        && !authority.contains('#')
        && !authority.contains('@')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_config(host: &str, static_token: Option<&str>, allow_localhost_only: bool) -> AppConfig {
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
            transport: transport_config(),
        }
    }

    fn transport_config() -> TransportConfig {
        TransportConfig {
            allowed_hosts: vec!["localhost:8000".to_owned()],
            allowed_origins: vec!["http://localhost:8000".to_owned()],
            allow_missing_origin: false,
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

        assert!(err
            .to_string()
            .contains("MCP__AUTH__STATIC_TOKEN is required"));
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

    #[test]
    fn transport_security_config_accepts_exact_hosts_and_origins() {
        validate_transport_security(&transport_config())
            .expect("exact transport security allowlists should validate");
    }

    #[test]
    fn transport_security_config_rejects_empty_allowed_hosts() {
        let transport = TransportConfig {
            allowed_hosts: vec![],
            ..transport_config()
        };

        let err = validate_transport_security(&transport)
            .expect_err("empty transport host allowlist must fail closed");

        assert!(err.to_string().contains("ALLOWED_HOSTS"));
    }

    #[test]
    fn transport_security_config_rejects_wildcard_hosts() {
        let transport = TransportConfig {
            allowed_hosts: vec!["*".to_owned()],
            ..transport_config()
        };

        let err = validate_transport_security(&transport)
            .expect_err("wildcard transport host allowlist must fail closed");

        assert!(err.to_string().contains("invalid host"));
    }

    #[test]
    fn transport_security_config_rejects_empty_allowed_origins() {
        let transport = TransportConfig {
            allowed_origins: vec![],
            ..transport_config()
        };

        let err = validate_transport_security(&transport)
            .expect_err("empty transport origin allowlist must fail closed");

        assert!(err.to_string().contains("ALLOWED_ORIGINS"));
    }

    #[test]
    fn transport_security_config_rejects_non_http_origins() {
        let transport = TransportConfig {
            allowed_origins: vec!["chrome-extension://example".to_owned()],
            ..transport_config()
        };

        let err = validate_transport_security(&transport)
            .expect_err("non-http transport origins must fail closed");

        assert!(err.to_string().contains("invalid origin"));
    }

    #[test]
    fn transport_security_config_rejects_origin_paths_queries_fragments_and_userinfo() {
        for origin in [
            "http://localhost:8000/",
            "http://localhost:8000/path",
            "http://localhost:8000?debug=true",
            "http://localhost:8000#fragment",
            "https://user@example.com",
        ] {
            let transport = TransportConfig {
                allowed_origins: vec![origin.to_owned()],
                ..transport_config()
            };

            let err = validate_transport_security(&transport)
                .expect_err("transport origin allowlist values must be exact origins only");

            assert!(err.to_string().contains("invalid origin"));
        }
    }
}
