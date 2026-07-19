//! Termux MCP Edge server entrypoint.
//!
//! Key design principles:
//! - Fail-closed startup authentication posture
//! - Request authentication before staged MCP discovery or tool dispatch
//! - Bounded MCP concurrency, request duration, and request-body size
//! - Memory-safe async task management
//! - Hardened filesystem operations resistant to traversal and symlink attacks
//! - Graceful shutdown under runit supervision
//! - Single-binary deployment optimized for Android Termux

#![recursion_limit = "256"]

#[cfg(feature = "android-battery-status")]
pub mod android_battery;
#[cfg(any(feature = "android-battery-status", feature = "android-volume-status"))]
mod android_provider;
pub mod android_status;
#[cfg(feature = "android-volume-status")]
pub mod android_volume;
#[cfg(feature = "android-volume-control")]
pub mod android_volume_control;
#[cfg(feature = "android-volume-control")]
pub mod android_volume_grant;
pub mod audit;
pub mod auth;
#[cfg(any(
    feature = "android-battery-status",
    feature = "android-volume-status",
    feature = "android-volume-control",
    feature = "command-execution"
))]
mod bounded_process;
pub mod capability_token;
#[cfg(feature = "command-execution")]
mod command_execution;
pub mod command_policy;
pub mod config;
#[cfg(feature = "mcp-runtime")]
pub mod copy_file_grant;
#[cfg(feature = "mcp-runtime")]
pub mod create_directory_grant;
pub mod error;
#[cfg(feature = "mcp-runtime")]
mod grant_replay;
pub mod health;
pub mod json_rpc;
#[cfg(feature = "mcp-runtime")]
mod mcp_session;
#[cfg(feature = "mcp-runtime")]
pub mod mcp_transport;
pub mod platform_info;
#[cfg(feature = "mcp-runtime")]
mod request_grant_capability;
pub mod request_limits;
pub mod service_status;
pub mod tools;
pub mod transport_security;
#[cfg(feature = "mcp-runtime")]
pub mod write_file_grant;
pub mod write_policy;

use std::ffi::OsStr;

#[cfg(feature = "mcp-runtime")]
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

#[cfg(feature = "mcp-runtime")]
use crate::health::McpRequestLimitReadiness;
#[cfg(feature = "android-volume-control")]
use crate::{
    android_volume_control::AndroidVolumeStreamName,
    android_volume_grant::{AndroidVolumeGrantAuthority, AndroidVolumeGrantTarget},
};
#[cfg(feature = "mcp-runtime")]
use crate::{
    auth::McpAuthPolicy,
    copy_file_grant::CopyFileGrantAuthority,
    create_directory_grant::CreateDirectoryGrantAuthority,
    mcp_transport::McpRouterBuilder,
    request_limits::McpRequestLimits,
    transport_security::TransportSecurityPolicy,
    write_file_grant::{WriteFileDisposition, WriteFileGrantAuthority},
    write_policy::DEFAULT_MAX_WRITE_BYTES,
};
use crate::{
    config::{validate_runtime_auth_posture, AppConfig, AuthPosture},
    health::{build_readiness_response, ReadinessResponse},
    tools::FileSystemTools,
};
use axum::{extract::State, routing::get, Json, Router};
#[cfg(feature = "mcp-runtime")]
use rustix::fs::{fstat, open, FileType, Mode, OFlags};
use tokio::signal;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const CLI_HELP: &str = "Termux MCP Edge\n\nUsage:\n  termux-mcp-server\n  termux-mcp-server --version\n  termux-mcp-server --help\n  termux-mcp-server --issue-create-directory-grant\n  termux-mcp-server --issue-copy-file-grant\n  termux-mcp-server --issue-write-file-grant\n  termux-mcp-server --issue-android-volume-grant\n";

