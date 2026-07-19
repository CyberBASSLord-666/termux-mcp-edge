//! Request-scoped authorization for `copy_file` mutation.
//!
//! Grants are short-lived HMAC-SHA-256 capabilities bound to one authenticated
//! static principal, one MCP session, exact anchored source and destination
//! safe roots, both normalized root-relative paths, one high-resolution
//! single-link source identity and size, the source SHA-256 digest, and an
//! absent-destination/no-replace mutation posture. The serialized payload
//! contains only a random grant ID, a signed capability byte, a keyed opaque
//! operation binding, and issuance timestamps. It therefore does not disclose
//! stable principal, session, filesystem, path, identity, size, or content
//! fingerprints. Grant material is never logged or exposed by runtime
//! responses, and a valid grant is atomically consumed before the first
//! filesystem mutation attempt.

use std::{
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    grant_replay::{shared_replay_state, SharedReplayError, SharedReplayState},
    request_grant_capability::{
        RequestGrantCapability, MAX_REQUEST_GRANT_HEADER_BYTES, REQUEST_GRANT_HEADER,
    },
};

type HmacSha256 = Hmac<Sha256>;

pub const COPY_FILE_GRANT_HEADER: &str = REQUEST_GRANT_HEADER;
pub const COPY_FILE_GRANT_VERSION: &str = "v1";
pub const COPY_FILE_GRANT_TTL_SECONDS: u64 = 60;
pub const MAX_COPY_FILE_GRANT_LIFETIME_SECONDS: u64 = 120;
pub const MAX_COPY_FILE_GRANT_FUTURE_SKEW_SECONDS: u64 = 5;
pub const MAX_COPY_FILE_GRANT_HEADER_BYTES: usize = MAX_REQUEST_GRANT_HEADER_BYTES;
pub const MAX_COPY_FILE_GRANT_KEY_ID_BYTES: usize = 32;
pub const COPY_FILE_GRANT_KEY_BYTES: usize = 32;
pub const COPY_FILE_GRANT_KEY_HEX_BYTES: usize = COPY_FILE_GRANT_KEY_BYTES * 2;
pub const MAX_CONSUMED_COPY_FILE_GRANTS: usize = 4_096;

const NO_REPLACE_MUTATING_POSTURE: u8 = 1;
const GRANT_ID_BYTES: usize = 16;
const DIGEST_BYTES: usize = 32;
const SESSION_BYTES: usize = 16;
const BINDING_BYTES: usize = 32;
const PAYLOAD_BYTES: usize = GRANT_ID_BYTES + 1 + BINDING_BYTES + 8 + 8;
const PAYLOAD_HEX_BYTES: usize = PAYLOAD_BYTES * 2;
const MAC_BYTES: usize = 32;
const MAC_HEX_BYTES: usize = MAC_BYTES * 2;
const PATH_DIGEST_DOMAIN: &[u8] = b"termux-mcp:copy-file-path:v1\0";
const PRINCIPAL_BINDING_DOMAIN: &[u8] = b"termux-mcp:copy-file-principal:v1\0";
const OPERATION_BINDING_DOMAIN: &[u8] = b"termux-mcp:copy-file-operation-binding:v1\0";

#[derive(Clone, PartialEq, Eq)]
pub struct CopyFileGrantTarget {
    source_root_device: u64,
    source_root_inode: u64,
    source_path_digest: [u8; DIGEST_BYTES],
    source_identity: CopyFileSourceIdentity,
    source_sha256: [u8; DIGEST_BYTES],
    destination_root_device: u64,
    destination_root_inode: u64,
    destination_path_digest: [u8; DIGEST_BYTES],
}

/// Descriptor-derived identity bound to one copy authorization.
///
/// The high-resolution change time and size make inode reuse and same-inode
/// source changes fail closed. Live copy additionally requires the link count
/// to remain exactly one so the authorized source has no untracked alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CopyFileSourceIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) size: u64,
    pub(crate) ctime_seconds: i64,
    pub(crate) ctime_nanoseconds: i64,
    pub(crate) link_count: u64,
}

impl CopyFileSourceIdentity {
    pub(crate) fn new(
        device: u64,
        inode: u64,
        size: u64,
        ctime_seconds: i64,
        ctime_nanoseconds: i64,
        link_count: u64,
    ) -> Result<Self, CopyFileGrantError> {
        if link_count != 1 || !(0..1_000_000_000).contains(&ctime_nanoseconds) {
            return Err(CopyFileGrantError::TargetInvalid);
        }
        Ok(Self {
            device,
            inode,
            size,
            ctime_seconds,
            ctime_nanoseconds,
            link_count,
        })
    }
}

impl CopyFileGrantTarget {
    #[expect(
        clippy::too_many_arguments,
        reason = "the authorization boundary must receive both anchored roots and exact source evidence explicitly"
    )]
    pub(crate) fn from_normalized_components<'source, 'destination>(
        source_root_device: u64,
        source_root_inode: u64,
        source_components: impl IntoIterator<Item = &'source [u8]>,
        source_identity: CopyFileSourceIdentity,
        source_sha256: [u8; DIGEST_BYTES],
        destination_root_device: u64,
        destination_root_inode: u64,
        destination_components: impl IntoIterator<Item = &'destination [u8]>,
    ) -> Result<Self, CopyFileGrantError> {
        let source_path_digest = normalized_path_digest(source_components)?;
        let destination_path_digest = normalized_path_digest(destination_components)?;
        if source_root_device == destination_root_device
            && source_root_inode == destination_root_inode
            && source_path_digest == destination_path_digest
        {
            return Err(CopyFileGrantError::TargetInvalid);
        }

        Ok(Self {
            source_root_device,
            source_root_inode,
            source_path_digest,
            source_identity,
            source_sha256,
            destination_root_device,
            destination_root_inode,
            destination_path_digest,
        })
    }

    #[cfg(test)]
    #[expect(
        clippy::too_many_arguments,
        reason = "test construction mirrors the production authorization boundary"
    )]
    fn test(
        source_root_device: u64,
        source_root_inode: u64,
        source_components: &[&[u8]],
        source_identity: CopyFileSourceIdentity,
        source_sha256: [u8; DIGEST_BYTES],
        destination_root_device: u64,
        destination_root_inode: u64,
        destination_components: &[&[u8]],
    ) -> Self {
        Self::from_normalized_components(
            source_root_device,
            source_root_inode,
            source_components.iter().copied(),
            source_identity,
            source_sha256,
            destination_root_device,
            destination_root_inode,
            destination_components.iter().copied(),
        )
        .expect("test target must be valid")
    }
}

