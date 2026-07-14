//! Crash-durable, bounded, single-use replay consumption for directory grants.
//!
//! This subsystem is intentionally loaded and validated before it is connected to
//! filesystem mutation. It stores only keyed, domain-separated grant-ID digests and
//! minimal bounded metadata. Raw grant IDs, credentials, verification keys, paths,
//! principals, and sessions never enter the ledger.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fmt,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, bail, Context};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use rustix::{
    fd::{AsFd, OwnedFd},
    fs::{
        self as descriptor_fs, AtFlags, FileType, FlockOperation, Mode, OFlags,
    },
    process,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    config::DirectoryGrantConfig,
    directory_grant::{DirectoryGrantVerifier, VerifiedDirectoryGrant},
};

pub const MAX_REPLAY_KEYRING_BYTES: usize = 65_536;
pub const MAX_REPLAY_KEYS: usize = 16;
pub const MAX_REPLAY_KEY_BYTES: usize = 64;
pub const MIN_REPLAY_KEY_BYTES: usize = 32;
pub const MAX_REPLAY_KEY_ID_BYTES: usize = 64;
pub const REPLAY_RECORD_BYTES: usize = 224;
pub const REPLAY_HEADER_BYTES: usize = 16;

const LEDGER_MAGIC: &[u8; 8] = b"TMCPRPL1";
const RECORD_MAGIC: &[u8; 4] = b"RPL1";
const LEDGER_VERSION: u8 = 1;
const RECORD_VERSION: u8 = 1;
const RECORD_KIND_CONSUME: u8 = 1;
const RECORD_KIND_WATERMARK: u8 = 2;
const KEY_ID_FIELD_BYTES: usize = 64;
const DIGEST_BYTES: usize = 32;
const MAC_BYTES: usize = 32;
const RECORD_AUTHENTICATED_BYTES: usize = REPLAY_RECORD_BYTES - MAC_BYTES;
const DIGEST_DOMAIN: &[u8] = b"termux-mcp-directory-replay-digest-v1\0";
const RECORD_DOMAIN: &[u8] = b"termux-mcp-directory-replay-record-v1\0";

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct DirectoryReplayLedger {
    inner: Arc<ReplayLedgerInner>,
    process_mutex: Arc<Mutex<()>>,
}

struct ReplayLedgerInner {
    directory: PathBuf,
    ledger_name: OsString,
    lock_name: OsString,
    keyring: ReplayKeyring,
    max_records: usize,
    max_bytes: usize,
}

impl fmt::Debug for DirectoryReplayLedger {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectoryReplayLedger")
            .field("directory", &"<redacted>")
            .field("ledger_name", &"<redacted>")
            .field("lock_name", &"<redacted>")
            .field("replay_key_count", &self.inner.keyring.keys.len())
            .field("replay_keys", &"<redacted>")
            .field("max_records", &self.inner.max_records)
            .field("max_bytes", &self.inner.max_bytes)
            .finish()
    }
}

struct ReplayKeyring {
    active_key_id: String,
    keys: BTreeMap<String, ReplayKey>,
    fingerprints: BTreeSet<[u8; DIGEST_BYTES]>,
}

struct ReplayKey {
    material: Zeroizing<Vec<u8>>,
    not_before_unix_seconds: u64,
    digest_until_unix_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReplayKeyring {
    version: u8,
    active_kid: String,
    keys: Vec<RawReplayKey>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReplayKey {
    kid: String,
    key_b64url: String,
    not_before_unix_seconds: u64,
    digest_until_unix_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayConsumeError {
    ReplayDetected,
    ClockRollback,
    CapacityExhausted,
    CorruptLedger,
    KeyUnavailable,
    StorageUnavailable,
}

impl ReplayConsumeError {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::ReplayDetected => "directory_grant_replay_rejected",
            Self::ClockRollback => "directory_replay_clock_rollback",
            Self::CapacityExhausted => "directory_replay_capacity_exhausted",
            Self::CorruptLedger => "directory_replay_ledger_corrupt",
            Self::KeyUnavailable => "directory_replay_key_unavailable",
            Self::StorageUnavailable => "directory_replay_storage_unavailable",
        }
    }
}

impl fmt::Display for ReplayConsumeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("directory grant replay consumption failed")
    }
}

impl std::error::Error for ReplayConsumeError {}

#[derive(Clone)]
struct ParsedRecord {
    raw: [u8; REPLAY_RECORD_BYTES],
    kind: u8,
    grant_version: u8,
    replay_key_id: String,
    verification_key_id: String,
    retention_until_unix_seconds: u64,
    observed_unix_seconds: u64,
    digest: [u8; DIGEST_BYTES],
}