#[cfg(feature = "mcp-runtime")]
const CAPABILITY_SESSION_ENV: &str = "MCP__CAPABILITY__SESSION_ID";
#[cfg(feature = "mcp-runtime")]
const CAPABILITY_CREATE_DIRECTORY_TARGET_ENV: &str = "MCP__CAPABILITY__CREATE_DIRECTORY_TARGET";
#[cfg(feature = "mcp-runtime")]
const CAPABILITY_COPY_FILE_SOURCE_ENV: &str = "MCP__CAPABILITY__COPY_FILE_SOURCE";
#[cfg(feature = "mcp-runtime")]
const CAPABILITY_COPY_FILE_DESTINATION_ENV: &str = "MCP__CAPABILITY__COPY_FILE_DESTINATION";
#[cfg(feature = "mcp-runtime")]
const CAPABILITY_WRITE_FILE_TARGET_ENV: &str = "MCP__CAPABILITY__WRITE_FILE_TARGET";
#[cfg(feature = "mcp-runtime")]
const CAPABILITY_WRITE_FILE_CONTENT_FILE_ENV: &str = "MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE";
#[cfg(feature = "mcp-runtime")]
const CAPABILITY_WRITE_FILE_DISPOSITION_ENV: &str = "MCP__CAPABILITY__WRITE_FILE_DISPOSITION";
#[cfg(feature = "android-volume-control")]
const CAPABILITY_VOLUME_STREAM_ENV: &str = "MCP__CAPABILITY__VOLUME_STREAM";
#[cfg(feature = "android-volume-control")]
const CAPABILITY_VOLUME_LEVEL_ENV: &str = "MCP__CAPABILITY__VOLUME_LEVEL";
#[cfg(feature = "mcp-runtime")]
const CAPABILITY_CONFIG_FILE_ENV: &str = "MCP__CAPABILITY__CONFIG_FILE";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if handle_cli()? {
        return Ok(());
    }

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "termux_mcp_server=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "Starting Termux MCP Edge"
    );

    let config = AppConfig::load()?;
    info!(?config.server, "Configuration loaded");

    let auth_posture = validate_runtime_auth_posture(&config)?;
    let auth_posture_label = match auth_posture {
        AuthPosture::StaticTokenConfigured => {
            info!("Static token authentication configured");
            "static_token"
        }
        AuthPosture::UnauthenticatedLocalhostOnly => {
            warn!(
                "Unauthenticated local-only development mode enabled; do not expose this listener remotely"
            );
            "unauthenticated_localhost_only"
        }
    };

    #[cfg(feature = "mcp-runtime")]
    let mcp_auth_policy = McpAuthPolicy::from_config(&config.auth, auth_posture)?;

    #[cfg(feature = "mcp-runtime")]
    let mcp_request_limits = McpRequestLimits::from_seconds(
        config.transport.max_concurrent_requests,
        config.transport.request_timeout_seconds,
        config.transport.max_body_bytes,
    )?;

    #[cfg(feature = "mcp-runtime")]
    info!(
        max_concurrent_requests = config.transport.max_concurrent_requests,
        request_timeout_seconds = config.transport.request_timeout_seconds,
        max_body_bytes = config.transport.max_body_bytes,
        sse_enabled = config.transport.sse_enabled,
        "MCP request limits configured"
    );

    let display_addr = format!("{}:{}", config.server.host, config.server.port);
    let bind_addr = (config.server.host.as_str(), config.server.port);

    // Construct the complete MCP boundary before any listener is opened. The
    // one public builder validates listener/auth/limits/transport composition
    // and lifetime-pins every configured filesystem root.
    #[cfg(feature = "mcp-runtime")]
    let mcp_router_builder = McpRouterBuilder::new(
        &config.server.host,
        mcp_auth_policy,
        mcp_request_limits,
        TransportSecurityPolicy::new(
            config.transport.allowed_hosts.clone(),
            config.transport.allowed_origins.clone(),
            config.transport.allow_missing_origin,
        )?,
        config.file.safe_roots.clone(),
    )?;
    #[cfg(feature = "mcp-runtime")]
    let safe_root_count = mcp_router_builder.safe_root_count();

    #[cfg(not(feature = "mcp-runtime"))]
    let file_tools = FileSystemTools::try_new(config.file.safe_roots.clone())?;
    #[cfg(not(feature = "mcp-runtime"))]
    let safe_root_count = file_tools.safe_root_count();

    #[cfg(feature = "mcp-runtime")]
    let create_directory_authority = configured_create_directory_authority(&config)?;

    #[cfg(feature = "mcp-runtime")]
    let write_file_authority = configured_write_file_authority(&config)?;

    #[cfg(feature = "mcp-runtime")]
    let copy_file_authority = configured_copy_file_authority(&config)?;

    #[cfg(feature = "android-volume-control")]
    let android_volume_control_authority = configured_android_volume_control_authority(&config)?;

    #[cfg(feature = "mcp-runtime")]
    let readiness_limits = Some(McpRequestLimitReadiness {
        max_concurrent_requests: config.transport.max_concurrent_requests,
        request_timeout_seconds: config.transport.request_timeout_seconds,
        max_body_bytes: config.transport.max_body_bytes,
        sse_enabled: config.transport.sse_enabled,
    });

    #[cfg(not(feature = "mcp-runtime"))]
    let readiness_limits = None;

    let readiness = build_readiness_response(safe_root_count, auth_posture_label, readiness_limits);
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        .with_state(readiness)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    #[cfg(feature = "mcp-runtime")]
    let app = {
        let mut builder = mcp_router_builder
            .with_transport_options(
                crate::mcp_transport::McpTransportOptions::default()
                    .with_sse_enabled(config.transport.sse_enabled),
            )
            .with_android_battery_status_enabled(config.android.battery_status_enabled)
            .with_android_volume_status_enabled(config.android.volume_status_enabled)
            .with_command_execution_enabled(config.command.enabled);

        if let Some(authority) = create_directory_authority {
            builder = builder.with_create_directory_authority(authority);
        }
        if let Some(authority) = copy_file_authority {
            builder = builder.with_copy_file_authority(authority);
        }
        if let Some(authority) = write_file_authority {
            builder = builder.with_write_file_authority(authority);
        }
        #[cfg(feature = "android-volume-control")]
        if let Some(authority) = android_volume_control_authority {
            builder = builder.with_android_volume_control_authority(authority);
        }

        app.merge(builder.build()?)
    };

    #[cfg(not(feature = "mcp-runtime"))]
    let _ = file_tools;

    info!("Listening on http://{}", display_addr);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;

    // Supplying peer connection metadata is part of the MCP authentication
    // boundary. Explicit localhost-only development mode rejects every `/mcp`
    // request whose actual TCP peer is absent or non-loopback.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    info!("Server shutdown complete");
    Ok(())
}

