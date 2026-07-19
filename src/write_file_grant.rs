//! Request-scoped authorization for `write_file` mutation.
//!
//! Grants are short-lived HMAC-SHA-256 capabilities bound to one authenticated
//! static principal, one MCP session, one anchored safe-root identity, one
//! normalized root-relative target, the exact UTF-8 content digest, the
//! create-versus-replace publication posture, and the mutating posture. The
//! runtime never serializes grant material and atomically consumes a valid
//! grant immediately before the first filesystem mutation attempt.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
};

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

pub const WRITE_FILE_GRANT_HEADER: &str = "mcp-capability-grant";
pub const WRITE_FILE_GRANT_VERSION: &str = "v1";
pub const WRITE_FILE_GRANT_TTL_SECONDS: u64 = 60;
pub const MAX_WRITE_FILE_GRANT_LIFETIME_SECONDS: u64 = 120;
pub const MAX_WRITE_FILE_GRANT_FUTURE_SKEW_SECONDS: u64 = 5;
pub const MAX_WRITE_FILE_GRANT_HEADER_BYTES: usize = 384;
pub const MAX_WRITE_FILE_GRANT_KEY_ID_BYTES: usize = 32;
pub const WRITE_FILE_GRANT_KEY_BYTES: usize = 32;
pub const WRITE_FILE_GRANT_KEY_HEX_BYTES: usize = WRITE_FILE_GRANT_KEY_BYTES * 2;
pub const MAX_CONSUMED_WRITE_FILE_GRANTS: usize = 4_096;

const WRITE_FILE_CAPABILITY: u8 = 2;
const MUTATING_POSTURE: u8 = 1;
const CREATE_PUBLICATION: u8 = 1;
const REPLACE_PUBLICATION: u8 = 2;
const GRANT_ID_BYTES: usize = 16;
const DIGEST_BYTES: usize = 32;
const SESSION_BYTES: usize = 16;
const PAYLOAD_BYTES: usize = GRANT_ID_BYTES
    + DIGEST_BYTES
    + SESSION_BYTES
    + 1
    + 8
    + 8
    + DIGEST_BYTES
    + 1
    + 8
    + 8;
const PAYLOAD_HEX_BYTES: usize = PAYLOAD_BYTES * 2;
const MAC_BYTES: usize = 32;
const MAC_HEX_BYTES: usize = MAC_BYTES * 2;
const TARGET_DIGEST_DOMAIN: &[u8] = b"termux-mcp:write-file-target:v1\0";
const PRINCIPAL_BINDING_DOMAIN: &[u8] = b"termux-mcp:static-principal:v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteFilePublication {
    Create,
    Replace,
}

impl WriteFilePublication {
    const fn encoded(self) -> u8 {
        match self {
            Self::Create => CREATE_PUBLICATION,
            Self::Replace => REPLACE_PUBLICATION,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Replace => "replace",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct WriteFileGrantTarget {
    root_device: u64,
    root_inode: u64,
    operation_digest: [u8; DIGEST_BYTES],
}

impl WriteFileGrantTarget {
    pub(crate) fn from_normalized_components<'a>(
        root_device: u64,
        root_inode: u64,
        components: impl IntoIterator<Item = &'a [u8]>,
        content_digest: [u8; DIGEST_BYTES],
        publication: WriteFilePublication,
    ) -> Result<Self, WriteFileGrantError> {
        let mut digest = Sha256::new();
        digest.update(TARGET_DIGEST_DOMAIN);
        let mut component_count = 0_u32;
        for component in components {
            let length = u32::try_from(component.len())
                .map_err(|_| WriteFileGrantError::TargetInvalid)?;
            digest.update(length.to_be_bytes());
            digest.update(component);
            component_count = component_count
                .checked_add(1)
                .ok_or(WriteFileGrantError::TargetInvalid)?;
        }
        if component_count == 0 {
            return Err(WriteFileGrantError::TargetInvalid);
        }
        digest.update(component_count.to_be_bytes());
        digest.update(content_digest);
        digest.update([publication.encoded(), MUTATING_POSTURE]);
        Ok(Self {
            root_device,
            root_inode,
            operation_digest: digest.finalize().into(),
        })
    }

    #[cfg(test)]
    fn test(
        root_device: u64,
        root_inode: u64,
        components: &[&[u8]],
        content: &[u8],
        publication: WriteFilePublication,
    ) -> Self {
        Self::from_normalized_components(
            root_device,
            root_inode,
            components.iter().copied(),
            content_sha256(content),
            publication,
        )
        .expect("test target must be valid")
    }
}

impl fmt::Debug for WriteFileGrantTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WriteFileGrantTarget")
            .field("binding", &"<redacted>")
            .finish()
    }
}