fn normalized_path_digest<'a>(
    components: impl IntoIterator<Item = &'a [u8]>,
) -> Result<[u8; DIGEST_BYTES], CopyFileGrantError> {
    let mut digest = Sha256::new();
    digest.update(PATH_DIGEST_DOMAIN);
    let mut component_count = 0_u32;
    for component in components {
        let length =
            u32::try_from(component.len()).map_err(|_| CopyFileGrantError::TargetInvalid)?;
        digest.update(length.to_be_bytes());
        digest.update(component);
        component_count = component_count
            .checked_add(1)
            .ok_or(CopyFileGrantError::TargetInvalid)?;
    }
    if component_count == 0 {
        return Err(CopyFileGrantError::TargetInvalid);
    }
    digest.update(component_count.to_be_bytes());
    Ok(digest.finalize().into())
}

impl fmt::Debug for CopyFileGrantTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CopyFileGrantTarget")
            .field("binding", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyFileGrantError {
    ConfigurationInvalid,
    TargetInvalid,
    SessionInvalid,
    Missing,
    Malformed,
    UnknownVersion,
    UnknownKey,
    SignatureInvalid,
    Expired,
    FutureIssued,
    LifetimeExceeded,
    BindingMismatch,
    Replayed,
    ClockRollback,
    ReplayCapacityExhausted,
    StateUnavailable,
}

impl CopyFileGrantError {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::ConfigurationInvalid => "capability_configuration_invalid",
            Self::TargetInvalid => "capability_target_invalid",
            Self::SessionInvalid => "capability_session_invalid",
            Self::Missing => "capability_grant_missing",
            Self::Malformed => "capability_grant_malformed",
            Self::UnknownVersion => "capability_grant_version_unknown",
            Self::UnknownKey => "capability_grant_key_unknown",
            Self::SignatureInvalid => "capability_grant_signature_invalid",
            Self::Expired => "capability_grant_expired",
            Self::FutureIssued => "capability_grant_future_issued",
            Self::LifetimeExceeded => "capability_grant_lifetime_exceeded",
            Self::BindingMismatch => "capability_grant_binding_mismatch",
            Self::Replayed => "capability_grant_replayed",
            Self::ClockRollback => "capability_clock_rollback",
            Self::ReplayCapacityExhausted => "capability_replay_capacity_exhausted",
            Self::StateUnavailable => "capability_state_unavailable",
        }
    }
}

impl fmt::Display for CopyFileGrantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason_code())
    }
}

impl std::error::Error for CopyFileGrantError {}

#[derive(Clone)]
pub struct CopyFileGrantAuthority {
    key_id: Arc<str>,
    key: Arc<[u8; COPY_FILE_GRANT_KEY_BYTES]>,
    principal_digest: [u8; DIGEST_BYTES],
    replay: SharedReplayState,
}

struct ParsedGrant {
    grant_id: [u8; GRANT_ID_BYTES],
    capability: u8,
    operation_binding: [u8; BINDING_BYTES],
    issued_unix_seconds: u64,
    expires_unix_seconds: u64,
}

impl CopyFileGrantAuthority {
    pub fn from_hex_key(
        key_id: impl Into<String>,
        key_hex: &str,
        static_principal_secret: &str,
    ) -> Result<Self, CopyFileGrantError> {
        Self::from_hex_key_with_capacity(
            key_id,
            key_hex,
            static_principal_secret,
            MAX_CONSUMED_COPY_FILE_GRANTS,
        )
    }

    fn from_hex_key_with_capacity(
        key_id: impl Into<String>,
        key_hex: &str,
        static_principal_secret: &str,
        replay_capacity: usize,
    ) -> Result<Self, CopyFileGrantError> {
        let key_id = key_id.into();
        if !valid_key_id(&key_id)
            || key_hex.len() != COPY_FILE_GRANT_KEY_HEX_BYTES
            || static_principal_secret.is_empty()
            || replay_capacity == 0
        {
            return Err(CopyFileGrantError::ConfigurationInvalid);
        }
        let key = decode_hex_array::<COPY_FILE_GRANT_KEY_BYTES>(key_hex)
            .ok_or(CopyFileGrantError::ConfigurationInvalid)?;
        // Derive the principal binding under the independent capability key.
        // A disclosed grant therefore cannot be used as an offline verifier
        // for guesses of a weak transport bearer token.
        let mut principal =
            HmacSha256::new_from_slice(&key).expect("fixed-size HMAC key is always valid");
        principal.update(PRINCIPAL_BINDING_DOMAIN);
        principal.update(static_principal_secret.as_bytes());
        let principal_digest = principal.finalize().into_bytes().into();
        let replay = shared_replay_state(
            RequestGrantCapability::CopyFile,
            &key_id,
            &key,
            &principal_digest,
            replay_capacity,
        )
        .map_err(|_| CopyFileGrantError::StateUnavailable)?;

        Ok(Self {
            key_id: Arc::from(key_id),
            key: Arc::new(key),
            principal_digest,
            replay,
        })
    }

