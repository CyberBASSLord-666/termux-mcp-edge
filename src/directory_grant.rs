//! Request-scoped `create_directory` grant transport and cryptographic verification.
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
    let count = headers.get_all(MCP_DIRECTORY_GRANT_HEADER).iter().count();
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

#[derive(PartialEq, Eq)]
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
        Self::load_optional_at(config, authentication_token, current_unix_seconds()?)
    }

    fn load_optional_at(
        config: &DirectoryGrantConfig,
        authentication_token: Option<&str>,
        now: u64,
    ) -> anyhow::Result<Option<Self>> {
        if !config.verification_enabled {
            return Ok(None);
        }
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
    let value: T =
        serde_json::from_slice(&bytes).map_err(|_| DirectoryGrantVerificationError::Malformed)?;
    let canonical =
        serde_json::to_vec(&value).map_err(|_| DirectoryGrantVerificationError::Malformed)?;
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
    let parsed =
        uuid::Uuid::parse_str(value).map_err(|_| DirectoryGrantVerificationError::Malformed)?;
    if parsed.to_string() != value {
        return Err(DirectoryGrantVerificationError::Malformed);
    }
    Ok(())
}

fn validate_safe_root_id(value: &str) -> Result<(), DirectoryGrantVerificationError> {
    validate_label(value, 64).map_err(|_| DirectoryGrantVerificationError::Malformed)
}