fn handle_cli() -> anyhow::Result<bool> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let first = arguments.next();
    let second = arguments.next();

    match (first.as_deref(), second.as_deref()) {
        (None, None) => Ok(false),
        (Some(argument), None) if argument == OsStr::new("--version") => {
            println!("termux-mcp-server {}", env!("CARGO_PKG_VERSION"));
            Ok(true)
        }
        (Some(argument), None)
            if argument == OsStr::new("--help") || argument == OsStr::new("-h") =>
        {
            print!("{CLI_HELP}");
            Ok(true)
        }
        (Some(argument), None) if argument == OsStr::new("--self-check-command-boundary") => {
            verify_command_execution_boundary()?;
            println!("termux-mcp-command-boundary ok");
            Ok(true)
        }
        (Some(argument), None) if argument == OsStr::new("--issue-create-directory-grant") => {
            #[cfg(feature = "mcp-runtime")]
            {
                issue_create_directory_grant()?;
                Ok(true)
            }
            #[cfg(not(feature = "mcp-runtime"))]
            {
                anyhow::bail!(
                    "create_directory grant issuance requires a binary built with the mcp-runtime feature"
                )
            }
        }
        (Some(argument), None) if argument == OsStr::new("--issue-write-file-grant") => {
            #[cfg(feature = "mcp-runtime")]
            {
                issue_write_file_grant()?;
                Ok(true)
            }
            #[cfg(not(feature = "mcp-runtime"))]
            {
                anyhow::bail!(
                    "write_file grant issuance requires a binary built with the mcp-runtime feature"
                )
            }
        }
        (Some(argument), None) if argument == OsStr::new("--issue-copy-file-grant") => {
            #[cfg(feature = "mcp-runtime")]
            {
                issue_copy_file_grant()?;
                Ok(true)
            }
            #[cfg(not(feature = "mcp-runtime"))]
            {
                anyhow::bail!(
                    "copy_file grant issuance requires a binary built with the mcp-runtime feature"
                )
            }
        }
        (Some(argument), None) if argument == OsStr::new("--issue-android-volume-grant") => {
            #[cfg(feature = "android-volume-control")]
            {
                issue_android_volume_grant()?;
                Ok(true)
            }
            #[cfg(not(feature = "android-volume-control"))]
            {
                anyhow::bail!(
                    "Android volume grant issuance requires a binary built with the android-volume-control feature"
                )
            }
        }
        _ => anyhow::bail!("unsupported command-line arguments; use --help"),
    }
}

