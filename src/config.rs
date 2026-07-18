//! Configuration management with strong validation.

use std::{env, fmt, net::IpAddr, path::PathBuf};

#[cfg(feature = "mcp-runtime")]
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

use anyhow::{anyhow, bail};

#[cfg(feature = "mcp-runtime")]
use rustix::fs::{fstat, open, FileType, Mode, OFlags};

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
    pub capability: CapabilityConfig,
    pub android: AndroidConfig,
    pub command: CommandConfig,
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
    /// Dedicated default-disabled runtime gate for `create_directory` mutation.
    pub create_directory_mutation_enabled: bool,
}

#[derive(Clone)]
pub struct CapabilityConfig {
    /// Identifier for the one active request-capability signing key.
    pub key_id: Option<String>,
    /// Exact 32-byte HMAC key encoded as 64 lowercase hexadecimal characters.
    hmac_key_hex: Option<String>,
}

impl CapabilityConfig {
    pub fn hmac_key_hex(&self) -> Option<&str> {
        self.hmac_key_hex.as_deref()
    }
}

impl fmt::Debug for CapabilityConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapabilityConfig")
            .field("key_id", &self.key_id)
            .field(
                "hmac_key_hex",
                &self.hmac_key_hex.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AndroidConfig {
    /// Explicit runtime opt-in for the separately feature-gated battery tool.
    pub battery_status_enabled: bool,
    /// Explicit runtime opt-in for the separately feature-gated volume tool.
    pub volume_status_enabled: bool,
    /// Explicit runtime opt-in for request-authorized volume mutation.
    pub volume_control_enabled: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CommandConfig {
    /// Explicit runtime opt-in for fixed-profile command diagnostics.
    pub enabled: bool,
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
        Self::load_with(|name| env::var(name))
    }

    /// Load the deployed literal `NAME=value` runtime file for an offline
    /// capability-issuance process. The file is opened without following its
    /// final component, bounded before parsing, and never evaluated as shell.
    #[cfg(feature = "mcp-runtime")]
    pub fn load_from_literal_file(path: &Path) -> anyhow::Result<Self> {
        const MAX_RUNTIME_CONFIG_BYTES: usize = 65_536;

        if !path.is_absolute() {
            bail!("offline issuer runtime configuration path must be absolute");
        }
        let descriptor = open(
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| anyhow!("offline issuer runtime configuration could not be opened"))?;
        let metadata = fstat(&descriptor)
            .map_err(|_| anyhow!("offline issuer runtime configuration could not be inspected"))?;
        if !FileType::from_raw_mode(metadata.st_mode).is_file() {
            bail!("offline issuer runtime configuration must be a regular non-symlink file");
        }
        if metadata.st_mode & 0o077 != 0 || metadata.st_mode & 0o400 == 0 {
            bail!(
                "offline issuer runtime configuration must be owner-readable and inaccessible to group/other"
            );
        }
        let configured_bytes = usize::try_from(metadata.st_size)
            .map_err(|_| anyhow!("offline issuer runtime configuration size is invalid"))?;
        if configured_bytes > MAX_RUNTIME_CONFIG_BYTES {
            bail!("offline issuer runtime configuration exceeds its byte limit");
        }

        let mut contents = String::new();
        File::from(descriptor)
            .take((MAX_RUNTIME_CONFIG_BYTES + 1) as u64)
            .read_to_string(&mut contents)
            .map_err(|_| anyhow!("offline issuer runtime configuration is not valid UTF-8"))?;
        if contents.len() > MAX_RUNTIME_CONFIG_BYTES {
            bail!("offline issuer runtime configuration exceeds its byte limit");
        }
        if contents.contains('\r') || contents.contains('\0') {
            bail!("offline issuer runtime configuration contains an invalid byte");
        }

        let mut values = BTreeMap::new();
        for line in contents.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (name, value) = line.split_once('=').ok_or_else(|| {
                anyhow!("offline issuer runtime configuration must use NAME=value records")
            })?;
            if name.is_empty()
                || !name.bytes().enumerate().all(|(index, byte)| match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'_' => true,
                    b'0'..=b'9' => index > 0,
                    _ => false,
                })
            {
                bail!("offline issuer runtime configuration contains an invalid variable name");
            }
            if !(name.starts_with("MCP__") || matches!(name, "RUST_LOG" | "RUST_BACKTRACE")) {
                bail!("offline issuer runtime configuration variable is not allowlisted");
            }
            if values.insert(name.to_owned(), value.to_owned()).is_some() {
                bail!("offline issuer runtime configuration contains a duplicate variable");
            }
        }

        Self::load_with(|name| values.get(name).cloned().ok_or(env::VarError::NotPresent))
    }

    fn load_with(
        read_variable: impl Fn(&str) -> Result<String, env::VarError>,
    ) -> anyhow::Result<Self> {
        let config = Self {
            server: ServerConfig {
                host: env_string(&read_variable, "MCP__SERVER__HOST", "127.0.0.1")?,
                port: env_port(&read_variable, "MCP__SERVER__PORT", 8000)?,
            },
            auth: AuthConfig {
                static_token: optional_env_string(&read_variable, "MCP__AUTH__STATIC_TOKEN")?,
                allow_unauthenticated_localhost_only: env_bool(
                    &read_variable,
                    "MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY",
                    false,
                )?,
            },
            capability: CapabilityConfig {
                key_id: optional_env_string(&read_variable, "MCP__CAPABILITY__KEY_ID")?,
                hmac_key_hex: optional_env_string(&read_variable, "MCP__CAPABILITY__HMAC_KEY_HEX")?,
            },
            android: AndroidConfig {
                battery_status_enabled: env_bool(
                    &read_variable,
                    "MCP__ANDROID__BATTERY_STATUS_ENABLED",
                    false,
                )?,
                volume_status_enabled: env_bool(
                    &read_variable,
                    "MCP__ANDROID__VOLUME_STATUS_ENABLED",
                    false,
                )?,
                volume_control_enabled: env_bool(
                    &read_variable,
                    "MCP__ANDROID__VOLUME_CONTROL_ENABLED",
                    false,
                )?,
            },
            command: CommandConfig {
                enabled: env_bool(&read_variable, "MCP__COMMAND__ENABLED", false)?,
            },
            file: FileConfig {
                safe_roots: env_path_list(
                    &read_variable,
                    "MCP__FILE__SAFE_ROOTS",
                    &[DEFAULT_FILE_SAFE_ROOT],
                )?,
                create_directory_mutation_enabled: env_bool(
                    &read_variable,
                    "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED",
                    false,
                )?,
            },
            transport: TransportConfig {
                allowed_hosts: env_exact_string_list(
                    &read_variable,
                    "MCP__TRANSPORT__ALLOWED_HOSTS",
                    &["localhost:8000", "127.0.0.1:8000", "[::1]:8000"],
                )?,
                allowed_origins: env_exact_string_list(
                    &read_variable,
                    "MCP__TRANSPORT__ALLOWED_ORIGINS",
                    &[
                        "http://localhost:8000",
                        "http://127.0.0.1:8000",
                        "http://[::1]:8000",
                    ],
                )?,
                allow_missing_origin: env_bool(
                    &read_variable,
                    "MCP__TRANSPORT__ALLOW_MISSING_ORIGIN",
                    false,
                )?,
                max_concurrent_requests: env_usize(
                    &read_variable,
                    "MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS",
                    DEFAULT_MAX_CONCURRENT_REQUESTS,
                )?,
                request_timeout_seconds: env_u64(
                    &read_variable,
                    "MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS",
                    DEFAULT_REQUEST_TIMEOUT_SECONDS,
                )?,
                max_body_bytes: env_usize(
                    &read_variable,
                    "MCP__TRANSPORT__MAX_BODY_BYTES",
                    DEFAULT_MAX_BODY_BYTES,
                )?,
            },
        };

        validate_file_safe_roots(&config.file)?;
        validate_create_directory_mutation_capability(&config)?;
        validate_android_capabilities(&config.android)?;
        validate_android_volume_control_capability(&config)?;
        validate_command_capability(&config.command)?;
        validate_transport_security(&config.transport)?;
        Ok(config)
    }
}