fn validate_path_components(components: &[String]) -> Result<(), DirectoryGrantVerificationError> {
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
            || (separator && (index == 0 || index + 1 == value.len() || previous_separator))
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
        DirectoryGrantVerifier::load_optional_at(&config(path), Some("authentication-token"), NOW)
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

    fn binding<'a>(
        principal: &'a AuthenticatedPrincipal,
        target_components: &'a [String],
    ) -> DirectoryGrantBinding<'a> {
        DirectoryGrantBinding {
            principal,
            session_id: SESSION_ID,
            safe_root_id: ROOT_ID,
            target_components,
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
        let components = vec!["projects".to_owned(), "alpha".to_owned()];
        let grant = mint(&key, valid_header(), claims());

        let verified = verifier
            .verify(Some(&grant), &binding(&principal, &components), NOW)
            .unwrap();
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
        let components = vec!["projects".to_owned(), "alpha".to_owned()];

        let mut bad_signature = mint(&[0x44_u8; 32], valid_header(), claims());
        assert_eq!(
            verifier.verify(Some(&bad_signature), &binding(&principal, &components), NOW),
            Err(DirectoryGrantVerificationError::SignatureRejected)
        );
        bad_signature.serialized = Arc::from("bad");
        assert_eq!(
            verifier.verify(Some(&bad_signature), &binding(&principal, &components), NOW),
            Err(DirectoryGrantVerificationError::Malformed)
        );

        let mut header = valid_header();
        header.alg = "none".to_owned();
        assert_eq!(
            verifier.verify(
                Some(&mint(&key, header, claims())),
                &binding(&principal, &components),
                NOW
            ),
            Err(DirectoryGrantVerificationError::AlgorithmRejected)
        );
        let mut header = valid_header();
        header.kid = "unknown-key".to_owned();
        assert_eq!(
            verifier.verify(
                Some(&mint(&key, header, claims())),
                &binding(&principal, &components),
                NOW
            ),
            Err(DirectoryGrantVerificationError::UnknownKey)
        );

        for (claims, expected) in [
            (
                {
                    let mut c = claims();
                    c.iss = "other-issuer".to_owned();
                    c
                },
                DirectoryGrantVerificationError::IssuerMismatch,
            ),
            (
                {
                    let mut c = claims();
                    c.aud = "other-audience".to_owned();
                    c
                },
                DirectoryGrantVerificationError::AudienceMismatch,
            ),
            (
                {
                    let mut c = claims();
                    c.sub = "operator.other:v1".to_owned();
                    c
                },
                DirectoryGrantVerificationError::PrincipalMismatch,
            ),
            (
                {
                    let mut c = claims();
                    c.sid = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_owned();
                    c
                },
                DirectoryGrantVerificationError::SessionMismatch,
            ),
            (
                {
                    let mut c = claims();
                    c.cap = "filesystem.write-file".to_owned();
                    c
                },
                DirectoryGrantVerificationError::CapabilityMismatch,
            ),
            (
                {
                    let mut c = claims();
                    c.root = "other-root".to_owned();
                    c
                },
                DirectoryGrantVerificationError::RootMismatch,
            ),
            (
                {
                    let mut c = claims();
                    c.path = vec!["projects".to_owned(), "beta".to_owned()];
                    c
                },
                DirectoryGrantVerificationError::PathMismatch,
            ),
            (
                {
                    let mut c = claims();
                    c.posture = "dry-run".to_owned();
                    c
                },
                DirectoryGrantVerificationError::PostureMismatch,
            ),
        ] {
            let grant = mint(&key, valid_header(), claims);
            assert_eq!(
                verifier.verify(Some(&grant), &binding(&principal, &components), NOW),
                Err(expected)
            );
        }
    }

    #[test]
    fn rejects_time_window_and_clock_rollback_failures() {
        let directory = tempfile::tempdir().unwrap();
        let key = [0x45_u8; 32];
        let verifier = verifier(&directory.path().join("keyring.json"), &key);
        let principal = AuthenticatedPrincipal::configured(PRINCIPAL).unwrap();
        let components = vec!["projects".to_owned(), "alpha".to_owned()];

        let mut future = claims();
        future.iat = NOW + 10;
        future.nbf = Some(NOW + 10);
        future.exp = NOW + 70;
        assert_eq!(
            verifier.verify(
                Some(&mint(&key, valid_header(), future)),
                &binding(&principal, &components),
                NOW
            ),
            Err(DirectoryGrantVerificationError::FutureIssued)
        );

        let mut not_yet = claims();
        not_yet.nbf = Some(NOW + 10);
        assert_eq!(
            verifier.verify(
                Some(&mint(&key, valid_header(), not_yet)),
                &binding(&principal, &components),
                NOW
            ),
            Err(DirectoryGrantVerificationError::NotYetValid)
        );

        let mut expired = claims();
        expired.iat = NOW - 100;
        expired.nbf = Some(NOW - 100);
        expired.exp = NOW - 6;
        assert_eq!(
            verifier.verify(
                Some(&mint(&key, valid_header(), expired)),
                &binding(&principal, &components),
                NOW
            ),
            Err(DirectoryGrantVerificationError::Expired)
        );

        let mut excessive = claims();
        excessive.exp = NOW + 121;
        assert_eq!(
            verifier.verify(
                Some(&mint(&key, valid_header(), excessive)),
                &binding(&principal, &components),
                NOW
            ),
            Err(DirectoryGrantVerificationError::ExcessiveLifetime)
        );

        let valid = mint(&key, valid_header(), claims());
        verifier
            .verify(Some(&valid), &binding(&principal, &components), NOW + 1)
            .unwrap();
        assert_eq!(
            verifier.verify(Some(&valid), &binding(&principal, &components), NOW),
            Err(DirectoryGrantVerificationError::ClockRollback)
        );
    }

    #[test]
    fn keyring_is_owner_only_bounded_unique_and_separate_from_authentication() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keyring.json");
        let key = [0x46_u8; 32];

        write_keyring(&path, &key, 0o644, false);
        assert!(
            DirectoryGrantVerifier::load_optional_at(&config(&path), Some("auth-token"), NOW)
                .is_err()
        );

        write_keyring(&path, &key, 0o600, true);
        assert!(
            DirectoryGrantVerifier::load_optional_at(&config(&path), Some("auth-token"), NOW)
                .is_err()
        );

        let auth_token = "a".repeat(32);
        write_keyring(&path, auth_token.as_bytes(), 0o600, false);
        let error =
            DirectoryGrantVerifier::load_optional_at(&config(&path), Some(&auth_token), NOW)
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
        headers.insert(
            MCP_DIRECTORY_GRANT_HEADER,
            grant.serialized().parse().unwrap(),
        );
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
        malformed.insert(
            MCP_DIRECTORY_GRANT_HEADER,
            "contains whitespace".parse().unwrap(),
        );
        assert_eq!(
            take_directory_grant_authorization(&mut malformed),
            Err(DirectoryGrantContextError::Malformed)
        );
        assert!(!malformed.contains_key(MCP_DIRECTORY_GRANT_HEADER));
    }
}