pub fn content_sha256(content: &[u8]) -> [u8; DIGEST_BYTES] {
    Sha256::digest(content).into()
}

pub fn parse_content_sha256_hex(
    value: &str,
) -> Result<[u8; DIGEST_BYTES], WriteFileGrantError> {
    decode_hex_array::<DIGEST_BYTES>(value).ok_or(WriteFileGrantError::TargetInvalid)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteFileGrantError {
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

impl WriteFileGrantError {
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

impl fmt::Display for WriteFileGrantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason_code())
    }
}

impl std::error::Error for WriteFileGrantError {}

#[derive(Clone)]
pub struct WriteFileGrantAuthority {
    key_id: Arc<str>,
    key: Arc<[u8; WRITE_FILE_GRANT_KEY_BYTES]>,
    principal_digest: [u8; DIGEST_BYTES],
    replay: Arc<Mutex<ReplayState>>,
    replay_capacity: usize,
}

#[derive(Default)]
struct ReplayState {
    consumed: BTreeMap<[u8; GRANT_ID_BYTES], u64>,
    last_observed_unix_seconds: Option<u64>,
}

struct ParsedGrant {
    grant_id: [u8; GRANT_ID_BYTES],
    principal_digest: [u8; DIGEST_BYTES],
    session_id: [u8; SESSION_BYTES],
    capability: u8,
    root_device: u64,
    root_inode: u64,
    operation_digest: [u8; DIGEST_BYTES],
    posture: u8,
    issued_unix_seconds: u64,
    expires_unix_seconds: u64,
}

impl WriteFileGrantAuthority {
    pub fn from_hex_key(
        key_id: impl Into<String>,
        key_hex: &str,
        static_principal_secret: &str,
    ) -> Result<Self, WriteFileGrantError> {
        Self::from_hex_key_with_capacity(
            key_id,
            key_hex,
            static_principal_secret,
            MAX_CONSUMED_WRITE_FILE_GRANTS,
        )
    }

    fn from_hex_key_with_capacity(
        key_id: impl Into<String>,
        key_hex: &str,
        static_principal_secret: &str,
        replay_capacity: usize,
    ) -> Result<Self, WriteFileGrantError> {
        let key_id = key_id.into();
        if !valid_key_id(&key_id)
            || key_hex.len() != WRITE_FILE_GRANT_KEY_HEX_BYTES
            || static_principal_secret.is_empty()
            || replay_capacity == 0
        {
            return Err(WriteFileGrantError::ConfigurationInvalid);
        }
        let key = decode_hex_array::<WRITE_FILE_GRANT_KEY_BYTES>(key_hex)
            .ok_or(WriteFileGrantError::ConfigurationInvalid)?;
        let mut principal =
            HmacSha256::new_from_slice(&key).expect("fixed-size HMAC key is always valid");
        principal.update(PRINCIPAL_BINDING_DOMAIN);
        principal.update(static_principal_secret.as_bytes());
        let principal_digest = principal.finalize().into_bytes().into();

        Ok(Self {
            key_id: Arc::from(key_id),
            key: Arc::new(key),
            principal_digest,
            replay: Arc::new(Mutex::new(ReplayState::default())),
            replay_capacity,
        })
    }

