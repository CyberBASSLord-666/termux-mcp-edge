#!/usr/bin/env python3
from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}: {old[:160]!r}")
    path.write_text(text.replace(old, new, 1))


Path("src/directory_grant.rs").write_text(r'''//! Request-scoped `create_directory` grant transport and cryptographic verification.
//!
//! This module deliberately stops before mutation authorization. A verified grant
//! cannot open the runtime mutation gate until durable single-use replay consumption
//! is implemented and connected immediately before the filesystem mutation attempt.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fs::File,
    io::Read,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context};
use axum::http::HeaderMap;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use rustix::{
    fs::{self as descriptor_fs, FileType, Mode, OFlags},
    process,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

use crate::{auth::AuthenticatedPrincipal, config::DirectoryGrantConfig};

pub const MCP_DIRECTORY_GRANT_HEADER: &str = "mcp-directory-capability-grant";
pub const DIRECTORY_CREATE_CAPABILITY: &str = "filesystem.create-directory";
pub const DIRECTORY_CREATE_POSTURE: &str = "mutating";
pub const DIRECTORY_GRANT_FORMAT_VERSION: u8 = 1;
pub const DIRECTORY_GRANT_ALGORITHM: &str = "HS256";
pub const DIRECTORY_GRANT_TYPE: &str = "MCP-DIRECTORY-GRANT";
pub const MAX_DIRECTORY_GRANT_BYTES: usize = 8_192;
pub const MAX_DIRECTORY_GRANT_KEYS: usize = 16;
pub const MAX_DIRECTORY_GRANT_KEYRING_BYTES: usize = 65_536;
pub const MAX_DIRECTORY_GRANT_PATH_COMPONENTS: usize = 64;
pub const MAX_DIRECTORY_GRANT_PATH_COMPONENT_BYTES: usize = 255;
pub const DIRECTORY_GRANT_ID_BYTES: usize = 32;

const MAX_HEADER_SEGMENT_BYTES: usize = 512;
const MAX_CLAIMS_SEGMENT_BYTES: usize = 6_144;
const MAX_SIGNATURE_SEGMENT_BYTES: usize = 64;
const MIN_HMAC_KEY_BYTES: usize = 32;
const MAX_HMAC_KEY_BYTES: usize = 64;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, PartialEq, Eq)]
pub struct DirectoryGrantAuthorization {
    serialized: Arc<str>,
}

impl DirectoryGrantAuthorization {
    fn parse(value: &str) -> Result<Self, DirectoryGrantContextError> {
        if value.is_empty()
            || value.len() > MAX_DIRECTORY_GRANT_BYTES
            || value
                .bytes()
                .any(|byte| !(0x21..=0x7e).contains(&byte) || byte.is_ascii_whitespace())
        {
            return Err(DirectoryGrantContextError::Malformed);
        }
        Ok(Self {
            serialized: Arc::from(value),
        })
    }

    fn serialized(&self) -> &str {
        &self.serialized
    }
}

impl fmt::Debug for DirectoryGrantAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DirectoryGrantAuthorization(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryGrantContextError {
    Duplicate,
    Malformed,
}

pub fn has_directory_grant_header(headers: &HeaderMap) -> bool {
    headers.contains_key(MCP_DIRECTORY_GRANT_HEADER)
}

/// Remove the raw grant header before transport parsing or dispatch and retain
/// only an opaque redacted request extension.
pub fn take_directory_grant_authorization(
    headers: &mut HeaderMap,
) -> Result<Option<DirectoryGrantAuthorization>, DirectoryGrantContextError> {
    let count = headers
        .get_all(MCP_DIRECTORY_GRANT_HEADER)
        .iter()
        .count();
    if count == 0 {
        return Ok(None);
    }
    if count != 1 {
        headers.remove(MCP_DIRECTORY_GRANT_HEADER);
        return Err(DirectoryGrantContextError::Duplicate);
    }

    let value = headers
        .remove(MCP_DIRECTORY_GRANT_HEADER)
        .ok_or(DirectoryGrantContextError::Malformed)?;
    let value = value
        .to_str()
        .map_err(|_| DirectoryGrantContextError::Malformed)?;
    DirectoryGrantAuthorization::parse(value).map(Some)
}

#[derive(Clone)]
pub struct DirectoryGrantVerifier {
    inner: Arc<VerifierInner>,
    last_observed_unix_seconds: Arc<AtomicU64>,
}

struct VerifierInner {
    issuer: Arc<str>,
    audience: Arc<str>,
    safe_root_ids: BTreeSet<String>,
    keys: BTreeMap<String, VerificationKey>,
    max_lifetime_seconds: u64,
    clock_skew_seconds: u64,
}

struct VerificationKey {
    material: Zeroizing<Vec<u8>>,
    not_before_unix_seconds: u64,
    verify_until_unix_seconds: u64,
}

impl fmt::Debug for DirectoryGrantVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectoryGrantVerifier")
            .field("issuer", &"<redacted>")
            .field("audience", &"<redacted>")
            .field("safe_root_count", &self.inner.safe_root_ids.len())
            .field("verification_key_count", &self.inner.keys.len())
            .field("verification_keys", &"<redacted>")
            .field("max_lifetime_seconds", &self.inner.max_lifetime_seconds)
            .field("clock_skew_seconds", &self.inner.clock_skew_seconds)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryGrantVerificationError {
    Missing,
    Malformed,
    UnknownVersion,
    AlgorithmRejected,
    UnknownKey,
    SignatureRejected,
    IssuerMismatch,
    AudienceMismatch,
    PrincipalMismatch,
    SessionMismatch,
    CapabilityMismatch,
    RootMismatch,
    PathMismatch,
    PostureMismatch,
    FutureIssued,
    NotYetValid,
    Expired,
    ExcessiveLifetime,
    KeyWindowMismatch,
    ClockRollback,
}

impl DirectoryGrantVerificationError {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::Missing => "directory_grant_missing",
            Self::Malformed => "directory_grant_malformed",
            Self::UnknownVersion => "directory_grant_version_rejected",
            Self::AlgorithmRejected => "directory_grant_algorithm_rejected",
            Self::UnknownKey => "directory_grant_key_rejected",
            Self::SignatureRejected => "directory_grant_signature_rejected",
            Self::IssuerMismatch => "directory_grant_issuer_rejected",
            Self::AudienceMismatch => "directory_grant_audience_rejected",
            Self::PrincipalMismatch => "directory_grant_principal_rejected",
            Self::SessionMismatch => "directory_grant_session_rejected",
            Self::CapabilityMismatch => "directory_grant_capability_rejected",
            Self::RootMismatch => "directory_grant_root_rejected",
            Self::PathMismatch => "directory_grant_path_rejected",
            Self::PostureMismatch => "directory_grant_posture_rejected",
            Self::FutureIssued => "directory_grant_future_issued",
            Self::NotYetValid => "directory_grant_not_yet_valid",
            Self::Expired => "directory_grant_expired",
            Self::ExcessiveLifetime => "directory_grant_lifetime_rejected",
            Self::KeyWindowMismatch => "directory_grant_key_window_rejected",
            Self::ClockRollback => "directory_grant_clock_rollback",
        }
    }
}

impl fmt::Display for DirectoryGrantVerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("directory capability grant rejected")
    }
}

impl std::error::Error for DirectoryGrantVerificationError {}

pub struct DirectoryGrantBinding<'a> {
    pub principal: &'a AuthenticatedPrincipal,
    pub session_id: &'a str,
    pub safe_root_id: &'a str,
    pub target_components: &'a [String],
}

pub struct VerifiedDirectoryGrant {
    grant_identifier: [u8; DIRECTORY_GRANT_ID_BYTES],
    expires_unix_seconds: u64,
    format_version: u8,
    key_id: String,
}

impl fmt::Debug for VerifiedDirectoryGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedDirectoryGrant")
            .field("grant_identifier", &"<redacted>")
            .field("expires_unix_seconds", &self.expires_unix_seconds)
            .field("format_version", &self.format_version)
            .field("key_id", &"<redacted>")
            .finish()
    }
}

impl Drop for VerifiedDirectoryGrant {
    fn drop(&mut self) {
        self.grant_identifier.zeroize();
        self.key_id.zeroize();
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantHeader {
    v: u8,
    typ: String,
    alg: String,
    kid: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantClaims {
    v: u8,
    iss: String,
    aud: String,
    sub: String,
    sid: String,
    cap: String,
    root: String,
    path: Vec<String>,
    posture: String,
    jti: String,
    iat: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    nbf: Option<u64>,
    exp: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeyring {
    version: u8,
    keys: Vec<RawVerificationKey>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVerificationKey {
    kid: String,
    alg: String,
    key_b64url: String,
    not_before_unix_seconds: u64,
    verify_until_unix_seconds: u64,
}

impl DirectoryGrantVerifier {
    pub fn load_optional(
        config: &DirectoryGrantConfig,
        authentication_token: Option<&str>,
    ) -> anyhow::Result<Option<Self>> {
        if !config.verification_enabled {
            return Ok(None);
        }

        let now = current_unix_seconds()?;
        let issuer = config
            .issuer
            .as_deref()
            .ok_or_else(|| anyhow!("directory grant verification configuration is incomplete"))?;
        let audience = config
            .audience
            .as_deref()
            .ok_or_else(|| anyhow!("directory grant verification configuration is incomplete"))?;
        let path = config
            .keyring_path
            .as_deref()
            .ok_or_else(|| anyhow!("directory grant verification configuration is incomplete"))?;
        let safe_root_ids = config
            .safe_root_ids
            .as_ref()
            .ok_or_else(|| anyhow!("directory grant verification configuration is incomplete"))?;
        let keys = load_keyring(
            path,
            authentication_token,
            now,
            config.max_lifetime_seconds,
            config.clock_skew_seconds,
        )?;

        Ok(Some(Self {
            inner: Arc::new(VerifierInner {
                issuer: Arc::from(issuer),
                audience: Arc::from(audience),
                safe_root_ids: safe_root_ids.iter().cloned().collect(),
                keys,
                max_lifetime_seconds: config.max_lifetime_seconds,
                clock_skew_seconds: config.clock_skew_seconds,
            }),
            last_observed_unix_seconds: Arc::new(AtomicU64::new(now)),
        }))
    }

    pub fn verify(
        &self,
        authorization: Option<&DirectoryGrantAuthorization>,
        binding: &DirectoryGrantBinding<'_>,
        now_unix_seconds: u64,
    ) -> Result<VerifiedDirectoryGrant, DirectoryGrantVerificationError> {
        self.observe_clock(now_unix_seconds)?;
        let authorization = authorization.ok_or(DirectoryGrantVerificationError::Missing)?;
        validate_binding(binding)?;

        let mut segments = authorization.serialized().split('.');
        let header_segment = segments
            .next()
            .ok_or(DirectoryGrantVerificationError::Malformed)?;
        let claims_segment = segments
            .next()
            .ok_or(DirectoryGrantVerificationError::Malformed)?;
        let signature_segment = segments
            .next()
            .ok_or(DirectoryGrantVerificationError::Malformed)?;
        if segments.next().is_some() {
            return Err(DirectoryGrantVerificationError::Malformed);
        }
        if header_segment.len() > MAX_HEADER_SEGMENT_BYTES
            || claims_segment.len() > MAX_CLAIMS_SEGMENT_BYTES
            || signature_segment.len() > MAX_SIGNATURE_SEGMENT_BYTES
        {
            return Err(DirectoryGrantVerificationError::Malformed);
        }

        let header: GrantHeader = decode_canonical_json(header_segment)?;
        if header.v != DIRECTORY_GRANT_FORMAT_VERSION {
            return Err(DirectoryGrantVerificationError::UnknownVersion);
        }
        if header.typ != DIRECTORY_GRANT_TYPE || header.alg != DIRECTORY_GRANT_ALGORITHM {
            return Err(DirectoryGrantVerificationError::AlgorithmRejected);
        }
        validate_key_id(&header.kid).map_err(|_| DirectoryGrantVerificationError::Malformed)?;
        let key = self
            .inner
            .keys
            .get(&header.kid)
            .ok_or(DirectoryGrantVerificationError::UnknownKey)?;

        let signature = decode_canonical_base64(signature_segment, 32)?;
        if signature.len() != 32 {
            return Err(DirectoryGrantVerificationError::Malformed);
        }
        let signing_input = format!("{header_segment}.{claims_segment}");
        let mut mac = HmacSha256::new_from_slice(&key.material)
            .map_err(|_| DirectoryGrantVerificationError::SignatureRejected)?;
        mac.update(signing_input.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| DirectoryGrantVerificationError::SignatureRejected)?;

        let claims: GrantClaims = decode_canonical_json(claims_segment)?;
        self.verify_claims(&header, &claims, key, binding, now_unix_seconds)?;
        let grant_identifier = decode_grant_identifier(&claims.jti)?;

        Ok(VerifiedDirectoryGrant {
            grant_identifier,
            expires_unix_seconds: claims.exp,
            format_version: claims.v,
            key_id: header.kid,
        })
    }

    fn verify_claims(
        &self,
        header: &GrantHeader,
        claims: &GrantClaims,
        key: &VerificationKey,
        binding: &DirectoryGrantBinding<'_>,
        now: u64,
    ) -> Result<(), DirectoryGrantVerificationError> {
        let skew = self.inner.clock_skew_seconds;
        if claims.v != DIRECTORY_GRANT_FORMAT_VERSION {
            return Err(DirectoryGrantVerificationError::UnknownVersion);
        }
        if claims.iss != self.inner.issuer.as_ref() {
            return Err(DirectoryGrantVerificationError::IssuerMismatch);
        }
        if claims.aud != self.inner.audience.as_ref() {
            return Err(DirectoryGrantVerificationError::AudienceMismatch);
        }
        if claims.sub != binding.principal.stable_id() {
            return Err(DirectoryGrantVerificationError::PrincipalMismatch);
        }
        if claims.sid != binding.session_id {
            return Err(DirectoryGrantVerificationError::SessionMismatch);
        }
        if claims.cap != DIRECTORY_CREATE_CAPABILITY {
            return Err(DirectoryGrantVerificationError::CapabilityMismatch);
        }
        if !self.inner.safe_root_ids.contains(binding.safe_root_id)
            || claims.root != binding.safe_root_id
        {
            return Err(DirectoryGrantVerificationError::RootMismatch);
        }
        if claims.path != binding.target_components {
            return Err(DirectoryGrantVerificationError::PathMismatch);
        }
        if claims.posture != DIRECTORY_CREATE_POSTURE {
            return Err(DirectoryGrantVerificationError::PostureMismatch);
        }
        validate_claim_text(&claims.iss, 256)?;
        validate_claim_text(&claims.aud, 256)?;
        validate_claim_text(&claims.sub, 128)?;
        validate_session_id(&claims.sid)?;
        validate_safe_root_id(&claims.root)?;
        validate_path_components(&claims.path)?;
        let _ = decode_grant_identifier(&claims.jti)?;

        if claims.iat > now.saturating_add(skew) {
            return Err(DirectoryGrantVerificationError::FutureIssued);
        }
        let not_before = claims.nbf.unwrap_or(claims.iat);
        if not_before > now.saturating_add(skew) {
            return Err(DirectoryGrantVerificationError::NotYetValid);
        }
        if claims.exp <= claims.iat
            || claims.exp.saturating_sub(claims.iat) > self.inner.max_lifetime_seconds
        {
            return Err(DirectoryGrantVerificationError::ExcessiveLifetime);
        }
        if not_before > claims.exp || not_before.saturating_add(skew) < claims.iat {
            return Err(DirectoryGrantVerificationError::Malformed);
        }
        if now >= claims.exp.saturating_add(skew) {
            return Err(DirectoryGrantVerificationError::Expired);
        }
        if claims.iat.saturating_add(skew) < key.not_before_unix_seconds
            || claims.exp > key.verify_until_unix_seconds
        {
            return Err(DirectoryGrantVerificationError::KeyWindowMismatch);
        }
        if header.kid.is_empty() {
            return Err(DirectoryGrantVerificationError::UnknownKey);
        }
        Ok(())
    }

    fn observe_clock(&self, now: u64) -> Result<(), DirectoryGrantVerificationError> {
        let mut previous = self.last_observed_unix_seconds.load(Ordering::Acquire);
        loop {
            if now < previous {
                return Err(DirectoryGrantVerificationError::ClockRollback);
            }
            if now == previous {
                return Ok(());
            }
            match self.last_observed_unix_seconds.compare_exchange_weak(
                previous,
                now,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(observed) => previous = observed,
            }
        }
    }
}

fn load_keyring(
    path: &std::path::Path,
    authentication_token: Option<&str>,
    now: u64,
    max_lifetime_seconds: u64,
    clock_skew_seconds: u64,
) -> anyhow::Result<BTreeMap<String, VerificationKey>> {
    let fd = descriptor_fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| anyhow!("directory grant keyring could not be opened safely"))?;
    let metadata = descriptor_fs::fstat(&fd)
        .map_err(|_| anyhow!("directory grant keyring metadata could not be verified"))?;
    if !FileType::from_raw_mode(metadata.st_mode).is_file() {
        bail!("directory grant keyring must be a regular file");
    }
    if metadata.st_uid != process::geteuid().as_raw() {
        bail!("directory grant keyring must be owned by the effective server user");
    }
    if metadata.st_mode & 0o077 != 0 {
        bail!("directory grant keyring permissions must exclude group and other access");
    }
    if metadata.st_size < 0 || metadata.st_size as usize > MAX_DIRECTORY_GRANT_KEYRING_BYTES {
        bail!("directory grant keyring exceeds its fixed byte limit");
    }

    let mut bytes = Zeroizing::new(Vec::with_capacity(metadata.st_size as usize));
    File::from(fd)
        .take((MAX_DIRECTORY_GRANT_KEYRING_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .context("directory grant keyring could not be read")?;
    if bytes.len() > MAX_DIRECTORY_GRANT_KEYRING_BYTES {
        bail!("directory grant keyring exceeds its fixed byte limit");
    }
    let raw: RawKeyring = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("directory grant keyring is malformed"))?;
    if raw.version != DIRECTORY_GRANT_FORMAT_VERSION {
        bail!("directory grant keyring version is unsupported");
    }
    if raw.keys.is_empty() || raw.keys.len() > MAX_DIRECTORY_GRANT_KEYS {
        bail!("directory grant keyring key count is invalid");
    }

    let required_window_end = now
        .checked_add(max_lifetime_seconds)
        .and_then(|value| value.checked_add(clock_skew_seconds))
        .ok_or_else(|| anyhow!("directory grant key validity window overflow"))?;
    let mut active_key_available = false;
    let mut keys = BTreeMap::new();
    for raw_key in raw.keys {
        validate_key_id(&raw_key.kid)?;
        if raw_key.alg != DIRECTORY_GRANT_ALGORITHM {
            bail!("directory grant keyring contains a disallowed algorithm");
        }
        if raw_key.not_before_unix_seconds >= raw_key.verify_until_unix_seconds {
            bail!("directory grant keyring contains an invalid verification window");
        }

        let mut encoded = raw_key.key_b64url;
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded.as_bytes())
            .map_err(|_| anyhow!("directory grant keyring contains malformed key material"))?;
        if URL_SAFE_NO_PAD.encode(&decoded) != encoded {
            encoded.zeroize();
            bail!("directory grant keyring contains non-canonical key material");
        }
        encoded.zeroize();
        if !(MIN_HMAC_KEY_BYTES..=MAX_HMAC_KEY_BYTES).contains(&decoded.len()) {
            bail!("directory grant keyring contains invalid key material length");
        }
        if authentication_token.is_some_and(|token| decoded.as_slice() == token.as_bytes()) {
            bail!("directory grant verification keys must not reuse authentication credentials");
        }
        if raw_key.not_before_unix_seconds <= now.saturating_add(clock_skew_seconds)
            && raw_key.verify_until_unix_seconds >= required_window_end
        {
            active_key_available = true;
        }

        let key = VerificationKey {
            material: Zeroizing::new(decoded),
            not_before_unix_seconds: raw_key.not_before_unix_seconds,
            verify_until_unix_seconds: raw_key.verify_until_unix_seconds,
        };
        if keys.insert(raw_key.kid, key).is_some() {
            bail!("directory grant keyring contains duplicate key identifiers");
        }
    }
    if !active_key_available {
        bail!("directory grant keyring has no active key covering the maximum grant lifetime");
    }
    Ok(keys)
}

fn decode_canonical_json<T>(segment: &str) -> Result<T, DirectoryGrantVerificationError>
where
    T: DeserializeOwned + Serialize,
{
    let bytes = decode_canonical_base64(segment, MAX_CLAIMS_SEGMENT_BYTES)?;
    let value: T = serde_json::from_slice(&bytes)
        .map_err(|_| DirectoryGrantVerificationError::Malformed)?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|_| DirectoryGrantVerificationError::Malformed)?;
    if canonical != bytes {
        return Err(DirectoryGrantVerificationError::Malformed);
    }
    Ok(value)
}

fn decode_canonical_base64(
    segment: &str,
    max_decoded_bytes: usize,
) -> Result<Vec<u8>, DirectoryGrantVerificationError> {
    if segment.is_empty() || segment.len() > MAX_DIRECTORY_GRANT_BYTES {
        return Err(DirectoryGrantVerificationError::Malformed);
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(segment.as_bytes())
        .map_err(|_| DirectoryGrantVerificationError::Malformed)?;
    if decoded.len() > max_decoded_bytes || URL_SAFE_NO_PAD.encode(&decoded) != segment {
        return Err(DirectoryGrantVerificationError::Malformed);
    }
    Ok(decoded)
}

fn decode_grant_identifier(
    value: &str,
) -> Result<[u8; DIRECTORY_GRANT_ID_BYTES], DirectoryGrantVerificationError> {
    let decoded = decode_canonical_base64(value, DIRECTORY_GRANT_ID_BYTES)?;
    decoded
        .try_into()
        .map_err(|_| DirectoryGrantVerificationError::Malformed)
}

fn validate_binding(
    binding: &DirectoryGrantBinding<'_>,
) -> Result<(), DirectoryGrantVerificationError> {
    validate_session_id(binding.session_id)?;
    validate_safe_root_id(binding.safe_root_id)?;
    validate_path_components(binding.target_components)
}

fn validate_session_id(value: &str) -> Result<(), DirectoryGrantVerificationError> {
    let parsed = uuid::Uuid::parse_str(value).map_err(|_| DirectoryGrantVerificationError::Malformed)?;
    if parsed.to_string() != value {
        return Err(DirectoryGrantVerificationError::Malformed);
    }
    Ok(())
}

fn validate_safe_root_id(value: &str) -> Result<(), DirectoryGrantVerificationError> {
    validate_label(value, 64).map_err(|_| DirectoryGrantVerificationError::Malformed)
}

fn validate_path_components(
    components: &[String],
) -> Result<(), DirectoryGrantVerificationError> {
    if components.is_empty() || components.len() > MAX_DIRECTORY_GRANT_PATH_COMPONENTS {
        return Err(DirectoryGrantVerificationError::Malformed);
    }
    for component in components {
        if component.is_empty()
            || component.len() > MAX_DIRECTORY_GRANT_PATH_COMPONENT_BYTES
            || matches!(component.as_str(), "." | "..")
            || component
                .chars()
                .any(|character| matches!(character, '\0' | '/' | '\\'))
        {
            return Err(DirectoryGrantVerificationError::Malformed);
        }
    }
    Ok(())
}

fn validate_claim_text(
    value: &str,
    max_bytes: usize,
) -> Result<(), DirectoryGrantVerificationError> {
    if value.is_empty()
        || value.len() > max_bytes
        || value.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
    {
        return Err(DirectoryGrantVerificationError::Malformed);
    }
    Ok(())
}

fn validate_key_id(value: &str) -> anyhow::Result<()> {
    validate_label(value, 64)
}

fn validate_label(value: &str, max_bytes: usize) -> anyhow::Result<()> {
    if value.is_empty() || value.len() > max_bytes {
        bail!("bounded identifier is invalid");
    }
    let mut previous_separator = false;
    for (index, byte) in value.bytes().enumerate() {
        let separator = matches!(byte, b'-' | b'_' | b':' | b'.');
        if !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || separator)
            || (separator
                && (index == 0 || index + 1 == value.len() || previous_separator))
        {
            bail!("bounded identifier is invalid");
        }
        previous_separator = separator;
    }
    Ok(())
}

fn current_unix_seconds() -> anyhow::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| anyhow!("system clock is before the Unix epoch"))
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    use super::*;

    const NOW: u64 = 1_800_000_000;
    const KEY_ID: &str = "grant-key-01";
    const ISSUER: &str = "termux-grant-authority:v1";
    const AUDIENCE: &str = "termux-mcp-edge:v1";
    const ROOT_ID: &str = "primary-root";
    const SESSION_ID: &str = "11111111-2222-4333-8444-555555555555";
    const PRINCIPAL: &str = "operator.primary:v1";

    fn write_keyring(path: &Path, key: &[u8], mode: u32, duplicate: bool) {
        let entry = serde_json::json!({
            "kid": KEY_ID,
            "alg": DIRECTORY_GRANT_ALGORITHM,
            "key_b64url": URL_SAFE_NO_PAD.encode(key),
            "not_before_unix_seconds": NOW - 60,
            "verify_until_unix_seconds": NOW + 600,
        });
        let keys = if duplicate {
            vec![entry.clone(), entry]
        } else {
            vec![entry]
        };
        fs::write(
            path,
            serde_json::to_vec(&serde_json::json!({"version": 1, "keys": keys})).unwrap(),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    fn config(path: &Path) -> DirectoryGrantConfig {
        DirectoryGrantConfig {
            verification_enabled: true,
            issuer: Some(ISSUER.to_owned()),
            audience: Some(AUDIENCE.to_owned()),
            keyring_path: Some(path.to_path_buf()),
            safe_root_ids: Some(vec![ROOT_ID.to_owned()]),
            max_lifetime_seconds: 120,
            clock_skew_seconds: 5,
        }
    }

    fn verifier(path: &Path, key: &[u8]) -> DirectoryGrantVerifier {
        write_keyring(path, key, 0o600, false);
        DirectoryGrantVerifier::load_optional(&config(path), Some("authentication-token"))
            .unwrap()
            .unwrap()
    }

    fn claims() -> GrantClaims {
        GrantClaims {
            v: DIRECTORY_GRANT_FORMAT_VERSION,
            iss: ISSUER.to_owned(),
            aud: AUDIENCE.to_owned(),
            sub: PRINCIPAL.to_owned(),
            sid: SESSION_ID.to_owned(),
            cap: DIRECTORY_CREATE_CAPABILITY.to_owned(),
            root: ROOT_ID.to_owned(),
            path: vec!["projects".to_owned(), "alpha".to_owned()],
            posture: DIRECTORY_CREATE_POSTURE.to_owned(),
            jti: URL_SAFE_NO_PAD.encode([7_u8; DIRECTORY_GRANT_ID_BYTES]),
            iat: NOW - 1,
            nbf: Some(NOW - 1),
            exp: NOW + 60,
        }
    }

    fn mint(key: &[u8], header: GrantHeader, claims: GrantClaims) -> DirectoryGrantAuthorization {
        let header_segment = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
        let claims_segment = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        let signing_input = format!("{header_segment}.{claims_segment}");
        let mut mac = HmacSha256::new_from_slice(key).unwrap();
        mac.update(signing_input.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        DirectoryGrantAuthorization::parse(&format!(
            "{header_segment}.{claims_segment}.{signature}"
        ))
        .unwrap()
    }

    fn binding(principal: &AuthenticatedPrincipal) -> DirectoryGrantBinding<'_> {
        DirectoryGrantBinding {
            principal,
            session_id: SESSION_ID,
            safe_root_id: ROOT_ID,
            target_components: &["projects".to_owned(), "alpha".to_owned()],
        }
    }

    fn valid_header() -> GrantHeader {
        GrantHeader {
            v: DIRECTORY_GRANT_FORMAT_VERSION,
            typ: DIRECTORY_GRANT_TYPE.to_owned(),
            alg: DIRECTORY_GRANT_ALGORITHM.to_owned(),
            kid: KEY_ID.to_owned(),
        }
    }

    #[test]
    fn verifies_exact_canonical_request_binding_without_exposing_identifier() {
        let directory = tempfile::tempdir().unwrap();
        let key = [0x42_u8; 32];
        let verifier = verifier(&directory.path().join("keyring.json"), &key);
        let principal = AuthenticatedPrincipal::configured(PRINCIPAL).unwrap();
        let grant = mint(&key, valid_header(), claims());

        let verified = verifier.verify(Some(&grant), &binding(&principal), NOW).unwrap();
        let debug = format!("{verified:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(&URL_SAFE_NO_PAD.encode([7_u8; 32])));
        assert!(!format!("{verifier:?}").contains(&URL_SAFE_NO_PAD.encode(key)));
    }

    #[test]
    fn rejects_signature_algorithm_key_issuer_audience_and_binding_mismatches() {
        let directory = tempfile::tempdir().unwrap();
        let key = [0x43_u8; 32];
        let verifier = verifier(&directory.path().join("keyring.json"), &key);
        let principal = AuthenticatedPrincipal::configured(PRINCIPAL).unwrap();

        let mut bad_signature = mint(&[0x44_u8; 32], valid_header(), claims());
        assert_eq!(
            verifier.verify(Some(&bad_signature), &binding(&principal), NOW),
            Err(DirectoryGrantVerificationError::SignatureRejected)
        );
        bad_signature.serialized = Arc::from("bad");
        assert_eq!(
            verifier.verify(Some(&bad_signature), &binding(&principal), NOW),
            Err(DirectoryGrantVerificationError::Malformed)
        );

        let mut header = valid_header();
        header.alg = "none".to_owned();
        assert_eq!(
            verifier.verify(Some(&mint(&key, header, claims())), &binding(&principal), NOW),
            Err(DirectoryGrantVerificationError::AlgorithmRejected)
        );
        let mut header = valid_header();
        header.kid = "unknown-key".to_owned();
        assert_eq!(
            verifier.verify(Some(&mint(&key, header, claims())), &binding(&principal), NOW),
            Err(DirectoryGrantVerificationError::UnknownKey)
        );

        for (mut claims, expected) in [
            ({ let mut c = claims(); c.iss = "other-issuer".to_owned(); c }, DirectoryGrantVerificationError::IssuerMismatch),
            ({ let mut c = claims(); c.aud = "other-audience".to_owned(); c }, DirectoryGrantVerificationError::AudienceMismatch),
            ({ let mut c = claims(); c.sub = "operator.other:v1".to_owned(); c }, DirectoryGrantVerificationError::PrincipalMismatch),
            ({ let mut c = claims(); c.sid = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_owned(); c }, DirectoryGrantVerificationError::SessionMismatch),
            ({ let mut c = claims(); c.cap = "filesystem.write-file".to_owned(); c }, DirectoryGrantVerificationError::CapabilityMismatch),
            ({ let mut c = claims(); c.root = "other-root".to_owned(); c }, DirectoryGrantVerificationError::RootMismatch),
            ({ let mut c = claims(); c.path = vec!["projects".to_owned(), "beta".to_owned()]; c }, DirectoryGrantVerificationError::PathMismatch),
            ({ let mut c = claims(); c.posture = "dry-run".to_owned(); c }, DirectoryGrantVerificationError::PostureMismatch),
        ] {
            let grant = mint(&key, valid_header(), claims);
            assert_eq!(verifier.verify(Some(&grant), &binding(&principal), NOW), Err(expected));
        }
    }

    #[test]
    fn rejects_time_window_and_clock_rollback_failures() {
        let directory = tempfile::tempdir().unwrap();
        let key = [0x45_u8; 32];
        let verifier = verifier(&directory.path().join("keyring.json"), &key);
        let principal = AuthenticatedPrincipal::configured(PRINCIPAL).unwrap();

        let mut future = claims();
        future.iat = NOW + 10;
        future.nbf = Some(NOW + 10);
        future.exp = NOW + 70;
        assert_eq!(
            verifier.verify(Some(&mint(&key, valid_header(), future)), &binding(&principal), NOW),
            Err(DirectoryGrantVerificationError::FutureIssued)
        );

        let mut not_yet = claims();
        not_yet.nbf = Some(NOW + 10);
        assert_eq!(
            verifier.verify(Some(&mint(&key, valid_header(), not_yet)), &binding(&principal), NOW),
            Err(DirectoryGrantVerificationError::NotYetValid)
        );

        let mut expired = claims();
        expired.iat = NOW - 100;
        expired.nbf = Some(NOW - 100);
        expired.exp = NOW - 6;
        assert_eq!(
            verifier.verify(Some(&mint(&key, valid_header(), expired)), &binding(&principal), NOW),
            Err(DirectoryGrantVerificationError::Expired)
        );

        let mut excessive = claims();
        excessive.exp = NOW + 121;
        assert_eq!(
            verifier.verify(Some(&mint(&key, valid_header(), excessive)), &binding(&principal), NOW),
            Err(DirectoryGrantVerificationError::ExcessiveLifetime)
        );

        let valid = mint(&key, valid_header(), claims());
        verifier.verify(Some(&valid), &binding(&principal), NOW + 1).unwrap();
        assert_eq!(
            verifier.verify(Some(&valid), &binding(&principal), NOW),
            Err(DirectoryGrantVerificationError::ClockRollback)
        );
    }

    #[test]
    fn keyring_is_owner_only_bounded_unique_and_separate_from_authentication() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keyring.json");
        let key = [0x46_u8; 32];

        write_keyring(&path, &key, 0o644, false);
        assert!(DirectoryGrantVerifier::load_optional(&config(&path), Some("auth-token")).is_err());

        write_keyring(&path, &key, 0o600, true);
        assert!(DirectoryGrantVerifier::load_optional(&config(&path), Some("auth-token")).is_err());

        let auth_token = "a".repeat(32);
        write_keyring(&path, auth_token.as_bytes(), 0o600, false);
        let error = DirectoryGrantVerifier::load_optional(&config(&path), Some(&auth_token))
            .unwrap_err();
        assert!(error.to_string().contains("must not reuse"));
        assert!(!error.to_string().contains(&auth_token));

        let target = directory.path().join("target.json");
        write_keyring(&target, &key, 0o600, false);
        let link = directory.path().join("link.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(DirectoryGrantVerifier::load_optional(&config(&link), Some("auth-token")).is_err());
    }

    #[test]
    fn raw_header_is_removed_and_duplicate_or_malformed_context_is_rejected() {
        let key = [0x47_u8; 32];
        let grant = mint(&key, valid_header(), claims());
        let mut headers = HeaderMap::new();
        headers.insert(MCP_DIRECTORY_GRANT_HEADER, grant.serialized().parse().unwrap());
        let extracted = take_directory_grant_authorization(&mut headers)
            .unwrap()
            .unwrap();
        assert_eq!(extracted, grant);
        assert!(!headers.contains_key(MCP_DIRECTORY_GRANT_HEADER));
        assert!(format!("{extracted:?}").contains("<redacted>"));

        let mut duplicate = HeaderMap::new();
        duplicate.append(MCP_DIRECTORY_GRANT_HEADER, "one".parse().unwrap());
        duplicate.append(MCP_DIRECTORY_GRANT_HEADER, "two".parse().unwrap());
        assert_eq!(
            take_directory_grant_authorization(&mut duplicate),
            Err(DirectoryGrantContextError::Duplicate)
        );
        assert!(!duplicate.contains_key(MCP_DIRECTORY_GRANT_HEADER));

        let mut malformed = HeaderMap::new();
        malformed.insert(MCP_DIRECTORY_GRANT_HEADER, "contains whitespace".parse().unwrap());
        assert_eq!(
            take_directory_grant_authorization(&mut malformed),
            Err(DirectoryGrantContextError::Malformed)
        );
        assert!(!malformed.contains_key(MCP_DIRECTORY_GRANT_HEADER));
    }
}
''')

