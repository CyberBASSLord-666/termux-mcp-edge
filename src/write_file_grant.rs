//! Request-scoped authorization for `write_file` mutation.
//!
//! Grants are short-lived HMAC-SHA-256 capabilities bound to one authenticated
//! static principal, one MCP session, one anchored safe-root identity, one
//! normalized root-relative target, the exact content, the create-or-replace
//! disposition, the complete preflight identity of a replaced file, and the
//! mutating posture. The serialized payload contains only a random grant ID, a
//! signed capability byte, keyed opaque operation binding, and issuance timestamps. It therefore does
//! not disclose stable principal, session, filesystem, path, or content
//! fingerprints. Grant material is never logged or exposed by runtime
//! responses, and a valid grant is atomically consumed before the first
//! filesystem mutation attempt.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::request_grant_capability::{
    RequestGrantCapability, MAX_REQUEST_GRANT_HEADER_BYTES, REQUEST_GRANT_HEADER,
};

type HmacSha256 = Hmac<Sha256>;

pub const WRITE_FILE_GRANT_HEADER: &str = REQUEST_GRANT_HEADER;
pub const WRITE_FILE_GRANT_VERSION: &str = "v1";
pub const WRITE_FILE_GRANT_TTL_SECONDS: u64 = 60;
pub const MAX_WRITE_FILE_GRANT_LIFETIME_SECONDS: u64 = 120;
pub const MAX_WRITE_FILE_GRANT_FUTURE_SKEW_SECONDS: u64 = 5;
pub const MAX_WRITE_FILE_GRANT_HEADER_BYTES: usize = MAX_REQUEST_GRANT_HEADER_BYTES;
pub const MAX_WRITE_FILE_GRANT_KEY_ID_BYTES: usize = 32;
pub const WRITE_FILE_GRANT_KEY_BYTES: usize = 32;
pub const WRITE_FILE_GRANT_KEY_HEX_BYTES: usize = WRITE_FILE_GRANT_KEY_BYTES * 2;
pub const MAX_CONSUMED_WRITE_FILE_GRANTS: usize = 4_096;

const MUTATING_POSTURE: u8 = 1;
const GRANT_ID_BYTES: usize = 16;
const DIGEST_BYTES: usize = 32;
const SESSION_BYTES: usize = 16;
const BINDING_BYTES: usize = 32;
const PAYLOAD_BYTES: usize = GRANT_ID_BYTES + 1 + BINDING_BYTES + 8 + 8;
const PAYLOAD_HEX_BYTES: usize = PAYLOAD_BYTES * 2;
const MAC_BYTES: usize = 32;
const MAC_HEX_BYTES: usize = MAC_BYTES * 2;
const TARGET_DIGEST_DOMAIN: &[u8] = b"termux-mcp:write-file-target:v1\0";
const PRINCIPAL_BINDING_DOMAIN: &[u8] = b"termux-mcp:write-file-principal:v1\0";
const OPERATION_BINDING_DOMAIN: &[u8] = b"termux-mcp:write-file-operation-binding:v1\0";

/// Whether one exact `write_file` mutation creates an absent target or
/// atomically replaces an existing regular file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteFileDisposition {
    Create,
    Replace,
}

impl WriteFileDisposition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Replace => "replace",
        }
    }

    const fn grant_code(self) -> u8 {
        match self {
            Self::Create => 1,
            Self::Replace => 2,
        }
    }
}

impl std::str::FromStr for WriteFileDisposition {
    type Err = WriteFileGrantError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "create" => Ok(Self::Create),
            "replace" => Ok(Self::Replace),
            _ => Err(WriteFileGrantError::TargetInvalid),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct WriteFileGrantTarget {
    root_device: u64,
    root_inode: u64,
    target_digest: [u8; DIGEST_BYTES],
    content_digest: [u8; DIGEST_BYTES],
    disposition: WriteFileDisposition,
    existing_identity: Option<WriteFileExistingIdentity>,
}

/// Descriptor-derived identity bound to a replacement authorization.
///
/// The high-resolution change time and size make inode reuse and same-inode
/// content changes fail closed. Live replacement additionally requires the
/// link count to remain exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WriteFileExistingIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) size: u64,
    pub(crate) ctime_seconds: i64,
    pub(crate) ctime_nanoseconds: i64,
    pub(crate) link_count: u64,
}