    pub fn issue_at(
        &self,
        session_id: &str,
        target: &WriteFileGrantTarget,
        now_unix_seconds: u64,
    ) -> Result<String, WriteFileGrantError> {
        let session_id = parse_canonical_session(session_id)?;
        let expires_unix_seconds = now_unix_seconds
            .checked_add(WRITE_FILE_GRANT_TTL_SECONDS)
            .ok_or(WriteFileGrantError::LifetimeExceeded)?;
        let grant = ParsedGrant {
            grant_id: *Uuid::new_v4().as_bytes(),
            principal_digest: self.principal_digest,
            session_id,
            capability: WRITE_FILE_CAPABILITY,
            root_device: target.root_device,
            root_inode: target.root_inode,
            operation_digest: target.operation_digest,
            posture: MUTATING_POSTURE,
            issued_unix_seconds: now_unix_seconds,
            expires_unix_seconds,
        };
        Ok(self.encode_and_sign(&grant))
    }

    pub fn consume_at(
        &self,
        token: Option<&str>,
        session_id: &str,
        target: &WriteFileGrantTarget,
        now_unix_seconds: u64,
    ) -> Result<(), WriteFileGrantError> {
        let token = token.ok_or(WriteFileGrantError::Missing)?;
        let expected_session = parse_canonical_session(session_id)?;
        let grant = self.parse_and_verify(token)?;

        if grant.principal_digest != self.principal_digest
            || grant.session_id != expected_session
            || grant.capability != WRITE_FILE_CAPABILITY
            || grant.root_device != target.root_device
            || grant.root_inode != target.root_inode
            || grant.operation_digest != target.operation_digest
            || grant.posture != MUTATING_POSTURE
        {
            return Err(WriteFileGrantError::BindingMismatch);
        }

        let mut replay = self
            .replay
            .lock()
            .map_err(|_| WriteFileGrantError::StateUnavailable)?;
        if replay
            .last_observed_unix_seconds
            .is_some_and(|last| now_unix_seconds < last)
        {
            return Err(WriteFileGrantError::ClockRollback);
        }
        replay.last_observed_unix_seconds = Some(now_unix_seconds);

        let lifetime = grant
            .expires_unix_seconds
            .checked_sub(grant.issued_unix_seconds)
            .ok_or(WriteFileGrantError::LifetimeExceeded)?;
        if lifetime == 0 || lifetime > MAX_WRITE_FILE_GRANT_LIFETIME_SECONDS {
            return Err(WriteFileGrantError::LifetimeExceeded);
        }
        if grant.issued_unix_seconds
            > now_unix_seconds.saturating_add(MAX_WRITE_FILE_GRANT_FUTURE_SKEW_SECONDS)
        {
            return Err(WriteFileGrantError::FutureIssued);
        }
        if now_unix_seconds >= grant.expires_unix_seconds {
            return Err(WriteFileGrantError::Expired);
        }

        replay
            .consumed
            .retain(|_, expiry| *expiry > now_unix_seconds);
        if replay.consumed.contains_key(&grant.grant_id) {
            return Err(WriteFileGrantError::Replayed);
        }
        if replay.consumed.len() >= self.replay_capacity {
            return Err(WriteFileGrantError::ReplayCapacityExhausted);
        }
        replay
            .consumed
            .insert(grant.grant_id, grant.expires_unix_seconds);
        Ok(())
    }

    fn encode_and_sign(&self, grant: &ParsedGrant) -> String {
        let payload = encode_payload(grant);
        let payload_hex = encode_hex(&payload);
        let signed = format!("{WRITE_FILE_GRANT_VERSION}.{}.{}", self.key_id, payload_hex);
        let mut mac = HmacSha256::new_from_slice(self.key.as_ref())
            .expect("fixed-size HMAC key is always valid");
        mac.update(signed.as_bytes());
        format!("{signed}.{}", encode_hex(&mac.finalize().into_bytes()))
    }