cargo = Path("Cargo.toml")
replace_once(
    cargo,
    'mcp-runtime = ["dep:rustix"]\n',
    'mcp-runtime = ["dep:base64", "dep:hmac", "dep:rustix", "dep:sha2", "dep:zeroize"]\n',
)
replace_once(
    cargo,
    'metrics = "0.24"\nrustix = { version = "1", features = ["fs"], optional = true }\n',
    'metrics = "0.24"\nbase64 = { version = "0.22", optional = true }\nhmac = { version = "0.12", optional = true }\nrustix = { version = "1", features = ["fs", "process"], optional = true }\nsha2 = { version = "0.10", optional = true }\nzeroize = { version = "1", optional = true }\n',
)

lib = Path("src/lib.rs")
replace_once(
    lib,
    'pub mod config;\npub mod error;\n',
    'pub mod config;\n#[cfg(feature = "mcp-runtime")]\npub mod directory_grant;\npub mod error;\n',
)

auth = Path("src/auth.rs")
replace_once(
    auth,
    'use crate::config::{AuthConfig, AuthPosture};\n',
    'use crate::{\n    config::{AuthConfig, AuthPosture},\n    directory_grant::{\n        has_directory_grant_header, take_directory_grant_authorization,\n        DirectoryGrantAuthorization,\n    },\n};\n',
)
replace_once(
    auth,
    '''    match &policy {
        McpAuthPolicy::UnauthenticatedLocalhostOnly => next.run(request).await,
        McpAuthPolicy::StaticBearer { token, principal } => {
''',
    '''    match &policy {
        McpAuthPolicy::UnauthenticatedLocalhostOnly => {
            if has_directory_grant_header(request.headers()) {
                request.headers_mut().remove(crate::directory_grant::MCP_DIRECTORY_GRANT_HEADER);
                return authorization_context_response(
                    StatusCode::FORBIDDEN,
                    "authorization_context_requires_authentication",
                    "Capability-grant authorization context requires authenticated transport.",
                );
            }
            next.run(request).await
        }
        McpAuthPolicy::StaticBearer { token, principal } => {
''',
)
replace_once(
    auth,
    '''            if authorized {
                if let Some(principal) = principal {
                    request.extensions_mut().insert(principal.clone());
                }
                next.run(request).await
            } else {
''',
    '''            if authorized {
                let grant = match take_directory_grant_authorization(request.headers_mut()) {
                    Ok(grant) => grant,
                    Err(_) => {
                        return authorization_context_response(
                            StatusCode::BAD_REQUEST,
                            "invalid_authorization_context",
                            "Capability-grant authorization context is malformed.",
                        );
                    }
                };
                if let Some(principal) = principal {
                    request.extensions_mut().insert(principal.clone());
                }
                if let Some(grant) = grant {
                    request.extensions_mut().insert(grant);
                }
                next.run(request).await
            } else {
''',
)
replace_once(
    auth,
    '''fn unauthorized_response() -> Response {
''',
    '''fn authorization_context_response(
    status: StatusCode,
    error: &'static str,
    message: &'static str,
) -> Response {
    let mut response = (status, Json(json!({"error": error, "message": message}))).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn unauthorized_response() -> Response {
''',
)
# Ensure imported grant extension type is used in production code rather than only tests.
replace_once(
    auth,
    '                if let Some(grant) = grant {\n                    request.extensions_mut().insert(grant);\n                }\n',
    '                if let Some(grant): Option<DirectoryGrantAuthorization> = grant {\n                    request.extensions_mut().insert(grant);\n                }\n',
)
# Add focused middleware tests before the fixed-work comparison test.
replace_once(
    auth,
    '''    #[test]
    fn fixed_work_comparison_matches_only_equal_tokens() {
''',
    '''    #[tokio::test]
    async fn grant_context_requires_authentication_and_is_removed_before_dispatch() {
        use crate::directory_grant::{
            DirectoryGrantAuthorization, MCP_DIRECTORY_GRANT_HEADER,
        };

        async fn grant_status(
            grant: Option<Extension<DirectoryGrantAuthorization>>,
            headers: HeaderMap,
        ) -> &'static str {
            match (
                grant.is_some(),
                headers.contains_key(MCP_DIRECTORY_GRANT_HEADER),
            ) {
                (true, false) => "opaque-grant-only",
                _ => "grant-boundary-failed",
            }
        }

        let app = Router::new()
            .route("/mcp", post(grant_status))
            .route_layer(middleware::from_fn_with_state(
                McpAuthPolicy::static_bearer_for_principal(
                    "expected-value",
                    "operator.primary:v1",
                )
                .unwrap(),
                require_mcp_auth,
            ));
        let response = app
            .oneshot(
                HttpRequest::post("/mcp")
                    .header(header::AUTHORIZATION, "Bearer expected-value")
                    .header(MCP_DIRECTORY_GRANT_HEADER, "canonical.grant.value")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(text_body(response).await, "opaque-grant-only");

        let unauthenticated = Router::new()
            .route("/mcp", post(grant_status))
            .route_layer(middleware::from_fn_with_state(
                McpAuthPolicy::unauthenticated_localhost_only(),
                require_mcp_auth,
            ));
        let response = unauthenticated
            .oneshot(
                HttpRequest::post("/mcp")
                    .header(MCP_DIRECTORY_GRANT_HEADER, "canonical.grant.value")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn bearer_authentication_precedes_grant_context_validation() {
        use crate::directory_grant::MCP_DIRECTORY_GRANT_HEADER;

        let response = call(
            McpAuthPolicy::static_bearer("expected-value").unwrap(),
            Some("Bearer wrong-value"),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let app = Router::new()
            .route("/mcp", post(principal_status))
            .route_layer(middleware::from_fn_with_state(
                McpAuthPolicy::static_bearer("expected-value").unwrap(),
                require_mcp_auth,
            ));
        let response = app
            .oneshot(
                HttpRequest::post("/mcp")
                    .header(header::AUTHORIZATION, "Bearer wrong-value")
                    .header(MCP_DIRECTORY_GRANT_HEADER, "contains whitespace")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn fixed_work_comparison_matches_only_equal_tokens() {
''',
)