struct LedgerSnapshot {
    records: Vec<ParsedRecord>,
    high_water_unix_seconds: u64,
}

impl DirectoryReplayLedger {
    pub fn load_optional(
        config: &DirectoryGrantConfig,
        authentication_token: Option<&str>,
        verifier: Option<&DirectoryGrantVerifier>,
        now_unix_seconds: u64,
    ) -> anyhow::Result<Option<Self>> {
        if !config.replay_enabled {
            return Ok(None);
        }
        let keyring_path = config
            .replay_keyring_path
            .as_deref()
            .ok_or_else(|| anyhow!("directory replay configuration is incomplete"))?;
        let ledger_path = config
            .replay_ledger_path
            .as_deref()
            .ok_or_else(|| anyhow!("directory replay configuration is incomplete"))?;
        let keyring = load_replay_keyring(
            keyring_path,
            authentication_token,
            verifier,
            now_unix_seconds,
            config.max_lifetime_seconds,
            config.clock_skew_seconds,
        )?;
        let (directory, ledger_name, lock_name) = split_ledger_path(ledger_path)?;
        let ledger = Self {
            inner: Arc::new(ReplayLedgerInner {
                directory,
                ledger_name,
                lock_name,
                keyring,
                max_records: config.replay_max_records,
                max_bytes: config.replay_max_bytes,
            }),
            process_mutex: Arc::new(Mutex::new(())),
        };
        ledger.validate_or_initialize(now_unix_seconds)?;
        Ok(Some(ledger))
    }

    /// Atomically consume a verified grant identifier before mutation.
    ///
    /// Once this returns `Ok(())`, the grant remains consumed even if every later
    /// filesystem step fails or the process is terminated.
    pub fn consume(
        &self,
        grant: &VerifiedDirectoryGrant,
        now_unix_seconds: u64,
    ) -> Result<(), ReplayConsumeError> {
        let _process_guard = self
            .process_mutex
            .lock()
            .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
        let directory_fd = open_owner_only_directory(&self.inner.directory)
            .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
        let lock_fd = open_owner_only_file_at(
            &directory_fd,
            &self.inner.lock_name,
            true,
        )
        .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
        descriptor_fs::flock(&lock_fd, FlockOperation::LockExclusive)
            .map_err(|_| ReplayConsumeError::StorageUnavailable)?;

        let result = self.consume_locked(&directory_fd, grant, now_unix_seconds);
        let _ = descriptor_fs::flock(&lock_fd, FlockOperation::Unlock);
        result
    }

    fn validate_or_initialize(&self, now: u64) -> anyhow::Result<()> {
        let _process_guard = self
            .process_mutex
            .lock()
            .map_err(|_| anyhow!("directory replay process lock is unavailable"))?;
        let directory_fd = open_owner_only_directory(&self.inner.directory)?;
        let lock_fd = open_owner_only_file_at(&directory_fd, &self.inner.lock_name, true)?;
        descriptor_fs::flock(&lock_fd, FlockOperation::LockExclusive)
            .map_err(|_| anyhow!("directory replay lock could not be acquired"))?;
        let result = (|| {
            let mut file = open_ledger_file(&directory_fd, &self.inner.ledger_name)?;
            initialize_header_if_empty(&mut file, &directory_fd)?;
            let snapshot = self
                .scan_ledger(&mut file)
                .map_err(|_| anyhow!("directory replay ledger failed validation"))?;
            if now < snapshot.high_water_unix_seconds {
                bail!("directory replay ledger clock rollback detected");
            }
            Ok(())
        })();
        let _ = descriptor_fs::flock(&lock_fd, FlockOperation::Unlock);
        result
    }