fn read_env(
    read_variable: &impl Fn(&str) -> Result<String, env::VarError>,
    name: &str,
) -> anyhow::Result<Option<String>> {
    match read_variable(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => bail!("{name} must contain valid Unicode text"),
    }
}

fn optional_env_string(
    read_variable: &impl Fn(&str) -> Result<String, env::VarError>,
    name: &str,
) -> anyhow::Result<Option<String>> {
    read_env(read_variable, name)
}

fn env_string(
    read_variable: &impl Fn(&str) -> Result<String, env::VarError>,
    name: &str,
    default: &str,
) -> anyhow::Result<String> {
    Ok(read_env(read_variable, name)?.unwrap_or_else(|| default.to_owned()))
}

fn env_port(
    read_variable: &impl Fn(&str) -> Result<String, env::VarError>,
    name: &str,
    default: u16,
) -> anyhow::Result<u16> {
    match read_env(read_variable, name)? {
        Some(value) => value
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| anyhow!("{name} must be an integer between 1 and 65535")),
        None => Ok(default),
    }
}

fn env_usize(
    read_variable: &impl Fn(&str) -> Result<String, env::VarError>,
    name: &str,
    default: usize,
) -> anyhow::Result<usize> {
    match read_env(read_variable, name)? {
        Some(value) => parse_usize(name, &value),
        None => Ok(default),
    }
}