# Principal binding must be available internally to the verifier but never through Debug/serialization.
replace_once(
    auth,
    '''    pub fn configured(stable_id: impl AsRef<str>) -> anyhow::Result<Self> {
        let stable_id = stable_id.as_ref();
        validate_principal_id(stable_id)?;
        Ok(Self {
            stable_id: Arc::from(stable_id),
        })
    }
''',
    '''    pub fn configured(stable_id: impl AsRef<str>) -> anyhow::Result<Self> {
        let stable_id = stable_id.as_ref();
        validate_principal_id(stable_id)?;
        Ok(Self {
            stable_id: Arc::from(stable_id),
        })
    }

    pub(crate) fn stable_id(&self) -> &str {
        &self.stable_id
    }
''',
)

config = Path("src/config.rs")
replace_once(
    config,
    '''    pub command: CommandConfig,
    pub file: FileConfig,
    pub transport: TransportConfig,
''',
    '''    pub command: CommandConfig,
    pub file: FileConfig,
    pub directory_grant: DirectoryGrantConfig,
    pub transport: TransportConfig,
''',
)
replace_once(
    config,
    '''#[derive(Debug, Clone, Copy)]
pub struct AndroidConfig {
''',
    '''#[derive(Clone)]
pub struct DirectoryGrantConfig {
    pub verification_enabled: bool,
    pub issuer: Option<String>,
    pub audience: Option<String>,
    pub keyring_path: Option<PathBuf>,
    pub safe_root_ids: Option<Vec<String>>,
    pub max_lifetime_seconds: u64,
    pub clock_skew_seconds: u64,
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
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AndroidConfig {
''',
)
replace_once(
    config,
    '''            file: FileConfig {
                safe_roots: env_path_list(
                    &read_variable,
                    "MCP__FILE__SAFE_ROOTS",
                    &[DEFAULT_FILE_SAFE_ROOT],
                )?,
            },
            transport: TransportConfig {
''',
    '''            file: FileConfig {
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
                audience: optional_env_string(
                    &read_variable,
                    "MCP__DIRECTORY_GRANT__AUDIENCE",
                )?,
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
            },
            transport: TransportConfig {
''',
)
replace_once(
    config,
    '''        validate_file_safe_roots(&config.file)?;
        validate_android_capabilities(&config.android)?;
''',
    '''        validate_file_safe_roots(&config.file)?;
        validate_directory_grant_config(&config.directory_grant, &config.auth, &config.file)?;
        validate_android_capabilities(&config.android)?;
''',
)
replace_once(
    config,
    '''fn env_exact_string_list(
''',
    '''fn optional_env_exact_string_list(
    read_variable: &impl Fn(&str) -> Result<String, env::VarError>,
    name: &str,
) -> anyhow::Result<Option<Vec<String>>> {
    match read_env(read_variable, name)? {
        Some(value) => split_exact_env_list(name, &value).map(Some),
        None => Ok(None),
    }
}

fn env_exact_string_list(
''',
)
# Insert validation before runtime auth validation.
replace_once(
    config,
    '''pub fn validate_runtime_auth_posture(config: &AppConfig) -> anyhow::Result<AuthPosture> {
''',
    '''fn validate_directory_grant_config(
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
        || grant.safe_root_ids.is_some();
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
        bail!(
            "MCP__DIRECTORY_GRANT__CLOCK_SKEW_SECONDS must not exceed {MAX_CLOCK_SKEW_SECONDS}"
        );
    }
    Ok(())
}

pub fn validate_runtime_auth_posture(config: &AppConfig) -> anyhow::Result<AuthPosture> {
''',
)
# AppConfig test helper gets disabled defaults.
replace_once(
    config,
    '''            file: FileConfig {
                safe_roots: vec![PathBuf::from(DEFAULT_FILE_SAFE_ROOT)],
            },
            transport: transport_config(),
''',
    '''            file: FileConfig {
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
            },
            transport: transport_config(),
''',
)
replace_once(
    config,
    '''        assert!(!config.command.enabled);
        assert_eq!(
''',
    '''        assert!(!config.command.enabled);
        assert!(!config.directory_grant.verification_enabled);
        assert_eq!(config.directory_grant.issuer, None);
        assert_eq!(config.directory_grant.audience, None);
        assert_eq!(config.directory_grant.keyring_path, None);
        assert_eq!(config.directory_grant.safe_root_ids, None);
        assert_eq!(config.directory_grant.max_lifetime_seconds, 120);
        assert_eq!(config.directory_grant.clock_skew_seconds, 5);
        assert_eq!(
''',
)
replace_once(
    config,
    '''            "MCP__COMMAND__ENABLED",
            "MCP__TRANSPORT__ALLOWED_HOSTS",
''',
    '''            "MCP__COMMAND__ENABLED",
            "MCP__DIRECTORY_GRANT__VERIFICATION_ENABLED",
            "MCP__DIRECTORY_GRANT__ISSUER",
            "MCP__DIRECTORY_GRANT__AUDIENCE",
            "MCP__DIRECTORY_GRANT__KEYRING_PATH",
            "MCP__DIRECTORY_GRANT__SAFE_ROOT_IDS",
            "MCP__DIRECTORY_GRANT__MAX_LIFETIME_SECONDS",
            "MCP__DIRECTORY_GRANT__CLOCK_SKEW_SECONDS",
            "MCP__TRANSPORT__ALLOWED_HOSTS",
''',
)
# Add config tests before static token posture tests.
replace_once(
    config,
    '''    #[test]
    fn static_token_auth_posture_is_accepted_for_non_loopback_hosts() {
''',
    '''    #[test]
    fn directory_grant_settings_are_default_disabled_and_fail_closed_when_partial() {
        let mut config = app_config("127.0.0.1", Some("token"), false);
        config.directory_grant.issuer = Some("authority:v1".to_owned());
        let error = validate_directory_grant_config(
            &config.directory_grant,
            &config.auth,
            &config.file,
        )
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
            keyring_path: Some(PathBuf::from("/data/data/com.termux/files/home/.config/mcp/grants.json")),
            safe_root_ids: Some(vec!["primary-root".to_owned()]),
            max_lifetime_seconds: 120,
            clock_skew_seconds: 5,
        };
        validate_directory_grant_config(
            &config.directory_grant,
            &config.auth,
            &config.file,
        )
        .unwrap();

        config.auth.static_principal_id = None;
        assert!(validate_directory_grant_config(
            &config.directory_grant,
            &config.auth,
            &config.file,
        )
        .is_err());
        config.auth.static_principal_id = Some("operator.primary:v1".to_owned());
        config.directory_grant.safe_root_ids = Some(vec!["duplicate".to_owned(), "duplicate".to_owned()]);
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
''',
)