    fn consume_locked(
        &self,
        directory_fd: &OwnedFd,
        grant: &VerifiedDirectoryGrant,
        now: u64,
    ) -> Result<(), ReplayConsumeError> {
        let mut file = open_ledger_file(directory_fd, &self.inner.ledger_name)
            .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
        initialize_header_if_empty(&mut file, directory_fd)
            .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
        let mut snapshot = self.scan_ledger(&mut file)?;
        if now < snapshot.high_water_unix_seconds {
            return Err(ReplayConsumeError::ClockRollback);
        }

        let candidate_digests = self.candidate_digests(grant.grant_identifier(), now)?;
        for record in &snapshot.records {
            if record.kind == RECORD_KIND_CONSUME
                && record.retention_until_unix_seconds > now
                && candidate_digests
                    .get(&record.replay_key_id)
                    .is_some_and(|candidate| constant_time_eq(candidate, &record.digest))
            {
                return Err(ReplayConsumeError::ReplayDetected);
            }
        }

        if snapshot.records.len() >= self.inner.max_records
            || REPLAY_HEADER_BYTES
                .saturating_add((snapshot.records.len() + 1).saturating_mul(REPLAY_RECORD_BYTES))
                > self.inner.max_bytes
        {
            self.compact_locked(directory_fd, &snapshot, now)?;
            file = open_ledger_file(directory_fd, &self.inner.ledger_name)
                .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
            snapshot = self.scan_ledger(&mut file)?;
        }

        if snapshot.records.len() >= self.inner.max_records
            || REPLAY_HEADER_BYTES
                .saturating_add((snapshot.records.len() + 1).saturating_mul(REPLAY_RECORD_BYTES))
                > self.inner.max_bytes
        {
            return Err(ReplayConsumeError::CapacityExhausted);
        }

        let active_key = self.active_key(now)?;
        let digest = replay_digest(&active_key.material, grant.grant_identifier());
        let record = encode_record(
            RECORD_KIND_CONSUME,
            grant.format_version(),
            &self.inner.keyring.active_key_id,
            grant.verification_key_id(),
            grant.replay_retention_until_unix_seconds(),
            now,
            digest,
            &active_key.material,
        )?;
        file.seek(SeekFrom::End(0))
            .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
        file.write_all(&record)
            .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
        file.sync_all()
            .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
        Ok(())
    }

    fn scan_ledger(&self, file: &mut File) -> Result<LedgerSnapshot, ReplayConsumeError> {
        let metadata = file
            .metadata()
            .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
        let size = usize::try_from(metadata.len())
            .map_err(|_| ReplayConsumeError::CapacityExhausted)?;
        if size < REPLAY_HEADER_BYTES
            || size > self.inner.max_bytes
            || (size - REPLAY_HEADER_BYTES) % REPLAY_RECORD_BYTES != 0
        {
            return Err(ReplayConsumeError::CorruptLedger);
        }
        let record_count = (size - REPLAY_HEADER_BYTES) / REPLAY_RECORD_BYTES;
        if record_count > self.inner.max_records {
            return Err(ReplayConsumeError::CapacityExhausted);
        }

        file.seek(SeekFrom::Start(0))
            .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
        let mut header = [0_u8; REPLAY_HEADER_BYTES];
        file.read_exact(&mut header)
            .map_err(|_| ReplayConsumeError::CorruptLedger)?;
        if &header[..8] != LEDGER_MAGIC
            || header[8] != LEDGER_VERSION
            || header[9..].iter().any(|byte| *byte != 0)
        {
            return Err(ReplayConsumeError::CorruptLedger);
        }

        let mut records = Vec::with_capacity(record_count);
        let mut high_water = 0_u64;
        for _ in 0..record_count {
            let mut raw = [0_u8; REPLAY_RECORD_BYTES];
            file.read_exact(&mut raw)
                .map_err(|_| ReplayConsumeError::CorruptLedger)?;
            let parsed = self.parse_record(raw)?;
            high_water = high_water.max(parsed.observed_unix_seconds);
            records.push(parsed);
        }
        Ok(LedgerSnapshot {
            records,
            high_water_unix_seconds: high_water,
        })
    }

    fn parse_record(
        &self,
        raw: [u8; REPLAY_RECORD_BYTES],
    ) -> Result<ParsedRecord, ReplayConsumeError> {
        if &raw[..4] != RECORD_MAGIC || raw[4] != RECORD_VERSION {
            return Err(ReplayConsumeError::CorruptLedger);
        }
        let kind = raw[5];
        if !matches!(kind, RECORD_KIND_CONSUME | RECORD_KIND_WATERMARK) {
            return Err(ReplayConsumeError::CorruptLedger);
        }
        let grant_version = raw[6];
        let replay_key_len = usize::from(raw[7]);
        let verification_key_len = usize::from(raw[8]);
        if raw[9..16].iter().any(|byte| *byte != 0)
            || replay_key_len == 0
            || replay_key_len > KEY_ID_FIELD_BYTES
            || verification_key_len > KEY_ID_FIELD_BYTES
        {
            return Err(ReplayConsumeError::CorruptLedger);
        }
        let replay_key_id = decode_padded_id(&raw[32..96], replay_key_len)?;
        let verification_key_id = decode_padded_id(&raw[96..160], verification_key_len)?;
        if kind == RECORD_KIND_CONSUME && verification_key_id.is_empty() {
            return Err(ReplayConsumeError::CorruptLedger);
        }
        if kind == RECORD_KIND_WATERMARK
            && (grant_version != 0
                || !verification_key_id.is_empty()
                || raw[160..192].iter().any(|byte| *byte != 0))
        {
            return Err(ReplayConsumeError::CorruptLedger);
        }
        let key = self
            .inner
            .keyring
            .keys
            .get(&replay_key_id)
            .ok_or(ReplayConsumeError::KeyUnavailable)?;
        let expected_mac = record_mac(&key.material, &raw[..RECORD_AUTHENTICATED_BYTES]);
        if !constant_time_eq(&expected_mac, &raw[RECORD_AUTHENTICATED_BYTES..]) {
            return Err(ReplayConsumeError::CorruptLedger);
        }
        let retention_until_unix_seconds = u64::from_be_bytes(raw[16..24].try_into().unwrap());
        let observed_unix_seconds = u64::from_be_bytes(raw[24..32].try_into().unwrap());
        if observed_unix_seconds < key.not_before_unix_seconds
            || observed_unix_seconds > key.digest_until_unix_seconds
        {
            return Err(ReplayConsumeError::CorruptLedger);
        }
        let mut digest = [0_u8; DIGEST_BYTES];
        digest.copy_from_slice(&raw[160..192]);
        Ok(ParsedRecord {
            raw,
            kind,
            grant_version,
            replay_key_id,
            verification_key_id,
            retention_until_unix_seconds,
            observed_unix_seconds,
            digest,
        })
    }

