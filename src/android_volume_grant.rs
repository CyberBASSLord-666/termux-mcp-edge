//! Single-use request capabilities for Android volume mutation.
//!
//! A grant binds one authenticated static principal, one canonical MCP
//! session, the volume-control capability, one exact stream, one exact level,
//! and the mutating posture. Only keyed digests and random identifiers are
//! retained; raw credentials and grant strings are never serialized.

use std::{fmt, sync::Arc};

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use uuid::Uuid;

use crate::{
    android_volume_control::AndroidVolumeStreamName,
    grant_replay::{shared_replay_state, SharedReplayError, SharedReplayState},
    request_grant_capability::{
        RequestGrantCapability, MAX_REQUEST_GRANT_HEADER_BYTES, REQUEST_GRANT_HEADER,
    },
};

type HmacSha256 = Hmac<Sha256>;

pub const ANDROID_VOLUME_GRANT_HEADER: &str = REQUEST_GRANT_HEADER;
pub const ANDROID_VOLUME_GRANT_TTL_SECONDS: u64 = 60;
pub const MAX_ANDROID_VOLUME_GRANT_LIFETIME_SECONDS: u64 = 120;
pub const MAX_ANDROID_VOLUME_GRANT_FUTURE_SKEW_SECONDS: u64 = 5;
pub const MAX_ANDROID_VOLUME_GRANT_HEADER_BYTES: usize = MAX_REQUEST_GRANT_HEADER_BYTES;
pub const MAX_ANDROID_VOLUME_GRANT_KEY_ID_BYTES: usize = 32;
pub const ANDROID_VOLUME_GRANT_KEY_BYTES: usize = 32;
pub const ANDROID_VOLUME_GRANT_KEY_HEX_BYTES: usize = ANDROID_VOLUME_GRANT_KEY_BYTES * 2;
pub const MAX_CONSUMED_ANDROID_VOLUME_GRANTS: usize = 4_096;

const GRANT_VERSION: &str = "v1";
const MUTATING_POSTURE: u8 = 1;
const GRANT_ID_BYTES: usize = 16;
const DIGEST_BYTES: usize = 32;
const SESSION_BYTES: usize = 16;
const PAYLOAD_BYTES: usize = GRANT_ID_BYTES + DIGEST_BYTES + SESSION_BYTES + 1 + 1 + 8 + 1 + 8 + 8;
const PAYLOAD_HEX_BYTES: usize = PAYLOAD_BYTES * 2;
const MAC_BYTES: usize = 32;
const MAC_HEX_BYTES: usize = MAC_BYTES * 2;
const PRINCIPAL_BINDING_DOMAIN: &[u8] = b"termux-mcp:android-volume-principal:v1\0";

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AndroidVolumeGrantTarget {
    stream: AndroidVolumeStreamName,
    level: u64,
}

impl AndroidVolumeGrantTarget {
    pub fn new(
        stream: AndroidVolumeStreamName,
        level: i64,
    ) -> Result<Self, AndroidVolumeGrantError> {
        Ok(Self {
            stream,
            level: u64::try_from(level).map_err(|_| AndroidVolumeGrantError::TargetInvalid)?,
        })
    }
}

impl fmt::Debug for AndroidVolumeGrantTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AndroidVolumeGrantTarget")
            .field("binding", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndroidVolumeGrantError {
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

impl AndroidVolumeGrantError {
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

impl fmt::Display for AndroidVolumeGrantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason_code())
    }
}

impl std::error::Error for AndroidVolumeGrantError {}

#[derive(Clone)]
pub struct AndroidVolumeGrantAuthority {
    key_id: Arc<str>,
    key: Arc<[u8; ANDROID_VOLUME_GRANT_KEY_BYTES]>,
    principal_digest: [u8; DIGEST_BYTES],
    replay: SharedReplayState,
}