main = Path("src/main.rs")
replace_once(
    main,
    '''    auth::{require_mcp_auth, McpAuthPolicy},
    request_limits::{enforce_mcp_request_limits, McpRequestLimits},
''',
    '''    auth::{require_mcp_auth, McpAuthPolicy},
    directory_grant::DirectoryGrantVerifier,
    request_limits::{enforce_mcp_request_limits, McpRequestLimits},
''',
)
replace_once(
    main,
    '''    #[cfg(feature = "mcp-runtime")]
    let mcp_request_limits = McpRequestLimits::from_seconds(
''',
    '''    #[cfg(feature = "mcp-runtime")]
    let _directory_grant_verifier = DirectoryGrantVerifier::load_optional(
        &config.directory_grant,
        config.auth.static_token.as_deref(),
    )?;

    #[cfg(feature = "mcp-runtime")]
    info!(
        verification_configured = _directory_grant_verifier.is_some(),
        mutation_enabled = false,
        "Directory capability-grant verification posture loaded"
    );

    #[cfg(feature = "mcp-runtime")]
    let mcp_request_limits = McpRequestLimits::from_seconds(
''',
)

# Sanity checks that prevent accidental activation or raw grant transport through tool schemas.
for forbidden in [
    'create_directory_mutation_enabled: true',
    '"grant"',
    '"capability_grant"',
]:
    if forbidden in Path("src/mcp_transport.rs").read_text():
        raise SystemExit(f"unexpected mutation or tool-argument grant activation: {forbidden}")