    fn parse_and_verify(&self, token: &str) -> Result<ParsedGrant, WriteFileGrantError> {
        if token.is_empty() || token.len() > MAX_WRITE_FILE_GRANT_HEADER_BYTES || !token.is_ascii()
        {
            return Err(WriteFileGrantError::Malformed);
        }
        let mut segments = token.split('.');
        let version = segments.next().ok_or(WriteFileGrantError::Malformed)?;
        let key_id = segments.next().ok_or(WriteFileGrantError::Malformed)?;
        let payload_hex = segments.next().ok_or(WriteFileGrantError::Malformed)?;
        let signature_hex = segments.next().ok_or(WriteFileGrantError::Malformed)?;
        if segments.next().is_some() {
            return Err(WriteFileGrantError::Malformed);
        }
        if version != WRITE_FILE_GRANT_VERSION {
            return Err(WriteFileGrantError::UnknownVersion);
        }
        if key_id != self.key_id.as_ref() {
            return Err(WriteFileGrantError::UnknownKey);
        }
        if payload_hex.len() != PAYLOAD_HEX_BYTES || signature_hex.len() != MAC_HEX_BYTES {
            return Err(WriteFileGrantError::Malformed);
        }
        let payload = decode_hex_array::<PAYLOAD_BYTES>(payload_hex)
            .ok_or(WriteFileGrantError::Malformed)?;
        let signature = decode_hex_array::<MAC_BYTES>(signature_hex)
            .ok_or(WriteFileGrantError::Malformed)?;
        let signed_length = version.len() + 1 + key_id.len() + 1 + payload_hex.len();
        let signed = token
            .get(..signed_length)
            .ok_or(WriteFileGrantError::Malformed)?;
        let mut mac = HmacSha256::new_from_slice(self.key.as_ref())
            .expect("fixed-size HMAC key is always valid");
        mac.update(signed.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| WriteFileGrantError::SignatureInvalid)?;
        decode_payload(&payload).ok_or(WriteFileGrantError::Malformed)
    }
}

impl fmt::Debug for WriteFileGrantAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WriteFileGrantAuthority")
            .field("key_id", &self.key_id)
            .field("key", &"<redacted>")
            .field("principal", &"<redacted>")
            .field("replay_capacity", &self.replay_capacity)
            .finish()
    }
}