    fn candidate_digests(
        &self,
        identifier: &[u8; DIGEST_BYTES],
        now: u64,
    ) -> Result<BTreeMap<String, [u8; DIGEST_BYTES]>, ReplayConsumeError> {
        let mut digests = BTreeMap::new();
        for (key_id, key) in &self.inner.keyring.keys {
            if now <= key.digest_until_unix_seconds {
                digests.insert(key_id.clone(), replay_digest(&key.material, identifier));
            }
        }
        if digests.is_empty() {
            return Err(ReplayConsumeError::KeyUnavailable);
        }
        Ok(digests)
    }

    fn active_key(&self, now: u64) -> Result<&ReplayKey, ReplayConsumeError> {
        let key = self
            .inner
            .keyring
            .keys
            .get(&self.inner.keyring.active_key_id)
            .ok_or(ReplayConsumeError::KeyUnavailable)?;
        if now < key.not_before_unix_seconds || now > key.digest_until_unix_seconds {
            return Err(ReplayConsumeError::KeyUnavailable);
        }
        Ok(key)
    }

    fn compact_locked(
        &self,
        directory_fd: &OwnedFd,
        snapshot: &LedgerSnapshot,
        now: u64,
    ) -> Result<(), ReplayConsumeError> {
        let active_key = self.active_key(now)?;
        let mut retained: Vec<&ParsedRecord> = snapshot
            .records
            .iter()
            .filter(|record| {
                record.kind == RECORD_KIND_CONSUME
                    && record.retention_until_unix_seconds > now
            })
            .collect();
        if retained.len().saturating_add(1) >= self.inner.max_records
            || REPLAY_HEADER_BYTES
                .saturating_add((retained.len() + 1).saturating_mul(REPLAY_RECORD_BYTES))
                > self.inner.max_bytes
        {
            return Err(ReplayConsumeError::CapacityExhausted);
        }
        retained.sort_by_key(|record| {
            (
                record.retention_until_unix_seconds,
                &record.replay_key_id,
                record.digest,
            )
        });

        let temp_name = OsString::from(format!(
            ".{}.compact-{}",
            self.inner.ledger_name.to_string_lossy(),
            Uuid::new_v4()
        ));
        let temp_fd = descriptor_fs::openat(
            directory_fd,
            &temp_name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_bits_retain(0o600),
        )
        .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
        let mut temp = File::from(temp_fd);
        let result = (|| {
            temp.write_all(&ledger_header())
                .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
            let watermark = encode_record(
                RECORD_KIND_WATERMARK,
                0,
                &self.inner.keyring.active_key_id,
                "",
                0,
                snapshot.high_water_unix_seconds.max(now),
                [0_u8; DIGEST_BYTES],
                &active_key.material,
            )?;
            temp.write_all(&watermark)
                .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
            for record in retained {
                temp.write_all(&record.raw)
                    .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
            }
            temp.sync_all()
                .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
            descriptor_fs::renameat(
                directory_fd,
                &temp_name,
                directory_fd,
                &self.inner.ledger_name,
            )
            .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
            descriptor_fs::fsync(directory_fd)
                .map_err(|_| ReplayConsumeError::StorageUnavailable)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = descriptor_fs::unlinkat(directory_fd, &temp_name, AtFlags::empty());
        }
        result
    }
}

fn load_replay_keyring(
    path: &Path,
    authentication_token: Option<&str>,
    verifier: Option<&DirectoryGrantVerifier>,
    now: u64,
    max_lifetime_seconds: u64,
    clock_skew_seconds: u64,
) -> anyhow::Result<ReplayKeyring> {
    let mut bytes = read_owner_only_bounded_file(path, MAX_REPLAY_KEYRING_BYTES)?;
    let raw: RawReplayKeyring = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("directory replay keyring is malformed"))?;
    bytes.zeroize();
    if raw.version != LEDGER_VERSION {
        bail!("directory replay keyring version is unsupported");
    }
    if raw.keys.is_empty() || raw.keys.len() > MAX_REPLAY_KEYS {
        bail!("directory replay keyring key count is invalid");
    }
    validate_key_id(&raw.active_kid)?;
    let required_until = now
        .checked_add(max_lifetime_seconds)
        .and_then(|value| value.checked_add(clock_skew_seconds))
        .ok_or_else(|| anyhow!("directory replay key validity window overflow"))?;
    let mut keys = BTreeMap::new();
    let mut fingerprints = BTreeSet::new();
    for raw_key in raw.keys {
        validate_key_id(&raw_key.kid)?;
        if raw_key.not_before_unix_seconds >= raw_key.digest_until_unix_seconds {
            bail!("directory replay keyring contains an invalid key window");
        }
        let mut encoded = raw_key.key_b64url;
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded.as_bytes())
            .map_err(|_| anyhow!("directory replay keyring contains malformed key material"))?;
        if URL_SAFE_NO_PAD.encode(&decoded) != encoded {
            encoded.zeroize();
            bail!("directory replay keyring contains non-canonical key material");
        }
        encoded.zeroize();
        if !(MIN_REPLAY_KEY_BYTES..=MAX_REPLAY_KEY_BYTES).contains(&decoded.len()) {
            bail!("directory replay keyring contains invalid key length");
        }
        if authentication_token.is_some_and(|token| decoded.as_slice() == token.as_bytes()) {
            bail!("directory replay keys must not reuse authentication credentials");
        }
        let fingerprint: [u8; DIGEST_BYTES] = Sha256::digest(&decoded).into();
        if verifier.is_some_and(|verifier| verifier.contains_key_fingerprint(&fingerprint)) {
            bail!("directory replay keys must not reuse grant verification keys");
        }
        if !fingerprints.insert(fingerprint) {
            bail!("directory replay keyring contains duplicate key material");
        }
        if keys
            .insert(
                raw_key.kid,
                ReplayKey {
                    material: Zeroizing::new(decoded),
                    not_before_unix_seconds: raw_key.not_before_unix_seconds,
                    digest_until_unix_seconds: raw_key.digest_until_unix_seconds,
                },
            )
            .is_some()
        {
            bail!("directory replay keyring contains duplicate key identifiers");
        }
    }
    let active = keys
        .get(&raw.active_kid)
        .ok_or_else(|| anyhow!("directory replay active key is unavailable"))?;
    if active.not_before_unix_seconds > now.saturating_add(clock_skew_seconds)
        || active.digest_until_unix_seconds < required_until
    {
        bail!("directory replay active key does not cover the maximum grant lifetime");
    }
    Ok(ReplayKeyring {
        active_key_id: raw.active_kid,
        keys,
        fingerprints,
    })
}