fn env_u64(
    read_variable: &impl Fn(&str) -> Result<String, env::VarError>,
    name: &str,
    default: u64,
) -> anyhow::Result<u64> {
    match read_env(read_variable, name)? {
        Some(value) => parse_u64(name, &value),
        None => Ok(default),
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

fn env_bool(
    read_variable: &impl Fn(&str) -> Result<String, env::VarError>,
    name: &str,
    default: bool,
) -> anyhow::Result<bool> {
    match read_env(read_variable, name)? {
        Some(value) => parse_bool(name, &value),
        None => Ok(default),
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

fn env_exact_string_list(
    read_variable: &impl Fn(&str) -> Result<String, env::VarError>,
    name: &str,
    defaults: &[&str],
) -> anyhow::Result<Vec<String>> {
    match read_env(read_variable, name)? {
        Some(value) => split_exact_env_list(name, &value),
        None => Ok(defaults.iter().copied().map(str::to_owned).collect()),
    }
}

fn env_path_list(
    read_variable: &impl Fn(&str) -> Result<String, env::VarError>,
    name: &str,
    defaults: &[&str],
) -> anyhow::Result<Vec<PathBuf>> {
    let paths = env_exact_string_list(read_variable, name, defaults)?;
    Ok(paths.into_iter().map(PathBuf::from).collect())
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
            bail!("MCP__FILE__SAFE_ROOTS contains a non-absolute safe root");
        }

        if root == std::path::Path::new("/") {
            bail!("MCP__FILE__SAFE_ROOTS must not include filesystem root /");
        }
    }

    Ok(())
}

fn validate_create_directory_mutation_capability(config: &AppConfig) -> anyhow::Result<()> {
    const MAX_KEY_ID_BYTES: usize = 32;
    const KEY_HEX_BYTES: usize = 64;

    if let Some(key_id) = config.capability.key_id.as_deref() {
        if key_id.is_empty()
            || key_id.len() > MAX_KEY_ID_BYTES
            || !key_id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
        {
            bail!(
                "MCP__CAPABILITY__KEY_ID must contain 1 to {MAX_KEY_ID_BYTES} lowercase ASCII letters, digits, hyphens, or underscores"
            );
        }
    }

    if let Some(key) = config.capability.hmac_key_hex() {
        if key.len() != KEY_HEX_BYTES
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            bail!(
                "MCP__CAPABILITY__HMAC_KEY_HEX must contain exactly {KEY_HEX_BYTES} lowercase hexadecimal characters"
            );
        }
    }

    if config.capability.key_id.is_some() != config.capability.hmac_key_hex().is_some() {
        bail!(
            "MCP__CAPABILITY__KEY_ID and MCP__CAPABILITY__HMAC_KEY_HEX must be configured together"
        );
    }

    if !config.file.create_directory_mutation_enabled {
        return Ok(());
    }
    if !cfg!(feature = "mcp-runtime") {
        bail!(
            "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED requires a binary built with the mcp-runtime feature"
        );
    }
    if config
        .auth
        .static_token
        .as_deref()
        .is_none_or(|token| token.trim().is_empty())
    {
        bail!("MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED requires MCP__AUTH__STATIC_TOKEN");
    }
    if config.capability.key_id.is_none() {
        bail!(
            "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED requires MCP__CAPABILITY__KEY_ID and MCP__CAPABILITY__HMAC_KEY_HEX"
        );
    }
    Ok(())
}

fn validate_android_capabilities(android: &AndroidConfig) -> anyhow::Result<()> {
    if android.battery_status_enabled && !cfg!(feature = "android-battery-status") {
        bail!(
            "MCP__ANDROID__BATTERY_STATUS_ENABLED requires a binary built with the android-battery-status feature"
        );
    }
    if android.volume_status_enabled && !cfg!(feature = "android-volume-status") {
        bail!(
            "MCP__ANDROID__VOLUME_STATUS_ENABLED requires a binary built with the android-volume-status feature"
        );
    }
    if android.volume_control_enabled && !cfg!(feature = "android-volume-control") {
        bail!(
            "MCP__ANDROID__VOLUME_CONTROL_ENABLED requires a binary built with the android-volume-control feature"
        );
    }
    Ok(())
}

fn validate_android_volume_control_capability(config: &AppConfig) -> anyhow::Result<()> {
    if !config.android.volume_control_enabled {
        return Ok(());
    }
    if config
        .auth
        .static_token
        .as_deref()
        .is_none_or(|token| token.trim().is_empty())
    {
        bail!("MCP__ANDROID__VOLUME_CONTROL_ENABLED requires MCP__AUTH__STATIC_TOKEN");
    }
    if config.capability.key_id.is_none() {
        bail!(
            "MCP__ANDROID__VOLUME_CONTROL_ENABLED requires MCP__CAPABILITY__KEY_ID and MCP__CAPABILITY__HMAC_KEY_HEX"
        );
    }
    Ok(())
}

fn validate_command_capability(command: &CommandConfig) -> anyhow::Result<()> {
    if command.enabled && !cfg!(feature = "command-execution") {
        bail!("MCP__COMMAND__ENABLED requires a binary built with the command-execution feature");
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
            bail!("MCP__TRANSPORT__ALLOWED_HOSTS contains an invalid host");
        }
    }

    for origin in &transport.allowed_origins {
        if normalize_origin(origin).is_none() {
            bail!("MCP__TRANSPORT__ALLOWED_ORIGINS contains an invalid origin");
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

    use std::{collections::BTreeMap, ffi::OsString};

    fn load_from_os_values(
        entries: impl IntoIterator<Item = (&'static str, OsString)>,
    ) -> anyhow::Result<AppConfig> {
        let values = entries
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect::<BTreeMap<_, _>>();

        AppConfig::load_with(|name| match values.get(name).cloned() {
            Some(value) => value.into_string().map_err(env::VarError::NotUnicode),
            None => Err(env::VarError::NotPresent),
        })
    }

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
            capability: CapabilityConfig {
                key_id: None,
                hmac_key_hex: None,
            },
            android: AndroidConfig {
                battery_status_enabled: false,
                volume_status_enabled: false,
                volume_control_enabled: false,
            },
            command: CommandConfig { enabled: false },
            file: FileConfig {
                safe_roots: vec![PathBuf::from(DEFAULT_FILE_SAFE_ROOT)],
                create_directory_mutation_enabled: false,
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
    fn capability_config_debug_output_redacts_hmac_key() {
        let capability = CapabilityConfig {
            key_id: Some("primary-1".to_owned()),
            hmac_key_hex: Some("a".repeat(64)),
        };
        let debug = format!("{capability:?}");

        assert!(debug.contains("primary-1"));
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(&"a".repeat(64)));
    }

    #[cfg(all(feature = "mcp-runtime", unix))]
    #[test]
    fn offline_issuer_literal_config_is_private_bounded_and_never_evaluated() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = tempfile::tempdir().unwrap();
        let safe_root = root.path().join("safe-root");
        std::fs::create_dir(&safe_root).unwrap();
        let config_path = root.path().join("runtime.env");
        let valid = format!(
            "MCP__AUTH__STATIC_TOKEN=private-issuer-principal\n\
             MCP__FILE__SAFE_ROOTS={}\n\
             MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true\n\
             MCP__CAPABILITY__KEY_ID=offline-1\n\
             MCP__CAPABILITY__HMAC_KEY_HEX={}\n\
             RUST_LOG=termux_mcp_server=info\n",
            safe_root.display(),
            "a".repeat(64),
        );
        std::fs::write(&config_path, &valid).unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let config = AppConfig::load_from_literal_file(&config_path).unwrap();
        assert_eq!(
            config.auth.static_token.as_deref(),
            Some("private-issuer-principal")
        );
        assert_eq!(config.capability.key_id.as_deref(), Some("offline-1"));
        assert!(config.file.create_directory_mutation_enabled);
        assert_eq!(config.file.safe_roots, vec![safe_root]);

        std::fs::write(
            &config_path,
            format!("{valid}MCP__CAPABILITY__KEY_ID=duplicate\n"),
        )
        .unwrap();
        assert!(AppConfig::load_from_literal_file(&config_path)
            .unwrap_err()
            .to_string()
            .contains("duplicate variable"));

        std::fs::write(&config_path, &valid).unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(AppConfig::load_from_literal_file(&config_path)
            .unwrap_err()
            .to_string()
            .contains("inaccessible to group/other"));

        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link_path = root.path().join("runtime-link.env");
        symlink(&config_path, &link_path).unwrap();
        assert!(AppConfig::load_from_literal_file(&link_path).is_err());
    }

    #[test]
    fn default_file_safe_root_is_narrow_termux_home_directory() {
        let file = FileConfig {
            safe_roots: vec![PathBuf::from(DEFAULT_FILE_SAFE_ROOT)],
            create_directory_mutation_enabled: false,
        };
        let broad_storage = PathBuf::from("/storage/emulated/0");
        let sdcard = PathBuf::from("/sdcard");

        validate_file_safe_roots(&file).expect("default safe root should validate");
        assert_eq!(file.safe_roots, vec![PathBuf::from(DEFAULT_FILE_SAFE_ROOT)]);
        assert!(!file.safe_roots.contains(&broad_storage));
        assert!(!file.safe_roots.contains(&sdcard));
    }

    #[test]
    fn absent_environment_values_use_documented_defaults() {
        let config = load_from_os_values([]).expect("absent values should use safe defaults");

        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 8000);
        assert_eq!(config.auth.static_token, None);
        assert_eq!(config.capability.key_id, None);
        assert_eq!(config.capability.hmac_key_hex(), None);
        assert!(!config.android.battery_status_enabled);
        assert!(!config.android.volume_status_enabled);
        assert!(!config.android.volume_control_enabled);
        assert!(!config.command.enabled);
        assert!(!config.file.create_directory_mutation_enabled);
        assert_eq!(
            config.file.safe_roots,
            vec![PathBuf::from(DEFAULT_FILE_SAFE_ROOT)]
        );
    }

    #[test]
    fn android_battery_status_requires_compile_and_runtime_opt_in() {
        let config = load_from_os_values([]).unwrap();
        assert!(!config.android.battery_status_enabled);

        let configured = load_from_os_values([(
            "MCP__ANDROID__BATTERY_STATUS_ENABLED",
            OsString::from("true"),
        )]);
        if cfg!(feature = "android-battery-status") {
            assert!(configured.unwrap().android.battery_status_enabled);
        } else {
            assert_eq!(
                configured.unwrap_err().to_string(),
                "MCP__ANDROID__BATTERY_STATUS_ENABLED requires a binary built with the android-battery-status feature"
            );
        }
    }

    #[test]
    fn android_battery_status_rejects_invalid_runtime_flag() {
        let error = load_from_os_values([(
            "MCP__ANDROID__BATTERY_STATUS_ENABLED",
            OsString::from("sometimes"),
        )])
        .unwrap_err();
        assert!(error
            .to_string()
            .starts_with("MCP__ANDROID__BATTERY_STATUS_ENABLED must be a boolean value"));
    }

    #[test]
    fn android_volume_status_requires_compile_and_runtime_opt_in() {
        let config = load_from_os_values([]).unwrap();
        assert!(!config.android.volume_status_enabled);

        let configured = load_from_os_values([(
            "MCP__ANDROID__VOLUME_STATUS_ENABLED",
            OsString::from("true"),
        )]);
        if cfg!(feature = "android-volume-status") {
            assert!(configured.unwrap().android.volume_status_enabled);
        } else {
            assert_eq!(
                configured.unwrap_err().to_string(),
                "MCP__ANDROID__VOLUME_STATUS_ENABLED requires a binary built with the android-volume-status feature"
            );
        }
    }

    #[test]
    fn android_volume_status_rejects_invalid_runtime_flag() {
        let error = load_from_os_values([(
            "MCP__ANDROID__VOLUME_STATUS_ENABLED",
            OsString::from("sometimes"),
        )])
        .unwrap_err();
        assert!(error
            .to_string()
            .starts_with("MCP__ANDROID__VOLUME_STATUS_ENABLED must be a boolean value"));
    }

    #[test]
    fn android_volume_control_requires_compile_runtime_auth_and_key_opt_in() {
        let config = load_from_os_values([]).unwrap();
        assert!(!config.android.volume_control_enabled);

        let entries = [
            (
                "MCP__ANDROID__VOLUME_CONTROL_ENABLED",
                OsString::from("true"),
            ),
            (
                "MCP__AUTH__STATIC_TOKEN",
                OsString::from("static-principal-secret"),
            ),
            ("MCP__CAPABILITY__KEY_ID", OsString::from("primary-1")),
            (
                "MCP__CAPABILITY__HMAC_KEY_HEX",
                OsString::from("a".repeat(64)),
            ),
        ];
        let configured = load_from_os_values(entries);
        if cfg!(feature = "android-volume-control") {
            assert!(configured.unwrap().android.volume_control_enabled);
        } else {
            assert_eq!(
                configured.unwrap_err().to_string(),
                "MCP__ANDROID__VOLUME_CONTROL_ENABLED requires a binary built with the android-volume-control feature"
            );
        }
    }

    #[cfg(feature = "android-volume-control")]
    #[test]
    fn android_volume_control_rejects_missing_principal_or_key() {
        let missing_principal = load_from_os_values([
            (
                "MCP__ANDROID__VOLUME_CONTROL_ENABLED",
                OsString::from("true"),
            ),
            ("MCP__CAPABILITY__KEY_ID", OsString::from("primary-1")),
            (
                "MCP__CAPABILITY__HMAC_KEY_HEX",
                OsString::from("a".repeat(64)),
            ),
        ])
        .unwrap_err();
        assert_eq!(
            missing_principal.to_string(),
            "MCP__ANDROID__VOLUME_CONTROL_ENABLED requires MCP__AUTH__STATIC_TOKEN"
        );

        let missing_key = load_from_os_values([
            (
                "MCP__ANDROID__VOLUME_CONTROL_ENABLED",
                OsString::from("true"),
            ),
            (
                "MCP__AUTH__STATIC_TOKEN",
                OsString::from("static-principal-secret"),
            ),
        ])
        .unwrap_err();
        assert_eq!(
            missing_key.to_string(),
            "MCP__ANDROID__VOLUME_CONTROL_ENABLED requires MCP__CAPABILITY__KEY_ID and MCP__CAPABILITY__HMAC_KEY_HEX"
        );
    }

    #[test]
    fn android_volume_control_rejects_invalid_runtime_flag() {
        let error = load_from_os_values([(
            "MCP__ANDROID__VOLUME_CONTROL_ENABLED",
            OsString::from("sometimes"),
        )])
        .unwrap_err();
        assert!(error
            .to_string()
            .starts_with("MCP__ANDROID__VOLUME_CONTROL_ENABLED must be a boolean value"));
    }

    #[test]
    fn command_execution_requires_compile_and_runtime_opt_in() {
        let config = load_from_os_values([]).unwrap();
        assert!(!config.command.enabled);

        let configured = load_from_os_values([("MCP__COMMAND__ENABLED", OsString::from("true"))]);
        if cfg!(feature = "command-execution") {
            assert!(configured.unwrap().command.enabled);
        } else {
            assert_eq!(
                configured.unwrap_err().to_string(),
                "MCP__COMMAND__ENABLED requires a binary built with the command-execution feature"
            );
        }
    }

    #[test]
    fn command_execution_rejects_invalid_runtime_flag() {
        let error = load_from_os_values([("MCP__COMMAND__ENABLED", OsString::from("sometimes"))])
            .unwrap_err();
        assert!(error
            .to_string()
            .starts_with("MCP__COMMAND__ENABLED must be a boolean value"));
    }

    #[test]
    fn create_directory_mutation_requires_compile_gate_static_auth_and_exact_key_pair() {
        let entries = [
            (
                "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED",
                OsString::from("true"),
            ),
            (
                "MCP__AUTH__STATIC_TOKEN",
                OsString::from("static-principal-secret"),
            ),
            ("MCP__CAPABILITY__KEY_ID", OsString::from("primary-1")),
            (
                "MCP__CAPABILITY__HMAC_KEY_HEX",
                OsString::from("a".repeat(64)),
            ),
        ];
        let configured = load_from_os_values(entries);
        if cfg!(feature = "mcp-runtime") {
            let configured = configured.unwrap();
            assert!(configured.file.create_directory_mutation_enabled);
            assert_eq!(configured.capability.key_id.as_deref(), Some("primary-1"));
            assert_eq!(
                configured.capability.hmac_key_hex(),
                Some("a".repeat(64).as_str())
            );
        } else {
            assert_eq!(
                configured.unwrap_err().to_string(),
                "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED requires a binary built with the mcp-runtime feature"
            );
        }
    }

    #[cfg(feature = "mcp-runtime")]
    #[test]
    fn create_directory_mutation_rejects_missing_static_principal_or_key() {
        let missing_principal = load_from_os_values([
            (
                "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED",
                OsString::from("true"),
            ),
            ("MCP__CAPABILITY__KEY_ID", OsString::from("primary-1")),
            (
                "MCP__CAPABILITY__HMAC_KEY_HEX",
                OsString::from("a".repeat(64)),
            ),
        ])
        .unwrap_err();
        assert_eq!(
            missing_principal.to_string(),
            "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED requires MCP__AUTH__STATIC_TOKEN"
        );

        let missing_key = load_from_os_values([
            (
                "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED",
                OsString::from("true"),
            ),
            (
                "MCP__AUTH__STATIC_TOKEN",
                OsString::from("static-principal-secret"),
            ),
        ])
        .unwrap_err();
        assert_eq!(
            missing_key.to_string(),
            "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED requires MCP__CAPABILITY__KEY_ID and MCP__CAPABILITY__HMAC_KEY_HEX"
        );
    }

    #[test]
    fn capability_key_configuration_is_exact_and_fail_closed_even_while_gate_is_disabled() {
        for key_id in ["", "Upper", "bad.key", &"a".repeat(33)] {
            let error = load_from_os_values([
                ("MCP__CAPABILITY__KEY_ID", OsString::from(key_id)),
                (
                    "MCP__CAPABILITY__HMAC_KEY_HEX",
                    OsString::from("a".repeat(64)),
                ),
            ])
            .unwrap_err();
            assert!(error.to_string().starts_with("MCP__CAPABILITY__KEY_ID"));
        }
        for key in ["a".repeat(63), "A".repeat(64), "z".repeat(64)] {
            let error = load_from_os_values([
                ("MCP__CAPABILITY__KEY_ID", OsString::from("primary-1")),
                ("MCP__CAPABILITY__HMAC_KEY_HEX", OsString::from(key)),
            ])
            .unwrap_err();
            assert!(error
                .to_string()
                .starts_with("MCP__CAPABILITY__HMAC_KEY_HEX"));
        }
        for entries in [
            vec![("MCP__CAPABILITY__KEY_ID", OsString::from("primary-1"))],
            vec![(
                "MCP__CAPABILITY__HMAC_KEY_HEX",
                OsString::from("a".repeat(64)),
            )],
        ] {
            assert_eq!(
                load_from_os_values(entries).unwrap_err().to_string(),
                "MCP__CAPABILITY__KEY_ID and MCP__CAPABILITY__HMAC_KEY_HEX must be configured together"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn present_non_unicode_security_environment_values_fail_closed() {
        use std::os::unix::ffi::OsStringExt;

        for name in [
            "MCP__AUTH__STATIC_TOKEN",
            "MCP__SERVER__HOST",
            "MCP__FILE__SAFE_ROOTS",
            "MCP__SERVER__PORT",
            "MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY",
            "MCP__CAPABILITY__KEY_ID",
            "MCP__CAPABILITY__HMAC_KEY_HEX",
            "MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED",
            "MCP__ANDROID__BATTERY_STATUS_ENABLED",
            "MCP__ANDROID__VOLUME_STATUS_ENABLED",
            "MCP__ANDROID__VOLUME_CONTROL_ENABLED",
            "MCP__COMMAND__ENABLED",
            "MCP__TRANSPORT__ALLOWED_HOSTS",
            "MCP__TRANSPORT__ALLOWED_ORIGINS",
            "MCP__TRANSPORT__ALLOW_MISSING_ORIGIN",
            "MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS",
            "MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS",
            "MCP__TRANSPORT__MAX_BODY_BYTES",
        ] {
            let invalid = OsString::from_vec(vec![b'x', 0xff, b'y']);
            let mut entries = vec![(name, invalid)];
            if name == "MCP__AUTH__STATIC_TOKEN" {
                entries.push((
                    "MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY",
                    OsString::from("true"),
                ));
            }
            let error = load_from_os_values(entries)
                .expect_err("present non-Unicode configuration must never default");
            let message = error.to_string();

            assert!(message.contains(name));
            assert!(message.contains("valid Unicode text"));
            assert!(!message.contains('�'));
        }
    }

    #[test]
    fn configured_server_port_must_be_between_one_and_65535() {
        for value in ["0", "65536", "-1", "not-a-port"] {
            let error = load_from_os_values([("MCP__SERVER__PORT", OsString::from(value))])
                .expect_err("invalid or ephemeral listener ports must fail closed");

            assert_eq!(
                error.to_string(),
                "MCP__SERVER__PORT must be an integer between 1 and 65535"
            );
        }

        let config = load_from_os_values([("MCP__SERVER__PORT", OsString::from(" 65535 "))])
            .expect("highest stable TCP port should be accepted");
        assert_eq!(config.server.port, 65535);
    }

    #[test]
    fn safe_root_list_rejects_empty_entries_without_trimming_paths() {
        for value in ["", ",", "/tmp/root,", ",/tmp/root", "/tmp/a,,/tmp/b"] {
            let error = load_from_os_values([("MCP__FILE__SAFE_ROOTS", OsString::from(value))])
                .expect_err("empty safe-root entries must fail closed");

            assert_eq!(
                error.to_string(),
                "MCP__FILE__SAFE_ROOTS must not contain empty list entries"
            );
        }

        let error = load_from_os_values([(
            "MCP__FILE__SAFE_ROOTS",
            OsString::from("/tmp/first, /tmp/second"),
        )])
        .expect_err("leading path whitespace must not be silently normalized");
        assert_eq!(
            error.to_string(),
            "MCP__FILE__SAFE_ROOTS contains a non-absolute safe root"
        );

        let config = load_from_os_values([(
            "MCP__FILE__SAFE_ROOTS",
            OsString::from("/tmp/root with space,/tmp/trailing "),
        )])
        .expect("absolute safe-root text should preserve exact path semantics");
        assert_eq!(
            config.file.safe_roots,
            vec![
                PathBuf::from("/tmp/root with space"),
                PathBuf::from("/tmp/trailing ")
            ]
        );
    }

    #[test]
    fn empty_safe_roots_are_rejected() {
        let file = FileConfig {
            safe_roots: vec![],
            create_directory_mutation_enabled: false,
        };

        let err = validate_file_safe_roots(&file).expect_err("empty safe roots must fail closed");
        assert!(err.to_string().contains("at least one absolute safe root"));
    }

    #[test]
    fn relative_safe_roots_are_rejected() {
        let file = FileConfig {
            safe_roots: vec![PathBuf::from("relative/path")],
            create_directory_mutation_enabled: false,
        };

        let err = validate_file_safe_roots(&file).expect_err("relative safe roots must fail");
        assert!(err.to_string().contains("non-absolute safe root"));
    }

    #[test]
    fn filesystem_root_is_rejected() {
        let file = FileConfig {
            safe_roots: vec![PathBuf::from("/")],
            create_directory_mutation_enabled: false,
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

        for value in [
            " http://localhost:8000",
            "http://localhost:8000 ",
            "http://localhost:8000,\thttp://127.0.0.1:8000",
        ] {
            let transport = TransportConfig {
                allowed_origins: split_exact_env_list("ALLOWED_ORIGINS", value).unwrap(),
                ..transport_config()
            };

            validate_transport_security(&transport)
                .expect_err("configured origin whitespace must fail closed");
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
