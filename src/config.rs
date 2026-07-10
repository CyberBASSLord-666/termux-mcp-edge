//! Configuration management with strong validation.

use std::{env, fmt, net::IpAddr, path::PathBuf};

use anyhow::{anyhow, bail};

use crate::request_limits::{
    DEFAULT_MAX_BODY_BYTES, DEFAULT_MAX_CONCURRENT_REQUESTS, DEFAULT_REQUEST_TIMEOUT_SECONDS,
    MAX_CONFIGURED_BODY_BYTES, MAX_CONFIGURED_CONCURRENT_REQUESTS,
    MAX_CONFIGURED_REQUEST_TIMEOUT_SECONDS, MIN_CONFIGURED_BODY_BYTES,
};
use crate::transport_security::{normalize_host, normalize_origin};

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

#[derive(Clone)]
pub struct AuthConfig {
    /// Static bearer token for simple deployments.
    /// For production, consider integrating with external IdP.
    pub static_token: Option<String>,
    /// Explicit unsafe/local-only opt-in for development without a bearer token.
    /// When true, startup still requires binding to localhost.
    pub allow_unauthenticated_localhost_only: bool,
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthConfig")
            .field(
                "static_token",
                &self.static_token.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "allow_unauthenticated_localhost_only",
                &self.allow_unauthenticated_localhost_only,
            )
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct FileConfig {
    /// Whitelisted root directories for file operations.
    /// All paths are resolved absolutely and checked against these roots.
    pub safe_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// Allowed HTTP Host header values for staged MCP transport routes.
    pub allowed_hosts: Vec<String>,
    /// Allowed browser Origin header values for staged MCP transport routes.
    pub allowed_origins: Vec<String>,
    /// Explicit compatibility switch for non-browser clients that omit Origin.
    pub allow_missing_origin: bool,
    /// Maximum number of authenticated MCP requests executing concurrently.
    pub max_concurrent_requests: usize,
    /// Maximum total duration for one authenticated MCP request.
    pub request_timeout_seconds: u64,
    /// Maximum buffered JSON-RPC request-body size.
    pub max_body_bytes: usize,
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
                allowed_hosts: env_exact_string_list(
                    "MCP__TRANSPORT__ALLOWED_HOSTS",
                    &["localhost:8000", "127.0.0.1:8000", "[::1]:8000"],
                )?,
                allowed_origins: env_exact_string_list(
                    "MCP__TRANSPORT__ALLOWED_ORIGINS",
                    &[
                        "http://localhost:8000",
                        "http://127.0.0.1:8000",
                        "http://[::1]:8000",
                    ],
                )?,
                allow_missing_origin: env_bool("MCP__TRANSPORT__ALLOW_MISSING_ORIGIN", false)?,
                max_concurrent_requests: env_usize(
                    "MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS",
                    DEFAULT_MAX_CONCURRENT_REQUESTS,
                )?,
                request_timeout_seconds: env_u64(
                    "MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS",
                    DEFAULT_REQUEST_TIMEOUT_SECONDS,
                )?,
                max_body_bytes: env_usize(
                    "MCP__TRANSPORT__MAX_BODY_BYTES",
                    DEFAULT_MAX_BODY_BYTES,
                )?,
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
            .trim()
            .parse::<u16>()
            .map_err(|source| anyhow!("{name} must be an integer between 0 and 65535: {source}")),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(source) => Err(anyhow!("{name} could not be read: {source}")),
    }
}

fn env_usize(name: &str, default: usize) -> anyhow::Result<usize> {
    match env::var(name) {
        Ok(value) => parse_usize(name, &value),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(source) => Err(anyhow!("{name} could not be read: {source}")),
    }
}

fn env_u64(name: &str, default: u64) -> anyhow::Result<u64> {
    match env::var(name) {
        Ok(value) => parse_u64(name, &value),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(source) => Err(anyhow!("{name} could not be read: {source}")),
    }
}

fn parse_usize(name: &str, value: &str) -> anyhow::Result<usize> {
    value
        .trim()
        .parse::<usize>()
        .map_err(|source| anyhow!("{name} must be a non-negative integer: {source}"))
}

fn parse_u64(name: &str, value: &str) -> anyhow::Result<u64> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|source| anyhow!("{name} must be a non-negative integer: {source}"))
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

