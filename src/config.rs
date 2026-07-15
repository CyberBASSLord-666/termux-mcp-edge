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
    pub android: AndroidConfig,
    pub command: CommandConfig,
    pub file: FileConfig,
    pub directory_grant: DirectoryGrantConfig,
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
    /// Optional trusted, non-secret stable identity for the configured static credential.
    /// Caller-controlled headers and tool arguments are never principal sources.
    pub static_principal_id: Option<String>,
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
                "static_principal_id_configured",
                &self.static_principal_id.is_some(),
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

#[derive(Clone)]
pub struct DirectoryGrantConfig {
    pub verification_enabled: bool,
    pub issuer: Option<String>,
    pub audience: Option<String>,
    pub keyring_path: Option<PathBuf>,
    pub safe_root_ids: Option<Vec<String>>,
    pub max_lifetime_seconds: u64,
    pub clock_skew_seconds: u64,
    pub replay_enabled: bool,
    pub replay_keyring_path: Option<PathBuf>,
    pub replay_ledger_path: Option<PathBuf>,
    pub replay_max_records: usize,
    pub replay_max_bytes: usize,
}

impl fmt::Debug for DirectoryGrantConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectoryGrantConfig")
            .field("verification_enabled", &self.verification_enabled)
            .field("issuer_configured", &self.issuer.is_some())
            .field("audience_configured", &self.audience.is_some())
            .field("keyring_path_configured", &self.keyring_path.is_some())
            .field(
                "safe_root_identity_count",
                &self.safe_root_ids.as_ref().map(Vec::len),
            )
            .field("max_lifetime_seconds", &self.max_lifetime_seconds)
            .field("clock_skew_seconds", &self.clock_skew_seconds)
            .field("replay_enabled", &self.replay_enabled)
            .field(
                "replay_keyring_path_configured",
                &self.replay_keyring_path.is_some(),
            )
            .field(
                "replay_ledger_path_configured",
                &self.replay_ledger_path.is_some(),
            )
            .field("replay_max_records", &self.replay_max_records)
            .field("replay_max_bytes", &self.replay_max_bytes)
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AndroidConfig {
    /// Explicit runtime opt-in for the separately feature-gated battery tool.
    pub battery_status_enabled: bool,
    /// Explicit runtime opt-in for the separately feature-gated volume tool.
    pub volume_status_enabled: bool,
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
                static_principal_id: optional_env_string(
                    &read_variable,
                    "MCP__AUTH__STATIC_PRINCIPAL_ID",
                )?,
                allow_unauthenticated_localhost_only: env_bool(
                    &read_variable,
                    "MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY",
                    false,
                )?,
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
            },
            directory_grant: DirectoryGrantConfig {
                verification_enabled: env_bool(
                    &read_variable,
                    "MCP__DIRECTORY_GRANT__VERIFICATION_ENABLED",
                    false,
                )?,
                issuer: optional_env_string(&read_variable, "MCP__DIRECTORY_GRANT__ISSUER")?,
                audience: optional_env_string(&read_variable, "MCP__DIRECTORY_GRANT__AUDIENCE")?,
                keyring_path: optional_env_string(
                    &read_variable,
                    "MCP__DIRECTORY_GRANT__KEYRING_PATH",
                )?
                .map(PathBuf::from),
                safe_root_ids: optional_env_exact_string_list(
                    &read_variable,
                    "MCP__DIRECTORY_GRANT__SAFE_ROOT_IDS",
                )?,
                max_lifetime_seconds: env_u64(
                    &read_variable,
                    "MCP__DIRECTORY_GRANT__MAX_LIFETIME_SECONDS",
                    120,
                )?,
                clock_skew_seconds: env_u64(
                    &read_variable,
                    "MCP__DIRECTORY_GRANT__CLOCK_SKEW_SECONDS",
                    5,
                )?,
                replay_enabled: env_bool(
                    &read_variable,
                    "MCP__DIRECTORY_GRANT__REPLAY_ENABLED",
                    false,
                )?,
                replay_keyring_path: optional_env_string(
                    &read_variable,
                    "MCP__DIRECTORY_GRANT__REPLAY_KEYRING_PATH",
                )?
                .map(PathBuf::from),
                replay_ledger_path: optional_env_string(
                    &read_variable,
                    "MCP__DIRECTORY_GRANT__REPLAY_LEDGER_PATH",
                )?
                .map(PathBuf::from),
                replay_max_records: env_usize(
                    &read_variable,
                    "MCP__DIRECTORY_GRANT__REPLAY_MAX_RECORDS",
                    4096,
                )?,
                replay_max_bytes: env_usize(
                    &read_variable,
                    "MCP__DIRECTORY_GRANT__REPLAY_MAX_BYTES",
                    1_048_576,
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
        validate_directory_grant_config(&config.directory_grant, &config.auth, &config.file)?;
        validate_android_capabilities(&config.android)?;
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

fn optional_env_exact_string_list(
    read_variable: &impl Fn(&str) -> Result<String, env::VarError>,
    name: &str,
) -> anyhow::Result<Option<Vec<String>>> {
    match read_env(read_variable, name)? {
        Some(value) => split_exact_env_list(name, &value).map(Some),
        None => Ok(None),
    }
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

fn validate_directory_grant_config(
    grant: &DirectoryGrantConfig,
    auth: &AuthConfig,
    file: &FileConfig,
) -> anyhow::Result<()> {
    const MAX_AUTHORITY_LABEL_BYTES: usize = 256;
    const MAX_SAFE_ROOT_ID_BYTES: usize = 64;
    const MAX_GRANT_LIFETIME_SECONDS: u64 = 300;
    const MAX_CLOCK_SKEW_SECONDS: u64 = 30;

    let subordinate_configured = grant.issuer.is_some()
        || grant.audience.is_some()
        || grant.keyring_path.is_some()
        || grant.safe_root_ids.is_some()
        || grant.replay_enabled
        || grant.replay_keyring_path.is_some()
        || grant.replay_ledger_path.is_some();
    if !grant.verification_enabled {
        if subordinate_configured {
            bail!(
                "directory grant verification settings require MCP__DIRECTORY_GRANT__VERIFICATION_ENABLED=true"
            );
        }
        return Ok(());
    }
    if !cfg!(feature = "mcp-runtime") {
        bail!(
            "MCP__DIRECTORY_GRANT__VERIFICATION_ENABLED requires a binary built with the mcp-runtime feature"
        );
    }
    if auth.static_token.is_none() || auth.static_principal_id.is_none() {
        bail!(
            "directory grant verification requires static bearer authentication with MCP__AUTH__STATIC_PRINCIPAL_ID"
        );
    }
    let issuer = grant
        .issuer
        .as_deref()
        .ok_or_else(|| anyhow!("MCP__DIRECTORY_GRANT__ISSUER is required"))?;
    let audience = grant
        .audience
        .as_deref()
        .ok_or_else(|| anyhow!("MCP__DIRECTORY_GRANT__AUDIENCE is required"))?;
    for (name, value) in [
        ("MCP__DIRECTORY_GRANT__ISSUER", issuer),
        ("MCP__DIRECTORY_GRANT__AUDIENCE", audience),
    ] {
        if value.is_empty()
            || value.len() > MAX_AUTHORITY_LABEL_BYTES
            || value.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
        {
            bail!("{name} must be bounded visible ASCII without whitespace");
        }
    }
    let keyring_path = grant
        .keyring_path
        .as_deref()
        .ok_or_else(|| anyhow!("MCP__DIRECTORY_GRANT__KEYRING_PATH is required"))?;
    if !keyring_path.is_absolute() {
        bail!("MCP__DIRECTORY_GRANT__KEYRING_PATH must be absolute");
    }
    let root_ids = grant
        .safe_root_ids
        .as_ref()
        .ok_or_else(|| anyhow!("MCP__DIRECTORY_GRANT__SAFE_ROOT_IDS is required"))?;
    if root_ids.len() != file.safe_roots.len() {
        bail!(
            "MCP__DIRECTORY_GRANT__SAFE_ROOT_IDS must contain exactly one identity per configured safe root"
        );
    }
    let mut unique = std::collections::BTreeSet::new();
    for root_id in root_ids {
        if root_id.is_empty()
            || root_id.len() > MAX_SAFE_ROOT_ID_BYTES
            || !root_id.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'_' | b':' | b'.')
            })
            || !unique.insert(root_id)
        {
            bail!("MCP__DIRECTORY_GRANT__SAFE_ROOT_IDS contains an invalid or duplicate identity");
        }
    }
    if !(1..=MAX_GRANT_LIFETIME_SECONDS).contains(&grant.max_lifetime_seconds) {
        bail!(
            "MCP__DIRECTORY_GRANT__MAX_LIFETIME_SECONDS must be between 1 and {MAX_GRANT_LIFETIME_SECONDS}"
        );
    }
    if grant.clock_skew_seconds > MAX_CLOCK_SKEW_SECONDS {
        bail!("MCP__DIRECTORY_GRANT__CLOCK_SKEW_SECONDS must not exceed {MAX_CLOCK_SKEW_SECONDS}");
    }
    let replay_paths_configured =
        grant.replay_keyring_path.is_some() || grant.replay_ledger_path.is_some();
    if !grant.replay_enabled && replay_paths_configured {
        bail!("directory replay paths require MCP__DIRECTORY_GRANT__REPLAY_ENABLED=true");
    }
    if grant.replay_enabled {
        let keyring = grant
            .replay_keyring_path
            .as_deref()
            .ok_or_else(|| anyhow!("MCP__DIRECTORY_GRANT__REPLAY_KEYRING_PATH is required"))?;
        let ledger = grant
            .replay_ledger_path
            .as_deref()
            .ok_or_else(|| anyhow!("MCP__DIRECTORY_GRANT__REPLAY_LEDGER_PATH is required"))?;
        if !keyring.is_absolute() || !ledger.is_absolute() || keyring == ledger {
            bail!("directory replay keyring and ledger paths must be distinct absolute paths");
        }
        if !(1..=16_384).contains(&grant.replay_max_records) {
            bail!("MCP__DIRECTORY_GRANT__REPLAY_MAX_RECORDS must be between 1 and 16384");
        }
        let minimum_bytes = 16_usize.saturating_add(224_usize);
        if grant.replay_max_bytes < minimum_bytes || grant.replay_max_bytes > 16_777_216 {
            bail!("MCP__DIRECTORY_GRANT__REPLAY_MAX_BYTES is outside its fixed safety range");
        }
    }
    Ok(())
}