fn split_ledger_path(path: &Path) -> anyhow::Result<(PathBuf, OsString, OsString)> {
    if !path.is_absolute() {
        bail!("directory replay ledger path must be absolute");
    }
    let directory = path
        .parent()
        .ok_or_else(|| anyhow!("directory replay ledger parent is unavailable"))?
        .to_path_buf();
    let ledger_name = path
        .file_name()
        .ok_or_else(|| anyhow!("directory replay ledger filename is unavailable"))?
        .to_os_string();
    validate_single_filename(&ledger_name)?;
    let lock_name = OsString::from(format!("{}.lock", ledger_name.to_string_lossy()));
    validate_single_filename(&lock_name)?;
    Ok((directory, ledger_name, lock_name))
}

fn validate_single_filename(value: &OsStr) -> anyhow::Result<()> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 200
        || matches!(bytes, b"." | b"..")
        || bytes.iter().any(|byte| *byte == 0 || *byte == b'/')
    {
        bail!("directory replay filename is invalid");
    }
    Ok(())
}

fn read_owner_only_bounded_file(path: &Path, max_bytes: usize) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    let fd = descriptor_fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| anyhow!("owner-only file could not be opened safely"))?;
    validate_owner_only_regular(&fd)?;
    let metadata = descriptor_fs::fstat(&fd)
        .map_err(|_| anyhow!("owner-only file metadata is unavailable"))?;
    if metadata.st_size < 0 || metadata.st_size as usize > max_bytes {
        bail!("owner-only file exceeds its fixed byte limit");
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(metadata.st_size as usize));
    File::from(fd)
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .context("owner-only file could not be read")?;
    if bytes.len() > max_bytes {
        bail!("owner-only file exceeds its fixed byte limit");
    }
    Ok(bytes)
}