#[cfg(feature = "mcp-runtime")]
fn configured_create_directory_authority(
    config: &AppConfig,
) -> anyhow::Result<Option<CreateDirectoryGrantAuthority>> {
    if !config.file.create_directory_mutation_enabled {
        return Ok(None);
    }
    let key_id = config.capability.key_id.as_deref().ok_or_else(|| {
        anyhow::anyhow!("create_directory capability configuration is incomplete")
    })?;
    let key = config.capability.hmac_key_hex().ok_or_else(|| {
        anyhow::anyhow!("create_directory capability configuration is incomplete")
    })?;
    let principal = config.auth.static_token.as_deref().ok_or_else(|| {
        anyhow::anyhow!("create_directory capability requires static-token authentication")
    })?;
    CreateDirectoryGrantAuthority::from_hex_key(key_id, key, principal)
        .map(Some)
        .map_err(|_| anyhow::anyhow!("create_directory capability configuration is invalid"))
}

#[cfg(feature = "mcp-runtime")]
fn configured_write_file_authority(
    config: &AppConfig,
) -> anyhow::Result<Option<WriteFileGrantAuthority>> {
    if !config.file.write_file_mutation_enabled {
        return Ok(None);
    }
    let key_id = config
        .capability
        .key_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("write_file capability configuration is incomplete"))?;
    let key = config
        .capability
        .hmac_key_hex()
        .ok_or_else(|| anyhow::anyhow!("write_file capability configuration is incomplete"))?;
    let principal = config.auth.static_token.as_deref().ok_or_else(|| {
        anyhow::anyhow!("write_file capability requires static-token authentication")
    })?;
    WriteFileGrantAuthority::from_hex_key(key_id, key, principal)
        .map(Some)
        .map_err(|_| anyhow::anyhow!("write_file capability configuration is invalid"))
}

#[cfg(feature = "mcp-runtime")]
fn configured_copy_file_authority(
    config: &AppConfig,
) -> anyhow::Result<Option<CopyFileGrantAuthority>> {
    if !config.file.copy_file_mutation_enabled {
        return Ok(None);
    }
    let key_id = config
        .capability
        .key_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("copy_file capability configuration is incomplete"))?;
    let key = config
        .capability
        .hmac_key_hex()
        .ok_or_else(|| anyhow::anyhow!("copy_file capability configuration is incomplete"))?;
    let principal = config.auth.static_token.as_deref().ok_or_else(|| {
        anyhow::anyhow!("copy_file capability requires static-token authentication")
    })?;
    CopyFileGrantAuthority::from_hex_key(key_id, key, principal)
        .map(Some)
        .map_err(|_| anyhow::anyhow!("copy_file capability configuration is invalid"))
}

#[cfg(feature = "android-volume-control")]
fn configured_android_volume_control_authority(
    config: &AppConfig,
) -> anyhow::Result<Option<AndroidVolumeGrantAuthority>> {
    if !config.android.volume_control_enabled {
        return Ok(None);
    }
    let key_id =
        config.capability.key_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Android volume capability configuration is incomplete")
        })?;
    let key = config
        .capability
        .hmac_key_hex()
        .ok_or_else(|| anyhow::anyhow!("Android volume capability configuration is incomplete"))?;
    let principal = config.auth.static_token.as_deref().ok_or_else(|| {
        anyhow::anyhow!("Android volume capability requires static-token authentication")
    })?;
    AndroidVolumeGrantAuthority::from_hex_key(key_id, key, principal)
        .map(Some)
        .map_err(|_| anyhow::anyhow!("Android volume capability configuration is invalid"))
}