struct ParsedGrant {
    grant_id: [u8; GRANT_ID_BYTES],
    principal_digest: [u8; DIGEST_BYTES],
    session_id: [u8; SESSION_BYTES],
    capability: u8,
    stream: u8,
    level: u64,
    posture: u8,
    issued_unix_seconds: u64,
    expires_unix_seconds: u64,
}

impl AndroidVolumeGrantAuthority {
    pub fn from_hex_key(
        key_id: impl Into<String>,
        key_hex: &str,
        static_principal_secret: &str,
    ) -> Result<Self, AndroidVolumeGrantError> {
        Self::from_hex_key_with_capacity(
            key_id,
            key_hex,
            static_principal_secret,
            MAX_CONSUMED_ANDROID_VOLUME_GRANTS,
        )
    }

    fn from_hex_key_with_capacity(
        key_id: impl Into<String>,
        key_hex: &str,
        static_principal_secret: &str,
        replay_capacity: usize,
    ) -> Result<Self, AndroidVolumeGrantError> {
        let key_id = key_id.into();
        if !valid_key_id(&key_id)
            || key_hex.len() != ANDROID_VOLUME_GRANT_KEY_HEX_BYTES
            || static_principal_secret.is_empty()
            || replay_capacity == 0
        {
            return Err(AndroidVolumeGrantError::ConfigurationInvalid);
        }
        let key = decode_hex_array::<ANDROID_VOLUME_GRANT_KEY_BYTES>(key_hex)
            .ok_or(AndroidVolumeGrantError::ConfigurationInvalid)?;
        let mut principal =
            HmacSha256::new_from_slice(&key).expect("fixed-size HMAC key is always valid");
        principal.update(PRINCIPAL_BINDING_DOMAIN);
        principal.update(static_principal_secret.as_bytes());
        let principal_digest = principal.finalize().into_bytes().into();
        let replay = shared_replay_state(
            RequestGrantCapability::AndroidVolume,
            &key_id,
            &key,
            &principal_digest,
            replay_capacity,
        )
        .map_err(|_| AndroidVolumeGrantError::StateUnavailable)?;

        Ok(Self {
            key_id: Arc::from(key_id),
            key: Arc::new(key),
            principal_digest,
            replay,
        })
    }

