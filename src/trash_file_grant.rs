//! Request-scoped authorization for reversible `trash_file` mutation.
//!
//! Grants are short-lived HMAC-SHA-256 capabilities bound to one authenticated
//! static principal, one MCP session, one lifetime-pinned safe-root identity,
//! one normalized root-relative target, the exact single-link regular-file
//! identity and content digest, and the fixed recovery-retaining posture. The
//! serialized payload contains only a random grant ID, a signed family byte, a
//! keyed opaque operation binding, and issuance timestamps. It therefore
//! discloses no stable principal, session, filesystem, path, identity, or
//! content fingerprint.

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

pub const TRASH_FILE_GRANT_HEADER: &str = REQUEST_GRANT_HEADER;
pub const TRASH_FILE_GRANT_VERSION: &str = "v1";
pub const TRASH_FILE_GRANT_TTL_SECONDS: u64 = 60;
pub const MAX_TRASH_FILE_GRANT_LIFETIME_SECONDS: u64 = 120;
pub const MAX_TRASH_FILE_GRANT_FUTURE_SKEW_SECONDS: u64 = 5;
pub const MAX_TRASH_FILE_GRANT_HEADER_BYTES: usize = MAX_REQUEST_GRANT_HEADER_BYTES;
pub const MAX_TRASH_FILE_GRANT_KEY_ID_BYTES: usize = 32;
pub const TRASH_FILE_GRANT_KEY_BYTES: usize = 32;
pub const TRASH_FILE_GRANT_KEY_HEX_BYTES: usize = TRASH_FILE_GRANT_KEY_BYTES * 2;
pub const MAX_CONSUMED_TRASH_FILE_GRANTS: usize = 4_096;

const MUTATING_POSTURE: u8 = 1;
const RECOVERY_RETAINED_POSTURE: u8 = 1;
const GRANT_ID_BYTES: usize = 16;
const DIGEST_BYTES: usize = 32;
const SESSION_BYTES: usize = 16;
const BINDING_BYTES: usize = 32;
const PAYLOAD_BYTES: usize = GRANT_ID_BYTES + 1 + BINDING_BYTES + 8 + 8;
const PAYLOAD_HEX_BYTES: usize = PAYLOAD_BYTES * 2;
const MAC_BYTES: usize = 32;
const MAC_HEX_BYTES: usize = MAC_BYTES * 2;
const TARGET_DIGEST_DOMAIN: &[u8] = b"termux-mcp:trash-file-target:v1\0";
const PRINCIPAL_BINDING_DOMAIN: &[u8] = b"termux-mcp:trash-file-principal:v1\0";
const OPERATION_BINDING_DOMAIN: &[u8] = b"termux-mcp:trash-file-operation-binding:v1\0";

/// Descriptor-derived identity bound to one reversible removal authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrashFileIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) size: u64,
    pub(crate) ctime_seconds: i64,
    pub(crate) ctime_nanoseconds: i64,
    pub(crate) link_count: u64,
}