fn open_owner_only_directory(path: &Path) -> anyhow::Result<OwnedFd> {
    let fd = descriptor_fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| anyhow!("directory replay parent could not be opened safely"))?;
    let metadata = descriptor_fs::fstat(&fd)
        .map_err(|_| anyhow!("directory replay parent metadata is unavailable"))?;
    if !FileType::from_raw_mode(metadata.st_mode).is_dir()
        || metadata.st_uid != process::geteuid().as_raw()
        || metadata.st_mode & 0o077 != 0
    {
        bail!("directory replay parent must be owner-only and owned by the server user");
    }
    Ok(fd)
}

fn open_owner_only_file_at(
    directory_fd: &OwnedFd,
    name: &OsStr,
    create: bool,
) -> anyhow::Result<OwnedFd> {
    let flags = OFlags::RDWR
        | OFlags::NOFOLLOW
        | OFlags::CLOEXEC
        | if create { OFlags::CREATE } else { OFlags::empty() };
    let fd = descriptor_fs::openat(
        directory_fd,
        name,
        flags,
        Mode::from_bits_retain(0o600),
    )
    .map_err(|_| anyhow!("owner-only file could not be opened safely"))?;
    validate_owner_only_regular(&fd)?;
    Ok(fd)
}

fn validate_owner_only_regular(fd: &OwnedFd) -> anyhow::Result<()> {
    let metadata = descriptor_fs::fstat(fd)
        .map_err(|_| anyhow!("owner-only file metadata is unavailable"))?;
    if !FileType::from_raw_mode(metadata.st_mode).is_file()
        || metadata.st_uid != process::geteuid().as_raw()
        || metadata.st_mode & 0o077 != 0
    {
        bail!("file must be owner-only and owned by the server user");
    }
    Ok(())
}

fn open_ledger_file(directory_fd: &OwnedFd, ledger_name: &OsStr) -> anyhow::Result<File> {
    open_owner_only_file_at(directory_fd, ledger_name, true).map(File::from)
}

fn initialize_header_if_empty(file: &mut File, directory_fd: &OwnedFd) -> anyhow::Result<()> {
    if file
        .metadata()
        .context("directory replay ledger metadata is unavailable")?
        .len()
        == 0
    {
        file.write_all(&ledger_header())
            .context("directory replay ledger header could not be written")?;
        file.sync_all()
            .context("directory replay ledger header could not be synchronized")?;
        descriptor_fs::fsync(directory_fd)
            .map_err(|_| anyhow!("directory replay ledger parent could not be synchronized"))?;
    }
    Ok(())
}

fn ledger_header() -> [u8; REPLAY_HEADER_BYTES] {
    let mut header = [0_u8; REPLAY_HEADER_BYTES];
    header[..8].copy_from_slice(LEDGER_MAGIC);
    header[8] = LEDGER_VERSION;
    header
}

#[allow(clippy::too_many_arguments)]
fn encode_record(
    kind: u8,
    grant_version: u8,
    replay_key_id: &str,
    verification_key_id: &str,
    retention_until_unix_seconds: u64,
    observed_unix_seconds: u64,
    digest: [u8; DIGEST_BYTES],
    key: &[u8],
) -> Result<[u8; REPLAY_RECORD_BYTES], ReplayConsumeError> {
    validate_key_id(replay_key_id).map_err(|_| ReplayConsumeError::KeyUnavailable)?;
    if kind == RECORD_KIND_CONSUME {
        validate_key_id(verification_key_id)
            .map_err(|_| ReplayConsumeError::CorruptLedger)?;
    } else if kind != RECORD_KIND_WATERMARK || !verification_key_id.is_empty() {
        return Err(ReplayConsumeError::CorruptLedger);
    }
    let mut record = [0_u8; REPLAY_RECORD_BYTES];
    record[..4].copy_from_slice(RECORD_MAGIC);
    record[4] = RECORD_VERSION;
    record[5] = kind;
    record[6] = grant_version;
    record[7] = replay_key_id.len() as u8;
    record[8] = verification_key_id.len() as u8;
    record[16..24].copy_from_slice(&retention_until_unix_seconds.to_be_bytes());
    record[24..32].copy_from_slice(&observed_unix_seconds.to_be_bytes());
    record[32..32 + replay_key_id.len()].copy_from_slice(replay_key_id.as_bytes());
    record[96..96 + verification_key_id.len()].copy_from_slice(verification_key_id.as_bytes());
    record[160..192].copy_from_slice(&digest);
    let mac = record_mac(key, &record[..RECORD_AUTHENTICATED_BYTES]);
    record[RECORD_AUTHENTICATED_BYTES..].copy_from_slice(&mac);
    Ok(record)
}