fn valid_key_id(key_id: &str) -> bool {
    !key_id.is_empty()
        && key_id.len() <= MAX_WRITE_FILE_GRANT_KEY_ID_BYTES
        && key_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn parse_canonical_session(session_id: &str) -> Result<[u8; SESSION_BYTES], WriteFileGrantError> {
    let parsed = Uuid::parse_str(session_id).map_err(|_| WriteFileGrantError::SessionInvalid)?;
    if parsed.to_string() != session_id {
        return Err(WriteFileGrantError::SessionInvalid);
    }
    Ok(*parsed.as_bytes())
}

fn encode_payload(grant: &ParsedGrant) -> [u8; PAYLOAD_BYTES] {
    let mut payload = [0_u8; PAYLOAD_BYTES];
    let mut offset = 0;
    put(&mut payload, &mut offset, &grant.grant_id);
    put(&mut payload, &mut offset, &grant.principal_digest);
    put(&mut payload, &mut offset, &grant.session_id);
    put(&mut payload, &mut offset, &[grant.capability]);
    put(&mut payload, &mut offset, &grant.root_device.to_be_bytes());
    put(&mut payload, &mut offset, &grant.root_inode.to_be_bytes());
    put(&mut payload, &mut offset, &grant.operation_digest);
    put(&mut payload, &mut offset, &[grant.posture]);
    put(&mut payload, &mut offset, &grant.issued_unix_seconds.to_be_bytes());
    put(&mut payload, &mut offset, &grant.expires_unix_seconds.to_be_bytes());
    debug_assert_eq!(offset, PAYLOAD_BYTES);
    payload
}

fn decode_payload(payload: &[u8; PAYLOAD_BYTES]) -> Option<ParsedGrant> {
    let mut offset = 0;
    let grant_id = take_array(payload, &mut offset)?;
    let principal_digest = take_array(payload, &mut offset)?;
    let session_id = take_array(payload, &mut offset)?;
    let capability = *payload.get(offset)?;
    offset += 1;
    let root_device = u64::from_be_bytes(take_array(payload, &mut offset)?);
    let root_inode = u64::from_be_bytes(take_array(payload, &mut offset)?);
    let operation_digest = take_array(payload, &mut offset)?;
    let posture = *payload.get(offset)?;
    offset += 1;
    let issued_unix_seconds = u64::from_be_bytes(take_array(payload, &mut offset)?);
    let expires_unix_seconds = u64::from_be_bytes(take_array(payload, &mut offset)?);
    (offset == PAYLOAD_BYTES).then_some(ParsedGrant {
        grant_id,
        principal_digest,
        session_id,
        capability,
        root_device,
        root_inode,
        operation_digest,
        posture,
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
        sync::{Arc, Barrier},
        thread,
    };

    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const PRINCIPAL: &str = "static-principal-secret";
    const SESSION: &str = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    const NOW: u64 = 1_725_000_000;

    fn authority() -> WriteFileGrantAuthority {
        WriteFileGrantAuthority::from_hex_key("primary-1", KEY, PRINCIPAL).unwrap()
    }

    fn target(publication: WriteFilePublication) -> WriteFileGrantTarget {
        WriteFileGrantTarget::test(41, 73, &[b"projects", b"file.txt"], b"content", publication)
    }

    fn resign_payload(
        authority: &WriteFileGrantAuthority,
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
    fn principal_binding_is_keyed_and_does_not_expose_a_bearer_token_verifier() {
        let authority = authority();
        let mut unkeyed = Sha256::new();
        unkeyed.update(PRINCIPAL_BINDING_DOMAIN);
        unkeyed.update(PRINCIPAL.as_bytes());
        let unkeyed: [u8; DIGEST_BYTES] = unkeyed.finalize().into();
        assert_ne!(authority.principal_digest, unkeyed);

        let other_key = "11".repeat(WRITE_FILE_GRANT_KEY_BYTES);
        let other =
            WriteFileGrantAuthority::from_hex_key("primary-1", &other_key, PRINCIPAL).unwrap();
        assert_ne!(authority.principal_digest, other.principal_digest);
    }

    #[test]
    fn rejects_invalid_authority_configuration() {
        let invalid_configurations = [
            WriteFileGrantAuthority::from_hex_key("", KEY, PRINCIPAL),
            WriteFileGrantAuthority::from_hex_key("PRIMARY", KEY, PRINCIPAL),
            WriteFileGrantAuthority::from_hex_key("primary.1", KEY, PRINCIPAL),
            WriteFileGrantAuthority::from_hex_key(
                "a".repeat(MAX_WRITE_FILE_GRANT_KEY_ID_BYTES + 1),
                KEY,
                PRINCIPAL,
            ),
            WriteFileGrantAuthority::from_hex_key("primary-1", &"0".repeat(63), PRINCIPAL),
            WriteFileGrantAuthority::from_hex_key("primary-1", &"G".repeat(64), PRINCIPAL),
            WriteFileGrantAuthority::from_hex_key("primary-1", KEY, ""),
            WriteFileGrantAuthority::from_hex_key_with_capacity(
                "primary-1",
                KEY,
                PRINCIPAL,
                0,
            ),
        ];
        for invalid in invalid_configurations {
            assert_eq!(
                invalid.unwrap_err(),
                WriteFileGrantError::ConfigurationInvalid
            );
        }
    }

    #[test]
    fn issues_and_consumes_one_fixed_shape_grant() {
        let authority = authority();
        let target = target(WriteFilePublication::Create);
        let token = authority.issue_at(SESSION, &target, NOW).unwrap();
        assert!(token.len() <= MAX_WRITE_FILE_GRANT_HEADER_BYTES);
        assert_eq!(token.split('.').count(), 4);
        authority.consume_at(Some(&token), SESSION, &target, NOW).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&token), SESSION, &target, NOW)
                .unwrap_err(),
            WriteFileGrantError::Replayed
        );
    }

    #[test]
    fn every_principal_session_root_target_content_and_publication_mismatch_is_private() {
        let authority = authority();
        let create_target = target(WriteFilePublication::Create);
        let token = authority.issue_at(SESSION, &create_target, NOW).unwrap();
        let other_principal =
            WriteFileGrantAuthority::from_hex_key("primary-1", KEY, "other").unwrap();
        let mismatches = [
            other_principal.consume_at(Some(&token), SESSION, &create_target, NOW),
            authority.consume_at(
                Some(&token),
                "0194f9f9-bbbb-7ccc-8ddd-ffffffffffff",
                &create_target,
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    42,
                    73,
                    &[b"projects", b"file.txt"],
                    b"content",
                    WriteFilePublication::Create,
                ),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    41,
                    74,
                    &[b"projects", b"file.txt"],
                    b"content",
                    WriteFilePublication::Create,
                ),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    41,
                    73,
                    &[b"projects", b"other.txt"],
                    b"content",
                    WriteFilePublication::Create,
                ),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    41,
                    73,
                    &[b"projects", b"file.txt"],
                    b"different",
                    WriteFilePublication::Create,
                ),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &target(WriteFilePublication::Replace),
                NOW,
            ),
        ];
        for mismatch in mismatches {
            assert_eq!(mismatch.unwrap_err(), WriteFileGrantError::BindingMismatch);
        }
    }

    #[test]
    fn rejects_missing_malformed_unknown_and_invalid_signature_tokens() {
        let authority = authority();
        let target = target(WriteFilePublication::Create);
        let token = authority.issue_at(SESSION, &target, NOW).unwrap();
        assert_eq!(
            authority.consume_at(None, SESSION, &target, NOW).unwrap_err(),
            WriteFileGrantError::Missing
        );
        for malformed in ["", "v1", "v1.primary-1.bad.bad", &"x".repeat(385)] {
            assert_eq!(
                authority
                    .consume_at(Some(malformed), SESSION, &target, NOW)
                    .unwrap_err(),
                WriteFileGrantError::Malformed
            );
        }
        let unknown_version = token.replacen("v1.", "v2.", 1);
        assert_eq!(
            authority
                .consume_at(Some(&unknown_version), SESSION, &target, NOW)
                .unwrap_err(),
            WriteFileGrantError::UnknownVersion
        );
        let unknown_key = token.replacen(".primary-1.", ".retired-1.", 1);
        assert_eq!(
            authority
                .consume_at(Some(&unknown_key), SESSION, &target, NOW)
                .unwrap_err(),
            WriteFileGrantError::UnknownKey
        );

        let mut invalid_signature = token.into_bytes();
        let last = invalid_signature.last_mut().unwrap();
        *last = if *last == b'0' { b'1' } else { b'0' };
        let invalid_signature = String::from_utf8(invalid_signature).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&invalid_signature), SESSION, &target, NOW)
                .unwrap_err(),
            WriteFileGrantError::SignatureInvalid
        );
    }

    #[test]
    fn rejects_malformed_and_noncanonical_sessions() {
        let authority = authority();
        let target = target(WriteFilePublication::Create);
        for invalid_session in [
            "",
            "not-a-session",
            "0194F9F9-BBBB-7CCC-8DDD-EEEEEEEEEEEE",
        ] {
            assert_eq!(
                authority
                    .issue_at(invalid_session, &target, NOW)
                    .unwrap_err(),
                WriteFileGrantError::SessionInvalid
            );
        }

        let token = authority.issue_at(SESSION, &target, NOW).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&token), "not-a-session", &target, NOW)
                .unwrap_err(),
            WriteFileGrantError::SessionInvalid
        );
    }

    #[test]
    fn rejects_expired_future_zero_or_excessive_lifetime_and_clock_rollback() {
        let authority = authority();
        let target = target(WriteFilePublication::Create);
        let expired = authority.issue_at(SESSION, &target, NOW - 60).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&expired), SESSION, &target, NOW)
                .unwrap_err(),
            WriteFileGrantError::Expired
        );
        let future = authority.issue_at(SESSION, &target, NOW + 6).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&future), SESSION, &target, NOW)
                .unwrap_err(),
            WriteFileGrantError::FutureIssued
        );

        let normal = authority.issue_at(SESSION, &target, NOW).unwrap();
        let zero_lifetime = resign_payload(&authority, &normal, |payload| {
            let expires_offset = PAYLOAD_BYTES - 8;
            payload[expires_offset..].copy_from_slice(&NOW.to_be_bytes());
        });
        assert_eq!(
            authority
                .consume_at(Some(&zero_lifetime), SESSION, &target, NOW)
                .unwrap_err(),
            WriteFileGrantError::LifetimeExceeded
        );
        let excessive_lifetime = resign_payload(&authority, &normal, |payload| {
            let expires_offset = PAYLOAD_BYTES - 8;
            payload[expires_offset..].copy_from_slice(
                &(NOW + MAX_WRITE_FILE_GRANT_LIFETIME_SECONDS + 1).to_be_bytes(),
            );
        });
        assert_eq!(
            authority
                .consume_at(Some(&excessive_lifetime), SESSION, &target, NOW)
                .unwrap_err(),
            WriteFileGrantError::LifetimeExceeded
        );

        authority
            .consume_at(Some(&normal), SESSION, &target, NOW + 1)
            .unwrap();
        let rollback = authority.issue_at(SESSION, &target, NOW).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&rollback), SESSION, &target, NOW)
                .unwrap_err(),
            WriteFileGrantError::ClockRollback
        );
    }

    #[test]
    fn concurrent_replay_allows_exactly_one_consumer() {
        let authority = Arc::new(authority());
        let target = Arc::new(target(WriteFilePublication::Create));
        let token = Arc::new(authority.issue_at(SESSION, &target, NOW).unwrap());
        let barrier = Arc::new(Barrier::new(9));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let authority = Arc::clone(&authority);
            let target = Arc::clone(&target);
            let token = Arc::clone(&token);
            let barrier = Arc::clone(&barrier);
            threads.push(thread::spawn(move || {
                barrier.wait();
                authority.consume_at(Some(&token), SESSION, &target, NOW)
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
                .filter(|result| matches!(result, Err(WriteFileGrantError::Replayed)))
                .count(),
            7
        );
    }

    #[test]
    fn replay_storage_is_bounded_and_expired_entries_are_pruned() {
        let authority = WriteFileGrantAuthority::from_hex_key_with_capacity(
            "primary-1",
            KEY,
            PRINCIPAL,
            1,
        )
        .unwrap();
        let target = target(WriteFilePublication::Create);
        let first = authority.issue_at(SESSION, &target, NOW).unwrap();
        authority
            .consume_at(Some(&first), SESSION, &target, NOW)
            .unwrap();
        let second = authority.issue_at(SESSION, &target, NOW + 1).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&second), SESSION, &target, NOW + 1)
                .unwrap_err(),
            WriteFileGrantError::ReplayCapacityExhausted
        );
        let after_expiry = authority
            .issue_at(SESSION, &target, NOW + WRITE_FILE_GRANT_TTL_SECONDS)
            .unwrap();
        authority
            .consume_at(
                Some(&after_expiry),
                SESSION,
                &target,
                NOW + WRITE_FILE_GRANT_TTL_SECONDS,
            )
            .unwrap();
    }

    #[test]
    fn digest_parser_and_debug_output_are_strict_and_private() {
        assert_eq!(
            parse_content_sha256_hex(&encode_hex(&content_sha256(b"content"))).unwrap(),
            content_sha256(b"content")
        );
        for invalid in ["", &"a".repeat(63), &"A".repeat(64), &"g".repeat(64)] {
            assert_eq!(
                parse_content_sha256_hex(invalid).unwrap_err(),
                WriteFileGrantError::TargetInvalid
            );
        }
        let authority = authority();
        let target = target(WriteFilePublication::Replace);
        let token = authority.issue_at(SESSION, &target, NOW).unwrap();
        let debug = format!("{authority:?} {target:?}");
        assert!(!debug.contains(KEY));
        assert!(!debug.contains(PRINCIPAL));
        assert!(!debug.contains(SESSION));
        assert!(!debug.contains(&token));
        assert!(!debug.contains(&encode_hex(&authority.principal_digest)));
        assert!(!debug.contains(&encode_hex(&target.operation_digest)));
        assert!(!debug.contains(&encode_hex(&content_sha256(b"content"))));
        assert!(!debug.contains("41"));
        assert!(!debug.contains("73"));
        assert!(!debug.contains("content"));
        assert!(!debug.contains("file.txt"));
        assert!(debug.contains("<redacted>"));
    }
}