#[cfg(feature = "mcp-runtime")]
fn issue_create_directory_grant() -> anyhow::Result<()> {
    let config = load_offline_issuer_config()?;
    if !config.file.create_directory_mutation_enabled {
        anyhow::bail!("create_directory mutation gate is disabled");
    }
    let _ = validate_runtime_auth_posture(&config)?;
    let authority = configured_create_directory_authority(&config)?
        .ok_or_else(|| anyhow::anyhow!("create_directory mutation gate is disabled"))?;
    let session_id = required_grant_environment(CAPABILITY_SESSION_ENV)?;
    let target_path = required_grant_environment(CAPABILITY_CREATE_DIRECTORY_TARGET_ENV)?;
    let file_tools = FileSystemTools::try_new(config.file.safe_roots)?;
    let target = file_tools
        .create_directory_grant_target(&target_path)
        .map_err(|_| anyhow::anyhow!("create_directory grant target validation failed"))?;
    let now_unix_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| anyhow::anyhow!("system clock is before the Unix epoch"))?
        .as_secs();
    let grant = authority
        .issue_at(&session_id, &target, now_unix_seconds)
        .map_err(|_| anyhow::anyhow!("create_directory grant issuance failed"))?;
    println!("{grant}");
    Ok(())
}

#[cfg(feature = "mcp-runtime")]
fn issue_write_file_grant() -> anyhow::Result<()> {
    let config = load_offline_issuer_config()?;
    if !config.file.write_file_mutation_enabled {
        anyhow::bail!("write_file mutation gate is disabled");
    }
    let _ = validate_runtime_auth_posture(&config)?;
    let authority = configured_write_file_authority(&config)?
        .ok_or_else(|| anyhow::anyhow!("write_file mutation gate is disabled"))?;
    let session_id = required_grant_environment(CAPABILITY_SESSION_ENV)?;
    let target_path = required_grant_environment(CAPABILITY_WRITE_FILE_TARGET_ENV)?;
    let content_file = required_grant_environment(CAPABILITY_WRITE_FILE_CONTENT_FILE_ENV)?;
    let disposition = required_grant_environment(CAPABILITY_WRITE_FILE_DISPOSITION_ENV)?
        .parse::<WriteFileDisposition>()
        .map_err(|_| anyhow::anyhow!("write_file grant disposition validation failed"))?;
    let content = read_write_grant_content(Path::new(&content_file))?;
    reject_write_grant_content_config_alias(&content)?;
    let file_tools = FileSystemTools::try_new(config.file.safe_roots)?;
    let target = file_tools
        .write_file_grant_target(&target_path, content.as_bytes(), disposition)
        .map_err(|_| anyhow::anyhow!("write_file grant target validation failed"))?;
    target
        .ensure_distinct_source_identity(content.device, content.inode)
        .map_err(|_| {
            anyhow::anyhow!("write_file grant content file must not alias the replacement target")
        })?;
    let grant = authority
        .issue(&session_id, &target)
        .map_err(|_| anyhow::anyhow!("write_file grant issuance failed"))?;
    println!("{grant}");
    Ok(())
}

#[cfg(feature = "mcp-runtime")]
fn issue_copy_file_grant() -> anyhow::Result<()> {
    let config = load_offline_issuer_config()?;
    if !config.file.copy_file_mutation_enabled {
        anyhow::bail!("copy_file mutation gate is disabled");
    }
    let _ = validate_runtime_auth_posture(&config)?;
    let authority = configured_copy_file_authority(&config)?
        .ok_or_else(|| anyhow::anyhow!("copy_file mutation gate is disabled"))?;
    let session_id = required_grant_environment(CAPABILITY_SESSION_ENV)?;
    let source_path = required_grant_environment(CAPABILITY_COPY_FILE_SOURCE_ENV)?;
    let destination_path = required_grant_environment(CAPABILITY_COPY_FILE_DESTINATION_ENV)?;
    let file_tools = FileSystemTools::try_new(config.file.safe_roots)?;
    let target = file_tools
        .copy_file_grant_target(&source_path, &destination_path)
        .map_err(|_| anyhow::anyhow!("copy_file grant target validation failed"))?;
    let grant = authority
        .issue(&session_id, &target)
        .map_err(|_| anyhow::anyhow!("copy_file grant issuance failed"))?;
    println!("{grant}");
    Ok(())
}