    pub fn issue_at(
        &self,
        session_id: &str,
        target: AndroidVolumeGrantTarget,
        now_unix_seconds: u64,
    ) -> Result<String, AndroidVolumeGrantError> {
        let expires_unix_seconds = now_unix_seconds
            .checked_add(ANDROID_VOLUME_GRANT_TTL_SECONDS)
            .ok_or(AndroidVolumeGrantError::LifetimeExceeded)?;
        let grant = ParsedGrant {
            grant_id: *Uuid::new_v4().as_bytes(),
            principal_digest: self.principal_digest,
            session_id: parse_canonical_session(session_id)?,
            capability: RequestGrantCapability::AndroidVolume.wire_code(),
            stream: target.stream.grant_code(),
            level: target.level,
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
        target: AndroidVolumeGrantTarget,
        now_unix_seconds: u64,
    ) -> Result<(), AndroidVolumeGrantError> {
        let token = token.ok_or(AndroidVolumeGrantError::Missing)?;
        let expected_session = parse_canonical_session(session_id)?;
        let grant = self.parse_and_verify(token)?;
        if grant.principal_digest != self.principal_digest
            || grant.session_id != expected_session
            || grant.capability != RequestGrantCapability::AndroidVolume.wire_code()
            || grant.stream != target.stream.grant_code()
            || grant.level != target.level
            || grant.posture != MUTATING_POSTURE
        {
            return Err(AndroidVolumeGrantError::BindingMismatch);
        }

        self.replay
            .consume(
                grant.grant_id,
                grant.issued_unix_seconds,
                grant.expires_unix_seconds,
                now_unix_seconds,
                MAX_ANDROID_VOLUME_GRANT_LIFETIME_SECONDS,
                MAX_ANDROID_VOLUME_GRANT_FUTURE_SKEW_SECONDS,
            )
            .map_err(map_replay_error)
    }

    fn encode_and_sign(&self, grant: &ParsedGrant) -> String {
        let payload_hex = encode_hex(&encode_payload(grant));
        let signed = format!("{GRANT_VERSION}.{}.{}", self.key_id, payload_hex);
        let mut mac = HmacSha256::new_from_slice(self.key.as_ref())
            .expect("fixed-size HMAC key is always valid");
        mac.update(signed.as_bytes());
        format!("{signed}.{}", encode_hex(&mac.finalize().into_bytes()))
    }

    fn parse_and_verify(&self, token: &str) -> Result<ParsedGrant, AndroidVolumeGrantError> {
        if token.is_empty()
            || token.len() > MAX_ANDROID_VOLUME_GRANT_HEADER_BYTES
            || !token.is_ascii()
        {
            return Err(AndroidVolumeGrantError::Malformed);
        }
        let mut segments = token.split('.');
        let version = segments.next().ok_or(AndroidVolumeGrantError::Malformed)?;
        let key_id = segments.next().ok_or(AndroidVolumeGrantError::Malformed)?;
        let payload_hex = segments.next().ok_or(AndroidVolumeGrantError::Malformed)?;
        let signature_hex = segments.next().ok_or(AndroidVolumeGrantError::Malformed)?;
        if segments.next().is_some() {
            return Err(AndroidVolumeGrantError::Malformed);
        }
        if version != GRANT_VERSION {
            return Err(AndroidVolumeGrantError::UnknownVersion);
        }
        if key_id != self.key_id.as_ref() {
            return Err(AndroidVolumeGrantError::UnknownKey);
        }
        if payload_hex.len() != PAYLOAD_HEX_BYTES || signature_hex.len() != MAC_HEX_BYTES {
            return Err(AndroidVolumeGrantError::Malformed);
        }
        let payload = decode_hex_array::<PAYLOAD_BYTES>(payload_hex)
            .ok_or(AndroidVolumeGrantError::Malformed)?;
        let signature = decode_hex_array::<MAC_BYTES>(signature_hex)
            .ok_or(AndroidVolumeGrantError::Malformed)?;
        let signed_length = version.len() + 1 + key_id.len() + 1 + payload_hex.len();
        let signed = token
            .get(..signed_length)
            .ok_or(AndroidVolumeGrantError::Malformed)?;
        let mut mac = HmacSha256::new_from_slice(self.key.as_ref())
            .expect("fixed-size HMAC key is always valid");
        mac.update(signed.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| AndroidVolumeGrantError::SignatureInvalid)?;
        decode_payload(&payload).ok_or(AndroidVolumeGrantError::Malformed)
    }
}

impl fmt::Debug for AndroidVolumeGrantAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AndroidVolumeGrantAuthority")
            .field("key_id", &self.key_id)
            .field("key", &"<redacted>")
            .field("principal", &"<redacted>")
            .field("replay", &"<process-global>")
            .finish()
    }
}

const fn map_replay_error(error: SharedReplayError) -> AndroidVolumeGrantError {
    match error {
        SharedReplayError::Expired => AndroidVolumeGrantError::Expired,
        SharedReplayError::FutureIssued => AndroidVolumeGrantError::FutureIssued,
        SharedReplayError::LifetimeExceeded => AndroidVolumeGrantError::LifetimeExceeded,
        SharedReplayError::Replayed => AndroidVolumeGrantError::Replayed,
        SharedReplayError::ClockRollback => AndroidVolumeGrantError::ClockRollback,
        SharedReplayError::CapacityExhausted => AndroidVolumeGrantError::ReplayCapacityExhausted,
        SharedReplayError::StateUnavailable => AndroidVolumeGrantError::StateUnavailable,
    }
}