pub fn validate_runtime_auth_posture(config: &AppConfig) -> anyhow::Result<AuthPosture> {
    validate_static_principal_identity(&config.auth)?;

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

fn validate_static_principal_identity(auth: &AuthConfig) -> anyhow::Result<()> {
    const MAX_PRINCIPAL_ID_BYTES: usize = 128;

    let Some(principal) = auth.static_principal_id.as_deref() else {
        return Ok(());
    };
    let Some(token) = auth.static_token.as_deref() else {
        bail!("MCP__AUTH__STATIC_PRINCIPAL_ID requires MCP__AUTH__STATIC_TOKEN");
    };
    if principal.is_empty() {
        bail!("MCP__AUTH__STATIC_PRINCIPAL_ID must not be empty");
    }
    if principal.len() > MAX_PRINCIPAL_ID_BYTES {
        bail!(
            "MCP__AUTH__STATIC_PRINCIPAL_ID exceeds the {MAX_PRINCIPAL_ID_BYTES}-byte safety limit"
        );
    }
    if !principal
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        bail!(
            "MCP__AUTH__STATIC_PRINCIPAL_ID may contain only ASCII letters, digits, '.', '_', '-', and ':'"
        );
    }
    if principal == token {
        bail!("MCP__AUTH__STATIC_PRINCIPAL_ID must not equal MCP__AUTH__STATIC_TOKEN");
    }
    Ok(())
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
                static_principal_id: None,
                allow_unauthenticated_localhost_only: allow_localhost_only,
            },
            android: AndroidConfig {
                battery_status_enabled: false,
                volume_status_enabled: false,
            },
            command: CommandConfig { enabled: false },
            file: FileConfig {
                safe_roots: vec![PathBuf::from(DEFAULT_FILE_SAFE_ROOT)],
            },
            directory_grant: DirectoryGrantConfig {
                verification_enabled: false,
                issuer: None,
                audience: None,
                keyring_path: None,
                safe_root_ids: None,
                max_lifetime_seconds: 120,
                clock_skew_seconds: 5,
                replay_enabled: false,
                replay_keyring_path: None,
                replay_ledger_path: None,
                replay_max_records: 4096,
                replay_max_bytes: 1_048_576,
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
            static_principal_id: Some("operator.primary:v1".to_owned()),
            allow_unauthenticated_localhost_only: false,
        };
        let debug = format!("{auth:?}");

        assert!(debug.contains("<redacted>"));
        assert!(debug.contains("static_principal_id_configured: true"));
        assert!(!debug.contains("secret-value"));
        assert!(!debug.contains("operator.primary:v1"));
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
    fn absent_environment_values_use_documented_defaults() {
        let config = load_from_os_values([]).expect("absent values should use safe defaults");

        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 8000);
        assert_eq!(config.auth.static_token, None);
        assert_eq!(config.auth.static_principal_id, None);
        assert!(!config.android.battery_status_enabled);
        assert!(!config.android.volume_status_enabled);
        assert!(!config.command.enabled);
        assert!(!config.directory_grant.verification_enabled);
        assert_eq!(config.directory_grant.issuer, None);
        assert_eq!(config.directory_grant.audience, None);
        assert_eq!(config.directory_grant.keyring_path, None);
        assert_eq!(config.directory_grant.safe_root_ids, None);
        assert_eq!(config.directory_grant.max_lifetime_seconds, 120);
        assert_eq!(config.directory_grant.clock_skew_seconds, 5);
        assert!(!config.directory_grant.replay_enabled);
        assert_eq!(config.directory_grant.replay_keyring_path, None);
        assert_eq!(config.directory_grant.replay_ledger_path, None);
        assert_eq!(config.directory_grant.replay_max_records, 4096);
        assert_eq!(config.directory_grant.replay_max_bytes, 1_048_576);
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

    #[cfg(unix)]
    #[test]
    fn present_non_unicode_security_environment_values_fail_closed() {
        use std::os::unix::ffi::OsStringExt;

        for name in [
            "MCP__AUTH__STATIC_TOKEN",
            "MCP__AUTH__STATIC_PRINCIPAL_ID",
            "MCP__SERVER__HOST",
            "MCP__FILE__SAFE_ROOTS",
            "MCP__SERVER__PORT",
            "MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY",
            "MCP__ANDROID__BATTERY_STATUS_ENABLED",
            "MCP__ANDROID__VOLUME_STATUS_ENABLED",
            "MCP__COMMAND__ENABLED",
            "MCP__DIRECTORY_GRANT__VERIFICATION_ENABLED",
            "MCP__DIRECTORY_GRANT__ISSUER",
            "MCP__DIRECTORY_GRANT__AUDIENCE",
            "MCP__DIRECTORY_GRANT__KEYRING_PATH",
            "MCP__DIRECTORY_GRANT__SAFE_ROOT_IDS",
            "MCP__DIRECTORY_GRANT__MAX_LIFETIME_SECONDS",
            "MCP__DIRECTORY_GRANT__CLOCK_SKEW_SECONDS",
            "MCP__DIRECTORY_GRANT__REPLAY_ENABLED",
            "MCP__DIRECTORY_GRANT__REPLAY_KEYRING_PATH",
            "MCP__DIRECTORY_GRANT__REPLAY_LEDGER_PATH",
            "MCP__DIRECTORY_GRANT__REPLAY_MAX_RECORDS",
            "MCP__DIRECTORY_GRANT__REPLAY_MAX_BYTES",
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
    fn directory_grant_settings_are_default_disabled_and_fail_closed_when_partial() {
        let mut config = app_config("127.0.0.1", Some("token"), false);
        config.directory_grant.issuer = Some("authority:v1".to_owned());
        let error =
            validate_directory_grant_config(&config.directory_grant, &config.auth, &config.file)
                .unwrap_err();
        assert!(error.to_string().contains("VERIFICATION_ENABLED=true"));
        assert!(!error.to_string().contains("authority:v1"));
    }

    #[cfg(feature = "mcp-runtime")]
    #[test]
    fn directory_grant_verification_requires_complete_trusted_configuration() {
        let mut config = app_config("127.0.0.1", Some("token"), false);
        config.auth.static_principal_id = Some("operator.primary:v1".to_owned());
        config.directory_grant = DirectoryGrantConfig {
            verification_enabled: true,
            issuer: Some("authority:v1".to_owned()),
            audience: Some("server:v1".to_owned()),
            keyring_path: Some(PathBuf::from(
                "/data/data/com.termux/files/home/.config/mcp/grants.json",
            )),
            safe_root_ids: Some(vec!["primary-root".to_owned()]),
            max_lifetime_seconds: 120,
            clock_skew_seconds: 5,
            replay_enabled: false,
            replay_keyring_path: None,
            replay_ledger_path: None,
            replay_max_records: 4096,
            replay_max_bytes: 1_048_576,
        };
        validate_directory_grant_config(&config.directory_grant, &config.auth, &config.file)
            .unwrap();

        config.auth.static_principal_id = None;
        assert!(validate_directory_grant_config(
            &config.directory_grant,
            &config.auth,
            &config.file,
        )
        .is_err());
        config.auth.static_principal_id = Some("operator.primary:v1".to_owned());
        config.directory_grant.safe_root_ids =
            Some(vec!["duplicate".to_owned(), "duplicate".to_owned()]);
        assert!(validate_directory_grant_config(
            &config.directory_grant,
            &config.auth,
            &config.file,
        )
        .is_err());
    }

    #[test]
    fn directory_grant_debug_output_does_not_disclose_authority_or_keyring_path() {
        let config = DirectoryGrantConfig {
            verification_enabled: true,
            issuer: Some("private-authority:v1".to_owned()),
            audience: Some("private-audience:v1".to_owned()),
            keyring_path: Some(PathBuf::from("/private/keyring.json")),
            safe_root_ids: Some(vec!["private-root".to_owned()]),
            max_lifetime_seconds: 120,
            clock_skew_seconds: 5,
            replay_enabled: false,
            replay_keyring_path: None,
            replay_ledger_path: None,
            replay_max_records: 4096,
            replay_max_bytes: 1_048_576,
        };
        let debug = format!("{config:?}");
        assert!(debug.contains("issuer_configured: true"));
        assert!(debug.contains("keyring_path_configured: true"));
        assert!(!debug.contains("private-authority"));
        assert!(!debug.contains("private-audience"));
        assert!(!debug.contains("/private/keyring.json"));
        assert!(!debug.contains("private-root"));
    }

    #[test]
    fn static_token_auth_posture_is_accepted_for_non_loopback_hosts() {
        let config = app_config("0.0.0.0", Some("configured-token"), false);

        let posture = validate_runtime_auth_posture(&config).expect("token auth should validate");

        assert_eq!(posture, AuthPosture::StaticTokenConfigured);
    }

    #[test]
    fn configured_static_principal_identity_is_validated_without_disclosure() {
        let mut config = app_config("0.0.0.0", Some("configured-token"), false);
        config.auth.static_principal_id = Some("operator.primary:v1".to_owned());
        assert_eq!(
            validate_runtime_auth_posture(&config).unwrap(),
            AuthPosture::StaticTokenConfigured
        );

        config.auth.static_principal_id = Some(String::new());
        let error = validate_runtime_auth_posture(&config).unwrap_err();
        assert!(error.to_string().contains("must not be empty"));

        for invalid in ["contains whitespace", "slash/not-allowed", "é"] {
            config.auth.static_principal_id = Some(invalid.to_owned());
            let error = validate_runtime_auth_posture(&config).unwrap_err();
            assert!(!error.to_string().contains(invalid));
        }

        config.auth.static_principal_id = Some("configured-token".to_owned());
        let error = validate_runtime_auth_posture(&config).unwrap_err();
        assert!(error.to_string().contains("must not equal"));
        assert!(!error.to_string().contains("configured-token"));
    }

    #[test]
    fn static_principal_identity_requires_static_token_authentication() {
        let mut config = app_config("127.0.0.1", None, true);
        config.auth.static_principal_id = Some("operator.primary:v1".to_owned());

        let error = validate_runtime_auth_posture(&config).unwrap_err();
        assert_eq!(
            error.to_string(),
            "MCP__AUTH__STATIC_PRINCIPAL_ID requires MCP__AUTH__STATIC_TOKEN"
        );
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