impl TrashFileIdentity {
    pub(crate) fn new(
        device: u64,
        inode: u64,
        size: u64,
        ctime_seconds: i64,
        ctime_nanoseconds: i64,
        link_count: u64,
    ) -> Result<Self, TrashFileGrantError> {
        if link_count != 1 || !(0..1_000_000_000).contains(&ctime_nanoseconds) {
            return Err(TrashFileGrantError::TargetInvalid);
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

#[derive(Clone, PartialEq, Eq)]
pub struct TrashFileGrantTarget {
    root_device: u64,
    root_inode: u64,
    target_digest: [u8; DIGEST_BYTES],
    content_digest: [u8; DIGEST_BYTES],
    identity: TrashFileIdentity,
}

impl TrashFileGrantTarget {
    pub(crate) fn from_normalized_components<'a>(
        root_device: u64,
        root_inode: u64,
        components: impl IntoIterator<Item = &'a [u8]>,
        identity: TrashFileIdentity,
        content_digest: [u8; DIGEST_BYTES],
    ) -> Result<Self, TrashFileGrantError> {
        let mut target_digest = Sha256::new();
        target_digest.update(TARGET_DIGEST_DOMAIN);
        let mut component_count = 0_u32;
        for component in components {
            let length =
                u32::try_from(component.len()).map_err(|_| TrashFileGrantError::TargetInvalid)?;
            target_digest.update(length.to_be_bytes());
            target_digest.update(component);
            component_count = component_count
                .checked_add(1)
                .ok_or(TrashFileGrantError::TargetInvalid)?;
        }
        if component_count == 0 {
            return Err(TrashFileGrantError::TargetInvalid);
        }
        target_digest.update(component_count.to_be_bytes());

        Ok(Self {
            root_device,
            root_inode,
            target_digest: target_digest.finalize().into(),
            content_digest,
            identity,
        })
    }

    #[cfg(test)]
    fn test(
        root_device: u64,
        root_inode: u64,
        components: &[&[u8]],
        identity: TrashFileIdentity,
        content: &[u8],
    ) -> Self {
        Self::from_normalized_components(
            root_device,
            root_inode,
            components.iter().copied(),
            identity,
            Sha256::digest(content).into(),
        )
        .expect("test target must be valid")
    }
}

impl fmt::Debug for TrashFileGrantTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrashFileGrantTarget")
            .field("binding", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrashFileGrantError {
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

impl TrashFileGrantError {
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

impl fmt::Display for TrashFileGrantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason_code())
    }
}

impl std::error::Error for TrashFileGrantError {}

#[derive(Clone)]
pub struct TrashFileGrantAuthority {
    key_id: Arc<str>,
    key: Arc<[u8; TRASH_FILE_GRANT_KEY_BYTES]>,
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

impl TrashFileGrantAuthority {
    pub fn from_hex_key(
        key_id: impl Into<String>,
        key_hex: &str,
        static_principal_secret: &str,
    ) -> Result<Self, TrashFileGrantError> {
        Self::from_hex_key_with_capacity(
            key_id,
            key_hex,
            static_principal_secret,
            MAX_CONSUMED_TRASH_FILE_GRANTS,
        )
    }

    fn from_hex_key_with_capacity(
        key_id: impl Into<String>,
        key_hex: &str,
        static_principal_secret: &str,
        replay_capacity: usize,
    ) -> Result<Self, TrashFileGrantError> {
        let key_id = key_id.into();
        if !valid_key_id(&key_id)
            || key_hex.len() != TRASH_FILE_GRANT_KEY_HEX_BYTES
            || static_principal_secret.is_empty()
            || replay_capacity == 0
        {
            return Err(TrashFileGrantError::ConfigurationInvalid);
        }
        let key = decode_hex_array::<TRASH_FILE_GRANT_KEY_BYTES>(key_hex)
            .ok_or(TrashFileGrantError::ConfigurationInvalid)?;
        let mut principal =
            HmacSha256::new_from_slice(&key).expect("fixed-size HMAC key is always valid");
        principal.update(PRINCIPAL_BINDING_DOMAIN);
        principal.update(static_principal_secret.as_bytes());
        let principal_digest = principal.finalize().into_bytes().into();
        let replay = shared_replay_state(
            RequestGrantCapability::TrashFile,
            &key_id,
            &key,
            &principal_digest,
            replay_capacity,
        )
        .map_err(|_| TrashFileGrantError::StateUnavailable)?;

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
        target: &TrashFileGrantTarget,
    ) -> Result<String, TrashFileGrantError> {
        self.issue_at(session_id, target, current_unix_seconds()?)
    }

    pub fn consume(
        &self,
        token: Option<&str>,
        session_id: &str,
        target: &TrashFileGrantTarget,
    ) -> Result<(), TrashFileGrantError> {
        self.consume_at(token, session_id, target, current_unix_seconds()?)
    }

    pub(crate) fn issue_at(
        &self,
        session_id: &str,
        target: &TrashFileGrantTarget,
        now_unix_seconds: u64,
    ) -> Result<String, TrashFileGrantError> {
        let session_id = parse_canonical_session(session_id)?;
        let expires_unix_seconds = now_unix_seconds
            .checked_add(TRASH_FILE_GRANT_TTL_SECONDS)
            .ok_or(TrashFileGrantError::LifetimeExceeded)?;
        let grant_id = *Uuid::new_v4().as_bytes();
        let grant = ParsedGrant {
            grant_id,
            capability: RequestGrantCapability::TrashFile.wire_code(),
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
        target: &TrashFileGrantTarget,
        now_unix_seconds: u64,
    ) -> Result<(), TrashFileGrantError> {
        let token = token.ok_or(TrashFileGrantError::Missing)?;
        let expected_session = parse_canonical_session(session_id)?;
        let grant = self.parse_and_verify(token)?;
        if grant.capability != RequestGrantCapability::TrashFile.wire_code() {
            return Err(TrashFileGrantError::BindingMismatch);
        }
        self.operation_binding_mac(&grant.grant_id, &expected_session, target)
            .verify_slice(&grant.operation_binding)
            .map_err(|_| TrashFileGrantError::BindingMismatch)?;

        self.replay
            .consume(
                grant.grant_id,
                grant.issued_unix_seconds,
                grant.expires_unix_seconds,
                now_unix_seconds,
                MAX_TRASH_FILE_GRANT_LIFETIME_SECONDS,
                MAX_TRASH_FILE_GRANT_FUTURE_SKEW_SECONDS,
            )
            .map_err(map_replay_error)
    }

    fn encode_and_sign(&self, grant: &ParsedGrant) -> String {
        let payload_hex = encode_hex(&encode_payload(grant));
        let signed = format!("{TRASH_FILE_GRANT_VERSION}.{}.{}", self.key_id, payload_hex);
        let mut mac = HmacSha256::new_from_slice(self.key.as_ref())
            .expect("fixed-size HMAC key is always valid");
        mac.update(signed.as_bytes());
        format!("{signed}.{}", encode_hex(&mac.finalize().into_bytes()))
    }

    fn operation_binding_mac(
        &self,
        grant_id: &[u8; GRANT_ID_BYTES],
        session_id: &[u8; SESSION_BYTES],
        target: &TrashFileGrantTarget,
    ) -> HmacSha256 {
        let mut binding = HmacSha256::new_from_slice(self.key.as_ref())
            .expect("fixed-size HMAC key is always valid");
        binding.update(OPERATION_BINDING_DOMAIN);
        binding.update(grant_id);
        binding.update(&self.principal_digest);
        binding.update(session_id);
        binding.update(&[
            RequestGrantCapability::TrashFile.wire_code(),
            MUTATING_POSTURE,
            RECOVERY_RETAINED_POSTURE,
        ]);
        binding.update(&target.root_device.to_be_bytes());
        binding.update(&target.root_inode.to_be_bytes());
        binding.update(&target.target_digest);
        binding.update(&target.content_digest);
        binding.update(&target.identity.device.to_be_bytes());
        binding.update(&target.identity.inode.to_be_bytes());
        binding.update(&target.identity.size.to_be_bytes());
        binding.update(&target.identity.ctime_seconds.to_be_bytes());
        binding.update(&target.identity.ctime_nanoseconds.to_be_bytes());
        binding.update(&target.identity.link_count.to_be_bytes());
        binding
    }

    fn parse_and_verify(&self, token: &str) -> Result<ParsedGrant, TrashFileGrantError> {
        if token.is_empty() || token.len() > MAX_TRASH_FILE_GRANT_HEADER_BYTES || !token.is_ascii()
        {
            return Err(TrashFileGrantError::Malformed);
        }
        let mut segments = token.split('.');
        let version = segments.next().ok_or(TrashFileGrantError::Malformed)?;
        let key_id = segments.next().ok_or(TrashFileGrantError::Malformed)?;
        let payload_hex = segments.next().ok_or(TrashFileGrantError::Malformed)?;
        let signature_hex = segments.next().ok_or(TrashFileGrantError::Malformed)?;
        if segments.next().is_some() {
            return Err(TrashFileGrantError::Malformed);
        }
        if version != TRASH_FILE_GRANT_VERSION {
            return Err(TrashFileGrantError::UnknownVersion);
        }
        if key_id != self.key_id.as_ref() {
            return Err(TrashFileGrantError::UnknownKey);
        }
        if payload_hex.len() != PAYLOAD_HEX_BYTES || signature_hex.len() != MAC_HEX_BYTES {
            return Err(TrashFileGrantError::Malformed);
        }
        let payload =
            decode_hex_array::<PAYLOAD_BYTES>(payload_hex).ok_or(TrashFileGrantError::Malformed)?;
        let signature =
            decode_hex_array::<MAC_BYTES>(signature_hex).ok_or(TrashFileGrantError::Malformed)?;
        let signed_length = version.len() + 1 + key_id.len() + 1 + payload_hex.len();
        let signed = token
            .get(..signed_length)
            .ok_or(TrashFileGrantError::Malformed)?;
        let mut mac = HmacSha256::new_from_slice(self.key.as_ref())
            .expect("fixed-size HMAC key is always valid");
        mac.update(signed.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| TrashFileGrantError::SignatureInvalid)?;
        decode_payload(&payload).ok_or(TrashFileGrantError::Malformed)
    }
}

impl fmt::Debug for TrashFileGrantAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrashFileGrantAuthority")
            .field("key_id", &self.key_id)
            .field("key", &"<redacted>")
            .field("principal", &"<redacted>")
            .field("replay", &"<process-global>")
            .finish()
    }
}

const fn map_replay_error(error: SharedReplayError) -> TrashFileGrantError {
    match error {
        SharedReplayError::Expired => TrashFileGrantError::Expired,
        SharedReplayError::FutureIssued => TrashFileGrantError::FutureIssued,
        SharedReplayError::LifetimeExceeded => TrashFileGrantError::LifetimeExceeded,
        SharedReplayError::Replayed => TrashFileGrantError::Replayed,
        SharedReplayError::ClockRollback => TrashFileGrantError::ClockRollback,
        SharedReplayError::CapacityExhausted => TrashFileGrantError::ReplayCapacityExhausted,
        SharedReplayError::StateUnavailable => TrashFileGrantError::StateUnavailable,
    }
}

fn current_unix_seconds() -> Result<u64, TrashFileGrantError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| TrashFileGrantError::ClockRollback)
}