    pub(crate) fn binds_static_principal(&self, principal: Option<&str>) -> bool {
        let Some(principal) = principal else {
            return false;
        };
        let mut candidate =
            HmacSha256::new_from_slice(self.key.as_ref()).expect("fixed-size HMAC key is valid");
        candidate.update(PRINCIPAL_BINDING_DOMAIN);
        candidate.update(principal.as_bytes());
        let candidate: [u8; DIGEST_BYTES] = candidate.finalize().into_bytes().into();
        candidate == self.principal_digest
    }

    pub fn issue(
        &self,
        session_id: &str,
        target: &CopyFileGrantTarget,
    ) -> Result<String, CopyFileGrantError> {
        self.issue_at(session_id, target, current_unix_seconds()?)
    }

    pub fn consume(
        &self,
        token: Option<&str>,
        session_id: &str,
        target: &CopyFileGrantTarget,
    ) -> Result<(), CopyFileGrantError> {
        self.consume_at(token, session_id, target, current_unix_seconds()?)
    }

    pub(crate) fn issue_at(
        &self,
        session_id: &str,
        target: &CopyFileGrantTarget,
        now_unix_seconds: u64,
    ) -> Result<String, CopyFileGrantError> {
        let session_id = parse_canonical_session(session_id)?;
        let expires_unix_seconds = now_unix_seconds
            .checked_add(COPY_FILE_GRANT_TTL_SECONDS)
            .ok_or(CopyFileGrantError::LifetimeExceeded)?;
        let grant_id = *Uuid::new_v4().as_bytes();
        let grant = ParsedGrant {
            grant_id,
            capability: RequestGrantCapability::CopyFile.wire_code(),
            operation_binding: self
                .operation_binding_mac(&grant_id, &session_id, target)
                .finalize()
                .into_bytes()
                .into(),
            issued_unix_seconds: now_unix_seconds,
            expires_unix_seconds,
        };
        Ok(self.encode_and_sign(&grant))
    }

    pub(crate) fn consume_at(
        &self,
        token: Option<&str>,
        session_id: &str,
        target: &CopyFileGrantTarget,
        now_unix_seconds: u64,
    ) -> Result<(), CopyFileGrantError> {
        let token = token.ok_or(CopyFileGrantError::Missing)?;
        let expected_session = parse_canonical_session(session_id)?;
        let grant = self.parse_and_verify(token)?;
        if grant.capability != RequestGrantCapability::CopyFile.wire_code() {
            return Err(CopyFileGrantError::BindingMismatch);
        }
        self.operation_binding_mac(&grant.grant_id, &expected_session, target)
            .verify_slice(&grant.operation_binding)
            .map_err(|_| CopyFileGrantError::BindingMismatch)?;

        self.replay
            .consume(
                grant.grant_id,
                grant.issued_unix_seconds,
                grant.expires_unix_seconds,
                now_unix_seconds,
                MAX_COPY_FILE_GRANT_LIFETIME_SECONDS,
                MAX_COPY_FILE_GRANT_FUTURE_SKEW_SECONDS,
            )
            .map_err(map_replay_error)
    }

    fn encode_and_sign(&self, grant: &ParsedGrant) -> String {
        let payload_hex = encode_hex(&encode_payload(grant));
        let signed = format!("{COPY_FILE_GRANT_VERSION}.{}.{}", self.key_id, payload_hex);
        let mut mac = HmacSha256::new_from_slice(self.key.as_ref())
            .expect("fixed-size HMAC key is always valid");
        mac.update(signed.as_bytes());
        format!("{signed}.{}", encode_hex(&mac.finalize().into_bytes()))
    }

    fn operation_binding_mac(
        &self,
        grant_id: &[u8; GRANT_ID_BYTES],
        session_id: &[u8; SESSION_BYTES],
        target: &CopyFileGrantTarget,
    ) -> HmacSha256 {
        let mut binding = HmacSha256::new_from_slice(self.key.as_ref())
            .expect("fixed-size HMAC key is always valid");
        binding.update(OPERATION_BINDING_DOMAIN);
        binding.update(grant_id);
        binding.update(&self.principal_digest);
        binding.update(session_id);
        binding.update(&[
            RequestGrantCapability::CopyFile.wire_code(),
            NO_REPLACE_MUTATING_POSTURE,
        ]);
        binding.update(&target.source_root_device.to_be_bytes());
        binding.update(&target.source_root_inode.to_be_bytes());
        binding.update(&target.source_path_digest);
        binding.update(&target.source_identity.device.to_be_bytes());
        binding.update(&target.source_identity.inode.to_be_bytes());
        binding.update(&target.source_identity.size.to_be_bytes());
        binding.update(&target.source_identity.ctime_seconds.to_be_bytes());
        binding.update(&target.source_identity.ctime_nanoseconds.to_be_bytes());
        binding.update(&target.source_identity.link_count.to_be_bytes());
        binding.update(&target.source_sha256);
        binding.update(&target.destination_root_device.to_be_bytes());
        binding.update(&target.destination_root_inode.to_be_bytes());
        binding.update(&target.destination_path_digest);
        binding
    }

    fn parse_and_verify(&self, token: &str) -> Result<ParsedGrant, CopyFileGrantError> {
        if token.is_empty() || token.len() > MAX_COPY_FILE_GRANT_HEADER_BYTES || !token.is_ascii() {
            return Err(CopyFileGrantError::Malformed);
        }
        let mut segments = token.split('.');
        let version = segments.next().ok_or(CopyFileGrantError::Malformed)?;
        let key_id = segments.next().ok_or(CopyFileGrantError::Malformed)?;
        let payload_hex = segments.next().ok_or(CopyFileGrantError::Malformed)?;
        let signature_hex = segments.next().ok_or(CopyFileGrantError::Malformed)?;
        if segments.next().is_some() {
            return Err(CopyFileGrantError::Malformed);
        }
        if version != COPY_FILE_GRANT_VERSION {
            return Err(CopyFileGrantError::UnknownVersion);
        }
        if key_id != self.key_id.as_ref() {
            return Err(CopyFileGrantError::UnknownKey);
        }
        if payload_hex.len() != PAYLOAD_HEX_BYTES || signature_hex.len() != MAC_HEX_BYTES {
            return Err(CopyFileGrantError::Malformed);
        }
        let payload =
            decode_hex_array::<PAYLOAD_BYTES>(payload_hex).ok_or(CopyFileGrantError::Malformed)?;
        let signature =
            decode_hex_array::<MAC_BYTES>(signature_hex).ok_or(CopyFileGrantError::Malformed)?;
        let signed_length = version.len() + 1 + key_id.len() + 1 + payload_hex.len();
        let signed = token
            .get(..signed_length)
            .ok_or(CopyFileGrantError::Malformed)?;
        let mut mac = HmacSha256::new_from_slice(self.key.as_ref())
            .expect("fixed-size HMAC key is always valid");
        mac.update(signed.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| CopyFileGrantError::SignatureInvalid)?;
        decode_payload(&payload).ok_or(CopyFileGrantError::Malformed)
    }
}