#[cfg(feature = "mcp-runtime")]
struct PrivateWriteGrantContent {
    bytes: Vec<u8>,
    device: u64,
    inode: u64,
    _descriptor: File,
}

#[cfg(feature = "mcp-runtime")]
impl std::fmt::Debug for PrivateWriteGrantContent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PrivateWriteGrantContent")
            .field("content", &"<redacted>")
            .field("identity", &"<redacted>")
            .finish()
    }
}

#[cfg(feature = "mcp-runtime")]
impl PrivateWriteGrantContent {
    fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(feature = "mcp-runtime")]
fn read_write_grant_content(path: &Path) -> anyhow::Result<PrivateWriteGrantContent> {
    if !path.is_absolute() {
        anyhow::bail!("write_file grant content file must be absolute");
    }
    let descriptor = open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| anyhow::anyhow!("write_file grant content file could not be opened"))?;
    let opened = fstat(&descriptor)
        .map_err(|_| anyhow::anyhow!("write_file grant content file could not be inspected"))?;
    if !FileType::from_raw_mode(opened.st_mode).is_file() {
        anyhow::bail!("write_file grant content file must be a regular non-symlink file");
    }
    if opened.st_mode & 0o077 != 0 || opened.st_mode & 0o400 == 0 {
        anyhow::bail!(
            "write_file grant content file must be owner-readable and inaccessible to group/other"
        );
    }
    let configured_bytes = usize::try_from(opened.st_size)
        .map_err(|_| anyhow::anyhow!("write_file grant content file size is invalid"))?;
    if configured_bytes > DEFAULT_MAX_WRITE_BYTES {
        anyhow::bail!("write_file grant content file exceeds its byte limit");
    }

    let mut file = File::from(descriptor);
    let mut content = Vec::with_capacity(configured_bytes);
    (&mut file)
        .take((DEFAULT_MAX_WRITE_BYTES + 1) as u64)
        .read_to_end(&mut content)
        .map_err(|_| anyhow::anyhow!("write_file grant content file could not be read"))?;
    if content.len() > DEFAULT_MAX_WRITE_BYTES {
        anyhow::bail!("write_file grant content file exceeds its byte limit");
    }
    let read = fstat(&file)
        .map_err(|_| anyhow::anyhow!("write_file grant content file could not be inspected"))?;
    if !FileType::from_raw_mode(read.st_mode).is_file()
        || read.st_dev != opened.st_dev
        || read.st_ino != opened.st_ino
        || read.st_size != opened.st_size
        || read.st_mode & 0o7777 != opened.st_mode & 0o7777
        || read.st_ctime != opened.st_ctime
        || read.st_ctime_nsec != opened.st_ctime_nsec
        || read.st_mtime != opened.st_mtime
        || read.st_mtime_nsec != opened.st_mtime_nsec
        || usize::try_from(read.st_size).ok() != Some(content.len())
    {
        anyhow::bail!("write_file grant content file changed while it was read");
    }
    if std::str::from_utf8(&content).is_err() {
        anyhow::bail!("write_file grant content file must contain valid UTF-8");
    }
    Ok(PrivateWriteGrantContent {
        bytes: content,
        device: opened.st_dev,
        inode: opened.st_ino,
        _descriptor: file,
    })
}