fn valid_key_id(key_id: &str) -> bool {
    !key_id.is_empty()
        && key_id.len() <= MAX_TRASH_FILE_GRANT_KEY_ID_BYTES
        && key_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn parse_canonical_session(session_id: &str) -> Result<[u8; SESSION_BYTES], TrashFileGrantError> {
    let parsed = Uuid::parse_str(session_id).map_err(|_| TrashFileGrantError::SessionInvalid)?;
    if parsed.to_string() != session_id {
        return Err(TrashFileGrantError::SessionInvalid);
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
    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const PRINCIPAL: &str = "trash-static-principal";
    const SESSION: &str = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    const NOW: u64 = 1_725_000_000;

    fn authority(principal: &str) -> TrashFileGrantAuthority {
        TrashFileGrantAuthority::from_hex_key("trash-primary-1", KEY, principal).unwrap()
    }

    fn identity(inode: u64) -> TrashFileIdentity {
        TrashFileIdentity::new(101, inode, 7, 1_700_000_000, 123_456_789, 1).unwrap()
    }

    fn target() -> TrashFileGrantTarget {
        TrashFileGrantTarget::test(
            41,
            73,
            &[b"projects", b"obsolete.bin"],
            identity(202),
            b"payload",
        )
    }

    #[test]
    fn exact_grant_is_single_use_across_equivalent_authorities() {
        let first = authority("trash-equivalent-authorities");
        let equivalent = authority("trash-equivalent-authorities");
        let token = first.issue_at(SESSION, &target(), NOW).unwrap();

        equivalent
            .consume_at(Some(&token), SESSION, &target(), NOW + 1)
            .unwrap();
        assert_eq!(
            first.consume_at(Some(&token), SESSION, &target(), NOW + 1),
            Err(TrashFileGrantError::Replayed)
        );
    }

    #[test]
    fn every_operation_binding_dimension_is_exact_and_non_consuming_on_mismatch() {
        let authority = authority("trash-binding-dimensions");
        let original = target();
        let token = authority.issue_at(SESSION, &original, NOW).unwrap();
        let mismatches = [
            TrashFileGrantTarget::test(
                42,
                73,
                &[b"projects", b"obsolete.bin"],
                identity(202),
                b"payload",
            ),
            TrashFileGrantTarget::test(
                41,
                74,
                &[b"projects", b"obsolete.bin"],
                identity(202),
                b"payload",
            ),
            TrashFileGrantTarget::test(
                41,
                73,
                &[b"projects", b"other.bin"],
                identity(202),
                b"payload",
            ),
            TrashFileGrantTarget::test(
                41,
                73,
                &[b"projects", b"obsolete.bin"],
                identity(203),
                b"payload",
            ),
            TrashFileGrantTarget::test(
                41,
                73,
                &[b"projects", b"obsolete.bin"],
                identity(202),
                b"changed",
            ),
        ];
        for mismatch in mismatches {
            assert_eq!(
                authority.consume_at(Some(&token), SESSION, &mismatch, NOW + 1),
                Err(TrashFileGrantError::BindingMismatch)
            );
        }
        assert_eq!(
            authority.consume_at(
                Some(&token),
                "0194f9f9-bbbb-7ccc-8ddd-ffffffffffff",
                &original,
                NOW + 1,
            ),
            Err(TrashFileGrantError::BindingMismatch)
        );
        authority
            .consume_at(Some(&token), SESSION, &original, NOW + 1)
            .unwrap();
    }

    #[test]
    fn time_signature_and_shape_fail_closed() {
        let expired_authority = authority("trash-expiry");
        let target = target();
        let expired = expired_authority.issue_at(SESSION, &target, NOW).unwrap();
        assert_eq!(
            expired_authority.consume_at(
                Some(&expired),
                SESSION,
                &target,
                NOW + TRASH_FILE_GRANT_TTL_SECONDS,
            ),
            Err(TrashFileGrantError::Expired)
        );
        let future_authority = authority("trash-future-and-shape");
        let future = future_authority
            .issue_at(SESSION, &target, NOW + 20)
            .unwrap();
        assert_eq!(
            future_authority.consume_at(Some(&future), SESSION, &target, NOW + 10),
            Err(TrashFileGrantError::FutureIssued)
        );
        let mut tampered = future_authority
            .issue_at(SESSION, &target, NOW + 20)
            .unwrap();
        let last = tampered.pop().unwrap();
        tampered.push(if last == '0' { '1' } else { '0' });
        assert_eq!(
            future_authority.consume_at(Some(&tampered), SESSION, &target, NOW + 20),
            Err(TrashFileGrantError::SignatureInvalid)
        );
        for malformed in [
            None,
            Some(""),
            Some("v1.too.short"),
            Some("not-ascii-\u{00e9}"),
        ] {
            let expected = if malformed.is_none() {
                TrashFileGrantError::Missing
            } else {
                TrashFileGrantError::Malformed
            };
            assert_eq!(
                future_authority.consume_at(malformed, SESSION, &target, NOW + 20),
                Err(expected)
            );
        }
    }

    #[test]
    fn lifetime_key_version_clock_and_capacity_fail_closed_without_cross_namespace_state() {
        let lifetime_authority = authority("trash-lifetime-shape");
        let binding = target();
        let valid = lifetime_authority.issue_at(SESSION, &binding, NOW).unwrap();
        let mut excessive_lifetime = lifetime_authority.parse_and_verify(&valid).unwrap();
        excessive_lifetime.expires_unix_seconds = excessive_lifetime
            .issued_unix_seconds
            .checked_add(MAX_TRASH_FILE_GRANT_LIFETIME_SECONDS + 1)
            .unwrap();
        let excessive_lifetime = lifetime_authority.encode_and_sign(&excessive_lifetime);
        assert_eq!(
            lifetime_authority.consume_at(Some(&excessive_lifetime), SESSION, &binding, NOW + 1,),
            Err(TrashFileGrantError::LifetimeExceeded)
        );

        let unknown_version = valid.replacen("v1.", "v2.", 1);
        assert_eq!(
            lifetime_authority.consume_at(Some(&unknown_version), SESSION, &binding, NOW + 1),
            Err(TrashFileGrantError::UnknownVersion)
        );
        let unknown_key = valid.replacen(".trash-primary-1.", ".trash-secondary-1.", 1);
        assert_eq!(
            lifetime_authority.consume_at(Some(&unknown_key), SESSION, &binding, NOW + 1),
            Err(TrashFileGrantError::UnknownKey)
        );

        let rollback_authority = authority("trash-clock-rollback");
        let first = rollback_authority.issue_at(SESSION, &binding, NOW).unwrap();
        let second = rollback_authority.issue_at(SESSION, &binding, NOW).unwrap();
        rollback_authority
            .consume_at(Some(&first), SESSION, &binding, NOW + 2)
            .unwrap();
        assert_eq!(
            rollback_authority.consume_at(Some(&second), SESSION, &binding, NOW + 1),
            Err(TrashFileGrantError::ClockRollback)
        );

        let limited = TrashFileGrantAuthority::from_hex_key_with_capacity(
            "trash-capacity-1",
            KEY,
            "trash-capacity-principal",
            1,
        )
        .unwrap();
        let first = limited.issue_at(SESSION, &binding, NOW).unwrap();
        let second = limited.issue_at(SESSION, &binding, NOW).unwrap();
        limited
            .consume_at(Some(&first), SESSION, &binding, NOW + 1)
            .unwrap();
        assert_eq!(
            limited.consume_at(Some(&second), SESSION, &binding, NOW + 1),
            Err(TrashFileGrantError::ReplayCapacityExhausted)
        );
    }

    #[test]
    fn concurrent_replay_has_exactly_one_atomic_consumer() {
        let authority = Arc::new(authority("trash-concurrent-replay"));
        let binding = Arc::new(target());
        let token = Arc::new(authority.issue_at(SESSION, &binding, NOW).unwrap());
        let barrier = Arc::new(std::sync::Barrier::new(9));
        let mut workers = Vec::new();
        for _ in 0..8 {
            let authority = Arc::clone(&authority);
            let binding = Arc::clone(&binding);
            let token = Arc::clone(&token);
            let barrier = Arc::clone(&barrier);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                authority.consume_at(Some(&token), SESSION, &binding, NOW + 1)
            }));
        }
        barrier.wait();

        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| **result == Err(TrashFileGrantError::Replayed))
                .count(),
            7
        );
    }

    #[test]
    fn configuration_target_and_diagnostics_are_private() {
        for invalid in [
            TrashFileGrantAuthority::from_hex_key("", KEY, PRINCIPAL),
            TrashFileGrantAuthority::from_hex_key("UPPER", KEY, PRINCIPAL),
            TrashFileGrantAuthority::from_hex_key("trash-primary-2", "00", PRINCIPAL),
            TrashFileGrantAuthority::from_hex_key("trash-primary-3", KEY, ""),
        ] {
            assert!(matches!(
                invalid,
                Err(TrashFileGrantError::ConfigurationInvalid)
            ));
        }
        assert_eq!(
            TrashFileIdentity::new(1, 2, 3, 4, -1, 1),
            Err(TrashFileGrantError::TargetInvalid)
        );
        assert_eq!(
            TrashFileIdentity::new(1, 2, 3, 4, 0, 2),
            Err(TrashFileGrantError::TargetInvalid)
        );
        assert!(matches!(
            TrashFileGrantTarget::from_normalized_components(
                1,
                2,
                std::iter::empty::<&[u8]>(),
                identity(3),
                [0; DIGEST_BYTES],
            ),
            Err(TrashFileGrantError::TargetInvalid)
        ));

        let authority_debug = format!("{:?}", authority(PRINCIPAL));
        let target_debug = format!("{:?}", target());
        assert!(authority_debug.contains("<redacted>"));
        assert!(authority_debug.contains("<process-global>"));
        assert!(!authority_debug.contains(KEY));
        assert!(!authority_debug.contains(PRINCIPAL));
        assert_eq!(
            target_debug,
            "TrashFileGrantTarget { binding: \"<redacted>\" }"
        );
        for error in [
            TrashFileGrantError::Missing,
            TrashFileGrantError::BindingMismatch,
            TrashFileGrantError::Replayed,
        ] {
            assert_eq!(error.to_string(), error.reason_code());
            assert!(!error.to_string().contains("trash-static-principal"));
            assert!(!error.to_string().contains("obsolete.bin"));
        }
    }
}