impl fmt::Debug for CopyFileGrantAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CopyFileGrantAuthority")
            .field("key_id", &self.key_id)
            .field("key", &"<redacted>")
            .field("principal", &"<redacted>")
            .field("replay", &"<process-global>")
            .finish()
    }
}

const fn map_replay_error(error: SharedReplayError) -> CopyFileGrantError {
    match error {
        SharedReplayError::Expired => CopyFileGrantError::Expired,
        SharedReplayError::FutureIssued => CopyFileGrantError::FutureIssued,
        SharedReplayError::LifetimeExceeded => CopyFileGrantError::LifetimeExceeded,
        SharedReplayError::Replayed => CopyFileGrantError::Replayed,
        SharedReplayError::ClockRollback => CopyFileGrantError::ClockRollback,
        SharedReplayError::CapacityExhausted => CopyFileGrantError::ReplayCapacityExhausted,
        SharedReplayError::StateUnavailable => CopyFileGrantError::StateUnavailable,
    }
}

fn current_unix_seconds() -> Result<u64, CopyFileGrantError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| CopyFileGrantError::ClockRollback)
}

fn valid_key_id(key_id: &str) -> bool {
    !key_id.is_empty()
        && key_id.len() <= MAX_COPY_FILE_GRANT_KEY_ID_BYTES
        && key_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn parse_canonical_session(session_id: &str) -> Result<[u8; SESSION_BYTES], CopyFileGrantError> {
    let parsed = Uuid::parse_str(session_id).map_err(|_| CopyFileGrantError::SessionInvalid)?;
    if parsed.to_string() != session_id {
        return Err(CopyFileGrantError::SessionInvalid);
    }
    Ok(*parsed.as_bytes())
}

fn encode_payload(grant: &ParsedGrant) -> [u8; PAYLOAD_BYTES] {
    let mut payload = [0_u8; PAYLOAD_BYTES];
    let mut offset = 0;
    put(&mut payload, &mut offset, &grant.grant_id);
    put(&mut payload, &mut offset, &[grant.capability]);
    put(&mut payload, &mut offset, &grant.operation_binding);
    put(
        &mut payload,
        &mut offset,
        &grant.issued_unix_seconds.to_be_bytes(),
    );
    put(
        &mut payload,
        &mut offset,
        &grant.expires_unix_seconds.to_be_bytes(),
    );
    debug_assert_eq!(offset, PAYLOAD_BYTES);
    payload
}

fn decode_payload(payload: &[u8; PAYLOAD_BYTES]) -> Option<ParsedGrant> {
    let mut offset = 0;
    let grant_id = take_array(payload, &mut offset)?;
    let capability = *payload.get(offset)?;
    offset += 1;
    let operation_binding = take_array(payload, &mut offset)?;
    let issued_unix_seconds = u64::from_be_bytes(take_array(payload, &mut offset)?);
    let expires_unix_seconds = u64::from_be_bytes(take_array(payload, &mut offset)?);
    (offset == PAYLOAD_BYTES).then_some(ParsedGrant {
        grant_id,
        capability,
        operation_binding,
        issued_unix_seconds,
        expires_unix_seconds,
    })
}

fn put<const N: usize>(output: &mut [u8], offset: &mut usize, value: &[u8; N]) {
    let end = *offset + N;
    output[*offset..end].copy_from_slice(value);
    *offset = end;
}

fn take_array<const N: usize>(input: &[u8], offset: &mut usize) -> Option<[u8; N]> {
    let end = offset.checked_add(N)?;
    let value = input.get(*offset..end)?.try_into().ok()?;
    *offset = end;
    Some(value)
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn decode_hex_array<const N: usize>(input: &str) -> Option<[u8; N]> {
    if input.len() != N * 2 || !input.is_ascii() {
        return None;
    }
    let mut output = [0_u8; N];
    for (index, pair) in input.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (decode_hex_digit(pair[0])? << 4) | decode_hex_digit(pair[1])?;
    }
    Some(output)
}

fn decode_hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Barrier,
        },
        thread,
    };

    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const PRINCIPAL: &str = "static-principal-secret";
    const SESSION: &str = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    const NOW: u64 = 1_725_000_000;

    const CAPABILITY_OFFSET: usize = GRANT_ID_BYTES;
    const BINDING_OFFSET: usize = CAPABILITY_OFFSET + 1;
    const ISSUED_OFFSET: usize = BINDING_OFFSET + BINDING_BYTES;
    static AUTHORITY_SEQUENCE: AtomicUsize = AtomicUsize::new(1);

    fn authority() -> CopyFileGrantAuthority {
        let sequence = AUTHORITY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        CopyFileGrantAuthority::from_hex_key(format!("copy-test-{sequence}"), KEY, PRINCIPAL)
            .unwrap()
    }

    fn identity() -> CopyFileSourceIdentity {
        CopyFileSourceIdentity::new(101, 202, 303, 1_700_000_000, 404_505_606, 1).unwrap()
    }

    fn source_sha256() -> [u8; DIGEST_BYTES] {
        Sha256::digest(b"source-payload").into()
    }

    fn target() -> CopyFileGrantTarget {
        CopyFileGrantTarget::test(
            41,
            73,
            &[b"projects", b"source.bin"],
            identity(),
            source_sha256(),
            43,
            79,
            &[b"archive", b"destination.bin"],
        )
    }

    fn resign_payload(
        authority: &CopyFileGrantAuthority,
        token: &str,
        mutate: impl FnOnce(&mut [u8; PAYLOAD_BYTES]),
    ) -> String {
        let mut segments = token.split('.');
        let version = segments.next().unwrap();
        let key_id = segments.next().unwrap();
        let payload_hex = segments.next().unwrap();
        let _signature = segments.next().unwrap();
        let mut payload = decode_hex_array::<PAYLOAD_BYTES>(payload_hex).unwrap();
        mutate(&mut payload);
        let payload_hex = encode_hex(&payload);
        let signed = format!("{version}.{key_id}.{payload_hex}");
        let mut mac = HmacSha256::new_from_slice(authority.key.as_ref()).unwrap();
        mac.update(signed.as_bytes());
        format!("{signed}.{}", encode_hex(&mac.finalize().into_bytes()))
    }

    #[test]
    fn fixed_wire_contract_is_short_lived_opaque_and_no_replace_only() {
        assert_eq!(COPY_FILE_GRANT_HEADER, "mcp-capability-grant");
        assert_eq!(COPY_FILE_GRANT_VERSION, "v1");
        assert_eq!(COPY_FILE_GRANT_TTL_SECONDS, 60);
        assert_eq!(MAX_COPY_FILE_GRANT_LIFETIME_SECONDS, 120);
        assert_eq!(MAX_COPY_FILE_GRANT_FUTURE_SKEW_SECONDS, 5);
        assert_eq!(MAX_COPY_FILE_GRANT_HEADER_BYTES, 384);
        assert_eq!(NO_REPLACE_MUTATING_POSTURE, 1);
        assert_eq!(RequestGrantCapability::CopyFile.wire_code(), 4);
    }

    #[test]
    fn public_issue_and_consume_use_one_short_lived_single_use_grant() {
        let authority = authority();
        let token = authority.issue(SESSION, &target()).unwrap();
        assert!(token.len() <= MAX_COPY_FILE_GRANT_HEADER_BYTES);
        assert_eq!(token.split('.').count(), 4);
        let payload_hex = token.split('.').nth(2).unwrap();
        let payload = decode_hex_array::<PAYLOAD_BYTES>(payload_hex).unwrap();
        assert_eq!(
            payload[CAPABILITY_OFFSET],
            RequestGrantCapability::CopyFile.wire_code()
        );
        authority.consume(Some(&token), SESSION, &target()).unwrap();
        assert_eq!(
            authority
                .consume(Some(&token), SESSION, &target())
                .unwrap_err(),
            CopyFileGrantError::Replayed
        );
    }

    #[test]
    fn normalized_target_binds_both_roots_paths_source_identity_size_and_sha256() {
        let baseline = target();
        assert_ne!(
            baseline,
            CopyFileGrantTarget::test(
                41,
                73,
                &[b"projects-source", b"bin"],
                identity(),
                source_sha256(),
                43,
                79,
                &[b"archive", b"destination.bin"],
            )
        );
        assert_ne!(
            baseline,
            CopyFileGrantTarget::test(
                41,
                73,
                &[b"projects", b"source.bin"],
                identity(),
                Sha256::digest(b"different-source-payload").into(),
                43,
                79,
                &[b"archive", b"destination.bin"],
            )
        );
        assert_ne!(
            baseline,
            CopyFileGrantTarget::test(
                41,
                73,
                &[b"projects", b"source.bin"],
                CopyFileSourceIdentity {
                    size: 304,
                    ..identity()
                },
                source_sha256(),
                43,
                79,
                &[b"archive", b"destination.bin"],
            )
        );
        assert_ne!(
            baseline,
            CopyFileGrantTarget::test(
                41,
                73,
                &[b"projects", b"source.bin"],
                identity(),
                source_sha256(),
                43,
                80,
                &[b"archive", b"destination.bin"],
            )
        );
        assert_ne!(
            baseline,
            CopyFileGrantTarget::test(
                41,
                73,
                &[b"projects", b"source.bin"],
                identity(),
                source_sha256(),
                43,
                79,
                &[b"archive-destination", b"bin"],
            )
        );

        let empty_source = CopyFileGrantTarget::from_normalized_components(
            41,
            73,
            std::iter::empty::<&[u8]>(),
            identity(),
            source_sha256(),
            43,
            79,
            [b"destination.bin".as_slice()],
        );
        assert_eq!(empty_source.unwrap_err(), CopyFileGrantError::TargetInvalid);
        let empty_destination = CopyFileGrantTarget::from_normalized_components(
            41,
            73,
            [b"source.bin".as_slice()],
            identity(),
            source_sha256(),
            43,
            79,
            std::iter::empty::<&[u8]>(),
        );
        assert_eq!(
            empty_destination.unwrap_err(),
            CopyFileGrantError::TargetInvalid
        );
        let same_anchored_path = CopyFileGrantTarget::from_normalized_components(
            41,
            73,
            [b"same.bin".as_slice()],
            identity(),
            source_sha256(),
            41,
            73,
            [b"same.bin".as_slice()],
        );
        assert_eq!(
            same_anchored_path.unwrap_err(),
            CopyFileGrantError::TargetInvalid
        );
        assert_eq!(
            CopyFileSourceIdentity::new(1, 2, 3, 4, 1_000_000_000, 1).unwrap_err(),
            CopyFileGrantError::TargetInvalid
        );
        for invalid_links in [0, 2] {
            assert_eq!(
                CopyFileSourceIdentity::new(1, 2, 3, 4, 5, invalid_links).unwrap_err(),
                CopyFileGrantError::TargetInvalid
            );
        }
    }

    #[test]
    fn principal_binding_is_keyed_and_not_an_offline_bearer_verifier() {
        let authority = authority();
        let mut unkeyed = Sha256::new();
        unkeyed.update(PRINCIPAL_BINDING_DOMAIN);
        unkeyed.update(PRINCIPAL.as_bytes());
        let unkeyed: [u8; DIGEST_BYTES] = unkeyed.finalize().into();
        assert_ne!(authority.principal_digest, unkeyed);

        let other_key = "11".repeat(COPY_FILE_GRANT_KEY_BYTES);
        let other =
            CopyFileGrantAuthority::from_hex_key("primary-1", &other_key, PRINCIPAL).unwrap();
        assert_ne!(authority.principal_digest, other.principal_digest);
    }

    #[test]
    fn rejects_invalid_configuration_and_noncanonical_sessions() {
        for key_id in ["", "Primary", "bad.id", &"x".repeat(33)] {
            assert_eq!(
                CopyFileGrantAuthority::from_hex_key(key_id, KEY, PRINCIPAL).unwrap_err(),
                CopyFileGrantError::ConfigurationInvalid
            );
        }
        for key in ["", &"0".repeat(63), &"A".repeat(64)] {
            assert_eq!(
                CopyFileGrantAuthority::from_hex_key("primary-1", key, PRINCIPAL).unwrap_err(),
                CopyFileGrantError::ConfigurationInvalid
            );
        }
        assert_eq!(
            CopyFileGrantAuthority::from_hex_key("primary-1", KEY, "").unwrap_err(),
            CopyFileGrantError::ConfigurationInvalid
        );
        for session in ["", "not-a-uuid", "0194F9F9-BBBB-7CCC-8DDD-EEEEEEEEEEEE"] {
            assert_eq!(
                authority().issue_at(session, &target(), NOW).unwrap_err(),
                CopyFileGrantError::SessionInvalid
            );
        }
        assert_eq!(
            authority()
                .issue_at(SESSION, &target(), u64::MAX)
                .unwrap_err(),
            CopyFileGrantError::LifetimeExceeded
        );
    }

    #[test]
    fn every_error_has_one_stable_content_free_reason_code() {
        let cases = [
            (
                CopyFileGrantError::ConfigurationInvalid,
                "capability_configuration_invalid",
            ),
            (
                CopyFileGrantError::TargetInvalid,
                "capability_target_invalid",
            ),
            (
                CopyFileGrantError::SessionInvalid,
                "capability_session_invalid",
            ),
            (CopyFileGrantError::Missing, "capability_grant_missing"),
            (CopyFileGrantError::Malformed, "capability_grant_malformed"),
            (
                CopyFileGrantError::UnknownVersion,
                "capability_grant_version_unknown",
            ),
            (
                CopyFileGrantError::UnknownKey,
                "capability_grant_key_unknown",
            ),
            (
                CopyFileGrantError::SignatureInvalid,
                "capability_grant_signature_invalid",
            ),
            (CopyFileGrantError::Expired, "capability_grant_expired"),
            (
                CopyFileGrantError::FutureIssued,
                "capability_grant_future_issued",
            ),
            (
                CopyFileGrantError::LifetimeExceeded,
                "capability_grant_lifetime_exceeded",
            ),
            (
                CopyFileGrantError::BindingMismatch,
                "capability_grant_binding_mismatch",
            ),
            (CopyFileGrantError::Replayed, "capability_grant_replayed"),
            (
                CopyFileGrantError::ClockRollback,
                "capability_clock_rollback",
            ),
            (
                CopyFileGrantError::ReplayCapacityExhausted,
                "capability_replay_capacity_exhausted",
            ),
            (
                CopyFileGrantError::StateUnavailable,
                "capability_state_unavailable",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.reason_code(), expected);
            assert_eq!(error.to_string(), expected);
            for private in [KEY, PRINCIPAL, SESSION, "source.bin", "source-payload"] {
                assert!(!error.to_string().contains(private));
            }
        }
    }

    #[test]
    fn equivalent_authorities_cannot_silently_change_replay_capacity() {
        CopyFileGrantAuthority::from_hex_key_with_capacity(
            "copy-capacity-contract",
            KEY,
            "copy-capacity-contract-principal",
            1,
        )
        .unwrap();
        assert_eq!(
            CopyFileGrantAuthority::from_hex_key_with_capacity(
                "copy-capacity-contract",
                KEY,
                "copy-capacity-contract-principal",
                2,
            )
            .unwrap_err(),
            CopyFileGrantError::StateUnavailable
        );
    }

    #[test]
    fn rejects_missing_malformed_unknown_and_invalid_signature_tokens() {
        let authority = authority();
        let token = authority.issue_at(SESSION, &target(), NOW).unwrap();
        assert_eq!(
            authority
                .consume_at(None, SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::Missing
        );
        for malformed in [
            String::new(),
            "v1".to_owned(),
            format!("v1.{}.bad.bad", authority.key_id),
            format!("v1.{}.é.bad", authority.key_id),
            "x".repeat(MAX_COPY_FILE_GRANT_HEADER_BYTES + 1),
        ] {
            assert_eq!(
                authority
                    .consume_at(Some(&malformed), SESSION, &target(), NOW)
                    .unwrap_err(),
                CopyFileGrantError::Malformed
            );
        }
        let unknown_version = token.replacen("v1.", "v2.", 1);
        assert_eq!(
            authority
                .consume_at(Some(&unknown_version), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::UnknownVersion
        );
        let unknown_key = token.replacen(&format!(".{}.", authority.key_id), ".retired-1.", 1);
        assert_eq!(
            authority
                .consume_at(Some(&unknown_key), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::UnknownKey
        );
        let mut invalid_signature = token.clone().into_bytes();
        let last = invalid_signature.last_mut().unwrap();
        *last = if *last == b'0' { b'1' } else { b'0' };
        assert_eq!(
            authority
                .consume_at(
                    Some(&String::from_utf8(invalid_signature).unwrap()),
                    SESSION,
                    &target(),
                    NOW,
                )
                .unwrap_err(),
            CopyFileGrantError::SignatureInvalid
        );

        let mut uppercase_payload = token;
        let payload_start = uppercase_payload
            .match_indices('.')
            .nth(1)
            .map(|(index, _)| index + 1)
            .unwrap();
        uppercase_payload.replace_range(payload_start..=payload_start, "A");
        assert_eq!(
            authority
                .consume_at(Some(&uppercase_payload), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::Malformed
        );
    }

    #[test]
    fn rejects_every_principal_session_source_destination_identity_size_and_digest_mismatch() {
        let authority = authority();
        let token = authority.issue_at(SESSION, &target(), NOW).unwrap();
        let other_principal =
            CopyFileGrantAuthority::from_hex_key(authority.key_id.to_string(), KEY, "other")
                .unwrap();
        assert_eq!(
            other_principal
                .consume_at(Some(&token), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::BindingMismatch
        );
        assert_eq!(
            authority
                .consume_at(
                    Some(&token),
                    "0194f9f9-bbbb-7ccc-8ddd-ffffffffffff",
                    &target(),
                    NOW,
                )
                .unwrap_err(),
            CopyFileGrantError::BindingMismatch
        );

        let mut mismatches = Vec::new();
        let mut changed = target();
        changed.source_root_device += 1;
        mismatches.push(changed);
        let mut changed = target();
        changed.source_root_inode += 1;
        mismatches.push(changed);
        let mut changed = target();
        changed.source_path_digest[0] ^= 1;
        mismatches.push(changed);
        let mut changed = target();
        changed.source_identity.device += 1;
        mismatches.push(changed);
        let mut changed = target();
        changed.source_identity.inode += 1;
        mismatches.push(changed);
        let mut changed = target();
        changed.source_identity.size += 1;
        mismatches.push(changed);
        let mut changed = target();
        changed.source_identity.ctime_seconds += 1;
        mismatches.push(changed);
        let mut changed = target();
        changed.source_identity.ctime_nanoseconds += 1;
        mismatches.push(changed);
        let mut changed = target();
        changed.source_identity.link_count = 2;
        mismatches.push(changed);
        let mut changed = target();
        changed.source_sha256[0] ^= 1;
        mismatches.push(changed);
        let mut changed = target();
        changed.destination_root_device += 1;
        mismatches.push(changed);
        let mut changed = target();
        changed.destination_root_inode += 1;
        mismatches.push(changed);
        let mut changed = target();
        changed.destination_path_digest[0] ^= 1;
        mismatches.push(changed);

        for mismatched_target in mismatches {
            assert_eq!(
                authority
                    .consume_at(Some(&token), SESSION, &mismatched_target, NOW)
                    .unwrap_err(),
                CopyFileGrantError::BindingMismatch
            );
        }

        // Binding failures cannot burn a still-valid grant.
        authority
            .consume_at(Some(&token), SESSION, &target(), NOW)
            .unwrap();
    }

    #[test]
    fn serialized_payload_is_fixed_size_opaque_unlinkable_and_binding_tamper_is_private() {
        let authority = authority();
        let token = authority.issue_at(SESSION, &target(), NOW).unwrap();
        let second = authority.issue_at(SESSION, &target(), NOW).unwrap();
        let payload = decode_hex_array::<PAYLOAD_BYTES>(token.split('.').nth(2).unwrap()).unwrap();
        let second_payload =
            decode_hex_array::<PAYLOAD_BYTES>(second.split('.').nth(2).unwrap()).unwrap();

        assert_eq!(PAYLOAD_BYTES, 65);
        assert_eq!(token.split('.').nth(2).unwrap().len(), 130);
        assert_eq!(
            &payload[ISSUED_OFFSET..ISSUED_OFFSET + 8],
            &NOW.to_be_bytes()
        );
        assert_eq!(
            &payload[PAYLOAD_BYTES - 8..],
            &(NOW + COPY_FILE_GRANT_TTL_SECONDS).to_be_bytes()
        );
        assert_ne!(
            &payload[..GRANT_ID_BYTES],
            &second_payload[..GRANT_ID_BYTES]
        );
        assert_ne!(
            &payload[BINDING_OFFSET..ISSUED_OFFSET],
            &second_payload[BINDING_OFFSET..ISSUED_OFFSET]
        );

        let parsed_session = parse_canonical_session(SESSION).unwrap();
        for private_binding in [
            authority.principal_digest.as_slice(),
            parsed_session.as_slice(),
            target().source_path_digest.as_slice(),
            target().source_sha256.as_slice(),
            target().destination_path_digest.as_slice(),
        ] {
            assert!(!payload[..ISSUED_OFFSET]
                .windows(private_binding.len())
                .any(|window| window == private_binding));
        }
        for raw_identity in [
            41_u64.to_be_bytes(),
            73_u64.to_be_bytes(),
            43_u64.to_be_bytes(),
            79_u64.to_be_bytes(),
            identity().device.to_be_bytes(),
            identity().inode.to_be_bytes(),
            identity().size.to_be_bytes(),
        ] {
            assert!(!payload[..ISSUED_OFFSET]
                .windows(raw_identity.len())
                .any(|window| window == raw_identity));
        }

        let tampered_binding = resign_payload(&authority, &token, |payload| {
            payload[BINDING_OFFSET] ^= 1;
        });
        assert_eq!(
            authority
                .consume_at(Some(&tampered_binding), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::BindingMismatch
        );
    }

    #[test]
    fn operation_binding_matches_independent_known_answer_vector() {
        let authority = authority();
        let session = parse_canonical_session(SESSION).unwrap();
        let grant_id = [0x5a; GRANT_ID_BYTES];
        let actual = authority
            .operation_binding_mac(&grant_id, &session, &target())
            .finalize()
            .into_bytes();
        assert_eq!(
            encode_hex(&actual),
            "bb83b15d198a1acc760ae477253e653a5a7653019a8fd89cd4338a5baf10e993"
        );
    }

    #[test]
    fn rejects_expired_future_excessive_lifetime_and_clock_rollback() {
        let authority = authority();
        let expired = authority.issue_at(SESSION, &target(), NOW - 60).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&expired), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::Expired
        );
        let future = authority.issue_at(SESSION, &target(), NOW + 6).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&future), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::FutureIssued
        );
        let normal = authority.issue_at(SESSION, &target(), NOW).unwrap();
        let zero_lifetime = resign_payload(&authority, &normal, |payload| {
            payload[PAYLOAD_BYTES - 8..].copy_from_slice(&NOW.to_be_bytes());
        });
        assert_eq!(
            authority
                .consume_at(Some(&zero_lifetime), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::LifetimeExceeded
        );
        let inverted_lifetime = resign_payload(&authority, &normal, |payload| {
            payload[PAYLOAD_BYTES - 8..].copy_from_slice(&(NOW - 1).to_be_bytes());
        });
        assert_eq!(
            authority
                .consume_at(Some(&inverted_lifetime), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::LifetimeExceeded
        );
        let excessive = resign_payload(&authority, &normal, |payload| {
            let expires_offset = PAYLOAD_BYTES - 8;
            payload[expires_offset..]
                .copy_from_slice(&(NOW + MAX_COPY_FILE_GRANT_LIFETIME_SECONDS + 1).to_be_bytes());
        });
        assert_eq!(
            authority
                .consume_at(Some(&excessive), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::LifetimeExceeded
        );

        authority
            .consume_at(Some(&normal), SESSION, &target(), NOW + 1)
            .unwrap();
        let rollback = authority.issue_at(SESSION, &target(), NOW).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&rollback), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::ClockRollback
        );
    }

    #[test]
    fn concurrent_replay_allows_exactly_one_consumer() {
        let authority = Arc::new(authority());
        let token = Arc::new(authority.issue_at(SESSION, &target(), NOW).unwrap());
        let barrier = Arc::new(Barrier::new(9));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let authority = Arc::clone(&authority);
            let token = Arc::clone(&token);
            let barrier = Arc::clone(&barrier);
            threads.push(thread::spawn(move || {
                barrier.wait();
                authority.consume_at(Some(&token), SESSION, &target(), NOW)
            }));
        }
        barrier.wait();
        let results = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(CopyFileGrantError::Replayed)))
                .count(),
            7
        );
    }

    #[test]
    fn clones_share_replay_state() {
        let authority = authority();
        let clone = authority.clone();
        let token = authority.issue_at(SESSION, &target(), NOW).unwrap();
        clone
            .consume_at(Some(&token), SESSION, &target(), NOW)
            .unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&token), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::Replayed
        );
    }

    #[test]
    fn independently_constructed_equivalent_authorities_share_replay_and_clock_state() {
        const RECONSTRUCTED_PRINCIPAL: &str = "copy-reconstruction-principal";
        let first = CopyFileGrantAuthority::from_hex_key(
            "copy-reconstruction",
            KEY,
            RECONSTRUCTED_PRINCIPAL,
        )
        .unwrap();
        let second = CopyFileGrantAuthority::from_hex_key(
            "copy-reconstruction",
            KEY,
            RECONSTRUCTED_PRINCIPAL,
        )
        .unwrap();

        let consumed = first.issue_at(SESSION, &target(), NOW).unwrap();
        second
            .consume_at(Some(&consumed), SESSION, &target(), NOW + 1)
            .unwrap();
        assert_eq!(
            first
                .consume_at(Some(&consumed), SESSION, &target(), NOW + 1)
                .unwrap_err(),
            CopyFileGrantError::Replayed
        );

        let rollback = second.issue_at(SESSION, &target(), NOW).unwrap();
        assert_eq!(
            first
                .consume_at(Some(&rollback), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::ClockRollback
        );
    }

    #[test]
    fn replay_storage_is_bounded_and_expired_entries_are_pruned() {
        let authority = CopyFileGrantAuthority::from_hex_key_with_capacity(
            "copy-capacity",
            KEY,
            "copy-capacity-principal",
            1,
        )
        .unwrap();
        let first = authority.issue_at(SESSION, &target(), NOW).unwrap();
        authority
            .consume_at(Some(&first), SESSION, &target(), NOW)
            .unwrap();
        let second = authority.issue_at(SESSION, &target(), NOW + 1).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&second), SESSION, &target(), NOW + 1)
                .unwrap_err(),
            CopyFileGrantError::ReplayCapacityExhausted
        );
        let after_expiry = authority
            .issue_at(SESSION, &target(), NOW + COPY_FILE_GRANT_TTL_SECONDS)
            .unwrap();
        authority
            .consume_at(
                Some(&after_expiry),
                SESSION,
                &target(),
                NOW + COPY_FILE_GRANT_TTL_SECONDS,
            )
            .unwrap();
    }

    #[test]
    fn poisoned_replay_state_returns_one_private_error() {
        let authority = authority();
        authority.replay.poison_for_test();
        let token = authority.issue_at(SESSION, &target(), NOW).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&token), SESSION, &target(), NOW)
                .unwrap_err(),
            CopyFileGrantError::StateUnavailable
        );
    }

    #[test]
    fn debug_output_redacts_key_principal_paths_source_identity_size_and_digest_binding() {
        let authority = authority();
        let serialized = format!("{authority:?} {:?}", target());
        for secret in [
            KEY,
            PRINCIPAL,
            "41",
            "73",
            "101",
            "202",
            "projects",
            "source.bin",
            "archive",
            "destination.bin",
            "source-payload",
        ] {
            assert!(!serialized.contains(secret));
        }
        assert!(serialized.contains("<redacted>"));
    }
}