impl WriteFileExistingIdentity {
    pub(crate) fn new(
        device: u64,
        inode: u64,
        size: u64,
        ctime_seconds: i64,
        ctime_nanoseconds: i64,
        link_count: u64,
    ) -> Result<Self, WriteFileGrantError> {
        if link_count != 1 || !(0..1_000_000_000).contains(&ctime_nanoseconds) {
            return Err(WriteFileGrantError::TargetInvalid);
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

impl WriteFileGrantTarget {
    pub(crate) fn from_normalized_components<'a>(
        root_device: u64,
        root_inode: u64,
        components: impl IntoIterator<Item = &'a [u8]>,
        content_bytes: &[u8],
        disposition: WriteFileDisposition,
        existing_identity: Option<WriteFileExistingIdentity>,
    ) -> Result<Self, WriteFileGrantError> {
        if !matches!(
            (disposition, existing_identity),
            (WriteFileDisposition::Create, None) | (WriteFileDisposition::Replace, Some(_))
        ) {
            return Err(WriteFileGrantError::TargetInvalid);
        }
        let mut target_digest = Sha256::new();
        target_digest.update(TARGET_DIGEST_DOMAIN);
        let mut component_count = 0_u32;
        for component in components {
            let length =
                u32::try_from(component.len()).map_err(|_| WriteFileGrantError::TargetInvalid)?;
            target_digest.update(length.to_be_bytes());
            target_digest.update(component);
            component_count = component_count
                .checked_add(1)
                .ok_or(WriteFileGrantError::TargetInvalid)?;
        }
        if component_count == 0 {
            return Err(WriteFileGrantError::TargetInvalid);
        }
        target_digest.update(component_count.to_be_bytes());

        Ok(Self {
            root_device,
            root_inode,
            target_digest: target_digest.finalize().into(),
            content_digest: Sha256::digest(content_bytes).into(),
            disposition,
            existing_identity,
        })
    }

    #[doc(hidden)]
    pub fn ensure_distinct_source_identity(
        &self,
        device: u64,
        inode: u64,
    ) -> Result<(), WriteFileGrantError> {
        if self
            .existing_identity
            .is_some_and(|identity| identity.device == device && identity.inode == inode)
        {
            return Err(WriteFileGrantError::TargetInvalid);
        }
        Ok(())
    }

    #[cfg(test)]
    fn test(
        root_device: u64,
        root_inode: u64,
        components: &[&[u8]],
        content_bytes: &[u8],
        disposition: WriteFileDisposition,
        existing_identity: Option<WriteFileExistingIdentity>,
    ) -> Self {
        Self::from_normalized_components(
            root_device,
            root_inode,
            components.iter().copied(),
            content_bytes,
            disposition,
            existing_identity,
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
    capability: u8,
    operation_binding: [u8; BINDING_BYTES],
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
        // Derive the principal binding under the independent capability key.
        // A disclosed grant therefore cannot be used as an offline verifier
        // for guesses of a weak transport bearer token.
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

    pub fn issue(
        &self,
        session_id: &str,
        target: &WriteFileGrantTarget,
    ) -> Result<String, WriteFileGrantError> {
        self.issue_at(session_id, target, current_unix_seconds()?)
    }

    pub fn consume(
        &self,
        token: Option<&str>,
        session_id: &str,
        target: &WriteFileGrantTarget,
    ) -> Result<(), WriteFileGrantError> {
        self.consume_at(token, session_id, target, current_unix_seconds()?)
    }

    pub(crate) fn issue_at(
        &self,
        session_id: &str,
        target: &WriteFileGrantTarget,
        now_unix_seconds: u64,
    ) -> Result<String, WriteFileGrantError> {
        let session_id = parse_canonical_session(session_id)?;
        let expires_unix_seconds = now_unix_seconds
            .checked_add(WRITE_FILE_GRANT_TTL_SECONDS)
            .ok_or(WriteFileGrantError::LifetimeExceeded)?;
        let grant_id = *Uuid::new_v4().as_bytes();
        let grant = ParsedGrant {
            grant_id,
            capability: RequestGrantCapability::WriteFile.wire_code(),
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
        target: &WriteFileGrantTarget,
        now_unix_seconds: u64,
    ) -> Result<(), WriteFileGrantError> {
        let token = token.ok_or(WriteFileGrantError::Missing)?;
        let expected_session = parse_canonical_session(session_id)?;
        let grant = self.parse_and_verify(token)?;
        if grant.capability != RequestGrantCapability::WriteFile.wire_code() {
            return Err(WriteFileGrantError::BindingMismatch);
        }
        self.operation_binding_mac(&grant.grant_id, &expected_session, target)
            .verify_slice(&grant.operation_binding)
            .map_err(|_| WriteFileGrantError::BindingMismatch)?;

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
        let payload_hex = encode_hex(&encode_payload(grant));
        let signed = format!("{WRITE_FILE_GRANT_VERSION}.{}.{}", self.key_id, payload_hex);
        let mut mac = HmacSha256::new_from_slice(self.key.as_ref())
            .expect("fixed-size HMAC key is always valid");
        mac.update(signed.as_bytes());
        format!("{signed}.{}", encode_hex(&mac.finalize().into_bytes()))
    }

    fn operation_binding_mac(
        &self,
        grant_id: &[u8; GRANT_ID_BYTES],
        session_id: &[u8; SESSION_BYTES],
        target: &WriteFileGrantTarget,
    ) -> HmacSha256 {
        let mut binding = HmacSha256::new_from_slice(self.key.as_ref())
            .expect("fixed-size HMAC key is always valid");
        binding.update(OPERATION_BINDING_DOMAIN);
        binding.update(grant_id);
        binding.update(&self.principal_digest);
        binding.update(session_id);
        binding.update(&[
            RequestGrantCapability::WriteFile.wire_code(),
            MUTATING_POSTURE,
        ]);
        binding.update(&target.root_device.to_be_bytes());
        binding.update(&target.root_inode.to_be_bytes());
        binding.update(&target.target_digest);
        binding.update(&target.content_digest);
        binding.update(&[target.disposition.grant_code()]);
        match target.existing_identity {
            None => {
                binding.update(&[0]);
                binding.update(&[0; 56]);
            }
            Some(identity) => {
                binding.update(&[1]);
                binding.update(&identity.device.to_be_bytes());
                binding.update(&identity.inode.to_be_bytes());
                binding.update(&identity.size.to_be_bytes());
                binding.update(&identity.ctime_seconds.to_be_bytes());
                binding.update(&identity.ctime_nanoseconds.to_be_bytes());
                binding.update(&identity.link_count.to_be_bytes());
                // Keep the absent/present identity encodings fixed-width.
                binding.update(&[0; 8]);
            }
        }
        binding
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
        let payload =
            decode_hex_array::<PAYLOAD_BYTES>(payload_hex).ok_or(WriteFileGrantError::Malformed)?;
        let signature =
            decode_hex_array::<MAC_BYTES>(signature_hex).ok_or(WriteFileGrantError::Malformed)?;
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

fn current_unix_seconds() -> Result<u64, WriteFileGrantError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| WriteFileGrantError::ClockRollback)
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
        sync::{Arc, Barrier},
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

    fn authority() -> WriteFileGrantAuthority {
        WriteFileGrantAuthority::from_hex_key("primary-1", KEY, PRINCIPAL).unwrap()
    }

    fn identity() -> WriteFileExistingIdentity {
        WriteFileExistingIdentity::new(101, 202, 303, 1_700_000_000, 404_505_606, 1).unwrap()
    }

    fn target() -> WriteFileGrantTarget {
        WriteFileGrantTarget::test(
            41,
            73,
            &[b"projects", b"settings.json"],
            br#"{"enabled":true}"#,
            WriteFileDisposition::Replace,
            Some(identity()),
        )
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
    fn dispositions_have_exact_stable_names_codes_and_parsing() {
        assert_eq!(WriteFileDisposition::Create.as_str(), "create");
        assert_eq!(WriteFileDisposition::Replace.as_str(), "replace");
        assert_eq!(WriteFileDisposition::Create.grant_code(), 1);
        assert_eq!(WriteFileDisposition::Replace.grant_code(), 2);
        assert_eq!(
            "create".parse::<WriteFileDisposition>().unwrap(),
            WriteFileDisposition::Create
        );
        assert_eq!(
            "replace".parse::<WriteFileDisposition>().unwrap(),
            WriteFileDisposition::Replace
        );
        for invalid in ["", "Create", "overwrite", "replace "] {
            assert_eq!(
                invalid.parse::<WriteFileDisposition>().unwrap_err(),
                WriteFileGrantError::TargetInvalid
            );
        }
    }

    #[test]
    fn public_issue_and_consume_use_one_short_lived_single_use_grant() {
        let authority = authority();
        let token = authority.issue(SESSION, &target()).unwrap();
        assert!(token.len() <= MAX_WRITE_FILE_GRANT_HEADER_BYTES);
        assert_eq!(token.split('.').count(), 4);
        let payload_hex = token.split('.').nth(2).unwrap();
        let payload = decode_hex_array::<PAYLOAD_BYTES>(payload_hex).unwrap();
        assert_eq!(
            payload[CAPABILITY_OFFSET],
            RequestGrantCapability::WriteFile.wire_code()
        );
        authority.consume(Some(&token), SESSION, &target()).unwrap();
        assert_eq!(
            authority
                .consume(Some(&token), SESSION, &target())
                .unwrap_err(),
            WriteFileGrantError::Replayed
        );
    }

    #[test]
    fn normalized_target_binds_path_content_disposition_and_existing_identity() {
        let baseline = target();
        assert_ne!(
            baseline,
            WriteFileGrantTarget::test(
                41,
                73,
                &[b"projects-settings", b"json"],
                br#"{"enabled":true}"#,
                WriteFileDisposition::Replace,
                Some(identity()),
            )
        );
        assert_ne!(
            baseline,
            WriteFileGrantTarget::test(
                41,
                73,
                &[b"projects", b"settings.json"],
                br#"{"enabled":false}"#,
                WriteFileDisposition::Replace,
                Some(identity()),
            )
        );
        assert_ne!(
            baseline,
            WriteFileGrantTarget::test(
                41,
                73,
                &[b"projects", b"settings.json"],
                br#"{"enabled":true}"#,
                WriteFileDisposition::Create,
                None,
            )
        );
        assert_eq!(
            WriteFileGrantTarget::from_normalized_components(
                41,
                73,
                std::iter::empty::<&[u8]>(),
                b"content",
                WriteFileDisposition::Create,
                None,
            )
            .unwrap_err(),
            WriteFileGrantError::TargetInvalid
        );
        for (disposition, existing_identity) in [
            (WriteFileDisposition::Create, Some(identity())),
            (WriteFileDisposition::Replace, None),
        ] {
            assert_eq!(
                WriteFileGrantTarget::from_normalized_components(
                    41,
                    73,
                    [b"settings.json".as_slice()],
                    b"content",
                    disposition,
                    existing_identity,
                )
                .unwrap_err(),
                WriteFileGrantError::TargetInvalid
            );
        }
        assert_eq!(
            WriteFileExistingIdentity::new(1, 2, 3, 4, 1_000_000_000, 1).unwrap_err(),
            WriteFileGrantError::TargetInvalid
        );
        assert_eq!(
            WriteFileExistingIdentity::new(1, 2, 3, 4, 5, 2).unwrap_err(),
            WriteFileGrantError::TargetInvalid
        );
    }

    #[test]
    fn principal_binding_is_keyed_and_not_an_offline_bearer_verifier() {
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
    fn rejects_invalid_configuration_and_noncanonical_sessions() {
        for key_id in ["", "Primary", "bad.id", &"x".repeat(33)] {
            assert_eq!(
                WriteFileGrantAuthority::from_hex_key(key_id, KEY, PRINCIPAL).unwrap_err(),
                WriteFileGrantError::ConfigurationInvalid
            );
        }
        for key in ["", &"0".repeat(63), &"A".repeat(64)] {
            assert_eq!(
                WriteFileGrantAuthority::from_hex_key("primary-1", key, PRINCIPAL).unwrap_err(),
                WriteFileGrantError::ConfigurationInvalid
            );
        }
        assert_eq!(
            WriteFileGrantAuthority::from_hex_key("primary-1", KEY, "").unwrap_err(),
            WriteFileGrantError::ConfigurationInvalid
        );
        for session in ["", "not-a-uuid", "0194F9F9-BBBB-7CCC-8DDD-EEEEEEEEEEEE"] {
            assert_eq!(
                authority().issue_at(session, &target(), NOW).unwrap_err(),
                WriteFileGrantError::SessionInvalid
            );
        }
    }

    #[test]
    fn rejects_missing_malformed_unknown_and_invalid_signature_tokens() {
        let authority = authority();
        let token = authority.issue_at(SESSION, &target(), NOW).unwrap();
        assert_eq!(
            authority
                .consume_at(None, SESSION, &target(), NOW)
                .unwrap_err(),
            WriteFileGrantError::Missing
        );
        for malformed in [
            "",
            "v1",
            "v1.primary-1.bad.bad",
            &"x".repeat(MAX_WRITE_FILE_GRANT_HEADER_BYTES + 1),
        ] {
            assert_eq!(
                authority
                    .consume_at(Some(malformed), SESSION, &target(), NOW)
                    .unwrap_err(),
                WriteFileGrantError::Malformed
            );
        }
        let unknown_version = token.replacen("v1.", "v2.", 1);
        assert_eq!(
            authority
                .consume_at(Some(&unknown_version), SESSION, &target(), NOW)
                .unwrap_err(),
            WriteFileGrantError::UnknownVersion
        );
        let unknown_key = token.replacen(".primary-1.", ".retired-1.", 1);
        assert_eq!(
            authority
                .consume_at(Some(&unknown_key), SESSION, &target(), NOW)
                .unwrap_err(),
            WriteFileGrantError::UnknownKey
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
            WriteFileGrantError::SignatureInvalid
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
            WriteFileGrantError::Malformed
        );
    }

    #[test]
    fn rejects_every_principal_session_root_target_content_disposition_and_identity_mismatch() {
        let authority = authority();
        let token = authority.issue_at(SESSION, &target(), NOW).unwrap();
        let other_principal =
            WriteFileGrantAuthority::from_hex_key("primary-1", KEY, "other").unwrap();
        let mismatches = [
            other_principal.consume_at(Some(&token), SESSION, &target(), NOW),
            authority.consume_at(
                Some(&token),
                "0194f9f9-bbbb-7ccc-8ddd-ffffffffffff",
                &target(),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    42,
                    73,
                    &[b"projects", b"settings.json"],
                    br#"{"enabled":true}"#,
                    WriteFileDisposition::Replace,
                    Some(identity()),
                ),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    41,
                    74,
                    &[b"projects", b"settings.json"],
                    br#"{"enabled":true}"#,
                    WriteFileDisposition::Replace,
                    Some(identity()),
                ),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    41,
                    73,
                    &[b"projects", b"other.json"],
                    br#"{"enabled":true}"#,
                    WriteFileDisposition::Replace,
                    Some(identity()),
                ),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    41,
                    73,
                    &[b"projects", b"settings.json"],
                    br#"{"enabled":false}"#,
                    WriteFileDisposition::Replace,
                    Some(identity()),
                ),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    41,
                    73,
                    &[b"projects", b"settings.json"],
                    br#"{"enabled":true}"#,
                    WriteFileDisposition::Create,
                    None,
                ),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    41,
                    73,
                    &[b"projects", b"settings.json"],
                    br#"{"enabled":true}"#,
                    WriteFileDisposition::Replace,
                    Some(WriteFileExistingIdentity {
                        inode: 203,
                        ..identity()
                    }),
                ),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    41,
                    73,
                    &[b"projects", b"settings.json"],
                    br#"{"enabled":true}"#,
                    WriteFileDisposition::Replace,
                    Some(WriteFileExistingIdentity {
                        size: 304,
                        ..identity()
                    }),
                ),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    41,
                    73,
                    &[b"projects", b"settings.json"],
                    br#"{"enabled":true}"#,
                    WriteFileDisposition::Replace,
                    Some(WriteFileExistingIdentity {
                        ctime_seconds: 1_700_000_001,
                        ..identity()
                    }),
                ),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                &WriteFileGrantTarget::test(
                    41,
                    73,
                    &[b"projects", b"settings.json"],
                    br#"{"enabled":true}"#,
                    WriteFileDisposition::Replace,
                    Some(WriteFileExistingIdentity {
                        ctime_nanoseconds: 404_505_607,
                        ..identity()
                    }),
                ),
                NOW,
            ),
        ];
        for mismatch in mismatches {
            assert_eq!(mismatch.unwrap_err(), WriteFileGrantError::BindingMismatch);
        }
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
            &(NOW + WRITE_FILE_GRANT_TTL_SECONDS).to_be_bytes()
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
            target().target_digest.as_slice(),
            target().content_digest.as_slice(),
        ] {
            assert!(!payload[..ISSUED_OFFSET]
                .windows(private_binding.len())
                .any(|window| window == private_binding));
        }
        for raw_identity in [
            41_u64.to_be_bytes(),
            73_u64.to_be_bytes(),
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
            WriteFileGrantError::BindingMismatch
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
            "5a4bafd8b63d8c88c8a836362196ddd7136a7bdd6c6d24e46df3c9c5f2c32165"
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
            WriteFileGrantError::Expired
        );
        let future = authority.issue_at(SESSION, &target(), NOW + 6).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&future), SESSION, &target(), NOW)
                .unwrap_err(),
            WriteFileGrantError::FutureIssued
        );
        let normal = authority.issue_at(SESSION, &target(), NOW).unwrap();
        let excessive = resign_payload(&authority, &normal, |payload| {
            let expires_offset = PAYLOAD_BYTES - 8;
            payload[expires_offset..]
                .copy_from_slice(&(NOW + MAX_WRITE_FILE_GRANT_LIFETIME_SECONDS + 1).to_be_bytes());
        });
        assert_eq!(
            authority
                .consume_at(Some(&excessive), SESSION, &target(), NOW)
                .unwrap_err(),
            WriteFileGrantError::LifetimeExceeded
        );

        authority
            .consume_at(Some(&normal), SESSION, &target(), NOW + 1)
            .unwrap();
        let rollback = authority.issue_at(SESSION, &target(), NOW).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&rollback), SESSION, &target(), NOW)
                .unwrap_err(),
            WriteFileGrantError::ClockRollback
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
                .filter(|result| matches!(result, Err(WriteFileGrantError::Replayed)))
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
            WriteFileGrantError::Replayed
        );
    }

    #[test]
    fn replay_storage_is_bounded_and_expired_entries_are_pruned() {
        let authority =
            WriteFileGrantAuthority::from_hex_key_with_capacity("primary-1", KEY, PRINCIPAL, 1)
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
            WriteFileGrantError::ReplayCapacityExhausted
        );
        let after_expiry = authority
            .issue_at(SESSION, &target(), NOW + WRITE_FILE_GRANT_TTL_SECONDS)
            .unwrap();
        authority
            .consume_at(
                Some(&after_expiry),
                SESSION,
                &target(),
                NOW + WRITE_FILE_GRANT_TTL_SECONDS,
            )
            .unwrap();
    }

    #[test]
    fn poisoned_replay_state_returns_one_private_error() {
        let authority = authority();
        let replay = Arc::clone(&authority.replay);
        let _ = thread::spawn(move || {
            let _guard = replay.lock().unwrap();
            panic!("poison test replay lock");
        })
        .join();
        let token = authority.issue_at(SESSION, &target(), NOW).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&token), SESSION, &target(), NOW)
                .unwrap_err(),
            WriteFileGrantError::StateUnavailable
        );
    }

    #[test]
    fn debug_output_redacts_key_principal_target_content_and_disposition_binding() {
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
            "settings.json",
            "enabled",
            "replace",
        ] {
            assert!(!serialized.contains(secret));
        }
        assert!(serialized.contains("<redacted>"));
    }
}