fn valid_key_id(key_id: &str) -> bool {
    !key_id.is_empty()
        && key_id.len() <= MAX_ANDROID_VOLUME_GRANT_KEY_ID_BYTES
        && key_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn parse_canonical_session(
    session_id: &str,
) -> Result<[u8; SESSION_BYTES], AndroidVolumeGrantError> {
    let parsed =
        Uuid::parse_str(session_id).map_err(|_| AndroidVolumeGrantError::SessionInvalid)?;
    if parsed.to_string() != session_id {
        return Err(AndroidVolumeGrantError::SessionInvalid);
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
    put(&mut payload, &mut offset, &[grant.stream]);
    put(&mut payload, &mut offset, &grant.level.to_be_bytes());
    put(&mut payload, &mut offset, &[grant.posture]);
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
    let principal_digest = take_array(payload, &mut offset)?;
    let session_id = take_array(payload, &mut offset)?;
    let capability = *payload.get(offset)?;
    offset += 1;
    let stream = *payload.get(offset)?;
    AndroidVolumeStreamName::from_grant_code(stream)?;
    offset += 1;
    let level = u64::from_be_bytes(take_array(payload, &mut offset)?);
    let posture = *payload.get(offset)?;
    offset += 1;
    let issued_unix_seconds = u64::from_be_bytes(take_array(payload, &mut offset)?);
    let expires_unix_seconds = u64::from_be_bytes(take_array(payload, &mut offset)?);
    (offset == PAYLOAD_BYTES).then_some(ParsedGrant {
        grant_id,
        principal_digest,
        session_id,
        capability,
        stream,
        level,
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

    fn test_authority(test_principal: &str) -> AndroidVolumeGrantAuthority {
        AndroidVolumeGrantAuthority::from_hex_key("primary-1", KEY, test_principal).unwrap()
    }

    fn target() -> AndroidVolumeGrantTarget {
        AndroidVolumeGrantTarget::new(AndroidVolumeStreamName::Music, 9).unwrap()
    }

    fn resign_payload(
        authority: &AndroidVolumeGrantAuthority,
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
    fn one_grant_is_exactly_bound_and_single_use() {
        let authority = test_authority("volume-single-use");
        let token = authority.issue_at(SESSION, target(), NOW).unwrap();
        assert!(token.len() <= MAX_ANDROID_VOLUME_GRANT_HEADER_BYTES);
        authority
            .consume_at(Some(&token), SESSION, target(), NOW)
            .unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&token), SESSION, target(), NOW)
                .unwrap_err(),
            AndroidVolumeGrantError::Replayed
        );
    }

    #[test]
    fn rejects_legacy_volume_code_two_without_consuming_the_grant() {
        const CAPABILITY_OFFSET: usize = GRANT_ID_BYTES + DIGEST_BYTES + SESSION_BYTES;

        let authority = test_authority("volume-legacy-code");
        let token = authority.issue_at(SESSION, target(), NOW).unwrap();
        let payload_hex = token.split('.').nth(2).unwrap();
        let payload = decode_hex_array::<PAYLOAD_BYTES>(payload_hex).unwrap();
        assert_eq!(
            payload[CAPABILITY_OFFSET],
            RequestGrantCapability::AndroidVolume.wire_code()
        );

        let legacy = resign_payload(&authority, &token, |payload| {
            payload[CAPABILITY_OFFSET] = 2;
        });
        assert_eq!(
            authority
                .consume_at(Some(&legacy), SESSION, target(), NOW)
                .unwrap_err(),
            AndroidVolumeGrantError::BindingMismatch
        );
        authority
            .consume_at(Some(&token), SESSION, target(), NOW)
            .unwrap();
    }

    #[test]
    fn every_principal_session_stream_and_level_mismatch_collapses() {
        let authority = test_authority("volume-binding");
        let token = authority.issue_at(SESSION, target(), NOW).unwrap();
        let other_principal =
            AndroidVolumeGrantAuthority::from_hex_key("primary-1", KEY, "other").unwrap();
        for result in [
            other_principal.consume_at(Some(&token), SESSION, target(), NOW),
            authority.consume_at(
                Some(&token),
                "0194f9f9-bbbb-7ccc-8ddd-ffffffffffff",
                target(),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                AndroidVolumeGrantTarget::new(AndroidVolumeStreamName::Ring, 9).unwrap(),
                NOW,
            ),
            authority.consume_at(
                Some(&token),
                SESSION,
                AndroidVolumeGrantTarget::new(AndroidVolumeStreamName::Music, 8).unwrap(),
                NOW,
            ),
        ] {
            assert_eq!(
                result.unwrap_err(),
                AndroidVolumeGrantError::BindingMismatch
            );
        }
    }

    #[test]
    fn rejects_missing_malformed_expired_future_and_invalid_signature() {
        let authority = test_authority("volume-malformed-temporal");
        assert_eq!(
            authority
                .consume_at(None, SESSION, target(), NOW)
                .unwrap_err(),
            AndroidVolumeGrantError::Missing
        );
        for malformed in ["", "v1", "v1.primary-1.bad.bad", &"x".repeat(385)] {
            assert_eq!(
                authority
                    .consume_at(Some(malformed), SESSION, target(), NOW)
                    .unwrap_err(),
                AndroidVolumeGrantError::Malformed
            );
        }
        let expired = authority.issue_at(SESSION, target(), NOW - 60).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&expired), SESSION, target(), NOW)
                .unwrap_err(),
            AndroidVolumeGrantError::Expired
        );
        let future = authority.issue_at(SESSION, target(), NOW + 6).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&future), SESSION, target(), NOW)
                .unwrap_err(),
            AndroidVolumeGrantError::FutureIssued
        );
        let token = authority.issue_at(SESSION, target(), NOW).unwrap();
        let mut bytes = token.into_bytes();
        let last = bytes.last_mut().unwrap();
        *last = if *last == b'0' { b'1' } else { b'0' };
        let invalid = String::from_utf8(bytes).unwrap();
        assert_eq!(
            authority
                .consume_at(Some(&invalid), SESSION, target(), NOW)
                .unwrap_err(),
            AndroidVolumeGrantError::SignatureInvalid
        );
    }

    #[test]
    fn independently_constructed_authorities_share_clock_rollback_state() {
        let first = test_authority("volume-shared-clock");
        let reconstructed = test_authority("volume-shared-clock");
        let accepted = first.issue_at(SESSION, target(), NOW).unwrap();
        first
            .consume_at(Some(&accepted), SESSION, target(), NOW + 1)
            .unwrap();
        let rollback = reconstructed.issue_at(SESSION, target(), NOW).unwrap();
        assert_eq!(
            reconstructed
                .consume_at(Some(&rollback), SESSION, target(), NOW)
                .unwrap_err(),
            AndroidVolumeGrantError::ClockRollback
        );
    }

    #[test]
    fn independently_constructed_authorities_reject_sequential_replay() {
        let issuer = test_authority("volume-shared-sequential");
        let consumer = test_authority("volume-shared-sequential");
        let token = issuer.issue_at(SESSION, target(), NOW).unwrap();
        consumer
            .consume_at(Some(&token), SESSION, target(), NOW)
            .unwrap();
        assert_eq!(
            issuer
                .consume_at(Some(&token), SESSION, target(), NOW)
                .unwrap_err(),
            AndroidVolumeGrantError::Replayed
        );
    }

    #[test]
    fn independent_concurrent_authorities_allow_exactly_one_consumer() {
        let first = test_authority("volume-shared-concurrent");
        let second = test_authority("volume-shared-concurrent");
        let token = Arc::new(first.issue_at(SESSION, target(), NOW).unwrap());
        let barrier = Arc::new(Barrier::new(3));
        let threads = [first, second].map(|authority| {
            let token = Arc::clone(&token);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                authority.consume_at(Some(&token), SESSION, target(), NOW)
            })
        });
        barrier.wait();
        let results = threads.map(|thread| thread.join().unwrap());
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(AndroidVolumeGrantError::Replayed)))
                .count(),
            1
        );
    }

    #[test]
    fn independently_constructed_authorities_share_capacity_and_pruning() {
        let first = AndroidVolumeGrantAuthority::from_hex_key_with_capacity(
            "primary-1",
            KEY,
            "volume-capacity",
            1,
        )
        .unwrap();
        let second = AndroidVolumeGrantAuthority::from_hex_key_with_capacity(
            "primary-1",
            KEY,
            "volume-capacity",
            1,
        )
        .unwrap();
        let consumed = first.issue_at(SESSION, target(), NOW).unwrap();
        first
            .consume_at(Some(&consumed), SESSION, target(), NOW)
            .unwrap();
        let exhausted = second.issue_at(SESSION, target(), NOW + 1).unwrap();
        assert_eq!(
            second
                .consume_at(Some(&exhausted), SESSION, target(), NOW + 1)
                .unwrap_err(),
            AndroidVolumeGrantError::ReplayCapacityExhausted
        );
        let after_expiry = second
            .issue_at(SESSION, target(), NOW + ANDROID_VOLUME_GRANT_TTL_SECONDS)
            .unwrap();
        second
            .consume_at(
                Some(&after_expiry),
                SESSION,
                target(),
                NOW + ANDROID_VOLUME_GRANT_TTL_SECONDS,
            )
            .unwrap();
    }

    #[test]
    fn different_keys_and_principals_keep_clock_and_replay_state_isolated() {
        let baseline = test_authority("volume-isolation-baseline");
        let baseline_token = baseline.issue_at(SESSION, target(), NOW).unwrap();
        baseline
            .consume_at(Some(&baseline_token), SESSION, target(), NOW + 1)
            .unwrap();

        let other_principal = test_authority("volume-isolation-other");
        let other_principal_token = other_principal.issue_at(SESSION, target(), NOW).unwrap();
        other_principal
            .consume_at(Some(&other_principal_token), SESSION, target(), NOW)
            .unwrap();

        let other_key = "11".repeat(ANDROID_VOLUME_GRANT_KEY_BYTES);
        let other_key_authority = AndroidVolumeGrantAuthority::from_hex_key(
            "primary-1",
            &other_key,
            "volume-isolation-baseline",
        )
        .unwrap();
        let other_key_token = other_key_authority
            .issue_at(SESSION, target(), NOW)
            .unwrap();
        other_key_authority
            .consume_at(Some(&other_key_token), SESSION, target(), NOW)
            .unwrap();
    }

    #[test]
    fn concurrent_replay_has_exactly_one_winner() {
        let authority = Arc::new(test_authority("volume-concurrent-clones"));
        let token = Arc::new(authority.issue_at(SESSION, target(), NOW).unwrap());
        let barrier = Arc::new(Barrier::new(9));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let authority = Arc::clone(&authority);
            let token = Arc::clone(&token);
            let barrier = Arc::clone(&barrier);
            threads.push(thread::spawn(move || {
                barrier.wait();
                authority.consume_at(Some(&token), SESSION, target(), NOW)
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
                .filter(|result| matches!(result, Err(AndroidVolumeGrantError::Replayed)))
                .count(),
            7
        );
    }

    #[test]
    fn debug_output_redacts_key_principal_stream_and_level() {
        let authority_debug = format!("{:?}", test_authority(PRINCIPAL));
        let target_debug = format!("{:?}", target());
        assert!(!authority_debug.contains(KEY));
        assert!(!authority_debug.contains(PRINCIPAL));
        assert!(!target_debug.contains("music"));
        assert!(!target_debug.contains('9'));
        assert!(authority_debug.contains("<redacted>"));
        assert!(target_debug.contains("<redacted>"));
    }
}