fn decode_padded_id(field: &[u8], length: usize) -> Result<String, ReplayConsumeError> {
    if field[length..].iter().any(|byte| *byte != 0) {
        return Err(ReplayConsumeError::CorruptLedger);
    }
    let value = std::str::from_utf8(&field[..length])
        .map_err(|_| ReplayConsumeError::CorruptLedger)?;
    if length == 0 {
        return Ok(String::new());
    }
    validate_key_id(value).map_err(|_| ReplayConsumeError::CorruptLedger)?;
    Ok(value.to_owned())
}

fn validate_key_id(value: &str) -> anyhow::Result<()> {
    if value.is_empty() || value.len() > MAX_REPLAY_KEY_ID_BYTES {
        bail!("bounded key identifier is invalid");
    }
    let mut previous_separator = false;
    for (index, byte) in value.bytes().enumerate() {
        let separator = matches!(byte, b'-' | b'_' | b':' | b'.');
        if !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || separator)
            || (separator && (index == 0 || index + 1 == value.len() || previous_separator))
        {
            bail!("bounded key identifier is invalid");
        }
        previous_separator = separator;
    }
    Ok(())
}

fn replay_digest(key: &[u8], identifier: &[u8; DIGEST_BYTES]) -> [u8; DIGEST_BYTES] {
    let mut mac = HmacSha256::new_from_slice(key).expect("bounded HMAC key length is valid");
    mac.update(DIGEST_DOMAIN);
    mac.update(identifier);
    mac.finalize().into_bytes().into()
}