#[cfg(feature = "mcp-runtime")]
fn reject_write_grant_content_config_alias(
    content: &PrivateWriteGrantContent,
) -> anyhow::Result<()> {
    let Some(config_path) = std::env::var_os(CAPABILITY_CONFIG_FILE_ENV) else {
        return Ok(());
    };
    if config_path.is_empty() {
        anyhow::bail!("offline issuer runtime configuration could not be revalidated");
    }
    let descriptor = open(
        Path::new(&config_path),
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| {
        anyhow::anyhow!("offline issuer runtime configuration could not be revalidated")
    })?;
    let metadata = fstat(&descriptor).map_err(|_| {
        anyhow::anyhow!("offline issuer runtime configuration could not be revalidated")
    })?;
    if metadata.st_dev == content.device && metadata.st_ino == content.inode {
        anyhow::bail!(
            "write_file grant content file must not alias the runtime configuration file"
        );
    }
    Ok(())
}

#[cfg(feature = "android-volume-control")]
fn issue_android_volume_grant() -> anyhow::Result<()> {
    let config = load_offline_issuer_config()?;
    if !config.android.volume_control_enabled {
        anyhow::bail!("Android volume control gate is disabled");
    }
    let _ = validate_runtime_auth_posture(&config)?;
    let authority = configured_android_volume_control_authority(&config)?
        .ok_or_else(|| anyhow::anyhow!("Android volume control gate is disabled"))?;
    let session_id = required_grant_environment(CAPABILITY_SESSION_ENV)?;
    let stream = required_grant_environment(CAPABILITY_VOLUME_STREAM_ENV)?
        .parse::<AndroidVolumeStreamName>()
        .map_err(|_| anyhow::anyhow!("Android volume grant stream validation failed"))?;
    let level = required_grant_environment(CAPABILITY_VOLUME_LEVEL_ENV)?
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("Android volume grant level validation failed"))?;
    let target = AndroidVolumeGrantTarget::new(stream, level)
        .map_err(|_| anyhow::anyhow!("Android volume grant target validation failed"))?;
    let now_unix_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| anyhow::anyhow!("system clock is before the Unix epoch"))?
        .as_secs();
    let grant = authority
        .issue_at(&session_id, target, now_unix_seconds)
        .map_err(|_| anyhow::anyhow!("Android volume grant issuance failed"))?;
    println!("{grant}");
    Ok(())
}

#[cfg(feature = "mcp-runtime")]
fn load_offline_issuer_config() -> anyhow::Result<AppConfig> {
    let config = match std::env::var_os(CAPABILITY_CONFIG_FILE_ENV) {
        None => AppConfig::load()?,
        Some(config_file) if config_file.is_empty() => {
            anyhow::bail!("{CAPABILITY_CONFIG_FILE_ENV} must not be empty")
        }
        Some(config_file) => AppConfig::load_from_literal_file(&PathBuf::from(config_file))?,
    };
    let posture = validate_runtime_auth_posture(&config)?;
    let _ = McpAuthPolicy::from_config(&config.auth, posture)?;
    Ok(config)
}

#[cfg(feature = "mcp-runtime")]
fn required_grant_environment(name: &str) -> anyhow::Result<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => anyhow::bail!("{name} must not be empty"),
        Err(std::env::VarError::NotPresent) => anyhow::bail!("{name} is required"),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("{name} must contain valid Unicode text")
        }
    }
}

fn verify_command_execution_boundary() -> anyhow::Result<()> {
    if std::env::vars_os().next().is_some() {
        anyhow::bail!("command execution boundary check failed");
    }

    let working_directory = std::env::current_dir()
        .map_err(|_| anyhow::anyhow!("command execution boundary check failed"))?;
    if !working_directory.is_absolute() || working_directory == std::path::Path::new("/") {
        anyhow::bail!("command execution boundary check failed");
    }

    let stdin_target = std::fs::read_link("/proc/self/fd/0")
        .map_err(|_| anyhow::anyhow!("command execution boundary check failed"))?;
    if stdin_target != std::path::Path::new("/dev/null") {
        anyhow::bail!("command execution boundary check failed");
    }

    Ok(())
}

async fn health_check() -> &'static str {
    "ok"
}

async fn readiness_check(State(readiness): State<ReadinessResponse>) -> Json<ReadinessResponse> {
    Json(readiness)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received");
}