fn env_exact_string_list(name: &str, defaults: &[&str]) -> anyhow::Result<Vec<String>> {
    match env::var(name) {
        Ok(value) => split_exact_env_list(name, &value),
        Err(env::VarError::NotPresent) => Ok(defaults.iter().copied().map(str::to_owned).collect()),
        Err(env::VarError::NotUnicode(_)) => {
            bail!("{name} must contain valid Unicode text")
        }
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

fn split_exact_env_list(name: &str, value: &str) -> anyhow::Result<Vec<String>> {
    let items = value.split(',').map(str::to_owned).collect::<Vec<_>>();
    if items.iter().any(String::is_empty) {
        bail!("{name} must not contain empty list entries");
    }
    Ok(items)
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
        if normalize_host(host).is_none() {
            bail!("MCP__TRANSPORT__ALLOWED_HOSTS contains an invalid host: {host}");
        }
    }

    for origin in &transport.allowed_origins {
        if normalize_origin(origin).is_none() {
            bail!("MCP__TRANSPORT__ALLOWED_ORIGINS contains an invalid origin: {origin}");
        }
    }

    if !(1..=MAX_CONFIGURED_CONCURRENT_REQUESTS).contains(&transport.max_concurrent_requests) {
        bail!(
            "MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS must be between 1 and {MAX_CONFIGURED_CONCURRENT_REQUESTS}"
        );
    }

    if !(1..=MAX_CONFIGURED_REQUEST_TIMEOUT_SECONDS).contains(&transport.request_timeout_seconds) {
        bail!(
            "MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS must be between 1 and {MAX_CONFIGURED_REQUEST_TIMEOUT_SECONDS}"
        );
    }

    if !(MIN_CONFIGURED_BODY_BYTES..=MAX_CONFIGURED_BODY_BYTES).contains(&transport.max_body_bytes)
    {
        bail!(
            "MCP__TRANSPORT__MAX_BODY_BYTES must be between {MIN_CONFIGURED_BODY_BYTES} and {MAX_CONFIGURED_BODY_BYTES}"
        );
    }

    Ok(())
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
            max_concurrent_requests: DEFAULT_MAX_CONCURRENT_REQUESTS,
            request_timeout_seconds: DEFAULT_REQUEST_TIMEOUT_SECONDS,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }

    #[test]
    fn auth_config_debug_output_redacts_static_token() {
        let auth = AuthConfig {
            static_token: Some("secret-value".to_owned()),
            allow_unauthenticated_localhost_only: false,
        };
        let debug = format!("{auth:?}");

        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret-value"));
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
    fn transport_security_config_accepts_exact_hosts_origins_and_safe_limits() {
        validate_transport_security(&transport_config())
            .expect("exact transport security allowlists and safe limits should validate");
    }

    #[test]
    fn transport_security_config_uses_request_authority_contract() {
        let accepted = [
            ("LOCALHOST:8000", "HTTP://LOCALHOST:8000"),
            ("127.0.0.1:8000", "http://127.0.0.1:8000"),
            ("[0:0:0:0:0:0:0:1]:8000", "http://[0:0:0:0:0:0:0:1]:8000"),
        ];
        for (host, origin) in accepted {
            let transport = TransportConfig {
                allowed_hosts: vec![host.to_owned()],
                allowed_origins: vec![origin.to_owned()],
                ..transport_config()
            };
            validate_transport_security(&transport)
                .unwrap_or_else(|error| panic!("accepted authority rejected: {error}"));
        }

        let rejected = [
            "localhost\t:8000",
            "localhost\n:8000",
            "user@localhost:8000",
            "localhost:0",
            "localhost:65536",
            "localhost:",
            "::1",
            "[::1",
            "[::1]junk",
        ];
        for authority in rejected {
            let transport = TransportConfig {
                allowed_hosts: vec![authority.to_owned()],
                ..transport_config()
            };
            validate_transport_security(&transport)
                .expect_err("malformed configured host must fail startup");

            let transport = TransportConfig {
                allowed_origins: vec![format!("http://{authority}")],
                ..transport_config()
            };
            validate_transport_security(&transport)
                .expect_err("malformed configured origin must fail startup");
        }
    }

    #[test]
    fn exact_transport_list_parser_preserves_invalid_whitespace_for_validation() {
        for value in [
            " localhost:8000",
            "localhost:8000 ",
            "localhost:8000,\t127.0.0.1:8000",
        ] {
            let transport = TransportConfig {
                allowed_hosts: split_exact_env_list("ALLOWED_HOSTS", value).unwrap(),
                ..transport_config()
            };

            validate_transport_security(&transport)
                .expect_err("configured authority whitespace must fail closed");
        }
    }

    #[test]
    fn exact_transport_list_parser_rejects_empty_entries_instead_of_dropping_them() {
        for value in ["", ",", "localhost:8000,", ",localhost:8000", "a,,b"] {
            let error = split_exact_env_list("ALLOWED_HOSTS", value)
                .expect_err("empty configured authority entries must fail closed");

            assert!(error.to_string().contains("empty list entries"));
        }
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

    #[test]
    fn transport_request_limits_reject_zero_and_excessive_values() {
        let cases = [
            TransportConfig {
                max_concurrent_requests: 0,
                ..transport_config()
            },
            TransportConfig {
                max_concurrent_requests: MAX_CONFIGURED_CONCURRENT_REQUESTS + 1,
                ..transport_config()
            },
            TransportConfig {
                request_timeout_seconds: 0,
                ..transport_config()
            },
            TransportConfig {
                request_timeout_seconds: MAX_CONFIGURED_REQUEST_TIMEOUT_SECONDS + 1,
                ..transport_config()
            },
            TransportConfig {
                max_body_bytes: MIN_CONFIGURED_BODY_BYTES - 1,
                ..transport_config()
            },
            TransportConfig {
                max_body_bytes: MAX_CONFIGURED_BODY_BYTES + 1,
                ..transport_config()
            },
        ];

        for transport in cases {
            validate_transport_security(&transport)
                .expect_err("unsafe MCP request limits must fail closed");
        }
    }

    #[test]
    fn unsigned_limit_parsers_trim_and_reject_invalid_values() {
        assert_eq!(parse_usize("LIMIT", " 8 ").unwrap(), 8);
        assert_eq!(parse_u64("TIMEOUT", " 30 ").unwrap(), 30);
        assert!(parse_usize("LIMIT", "-1").is_err());
        assert!(parse_u64("TIMEOUT", "not-a-number").is_err());
    }

    #[test]
    fn malformed_boolean_is_rejected() {
        let err =
            parse_bool("TEST_BOOL", "sometimes").expect_err("malformed boolean must fail closed");
        assert!(err.to_string().contains("must be a boolean value"));
    }
}