fn record_mac(key: &[u8], authenticated: &[u8]) -> [u8; MAC_BYTES] {
    let mut mac = HmacSha256::new_from_slice(key).expect("bounded HMAC key length is valid");
    mac.update(RECORD_DOMAIN);
    mac.update(authenticated);
    mac.finalize().into_bytes().into()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::{Arc, Barrier},
        thread,
    };

    use super::*;
    use crate::{
        auth::AuthenticatedPrincipal,
        config::DirectoryGrantConfig,
        directory_grant::test_support::{mint_test_grant, TestGrantClaims, TestGrantKeyring},
    };

    const NOW: u64 = 1_800_000_000;

    fn write_replay_keyring(path: &Path, key: &[u8], mode: u32) {
        fs::write(
            path,
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "active_kid": "replay-key-01",
                "keys": [{
                    "kid": "replay-key-01",
                    "key_b64url": URL_SAFE_NO_PAD.encode(key),
                    "not_before_unix_seconds": NOW - 60,
                    "digest_until_unix_seconds": NOW + 600,
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    fn replay_config(directory: &Path, keyring_path: PathBuf) -> DirectoryGrantConfig {
        DirectoryGrantConfig {
            verification_enabled: true,
            issuer: Some("termux-grant-authority:v1".to_owned()),
            audience: Some("termux-mcp-edge:v1".to_owned()),
            keyring_path: Some(directory.join("verification-keys.json")),
            safe_root_ids: Some(vec!["primary-root".to_owned()]),
            max_lifetime_seconds: 120,
            clock_skew_seconds: 5,
            replay_enabled: true,
            replay_keyring_path: Some(keyring_path),
            replay_ledger_path: Some(directory.join("replay-ledger.bin")),
            replay_max_records: 64,
            replay_max_bytes: REPLAY_HEADER_BYTES + REPLAY_RECORD_BYTES * 64,
        }
    }

    fn verifier_and_grant(directory: &Path, grant_id: [u8; 32]) -> (DirectoryGrantVerifier, VerifiedDirectoryGrant) {
        let verification_key = [0x31_u8; 32];
        let keyring = TestGrantKeyring::write(
            directory.join("verification-keys.json"),
            verification_key,
            NOW,
        );
        let verifier = keyring.load_verifier(NOW);
        let principal = AuthenticatedPrincipal::configured("operator.primary:v1").unwrap();
        let components = vec!["projects".to_owned(), "alpha".to_owned()];
        let authorization = mint_test_grant(
            verification_key,
            TestGrantClaims {
                grant_id,
                now: NOW,
                ..TestGrantClaims::default()
            },
        );
        let grant = verifier
            .verify(
                Some(&authorization),
                &crate::directory_grant::DirectoryGrantBinding {
                    principal: &principal,
                    session_id: "11111111-2222-4333-8444-555555555555",
                    safe_root_id: "primary-root",
                    target_components: &components,
                },
                NOW,
            )
            .unwrap();
        (verifier, grant)
    }

    #[test]
    fn consumes_once_and_survives_reopen_without_disclosing_identifier() {
        let directory = tempfile::tempdir().unwrap();
        let replay_key = [0x71_u8; 32];
        let replay_keyring = directory.path().join("replay-keys.json");
        write_replay_keyring(&replay_keyring, &replay_key, 0o600);
        let (verifier, grant) = verifier_and_grant(directory.path(), [0x81_u8; 32]);
        let config = replay_config(directory.path(), replay_keyring);
        let ledger = DirectoryReplayLedger::load_optional(
            &config,
            Some("authentication-token"),
            Some(&verifier),
            NOW,
        )
        .unwrap()
        .unwrap();
        ledger.consume(&grant, NOW).unwrap();
        assert_eq!(
            ledger.consume(&grant, NOW),
            Err(ReplayConsumeError::ReplayDetected)
        );

        let reopened = DirectoryReplayLedger::load_optional(
            &config,
            Some("authentication-token"),
            Some(&verifier),
            NOW,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            reopened.consume(&grant, NOW),
            Err(ReplayConsumeError::ReplayDetected)
        );
        let bytes = fs::read(config.replay_ledger_path.as_ref().unwrap()).unwrap();
        assert!(!bytes.windows(32).any(|window| window == [0x81_u8; 32]));
        assert!(!format!("{reopened:?}").contains(directory.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn concurrent_consumers_have_exactly_one_winner() {
        let directory = tempfile::tempdir().unwrap();
        let replay_keyring = directory.path().join("replay-keys.json");
        write_replay_keyring(&replay_keyring, &[0x72_u8; 32], 0o600);
        let (verifier, grant) = verifier_and_grant(directory.path(), [0x82_u8; 32]);
        let ledger = Arc::new(
            DirectoryReplayLedger::load_optional(
                &replay_config(directory.path(), replay_keyring),
                Some("authentication-token"),
                Some(&verifier),
                NOW,
            )
            .unwrap()
            .unwrap(),
        );
        let grant = Arc::new(grant);
        let barrier = Arc::new(Barrier::new(16));
        let mut handles = Vec::new();
        for _ in 0..16 {
            let ledger = Arc::clone(&ledger);
            let grant = Arc::clone(&grant);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                ledger.consume(&grant, NOW)
            }));
        }
        let outcomes: Vec<_> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == Err(ReplayConsumeError::ReplayDetected))
                .count(),
            15
        );
    }

    #[test]
    fn clock_rollback_torn_records_and_permission_drift_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let replay_keyring = directory.path().join("replay-keys.json");
        write_replay_keyring(&replay_keyring, &[0x73_u8; 32], 0o600);
        let (verifier, first) = verifier_and_grant(directory.path(), [0x83_u8; 32]);
        let config = replay_config(directory.path(), replay_keyring.clone());
        let ledger = DirectoryReplayLedger::load_optional(
            &config,
            Some("authentication-token"),
            Some(&verifier),
            NOW,
        )
        .unwrap()
        .unwrap();
        ledger.consume(&first, NOW).unwrap();
        let (_, second) = verifier_and_grant(directory.path(), [0x84_u8; 32]);
        assert_eq!(
            ledger.consume(&second, NOW - 1),
            Err(ReplayConsumeError::ClockRollback)
        );

        let ledger_path = config.replay_ledger_path.as_ref().unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(ledger_path)
            .unwrap()
            .write_all(b"torn")
            .unwrap();
        assert!(DirectoryReplayLedger::load_optional(
            &config,
            Some("authentication-token"),
            Some(&verifier),
            NOW,
        )
        .is_err());

        fs::set_permissions(&replay_keyring, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(DirectoryReplayLedger::load_optional(
            &config,
            Some("authentication-token"),
            Some(&verifier),
            NOW,
        )
        .is_err());
    }

    #[test]
    fn replay_keys_are_independent_from_authentication_and_verification_keys() {
        let directory = tempfile::tempdir().unwrap();
        let replay_keyring = directory.path().join("replay-keys.json");
        let (verifier, _) = verifier_and_grant(directory.path(), [0x85_u8; 32]);
        let config = replay_config(directory.path(), replay_keyring.clone());

        let auth = "a".repeat(32);
        write_replay_keyring(&replay_keyring, auth.as_bytes(), 0o600);
        assert!(DirectoryReplayLedger::load_optional(
            &config,
            Some(&auth),
            Some(&verifier),
            NOW,
        )
        .is_err());

        write_replay_keyring(&replay_keyring, &[0x31_u8; 32], 0o600);
        assert!(DirectoryReplayLedger::load_optional(
            &config,
            Some("authentication-token"),
            Some(&verifier),
            NOW,
        )
        .is_err());
    }
}
