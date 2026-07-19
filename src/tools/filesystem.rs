//! Filesystem tools with safe-root enforcement, bounded traversal, and metrics.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use metrics::{counter, histogram};
use rustix::fs::{self as descriptor_fs, AtFlags, Dir, FileType, Mode, OFlags, RenameFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::audit::AuditEvent;
use crate::create_directory_grant::{CreateDirectoryGrantError, CreateDirectoryGrantTarget};
use crate::error::AppError;
use crate::write_policy::{WritePolicy, WritePolicyError};

const DEFAULT_LIST_DEPTH: u32 = 1;
const MAX_LIST_DEPTH: u32 = 5;
pub const MAX_LIST_ENTRIES: usize = 4_096;
pub const MAX_LIST_RESPONSE_BYTES: usize = 262_144;
pub const MAX_READ_BYTES: usize = 1_048_576;
pub const MAX_READ_RESPONSE_BYTES: usize = 1_114_112;
pub const MAX_BINARY_READ_BYTES: usize = 1_048_576;
pub const MAX_BINARY_READ_BASE64_BYTES: usize = 1_398_104;
pub const MAX_BINARY_READ_RESPONSE_BYTES: usize = 1_507_328;
pub const MAX_BINARY_RANGE_FILE_BYTES: usize = 67_108_864;
pub const MAX_BINARY_RANGE_BYTES: usize = 262_144;
pub const MAX_BINARY_RANGE_BASE64_BYTES: usize = 349_528;
pub const MAX_BINARY_RANGE_RESPONSE_BYTES: usize = 393_216;
pub const MAX_TEXT_RANGE_FILE_BYTES: usize = 67_108_864;
pub const MIN_TEXT_RANGE_BYTES: usize = 4;
pub const MAX_TEXT_RANGE_BYTES: usize = 262_144;
pub const MAX_TEXT_RANGE_ESCAPED_BYTES: usize = MAX_TEXT_RANGE_BYTES * 6;
pub const MAX_TEXT_RANGE_RESPONSE_BYTES: usize = 1_703_936;
pub const MAX_HASH_FILE_BYTES: usize = 16_777_216;
pub const MAX_HASH_FILE_RESPONSE_BYTES: usize = 16_384;
pub const MAX_PATH_METADATA_RESPONSE_BYTES: usize = 16_384;
pub const MAX_CREATE_DIRECTORY_RESPONSE_BYTES: usize = 16_384;
pub const CREATE_DIRECTORY_MODE: u32 = 0o700;
pub const MAX_COPY_FILE_BYTES: usize = 1_048_576;
pub const MAX_COPY_FILE_RESPONSE_BYTES: usize = 16_384;
pub const COPY_FILE_MODE: u32 = 0o600;
pub const MIN_FIND_DEPTH: u32 = 1;
pub const MAX_FIND_DEPTH: u32 = 5;
pub const MAX_FIND_QUERY_BYTES: usize = 256;
pub const MAX_FIND_ENTRIES: usize = 8_192;
pub const MAX_FIND_MATCHES: usize = 512;
pub const MAX_FIND_RESPONSE_BYTES: usize = 262_144;
pub const MIN_SEARCH_DEPTH: u32 = 1;
pub const MAX_SEARCH_DEPTH: u32 = 5;
pub const MAX_SEARCH_QUERY_BYTES: usize = 256;
pub const MAX_SEARCH_ENTRIES: usize = 8_192;
pub const MAX_SEARCH_FILES: usize = 4_096;
pub const MAX_SEARCH_FILE_BYTES: usize = 1_048_576;
pub const MAX_SEARCH_TOTAL_BYTES: usize = 8_388_608;
pub const MAX_SEARCH_MATCHES: usize = 256;
pub const MAX_SEARCH_RESPONSE_BYTES: usize = 262_144;

// Leave deterministic room for the JSON-RPC envelope, bounded summary, and a
// normally sized request id. The transport independently enforces the exact
// full-response ceilings above, including caller-controlled ids.
const MAX_LIST_STRUCTURED_BYTES: usize = MAX_LIST_RESPONSE_BYTES - 1_024;
const MAX_FIND_STRUCTURED_BYTES: usize = MAX_FIND_RESPONSE_BYTES - 1_024;
const MAX_SEARCH_STRUCTURED_BYTES: usize = MAX_SEARCH_RESPONSE_BYTES - 1_024;

struct DescriptorTempFileCleanup<'a> {
    parent: &'a OwnedFd,
    name: OsString,
    armed: bool,
}

struct DescriptorDirectoryCleanup<'a> {
    parent: &'a OwnedFd,
    name: OsString,
    expected_identity: Option<(u64, u64)>,
    armed: bool,
}

struct DescriptorCopiedFileCleanup<'a> {
    parent: &'a OwnedFd,
    name: OsString,
    expected_identity: Option<(u64, u64)>,
    armed: bool,
}

impl<'a> DescriptorCopiedFileCleanup<'a> {
    fn new(parent: &'a OwnedFd, name: OsString) -> Self {
        Self {
            parent,
            name,
            expected_identity: None,
            armed: true,
        }
    }

    fn set_expected_identity(&mut self, device: u64, inode: u64) {
        self.expected_identity = Some((device, inode));
    }

    fn published_as(&mut self, name: OsString) {
        self.name = name;
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DescriptorCopiedFileCleanup<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let Some((expected_device, expected_inode)) = self.expected_identity else {
            return;
        };
        let Ok(metadata) =
            descriptor_fs::statat(self.parent, &self.name, AtFlags::SYMLINK_NOFOLLOW)
        else {
            return;
        };
        let file_type = FileType::from_raw_mode(metadata.st_mode);
        if !file_type.is_file()
            || metadata.st_dev != expected_device
            || metadata.st_ino != expected_inode
        {
            return;
        }
        if descriptor_fs::unlinkat(self.parent, &self.name, AtFlags::empty()).is_ok() {
            let _ = descriptor_fs::fsync(self.parent);
        }
    }
}

impl<'a> DescriptorDirectoryCleanup<'a> {
    fn new(parent: &'a OwnedFd, name: OsString) -> Self {
        Self {
            parent,
            name,
            expected_identity: None,
            armed: true,
        }
    }

    fn set_expected_identity(&mut self, device: u64, inode: u64) {
        self.expected_identity = Some((device, inode));
    }

    fn published_as(&mut self, name: OsString) {
        self.name = name;
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DescriptorDirectoryCleanup<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let Some((expected_device, expected_inode)) = self.expected_identity else {
            return;
        };
        let Ok(metadata) =
            descriptor_fs::statat(self.parent, &self.name, AtFlags::SYMLINK_NOFOLLOW)
        else {
            return;
        };
        let file_type = FileType::from_raw_mode(metadata.st_mode);
        if !file_type.is_dir()
            || metadata.st_dev != expected_device
            || metadata.st_ino != expected_inode
        {
            return;
        }
        if descriptor_fs::unlinkat(self.parent, &self.name, AtFlags::REMOVEDIR).is_ok() {
            let _ = descriptor_fs::fsync(self.parent);
        }
    }
}

impl<'a> DescriptorTempFileCleanup<'a> {
    fn new(parent: &'a OwnedFd, name: OsString) -> Self {
        Self {
            parent,
            name,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DescriptorTempFileCleanup<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = descriptor_fs::unlinkat(self.parent, &self.name, AtFlags::empty());
        }
    }
}

struct AnchoredPath {
    display_path: PathBuf,
    root_path: PathBuf,
    relative_path: PathBuf,
}

pub(crate) struct PreparedCreateDirectoryMutation {
    result: CreateDirectoryResult,
    parent_fd: OwnedFd,
    directory_name: OsString,
    grant_target: CreateDirectoryGrantTarget,
    started: Instant,
}

pub(crate) enum AuthorizedCreateDirectoryError {
    Authorization(CreateDirectoryGrantError),
    Filesystem(AppError),
}

impl PreparedCreateDirectoryMutation {
    pub(crate) fn preview(self) -> CreateDirectoryResult {
        histogram!("mcp.fs.create_directory.latency_seconds")
            .record(self.started.elapsed().as_secs_f64());
        counter!("mcp.fs.create_directory.dry_runs_total").increment(1);
        self.result
    }

    pub(crate) fn execute_authorized(
        self,
        authorize: impl FnOnce(&CreateDirectoryGrantTarget) -> Result<(), CreateDirectoryGrantError>,
    ) -> Result<CreateDirectoryResult, AuthorizedCreateDirectoryError> {
        let temp_name = OsString::from(format!(
            ".termux-mcp-create-directory-{}.tmp",
            uuid::Uuid::new_v4()
        ));
        authorize(&self.grant_target).map_err(AuthorizedCreateDirectoryError::Authorization)?;

        let started = self.started;
        let result = self
            .execute_after_authorization(temp_name)
            .map_err(AuthorizedCreateDirectoryError::Filesystem)?;
        histogram!("mcp.fs.create_directory.latency_seconds")
            .record(started.elapsed().as_secs_f64());
        counter!("mcp.fs.create_directory.created_total").increment(1);
        Ok(result)
    }

    fn execute_after_authorization(
        self,
        temp_name: OsString,
    ) -> Result<CreateDirectoryResult, AppError> {
        descriptor_fs::mkdirat(
            &self.parent_fd,
            &temp_name,
            Mode::RUSR | Mode::WUSR | Mode::XUSR,
        )
        .map_err(descriptor_error)?;
        let mut cleanup = DescriptorDirectoryCleanup::new(&self.parent_fd, temp_name.clone());
        let temp_fd = open_child_directory(&self.parent_fd, &temp_name)?;
        let created_metadata = descriptor_fs::fstat(&temp_fd).map_err(descriptor_error)?;
        if !FileType::from_raw_mode(created_metadata.st_mode).is_dir() {
            return Err(AppError::Io(std::io::Error::other(
                "created directory verification failed",
            )));
        }
        cleanup.set_expected_identity(created_metadata.st_dev, created_metadata.st_ino);
        descriptor_fs::fchmod(&temp_fd, Mode::RUSR | Mode::WUSR | Mode::XUSR)
            .map_err(descriptor_error)?;
        let metadata = descriptor_fs::fstat(&temp_fd).map_err(descriptor_error)?;
        let file_type = FileType::from_raw_mode(metadata.st_mode);
        if !file_type.is_dir()
            || metadata.st_dev != created_metadata.st_dev
            || metadata.st_ino != created_metadata.st_ino
            || (metadata.st_mode & 0o7777) != CREATE_DIRECTORY_MODE
        {
            return Err(AppError::Io(std::io::Error::other(
                "created directory verification failed",
            )));
        }
        descriptor_fs::fsync(&temp_fd).map_err(descriptor_error)?;
        match descriptor_fs::renameat_with(
            &self.parent_fd,
            &temp_name,
            &self.parent_fd,
            &self.directory_name,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => cleanup.published_as(self.directory_name),
            Err(rustix::io::Errno::EXIST) => return Err(AppError::PathAlreadyExists),
            Err(error) => return Err(descriptor_error(error)),
        }
        let published_metadata =
            descriptor_fs::statat(&self.parent_fd, &cleanup.name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(descriptor_error)?;
        if !FileType::from_raw_mode(published_metadata.st_mode).is_dir()
            || published_metadata.st_dev != metadata.st_dev
            || published_metadata.st_ino != metadata.st_ino
        {
            return Err(AppError::Io(std::io::Error::other(
                "published directory verification failed",
            )));
        }
        descriptor_fs::fsync(&self.parent_fd).map_err(descriptor_error)?;
        cleanup.disarm();
        Ok(self.result)
    }
}

struct PendingEntry {
    info: FileInfo,
    name: OsString,
    display_path: PathBuf,
    encoded_bytes: usize,
}

struct SearchPendingEntry {
    name: OsString,
    display_path: PathBuf,
    file_type: FileType,
    size: u64,
}

struct FindPendingEntry {
    name: OsString,
    display_path: PathBuf,
    kind: FindPathKind,
}

struct FindPathsState<'a> {
    query: &'a str,
    kind_filter: FindPathFilter,
    matches: Vec<FindPathMatch>,
    entries_examined: usize,
    skipped_invalid_utf8_entries: usize,
    skipped_unsafe_entries: usize,
    skipped_unreadable_entries: usize,
    truncated: bool,
}

impl<'a> FindPathsState<'a> {
    fn new(query: &'a str, kind_filter: FindPathFilter) -> Self {
        Self {
            query,
            kind_filter,
            matches: Vec::new(),
            entries_examined: 0,
            skipped_invalid_utf8_entries: 0,
            skipped_unsafe_entries: 0,
            skipped_unreadable_entries: 0,
            truncated: false,
        }
    }

    fn execution_exhausted(&self) -> bool {
        self.entries_examined >= MAX_FIND_ENTRIES || self.matches.len() >= MAX_FIND_MATCHES
    }
}

struct SearchState<'a> {
    query: &'a str,
    matches: Vec<SearchTextMatch>,
    entries_examined: usize,
    files_scanned: usize,
    bytes_scanned: usize,
    skipped_oversized_files: usize,
    skipped_invalid_utf8_files: usize,
    skipped_unsafe_entries: usize,
    skipped_unreadable_entries: usize,
    truncated: bool,
}

impl<'a> SearchState<'a> {
    fn new(query: &'a str) -> Self {
        Self {
            query,
            matches: Vec::new(),
            entries_examined: 0,
            files_scanned: 0,
            bytes_scanned: 0,
            skipped_oversized_files: 0,
            skipped_invalid_utf8_files: 0,
            skipped_unsafe_entries: 0,
            skipped_unreadable_entries: 0,
            truncated: false,
        }
    }

    fn execution_exhausted(&self) -> bool {
        self.files_scanned >= MAX_SEARCH_FILES
            || self.bytes_scanned >= MAX_SEARCH_TOTAL_BYTES
            || self.matches.len() >= MAX_SEARCH_MATCHES
    }
}

#[derive(Clone)]
pub struct FileSystemTools {
    safe_roots: Vec<PathBuf>,
}

impl FileSystemTools {
    pub fn new(safe_roots: Vec<PathBuf>) -> Self {
        let safe_roots = safe_roots
            .into_iter()
            .map(|root| root.canonicalize().unwrap_or(root))
            .collect();

        Self { safe_roots }
    }

    pub fn safe_roots(&self) -> &[PathBuf] {
        &self.safe_roots
    }

    fn anchor(&self, input: &str) -> Result<AnchoredPath, AppError> {
        if input.trim().is_empty() || input.contains('\0') {
            return Err(path_rejected(input));
        }

        let candidate = Path::new(input);
        if !candidate.is_absolute()
            || candidate
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(path_rejected(input));
        }

        let Some((root_path, relative_path)) = self
            .safe_roots
            .iter()
            .filter_map(|root| {
                candidate
                    .strip_prefix(root)
                    .ok()
                    .map(|relative| (root, relative))
            })
            .max_by_key(|(root, _)| root.components().count())
        else {
            return Err(path_rejected(input));
        };

        if relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(path_rejected(input));
        }

        Ok(AnchoredPath {
            display_path: if relative_path.as_os_str().is_empty() {
                root_path.clone()
            } else {
                root_path.join(relative_path)
            },
            root_path: root_path.clone(),
            relative_path: relative_path.to_path_buf(),
        })
    }

    fn audit_write_decision(
        &self,
        timestamp_unix_seconds: u64,
        content_bytes: usize,
        dry_run: Option<bool>,
    ) -> AuditEvent {
        WritePolicy::default().audit_payload_decision(
            timestamp_unix_seconds,
            content_bytes,
            dry_run,
        )
    }

    /// Resolve a caller-supplied path and verify that it remains inside one of
    /// the configured safe roots.
    ///
    /// The method remains public for compatibility and static guard tests. Live
    /// filesystem operations additionally resolve every descendant from an open
    /// safe-root descriptor with no-follow semantics and never use this returned
    /// pathname for I/O.
    pub fn sanitize(&self, input: &str) -> Result<PathBuf, AppError> {
        if input.trim().is_empty() || input.contains('\0') {
            return Err(AppError::PathTraversal {
                attempted: input.to_string(),
            });
        }

        let candidate = Path::new(input);
        if !candidate.is_absolute() {
            return Err(AppError::PathTraversal {
                attempted: input.to_string(),
            });
        }

        if candidate
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(AppError::PathTraversal {
                attempted: input.to_string(),
            });
        }

        let resolved = if candidate.exists() {
            candidate
                .canonicalize()
                .map_err(|_| AppError::PathTraversal {
                    attempted: input.to_string(),
                })?
        } else {
            let parent = candidate.parent().ok_or_else(|| AppError::PathTraversal {
                attempted: input.to_string(),
            })?;
            let file_name = candidate
                .file_name()
                .ok_or_else(|| AppError::PathTraversal {
                    attempted: input.to_string(),
                })?;
            let canonical_parent = parent.canonicalize().map_err(|_| AppError::PathTraversal {
                attempted: input.to_string(),
            })?;
            canonical_parent.join(file_name)
        };

        if self
            .safe_roots
            .iter()
            .any(|root| resolved.starts_with(root))
        {
            Ok(resolved)
        } else {
            Err(AppError::PathTraversal {
                attempted: input.to_string(),
            })
        }
    }

    fn collect_entries_descriptor_relative(
        root_fd: OwnedFd,
        root_path: &Path,
        entries: &mut Vec<FileInfo>,
        max_depth: u32,
        structured_bytes: &mut usize,
    ) -> Result<bool, AppError> {
        let mut queue = VecDeque::new();
        queue.push_back((root_fd, root_path.to_path_buf(), 1_u32));
        let mut truncated = false;

        while let Some((dir_fd, dir_path, depth)) = queue.pop_front() {
            if entries.len() >= MAX_LIST_ENTRIES || *structured_bytes >= MAX_LIST_STRUCTURED_BYTES {
                truncated = true;
                break;
            }

            let mut read_dir = Dir::read_from(&dir_fd).map_err(descriptor_error)?;
            // Keep only the lexicographically smallest candidates that can fit
            // the remaining published entry and byte budgets. Removing the
            // largest key after each insertion makes the selected subset
            // independent of filesystem enumeration order while bounding
            // memory by the same constants as the response.
            let mut candidates = BTreeMap::new();
            let mut candidate_bytes = 0_usize;

            for entry in &mut read_dir {
                let entry = entry.map_err(descriptor_error)?;
                let name_bytes = entry.file_name().to_bytes();
                if name_bytes == b"." || name_bytes == b".." {
                    continue;
                }
                let name = OsString::from_vec(name_bytes.to_vec());
                let Ok(metadata) = descriptor_fs::statat(&dir_fd, &name, AtFlags::SYMLINK_NOFOLLOW)
                else {
                    counter!("mcp.fs.list.skipped_unreadable_entries_total").increment(1);
                    continue;
                };
                let file_type = FileType::from_raw_mode(metadata.st_mode);
                if file_type.is_symlink() {
                    counter!("mcp.fs.list.skipped_unsafe_entries_total").increment(1);
                    continue;
                }
                let child_path = dir_path.join(&name);

                let info = FileInfo {
                    path: child_path.to_string_lossy().to_string(),
                    size: u64::try_from(metadata.st_size).unwrap_or(0),
                    is_dir: file_type.is_dir(),
                    modified: stat_modified_time(&metadata),
                };
                let encoded_bytes = serde_json::to_vec(&info)
                    .map_err(std::io::Error::other)?
                    .len();
                let key = info.path.clone();

                candidate_bytes += encoded_bytes;
                if let Some(replaced) = candidates.insert(
                    key,
                    PendingEntry {
                        info,
                        name,
                        display_path: child_path,
                        encoded_bytes,
                    },
                ) {
                    candidate_bytes = candidate_bytes.saturating_sub(replaced.encoded_bytes);
                }

                while entries.len() + candidates.len() > MAX_LIST_ENTRIES
                    || *structured_bytes
                        + candidate_bytes
                        + usize::from(!entries.is_empty())
                        + candidates.len().saturating_sub(1)
                        > MAX_LIST_STRUCTURED_BYTES
                {
                    let Some((_, removed)) = candidates.pop_last() else {
                        break;
                    };
                    candidate_bytes = candidate_bytes.saturating_sub(removed.encoded_bytes);
                    truncated = true;
                }
            }

            for (_, pending) in candidates {
                let recurse = pending.info.is_dir && depth < max_depth;
                if !entries.is_empty() {
                    *structured_bytes += 1;
                }
                *structured_bytes += pending.encoded_bytes;
                entries.push(pending.info);
                if recurse {
                    match open_child_directory(&dir_fd, &pending.name) {
                        Ok(child_fd) => {
                            queue.push_back((child_fd, pending.display_path, depth + 1))
                        }
                        Err(_) => {
                            counter!("mcp.fs.list.skipped_unreadable_entries_total").increment(1)
                        }
                    }
                }
            }
        }

        if truncated {
            counter!("mcp.fs.list.truncated_total").increment(1);
        }

        Ok(truncated)
    }

    pub async fn list_directory(
        &self,
        path: String,
        max_depth: Option<u32>,
    ) -> Result<ListDirResult, AppError> {
        let start = Instant::now();
        let anchored = self.anchor(&path)?;
        let depth = max_depth
            .unwrap_or(DEFAULT_LIST_DEPTH)
            .clamp(1, MAX_LIST_DEPTH);
        let result = tokio::task::spawn_blocking(move || {
            let root_fd = open_root_directory(&anchored.root_path)?;
            let target_fd = open_descendant_directory(root_fd, &anchored.relative_path)?;
            let mut result = ListDirResult {
                path: anchored.display_path.to_string_lossy().to_string(),
                entries: Vec::new(),
                truncated: false,
                max_entries: MAX_LIST_ENTRIES,
                max_response_bytes: MAX_LIST_RESPONSE_BYTES,
            };
            let mut structured_bytes = serde_json::to_vec(&result)
                .map_err(std::io::Error::other)?
                .len();
            result.truncated = Self::collect_entries_descriptor_relative(
                target_fd,
                &anchored.display_path,
                &mut result.entries,
                depth,
                &mut structured_bytes,
            )?;
            result
                .entries
                .sort_unstable_by(|left, right| left.path.cmp(&right.path));

            debug_assert!(serde_json::to_vec(&result)
                .is_ok_and(|bytes| { bytes.len() <= MAX_LIST_STRUCTURED_BYTES }));
            Ok::<_, AppError>(result)
        })
        .await
        .map_err(filesystem_worker_error)??;

        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.fs.list.latency_seconds").record(duration);
        counter!("mcp.fs.list.calls_total").increment(1);

        Ok(result)
    }

    pub async fn read_file(&self, path: String) -> Result<ReadFileResult, AppError> {
        let start = Instant::now();
        let anchored = self.anchor(&path)?;
        let result = tokio::task::spawn_blocking(move || {
            let file = match open_verified_regular_file(&anchored, MAX_READ_BYTES) {
                Err(error @ AppError::FileTooLarge { .. }) => {
                    counter!("mcp.fs.read.rejected_too_large_total").increment(1);
                    return Err(error);
                }
                result => result?,
            };
            let bytes = match read_bounded_bytes(file.file, MAX_READ_BYTES) {
                Err(error @ AppError::FileTooLarge { .. }) => {
                    counter!("mcp.fs.read.rejected_too_large_total").increment(1);
                    return Err(error);
                }
                result => result?,
            };
            let bytes_read = bytes.len();

            let content = String::from_utf8(bytes).map_err(|_| AppError::InvalidFileEncoding)?;
            Ok::<_, AppError>(ReadFileResult {
                path: anchored.display_path.to_string_lossy().to_string(),
                content,
                size: bytes_read,
            })
        })
        .await
        .map_err(filesystem_worker_error)??;

        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.fs.read.latency_seconds").record(duration);
        counter!("mcp.fs.read.bytes_total").increment(result.size as u64);

        Ok(result)
    }

    /// Read one regular file as canonical padded RFC 4648 base64 through the
    /// exact descriptor retained after safe-root confinement.
    pub async fn read_binary_file(&self, path: String) -> Result<ReadBinaryFileResult, AppError> {
        let start = Instant::now();
        let anchored = self.anchor(&path)?;
        let result = tokio::task::spawn_blocking(move || {
            let file = match open_verified_regular_file(&anchored, MAX_BINARY_READ_BYTES) {
                Err(error @ AppError::FileTooLarge { .. }) => {
                    counter!("mcp.fs.read_binary.rejected_too_large_total").increment(1);
                    return Err(error);
                }
                result => result?,
            };
            let bytes = match read_bounded_bytes(file.file, MAX_BINARY_READ_BYTES) {
                Err(error @ AppError::FileTooLarge { .. }) => {
                    counter!("mcp.fs.read_binary.rejected_too_large_total").increment(1);
                    return Err(error);
                }
                result => result?,
            };

            let size_bytes = bytes.len();
            Ok::<_, AppError>(ReadBinaryFileResult {
                encoding: "base64".to_owned(),
                data: encode_base64(&bytes),
                size_bytes,
                max_file_bytes: MAX_BINARY_READ_BYTES,
                max_response_bytes: MAX_BINARY_READ_RESPONSE_BYTES,
            })
        })
        .await
        .map_err(filesystem_worker_error)??;

        histogram!("mcp.fs.read_binary.latency_seconds").record(start.elapsed().as_secs_f64());
        counter!("mcp.fs.read_binary.calls_total").increment(1);
        counter!("mcp.fs.read_binary.bytes_total").increment(result.size_bytes as u64);
        Ok(result)
    }

    /// Read one bounded byte range as canonical padded RFC 4648 base64 through
    /// the exact descriptor retained after safe-root confinement.
    pub async fn read_binary_range(
        &self,
        path: String,
        offset_bytes: u64,
        length_bytes: usize,
    ) -> Result<ReadBinaryRangeResult, AppError> {
        if offset_bytes > MAX_BINARY_RANGE_FILE_BYTES as u64
            || !(1..=MAX_BINARY_RANGE_BYTES).contains(&length_bytes)
        {
            counter!("mcp.fs.read_binary_range.rejected_invalid_total").increment(1);
            return Err(AppError::InvalidBinaryRange);
        }

        let start = Instant::now();
        let anchored = self.anchor(&path)?;
        let result = tokio::task::spawn_blocking(move || {
            let file = match open_verified_regular_file(&anchored, MAX_BINARY_RANGE_FILE_BYTES) {
                Err(error @ AppError::FileTooLarge { .. }) => {
                    counter!("mcp.fs.read_binary_range.rejected_too_large_total").increment(1);
                    return Err(error);
                }
                result => result?,
            };
            let range = read_verified_binary_range(file, offset_bytes, length_bytes)?;
            let size_bytes = range.bytes.len();
            Ok::<_, AppError>(ReadBinaryRangeResult {
                encoding: "base64".to_owned(),
                data: encode_base64(&range.bytes),
                offset_bytes,
                size_bytes,
                file_size_bytes: range.file_size_bytes,
                eof: range.eof,
                max_read_bytes: MAX_BINARY_RANGE_BYTES,
                max_file_bytes: MAX_BINARY_RANGE_FILE_BYTES,
                max_response_bytes: MAX_BINARY_RANGE_RESPONSE_BYTES,
            })
        })
        .await
        .map_err(filesystem_worker_error)??;

        histogram!("mcp.fs.read_binary_range.latency_seconds")
            .record(start.elapsed().as_secs_f64());
        counter!("mcp.fs.read_binary_range.calls_total").increment(1);
        counter!("mcp.fs.read_binary_range.bytes_total").increment(result.size_bytes as u64);
        Ok(result)
    }

    /// Read one bounded UTF-8 byte range through the exact descriptor retained
    /// after safe-root confinement. The returned content always starts and ends
    /// on code-point boundaries; an incomplete code point at a non-EOF range
    /// boundary is deferred to the next page.
    pub async fn read_text_range(
        &self,
        path: String,
        offset_bytes: u64,
        max_bytes: usize,
    ) -> Result<ReadTextRangeResult, AppError> {
        if offset_bytes > MAX_TEXT_RANGE_FILE_BYTES as u64
            || !(MIN_TEXT_RANGE_BYTES..=MAX_TEXT_RANGE_BYTES).contains(&max_bytes)
        {
            counter!("mcp.fs.read_text_range.rejected_invalid_total").increment(1);
            return Err(AppError::InvalidTextRange);
        }

        let start = Instant::now();
        let anchored = self.anchor(&path)?;
        let result = tokio::task::spawn_blocking(move || {
            let file = match open_verified_regular_file(&anchored, MAX_TEXT_RANGE_FILE_BYTES) {
                Err(error @ AppError::FileTooLarge { .. }) => {
                    counter!("mcp.fs.read_text_range.rejected_too_large_total").increment(1);
                    return Err(error);
                }
                result => result?,
            };
            let range = read_verified_text_range(file, offset_bytes, max_bytes)?;
            let size_bytes = range.content.len();
            let next_offset_bytes = offset_bytes
                .checked_add(size_bytes as u64)
                .ok_or(AppError::InvalidTextRange)?;
            Ok::<_, AppError>(ReadTextRangeResult {
                content: range.content,
                offset_bytes,
                next_offset_bytes,
                size_bytes,
                file_size_bytes: range.file_size_bytes,
                eof: range.eof,
                max_read_bytes: MAX_TEXT_RANGE_BYTES,
                max_file_bytes: MAX_TEXT_RANGE_FILE_BYTES,
                max_response_bytes: MAX_TEXT_RANGE_RESPONSE_BYTES,
            })
        })
        .await
        .map_err(filesystem_worker_error)??;

        histogram!("mcp.fs.read_text_range.latency_seconds").record(start.elapsed().as_secs_f64());
        counter!("mcp.fs.read_text_range.calls_total").increment(1);
        counter!("mcp.fs.read_text_range.bytes_total").increment(result.size_bytes as u64);
        Ok(result)
    }

    /// Hash one regular file through the exact descriptor retained after
    /// safe-root confinement. The bounded streaming read never returns file
    /// contents or a partial digest.
    pub async fn hash_file(&self, path: String) -> Result<HashFileResult, AppError> {
        let start = Instant::now();
        let anchored = self.anchor(&path)?;
        let result = tokio::task::spawn_blocking(move || {
            let file = match open_verified_regular_file(&anchored, MAX_HASH_FILE_BYTES) {
                Err(error @ AppError::FileTooLarge { .. }) => {
                    counter!("mcp.fs.hash.rejected_too_large_total").increment(1);
                    return Err(error);
                }
                result => result?,
            };
            let mut reader = file.file.take((MAX_HASH_FILE_BYTES + 1) as u64);
            let mut buffer = [0_u8; 64 * 1_024];
            let mut hasher = Sha256::new();
            let mut bytes_hashed = 0_usize;
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                let next_size = bytes_hashed.checked_add(read).ok_or_else(|| {
                    AppError::Io(std::io::Error::other("hashed byte count overflowed"))
                })?;
                if next_size > MAX_HASH_FILE_BYTES {
                    counter!("mcp.fs.hash.rejected_too_large_total").increment(1);
                    return Err(AppError::FileTooLarge {
                        size: next_size as u64,
                        max_size: MAX_HASH_FILE_BYTES as u64,
                    });
                }
                hasher.update(&buffer[..read]);
                bytes_hashed = next_size;
            }

            Ok::<_, AppError>(HashFileResult {
                algorithm: "sha256".to_owned(),
                digest: encode_lower_hex(&hasher.finalize()),
                size_bytes: bytes_hashed,
            })
        })
        .await
        .map_err(filesystem_worker_error)??;

        histogram!("mcp.fs.hash.latency_seconds").record(start.elapsed().as_secs_f64());
        counter!("mcp.fs.hash.calls_total").increment(1);
        counter!("mcp.fs.hash.bytes_total").increment(result.size_bytes as u64);

        Ok(result)
    }

    pub async fn path_metadata(&self, path: String) -> Result<PathMetadataResult, AppError> {
        let start = Instant::now();
        let anchored = self.anchor(&path)?;
        let result = tokio::task::spawn_blocking(move || {
            let root_fd = open_root_directory(&anchored.root_path)?;
            let target_fd = open_metadata_descriptor(root_fd, &anchored.relative_path)?;
            let metadata = descriptor_fs::fstat(&target_fd).map_err(descriptor_error)?;
            let file_type = FileType::from_raw_mode(metadata.st_mode);
            let (kind, size_bytes) = if file_type.is_file() {
                (
                    PathMetadataKind::RegularFile,
                    Some(u64::try_from(metadata.st_size).unwrap_or(0)),
                )
            } else if file_type.is_dir() {
                (PathMetadataKind::Directory, None)
            } else if file_type.is_symlink() {
                return Err(path_rejected(
                    anchored.display_path.to_string_lossy().as_ref(),
                ));
            } else {
                return Err(AppError::UnsupportedPathType);
            };

            Ok::<_, AppError>(PathMetadataResult {
                path: anchored.display_path.to_string_lossy().to_string(),
                kind,
                size_bytes,
                modified: stat_modified_time(&metadata),
                max_response_bytes: MAX_PATH_METADATA_RESPONSE_BYTES,
            })
        })
        .await
        .map_err(filesystem_worker_error)??;

        histogram!("mcp.fs.metadata.latency_seconds").record(start.elapsed().as_secs_f64());
        counter!("mcp.fs.metadata.calls_total").increment(1);

        Ok(result)
    }

    pub async fn create_directory(
        &self,
        path: String,
        dry_run: Option<bool>,
    ) -> Result<CreateDirectoryResult, AppError> {
        let dry_run = dry_run.unwrap_or(true);
        let prepared = self.prepare_create_directory(path, dry_run).await?;
        if dry_run {
            Ok(prepared.preview())
        } else {
            prepared
                .execute_authorized(|_| Ok(()))
                .map_err(|error| match error {
                    AuthorizedCreateDirectoryError::Authorization(_) => AppError::Io(
                        std::io::Error::other("unexpected direct create authorization failure"),
                    ),
                    AuthorizedCreateDirectoryError::Filesystem(error) => error,
                })
        }
    }

    pub(crate) async fn prepare_create_directory_mutation(
        &self,
        path: String,
    ) -> Result<PreparedCreateDirectoryMutation, AppError> {
        self.prepare_create_directory(path, false).await
    }

    pub fn create_directory_grant_target(
        &self,
        path: &str,
    ) -> Result<CreateDirectoryGrantTarget, AppError> {
        let anchored = self.anchor(path)?;
        Ok(prepare_create_directory(anchored, false)?.grant_target)
    }

    async fn prepare_create_directory(
        &self,
        path: String,
        dry_run: bool,
    ) -> Result<PreparedCreateDirectoryMutation, AppError> {
        let anchored = self.anchor(&path)?;
        tokio::task::spawn_blocking(move || prepare_create_directory(anchored, dry_run))
            .await
            .map_err(filesystem_worker_error)?
    }

    pub(crate) fn create_directory_response_preview(
        &self,
        path: &str,
        dry_run: bool,
    ) -> Result<CreateDirectoryResult, AppError> {
        let anchored = self.anchor(path)?;
        Ok(create_directory_result(&anchored, dry_run))
    }

    pub async fn copy_file(
        &self,
        source_path: String,
        destination_path: String,
        dry_run: Option<bool>,
    ) -> Result<CopyFileResult, AppError> {
        let start = Instant::now();
        let source = self.anchor(&source_path)?;
        let destination = self.anchor(&destination_path)?;
        if source.display_path == destination.display_path {
            return Err(AppError::CopySourceDestinationSame);
        }
        let dry_run = dry_run.unwrap_or(true);

        let result = tokio::task::spawn_blocking(move || {
            let (source_parent_relative, source_name) =
                split_parent_and_name(&source.relative_path)?;
            let source_root_fd = open_root_directory(&source.root_path)?;
            let source_parent_fd =
                open_metadata_parent_directory(source_root_fd, &source_parent_relative)
                    .map_err(copy_source_parent_error)?;
            let source_before = match descriptor_fs::statat(
                &source_parent_fd,
                &source_name,
                AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(metadata) => metadata,
                Err(rustix::io::Errno::NOENT) => return Err(AppError::CopySourceNotFound),
                Err(error) => return Err(descriptor_error(error)),
            };
            let source_type = FileType::from_raw_mode(source_before.st_mode);
            if source_type.is_symlink() {
                return Err(path_rejected(
                    source.display_path.to_string_lossy().as_ref(),
                ));
            }
            if !source_type.is_file() {
                return Err(AppError::UnsupportedPathType);
            }
            let source_size = copy_source_size(&source_before)?;
            if source_size > MAX_COPY_FILE_BYTES as u64 {
                return Err(AppError::FileTooLarge {
                    size: source_size,
                    max_size: MAX_COPY_FILE_BYTES as u64,
                });
            }

            let source_fd = descriptor_fs::openat(
                &source_parent_fd,
                &source_name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| match error {
                rustix::io::Errno::NOENT => AppError::CopySourceNotFound,
                rustix::io::Errno::LOOP => {
                    path_rejected(source.display_path.to_string_lossy().as_ref())
                }
                _ => descriptor_error(error),
            })?;
            let source_opened = descriptor_fs::fstat(&source_fd).map_err(descriptor_error)?;
            if !FileType::from_raw_mode(source_opened.st_mode).is_file() {
                return Err(AppError::UnsupportedPathType);
            }
            if source_opened.st_dev != source_before.st_dev
                || source_opened.st_ino != source_before.st_ino
                || source_opened.st_size != source_before.st_size
            {
                return Err(AppError::Io(std::io::Error::other(
                    "copy source changed before it was opened",
                )));
            }

            let mut source_file = File::from(source_fd);
            let mut bytes = Vec::with_capacity(MAX_COPY_FILE_BYTES.min(64 * 1_024));
            (&mut source_file)
                .take((MAX_COPY_FILE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)?;
            if bytes.len() > MAX_COPY_FILE_BYTES {
                return Err(AppError::FileTooLarge {
                    size: bytes.len() as u64,
                    max_size: MAX_COPY_FILE_BYTES as u64,
                });
            }
            let source_after = descriptor_fs::fstat(&source_file).map_err(descriptor_error)?;
            if !FileType::from_raw_mode(source_after.st_mode).is_file()
                || source_after.st_dev != source_opened.st_dev
                || source_after.st_ino != source_opened.st_ino
                || source_after.st_size != source_opened.st_size
                || copy_source_size(&source_after)? != bytes.len() as u64
            {
                return Err(AppError::Io(std::io::Error::other(
                    "copy source changed while it was read",
                )));
            }

            let (destination_parent_relative, destination_name) =
                split_parent_and_name(&destination.relative_path)?;
            let destination_root_fd = open_root_directory(&destination.root_path)?;
            let destination_parent_fd =
                open_mutation_parent_directory(destination_root_fd, &destination_parent_relative)
                    .map_err(copy_destination_parent_error)?;
            match descriptor_fs::statat(
                &destination_parent_fd,
                &destination_name,
                AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(metadata) if FileType::from_raw_mode(metadata.st_mode).is_symlink() => {
                    return Err(path_rejected(
                        destination.display_path.to_string_lossy().as_ref(),
                    ));
                }
                Ok(_) => return Err(AppError::PathAlreadyExists),
                Err(rustix::io::Errno::NOENT) => {}
                Err(error) => return Err(descriptor_error(error)),
            }

            let result = copy_file_result(&source, &destination, dry_run, bytes.len());
            if dry_run {
                return Ok(result);
            }

            let temp_name = OsString::from(format!(
                ".termux-mcp-copy-file-{}.tmp",
                uuid::Uuid::new_v4()
            ));
            let temp_fd = descriptor_fs::openat(
                &destination_parent_fd,
                &temp_name,
                OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(descriptor_error)?;
            let mut cleanup =
                DescriptorCopiedFileCleanup::new(&destination_parent_fd, temp_name.clone());
            let created_metadata = descriptor_fs::fstat(&temp_fd).map_err(descriptor_error)?;
            if !FileType::from_raw_mode(created_metadata.st_mode).is_file() {
                return Err(AppError::Io(std::io::Error::other(
                    "copy destination staging verification failed",
                )));
            }
            cleanup.set_expected_identity(created_metadata.st_dev, created_metadata.st_ino);
            descriptor_fs::fchmod(&temp_fd, Mode::RUSR | Mode::WUSR).map_err(descriptor_error)?;

            let mut destination_file = File::from(temp_fd);
            destination_file.write_all(&bytes)?;
            destination_file.sync_all()?;
            let staged_metadata =
                descriptor_fs::fstat(&destination_file).map_err(descriptor_error)?;
            if !copy_file_identity_and_contract_match(
                &staged_metadata,
                created_metadata.st_dev,
                created_metadata.st_ino,
                bytes.len(),
            ) {
                return Err(AppError::Io(std::io::Error::other(
                    "copy destination staging verification failed",
                )));
            }

            match descriptor_fs::renameat_with(
                &destination_parent_fd,
                &temp_name,
                &destination_parent_fd,
                &destination_name,
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => cleanup.published_as(destination_name),
                Err(rustix::io::Errno::EXIST) => return Err(AppError::PathAlreadyExists),
                Err(error) => return Err(descriptor_error(error)),
            }
            let published_metadata = descriptor_fs::statat(
                &destination_parent_fd,
                &cleanup.name,
                AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(descriptor_error)?;
            let held_metadata =
                descriptor_fs::fstat(&destination_file).map_err(descriptor_error)?;
            if !copy_file_identity_and_contract_match(
                &published_metadata,
                created_metadata.st_dev,
                created_metadata.st_ino,
                bytes.len(),
            ) || !copy_file_identity_and_contract_match(
                &held_metadata,
                created_metadata.st_dev,
                created_metadata.st_ino,
                bytes.len(),
            ) {
                return Err(AppError::Io(std::io::Error::other(
                    "published copy destination verification failed",
                )));
            }
            descriptor_fs::fsync(&destination_parent_fd).map_err(descriptor_error)?;
            cleanup.disarm();
            Ok(result)
        })
        .await
        .map_err(filesystem_worker_error)??;

        histogram!("mcp.fs.copy_file.latency_seconds").record(start.elapsed().as_secs_f64());
        if result.dry_run {
            counter!("mcp.fs.copy_file.dry_runs_total").increment(1);
        } else {
            counter!("mcp.fs.copy_file.copied_total").increment(1);
            counter!("mcp.fs.copy_file.bytes_total").increment(result.size_bytes as u64);
        }

        Ok(result)
    }

    pub(crate) fn copy_file_response_preview(
        &self,
        source_path: &str,
        destination_path: &str,
        dry_run: bool,
    ) -> Result<CopyFileResult, AppError> {
        let source = self.anchor(source_path)?;
        let destination = self.anchor(destination_path)?;
        if source.display_path == destination.display_path {
            return Err(AppError::CopySourceDestinationSame);
        }
        Ok(copy_file_result(
            &source,
            &destination,
            dry_run,
            MAX_COPY_FILE_BYTES,
        ))
    }

    pub async fn find_paths(
        &self,
        path: String,
        query: String,
        kind_filter: FindPathFilter,
        max_depth: Option<u32>,
    ) -> Result<FindPathsResult, AppError> {
        validate_find_query(&query)?;
        let start = Instant::now();
        let anchored = self.anchor(&path)?;
        let depth = max_depth
            .unwrap_or(MAX_FIND_DEPTH)
            .clamp(MIN_FIND_DEPTH, MAX_FIND_DEPTH);
        let result = tokio::task::spawn_blocking(move || {
            let root_fd = open_root_directory(&anchored.root_path)?;
            let target_fd = open_descendant_directory(root_fd, &anchored.relative_path)?;
            let mut state = FindPathsState::new(&query, kind_filter);
            collect_path_matches_descriptor_relative(
                target_fd,
                &anchored.display_path,
                depth,
                &mut state,
            )?;
            state
                .matches
                .sort_unstable_by(|left, right| left.path.cmp(&right.path));

            let mut result = FindPathsResult {
                path: anchored.display_path.to_string_lossy().to_string(),
                matches: state.matches,
                truncated: state.truncated,
                entries_examined: state.entries_examined,
                skipped_invalid_utf8_entries: state.skipped_invalid_utf8_entries,
                skipped_unsafe_entries: state.skipped_unsafe_entries,
                skipped_unreadable_entries: state.skipped_unreadable_entries,
                query_bytes: query.len(),
                kind_filter,
                max_depth: depth,
                max_entries: MAX_FIND_ENTRIES,
                max_matches: MAX_FIND_MATCHES,
                max_response_bytes: MAX_FIND_RESPONSE_BYTES,
            };
            while serde_json::to_vec(&result)
                .map_err(std::io::Error::other)?
                .len()
                > MAX_FIND_STRUCTURED_BYTES
            {
                if result.matches.pop().is_none() {
                    return Err(AppError::Io(std::io::Error::other(
                        "path-discovery response metadata exceeds its bound",
                    )));
                }
                result.truncated = true;
            }
            Ok::<_, AppError>(result)
        })
        .await
        .map_err(filesystem_worker_error)??;

        histogram!("mcp.fs.find_paths.latency_seconds").record(start.elapsed().as_secs_f64());
        counter!("mcp.fs.find_paths.calls_total").increment(1);
        counter!("mcp.fs.find_paths.entries_total").increment(result.entries_examined as u64);

        Ok(result)
    }

    pub async fn search_text(
        &self,
        path: String,
        query: String,
        max_depth: Option<u32>,
    ) -> Result<SearchTextResult, AppError> {
        validate_search_query(&query)?;
        let start = Instant::now();
        let anchored = self.anchor(&path)?;
        let depth = max_depth
            .unwrap_or(MAX_SEARCH_DEPTH)
            .clamp(MIN_SEARCH_DEPTH, MAX_SEARCH_DEPTH);
        let result = tokio::task::spawn_blocking(move || {
            let root_fd = open_root_directory(&anchored.root_path)?;
            let target_fd = open_descendant_directory(root_fd, &anchored.relative_path)?;
            let mut state = SearchState::new(&query);
            collect_text_matches_descriptor_relative(
                target_fd,
                &anchored.display_path,
                depth,
                &mut state,
            )?;
            state.matches.sort_unstable_by(|left, right| {
                (&left.path, left.line_number, left.column_byte).cmp(&(
                    &right.path,
                    right.line_number,
                    right.column_byte,
                ))
            });

            let mut result = SearchTextResult {
                path: anchored.display_path.to_string_lossy().to_string(),
                matches: state.matches,
                truncated: state.truncated,
                entries_examined: state.entries_examined,
                files_scanned: state.files_scanned,
                bytes_scanned: state.bytes_scanned,
                skipped_oversized_files: state.skipped_oversized_files,
                skipped_invalid_utf8_files: state.skipped_invalid_utf8_files,
                skipped_unsafe_entries: state.skipped_unsafe_entries,
                skipped_unreadable_entries: state.skipped_unreadable_entries,
                query_bytes: query.len(),
                max_depth: depth,
                max_entries: MAX_SEARCH_ENTRIES,
                max_files: MAX_SEARCH_FILES,
                max_file_bytes: MAX_SEARCH_FILE_BYTES,
                max_total_bytes: MAX_SEARCH_TOTAL_BYTES,
                max_matches: MAX_SEARCH_MATCHES,
                max_response_bytes: MAX_SEARCH_RESPONSE_BYTES,
            };
            while serde_json::to_vec(&result)
                .map_err(std::io::Error::other)?
                .len()
                > MAX_SEARCH_STRUCTURED_BYTES
            {
                if result.matches.pop().is_none() {
                    return Err(AppError::Io(std::io::Error::other(
                        "search response metadata exceeds its bound",
                    )));
                }
                result.truncated = true;
            }
            Ok::<_, AppError>(result)
        })
        .await
        .map_err(filesystem_worker_error)??;

        histogram!("mcp.fs.search.latency_seconds").record(start.elapsed().as_secs_f64());
        counter!("mcp.fs.search.calls_total").increment(1);
        counter!("mcp.fs.search.bytes_total").increment(result.bytes_scanned as u64);

        Ok(result)
    }

    pub async fn write_file(
        &self,
        path: String,
        content: String,
        dry_run: Option<bool>,
    ) -> Result<String, AppError> {
        let start = Instant::now();
        let policy = WritePolicy::default();
        let content_bytes = content.len();
        let _audit_event =
            self.audit_write_decision(unix_timestamp_seconds(), content_bytes, dry_run);
        policy
            .validate_payload_size(content_bytes)
            .map_err(write_policy_error_to_app_error)?;

        let anchored = self.anchor(&path)?;

        if dry_run.unwrap_or(true) {
            counter!("mcp.fs.write.dry_runs_total").increment(1);
            return Ok("DRY-RUN".to_string());
        }
        let written_bytes = content.len();
        let (parent_relative, file_name) = split_parent_and_name(&anchored.relative_path)?;
        let root_fd = open_root_directory(&anchored.root_path)?;
        let parent_fd = open_descendant_directory(root_fd, &parent_relative)?;
        match descriptor_fs::statat(&parent_fd, &file_name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(metadata) if FileType::from_raw_mode(metadata.st_mode).is_symlink() => {
                return Err(path_rejected(
                    anchored.display_path.to_string_lossy().as_ref(),
                ));
            }
            Ok(_) | Err(rustix::io::Errno::NOENT) => {}
            Err(error) => return Err(descriptor_error(error)),
        }
        let temp_name = OsString::from(format!(
            ".{}.{}.tmp",
            file_name.to_string_lossy(),
            uuid::Uuid::new_v4()
        ));
        let temp_fd = descriptor_fs::openat(
            &parent_fd,
            &temp_name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(descriptor_error)?;
        let mut cleanup = DescriptorTempFileCleanup::new(&parent_fd, temp_name.clone());
        let mut file = tokio::fs::File::from_std(File::from(temp_fd));
        file.write_all(content.as_bytes()).await?;
        file.sync_all().await?;
        drop(file);
        descriptor_fs::renameat(&parent_fd, &temp_name, &parent_fd, &file_name)
            .map_err(descriptor_error)?;
        cleanup.disarm();
        descriptor_fs::fsync(&parent_fd).map_err(descriptor_error)?;

        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.fs.write.latency_seconds").record(duration);
        counter!("mcp.fs.write.bytes_total").increment(written_bytes as u64);

        Ok(format!("Wrote {written_bytes} bytes"))
    }
}

fn path_rejected(input: &str) -> AppError {
    AppError::PathTraversal {
        attempted: input.to_string(),
    }
}

fn validate_find_query(query: &str) -> Result<(), AppError> {
    if query.is_empty()
        || query.len() > MAX_FIND_QUERY_BYTES
        || query
            .chars()
            .any(|character| matches!(character, '\0' | '\n' | '\r' | '/'))
    {
        return Err(AppError::InvalidFindQuery);
    }
    Ok(())
}

fn collect_path_matches_descriptor_relative(
    root_fd: OwnedFd,
    root_path: &Path,
    max_depth: u32,
    state: &mut FindPathsState<'_>,
) -> Result<(), AppError> {
    let mut queue = VecDeque::new();
    queue.push_back((root_fd, root_path.to_path_buf(), 1_u32));

    'traversal: while let Some((dir_fd, dir_path, depth)) = queue.pop_front() {
        if state.execution_exhausted() {
            state.truncated = true;
            break;
        }

        let mut read_dir = Dir::read_from(&dir_fd).map_err(descriptor_error)?;
        let mut candidates = BTreeMap::new();
        for entry in &mut read_dir {
            if state.entries_examined >= MAX_FIND_ENTRIES {
                state.truncated = true;
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    state.skipped_unreadable_entries += 1;
                    continue;
                }
            };
            let name_bytes = entry.file_name().to_bytes();
            if name_bytes == b"." || name_bytes == b".." {
                continue;
            }
            state.entries_examined += 1;
            let name = OsString::from_vec(name_bytes.to_vec());
            let Some(name_key) = name.to_str().map(str::to_owned) else {
                state.skipped_invalid_utf8_entries += 1;
                continue;
            };
            let metadata = match descriptor_fs::statat(&dir_fd, &name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(metadata) => metadata,
                Err(_) => {
                    state.skipped_unreadable_entries += 1;
                    continue;
                }
            };
            let file_type = FileType::from_raw_mode(metadata.st_mode);
            let kind = if file_type.is_file() {
                FindPathKind::RegularFile
            } else if file_type.is_dir() {
                FindPathKind::Directory
            } else {
                state.skipped_unsafe_entries += 1;
                continue;
            };
            let display_path = dir_path.join(&name);
            candidates.insert(
                name_key,
                FindPendingEntry {
                    name,
                    display_path,
                    kind,
                },
            );
        }

        for (name_key, pending) in candidates {
            if state.matches.len() >= MAX_FIND_MATCHES {
                state.truncated = true;
                break 'traversal;
            }
            if name_key.contains(state.query) && state.kind_filter.matches(pending.kind) {
                state.matches.push(FindPathMatch {
                    path: pending.display_path.to_string_lossy().to_string(),
                    kind: pending.kind,
                });
            }
            if pending.kind == FindPathKind::Directory && depth < max_depth {
                match open_child_directory(&dir_fd, &pending.name) {
                    Ok(child_fd) => {
                        queue.push_back((child_fd, pending.display_path, depth + 1));
                    }
                    Err(_) => state.skipped_unreadable_entries += 1,
                }
            }
        }
    }

    Ok(())
}

fn validate_search_query(query: &str) -> Result<(), AppError> {
    if query.is_empty()
        || query.len() > MAX_SEARCH_QUERY_BYTES
        || query
            .chars()
            .any(|character| matches!(character, '\0' | '\n' | '\r'))
    {
        return Err(AppError::InvalidSearchQuery);
    }
    Ok(())
}

fn collect_text_matches_descriptor_relative(
    root_fd: OwnedFd,
    root_path: &Path,
    max_depth: u32,
    state: &mut SearchState<'_>,
) -> Result<(), AppError> {
    let mut queue = VecDeque::new();
    queue.push_back((root_fd, root_path.to_path_buf(), 1_u32));

    while let Some((dir_fd, dir_path, depth)) = queue.pop_front() {
        if state.execution_exhausted() || state.entries_examined >= MAX_SEARCH_ENTRIES {
            state.truncated = true;
            break;
        }

        let mut read_dir = Dir::read_from(&dir_fd).map_err(descriptor_error)?;
        let mut candidates = BTreeMap::new();

        for entry in &mut read_dir {
            if state.entries_examined >= MAX_SEARCH_ENTRIES {
                state.truncated = true;
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    state.skipped_unreadable_entries += 1;
                    continue;
                }
            };
            let name_bytes = entry.file_name().to_bytes();
            if name_bytes == b"." || name_bytes == b".." {
                continue;
            }
            state.entries_examined += 1;
            let name = OsString::from_vec(name_bytes.to_vec());
            let Some(name_key) = name.to_str().map(str::to_owned) else {
                state.skipped_unsafe_entries += 1;
                continue;
            };
            let metadata = match descriptor_fs::statat(&dir_fd, &name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(metadata) => metadata,
                Err(_) => {
                    state.skipped_unreadable_entries += 1;
                    continue;
                }
            };
            let file_type = FileType::from_raw_mode(metadata.st_mode);
            if file_type.is_symlink() || (!file_type.is_dir() && !file_type.is_file()) {
                state.skipped_unsafe_entries += 1;
                continue;
            }
            let display_path = dir_path.join(&name);
            candidates.insert(
                name_key,
                SearchPendingEntry {
                    name,
                    display_path,
                    file_type,
                    size: u64::try_from(metadata.st_size).unwrap_or(u64::MAX),
                },
            );
        }

        for (_, pending) in candidates {
            if state.execution_exhausted() {
                state.truncated = true;
                break;
            }
            if pending.file_type.is_dir() {
                if depth < max_depth {
                    match open_child_directory(&dir_fd, &pending.name) {
                        Ok(child_fd) => {
                            queue.push_back((child_fd, pending.display_path, depth + 1));
                        }
                        Err(_) => state.skipped_unreadable_entries += 1,
                    }
                }
                continue;
            }

            scan_search_file(&dir_fd, &pending, state)?;
        }
    }

    Ok(())
}

fn scan_search_file(
    parent_fd: &OwnedFd,
    pending: &SearchPendingEntry,
    state: &mut SearchState<'_>,
) -> Result<(), AppError> {
    if pending.size > MAX_SEARCH_FILE_BYTES as u64
        || pending.size as usize > MAX_SEARCH_TOTAL_BYTES.saturating_sub(state.bytes_scanned)
    {
        state.skipped_oversized_files += 1;
        state.truncated = true;
        return Ok(());
    }

    let file_fd = match descriptor_fs::openat(
        parent_fd,
        &pending.name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file_fd) => file_fd,
        Err(_) => {
            state.skipped_unreadable_entries += 1;
            return Ok(());
        }
    };
    let opened_metadata = match descriptor_fs::fstat(&file_fd) {
        Ok(metadata) => metadata,
        Err(_) => {
            state.skipped_unreadable_entries += 1;
            return Ok(());
        }
    };
    if !FileType::from_raw_mode(opened_metadata.st_mode).is_file() {
        state.skipped_unsafe_entries += 1;
        return Ok(());
    }

    let remaining_total = MAX_SEARCH_TOTAL_BYTES.saturating_sub(state.bytes_scanned);
    let read_limit = MAX_SEARCH_FILE_BYTES.min(remaining_total).saturating_add(1);
    let mut bytes = Vec::with_capacity(pending.size as usize);
    if File::from(file_fd)
        .take(read_limit as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        state.skipped_unreadable_entries += 1;
        return Ok(());
    }
    if bytes.len() > MAX_SEARCH_FILE_BYTES || bytes.len() > remaining_total {
        state.skipped_oversized_files += 1;
        state.truncated = true;
        return Ok(());
    }
    state.files_scanned += 1;
    state.bytes_scanned += bytes.len();

    let Ok(content) = std::str::from_utf8(&bytes) else {
        state.skipped_invalid_utf8_files += 1;
        return Ok(());
    };
    for (line_index, line) in content.split('\n').enumerate() {
        for (column, _) in line.match_indices(state.query) {
            if state.matches.len() >= MAX_SEARCH_MATCHES {
                state.truncated = true;
                return Ok(());
            }
            state.matches.push(SearchTextMatch {
                path: pending.display_path.to_string_lossy().to_string(),
                line_number: line_index + 1,
                column_byte: column + 1,
            });
        }
    }
    Ok(())
}

struct VerifiedRegularFile {
    file: File,
    size_bytes: u64,
}

struct BinaryRangeRead {
    bytes: Vec<u8>,
    file_size_bytes: u64,
    eof: bool,
}

struct TextRangeRead {
    content: String,
    file_size_bytes: u64,
    eof: bool,
}

fn open_verified_regular_file(
    anchored: &AnchoredPath,
    max_bytes: usize,
) -> Result<VerifiedRegularFile, AppError> {
    let (parent_relative, file_name) = split_parent_and_name(&anchored.relative_path)?;
    let root_fd = open_root_directory(&anchored.root_path)?;
    let parent_fd = open_metadata_parent_directory(root_fd, &parent_relative)?;
    let path_metadata = descriptor_fs::statat(&parent_fd, &file_name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| {
            if error == rustix::io::Errno::NOENT {
                AppError::PathNotFound
            } else {
                descriptor_error(error)
            }
        })?;
    let path_type = FileType::from_raw_mode(path_metadata.st_mode);
    if path_type.is_symlink() {
        return Err(path_rejected(
            anchored.display_path.to_string_lossy().as_ref(),
        ));
    }
    if !path_type.is_file() {
        return Err(AppError::UnsupportedPathType);
    }
    let reported_size = u64::try_from(path_metadata.st_size)
        .map_err(|_| AppError::Io(std::io::Error::other("file reported an invalid size")))?;
    if reported_size > max_bytes as u64 {
        return Err(AppError::FileTooLarge {
            size: reported_size,
            max_size: max_bytes as u64,
        });
    }

    let file_fd = descriptor_fs::openat(
        &parent_fd,
        &file_name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| match error {
        rustix::io::Errno::NOENT => AppError::PathNotFound,
        rustix::io::Errno::LOOP => path_rejected(anchored.display_path.to_string_lossy().as_ref()),
        _ => descriptor_error(error),
    })?;
    let opened_metadata = descriptor_fs::fstat(&file_fd).map_err(descriptor_error)?;
    if !FileType::from_raw_mode(opened_metadata.st_mode).is_file() {
        return Err(AppError::UnsupportedPathType);
    }
    if opened_metadata.st_dev != path_metadata.st_dev
        || opened_metadata.st_ino != path_metadata.st_ino
    {
        return Err(path_rejected(
            anchored.display_path.to_string_lossy().as_ref(),
        ));
    }
    let opened_size = u64::try_from(opened_metadata.st_size)
        .map_err(|_| AppError::Io(std::io::Error::other("file reported an invalid size")))?;
    if opened_size > max_bytes as u64 {
        return Err(AppError::FileTooLarge {
            size: opened_size,
            max_size: max_bytes as u64,
        });
    }
    Ok(VerifiedRegularFile {
        file: File::from(file_fd),
        size_bytes: opened_size,
    })
}

fn read_verified_binary_range(
    mut verified: VerifiedRegularFile,
    offset_bytes: u64,
    length_bytes: usize,
) -> Result<BinaryRangeRead, AppError> {
    if offset_bytes > verified.size_bytes || !(1..=MAX_BINARY_RANGE_BYTES).contains(&length_bytes) {
        return Err(AppError::InvalidBinaryRange);
    }

    verified.file.seek(SeekFrom::Start(offset_bytes))?;
    let mut bytes = Vec::with_capacity(length_bytes.min(64 * 1_024));
    {
        let mut reader = (&mut verified.file).take(length_bytes as u64);
        reader.read_to_end(&mut bytes)?;
    }

    let post_metadata = descriptor_fs::fstat(&verified.file).map_err(descriptor_error)?;
    if !FileType::from_raw_mode(post_metadata.st_mode).is_file() {
        return Err(AppError::UnsupportedPathType);
    }
    let post_size = u64::try_from(post_metadata.st_size)
        .map_err(|_| AppError::Io(std::io::Error::other("file reported an invalid size")))?;
    if post_size != verified.size_bytes {
        return Err(AppError::FileChangedDuringRead);
    }

    let end_bytes = offset_bytes
        .checked_add(bytes.len() as u64)
        .ok_or(AppError::InvalidBinaryRange)?;
    Ok(BinaryRangeRead {
        bytes,
        file_size_bytes: verified.size_bytes,
        eof: end_bytes >= verified.size_bytes,
    })
}

fn read_verified_text_range(
    mut verified: VerifiedRegularFile,
    offset_bytes: u64,
    max_bytes: usize,
) -> Result<TextRangeRead, AppError> {
    if offset_bytes > verified.size_bytes
        || !(MIN_TEXT_RANGE_BYTES..=MAX_TEXT_RANGE_BYTES).contains(&max_bytes)
    {
        return Err(AppError::InvalidTextRange);
    }

    verified.file.seek(SeekFrom::Start(offset_bytes))?;
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1_024));
    {
        let mut reader = (&mut verified.file).take(max_bytes as u64);
        reader.read_to_end(&mut bytes)?;
    }

    let post_metadata = descriptor_fs::fstat(&verified.file).map_err(descriptor_error)?;
    if !FileType::from_raw_mode(post_metadata.st_mode).is_file() {
        return Err(AppError::UnsupportedPathType);
    }
    let post_size = u64::try_from(post_metadata.st_size)
        .map_err(|_| AppError::Io(std::io::Error::other("file reported an invalid size")))?;
    if post_size != verified.size_bytes {
        return Err(AppError::FileChangedDuringRead);
    }

    if bytes
        .first()
        .is_some_and(|byte| byte & 0b1100_0000 == 0b1000_0000)
    {
        return Err(AppError::InvalidTextRange);
    }

    let physical_end = offset_bytes
        .checked_add(bytes.len() as u64)
        .ok_or(AppError::InvalidTextRange)?;
    let content = match std::str::from_utf8(&bytes) {
        Ok(content) => content.to_owned(),
        Err(error) if error.error_len().is_none() && physical_end < verified.size_bytes => {
            bytes.truncate(error.valid_up_to());
            String::from_utf8(bytes).expect("the UTF-8 validator supplied a valid prefix")
        }
        Err(_) => return Err(AppError::InvalidFileEncoding),
    };
    let logical_end = offset_bytes
        .checked_add(content.len() as u64)
        .ok_or(AppError::InvalidTextRange)?;

    Ok(TextRangeRead {
        content,
        file_size_bytes: verified.size_bytes,
        eof: logical_end >= verified.size_bytes,
    })
}

fn base64_encoded_len(byte_len: usize) -> Option<usize> {
    byte_len.checked_add(2)?.checked_div(3)?.checked_mul(4)
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn read_bounded_bytes(reader: impl Read, max_bytes: usize) -> Result<Vec<u8>, AppError> {
    let read_limit = max_bytes
        .checked_add(1)
        .ok_or_else(|| AppError::Io(std::io::Error::other("file byte limit overflowed")))?;
    let mut reader = reader.take(read_limit as u64);
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1_024));
    reader.read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(AppError::FileTooLarge {
            size: bytes.len() as u64,
            max_size: max_bytes as u64,
        });
    }
    Ok(bytes)
}

fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(
        base64_encoded_len(bytes.len()).expect("bounded file length has a base64 length"),
    );
    let mut index = 0;
    while index + 3 <= bytes.len() {
        let first = bytes[index];
        let second = bytes[index + 1];
        let third = bytes[index + 2];
        encoded.push(ALPHABET[(first >> 2) as usize] as char);
        encoded.push(ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        encoded.push(ALPHABET[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char);
        encoded.push(ALPHABET[(third & 0x3f) as usize] as char);
        index += 3;
    }

    match bytes.len() - index {
        1 => {
            let first = bytes[index];
            encoded.push(ALPHABET[(first >> 2) as usize] as char);
            encoded.push(ALPHABET[((first & 0x03) << 4) as usize] as char);
            encoded.push('=');
            encoded.push('=');
        }
        2 => {
            let first = bytes[index];
            let second = bytes[index + 1];
            encoded.push(ALPHABET[(first >> 2) as usize] as char);
            encoded.push(ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
            encoded.push(ALPHABET[((second & 0x0f) << 2) as usize] as char);
            encoded.push('=');
        }
        _ => {}
    }
    debug_assert_eq!(encoded.len(), base64_encoded_len(bytes.len()).unwrap());
    encoded
}

fn descriptor_error(error: rustix::io::Errno) -> AppError {
    AppError::Io(std::io::Error::from_raw_os_error(error.raw_os_error()))
}

fn filesystem_worker_error(_error: tokio::task::JoinError) -> AppError {
    AppError::Io(std::io::Error::other("filesystem worker failed"))
}

fn open_root_directory(root: &Path) -> Result<OwnedFd, AppError> {
    descriptor_fs::open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(descriptor_error)
}

fn open_child_directory(parent: &OwnedFd, name: &OsStr) -> Result<OwnedFd, AppError> {
    descriptor_fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(descriptor_error)
}

fn open_descendant_directory(
    mut directory: OwnedFd,
    relative_path: &Path,
) -> Result<OwnedFd, AppError> {
    for component in relative_path.components() {
        let Component::Normal(name) = component else {
            return Err(path_rejected(relative_path.to_string_lossy().as_ref()));
        };
        directory = open_child_directory(&directory, name)
            .map_err(|_| path_rejected(relative_path.to_string_lossy().as_ref()))?;
    }
    Ok(directory)
}

fn open_metadata_descriptor(root_fd: OwnedFd, relative_path: &Path) -> Result<OwnedFd, AppError> {
    if relative_path.as_os_str().is_empty() {
        return Ok(root_fd);
    }

    let (parent_relative, file_name) = split_parent_and_name(relative_path)?;
    let parent_fd = open_metadata_parent_directory(root_fd, &parent_relative)?;
    descriptor_fs::openat(
        &parent_fd,
        &file_name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        if error == rustix::io::Errno::NOENT {
            AppError::PathNotFound
        } else {
            descriptor_error(error)
        }
    })
}

fn open_metadata_parent_directory(
    mut directory: OwnedFd,
    relative_path: &Path,
) -> Result<OwnedFd, AppError> {
    for component in relative_path.components() {
        let Component::Normal(name) = component else {
            return Err(path_rejected(relative_path.to_string_lossy().as_ref()));
        };
        let child = descriptor_fs::openat(
            &directory,
            name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| {
            if error == rustix::io::Errno::NOENT {
                AppError::PathNotFound
            } else {
                descriptor_error(error)
            }
        })?;
        let metadata = descriptor_fs::fstat(&child).map_err(descriptor_error)?;
        let file_type = FileType::from_raw_mode(metadata.st_mode);
        if file_type.is_symlink() {
            return Err(path_rejected(relative_path.to_string_lossy().as_ref()));
        }
        if !file_type.is_dir() {
            return Err(AppError::PathNotFound);
        }
        directory = child;
    }
    Ok(directory)
}

fn open_mutation_parent_directory(
    root_fd: OwnedFd,
    relative_path: &Path,
) -> Result<OwnedFd, AppError> {
    let parent = open_metadata_parent_directory(root_fd, relative_path)?;
    descriptor_fs::openat(
        &parent,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(descriptor_error)
}

fn split_parent_and_name(relative_path: &Path) -> Result<(PathBuf, OsString), AppError> {
    let Some(name) = relative_path.file_name() else {
        return Err(path_rejected(relative_path.to_string_lossy().as_ref()));
    };
    let parent = relative_path.parent().unwrap_or_else(|| Path::new(""));
    Ok((parent.to_path_buf(), name.to_os_string()))
}

fn create_directory_result(anchored: &AnchoredPath, dry_run: bool) -> CreateDirectoryResult {
    CreateDirectoryResult {
        path: anchored.display_path.to_string_lossy().to_string(),
        dry_run,
        mode: "0700".to_owned(),
        max_response_bytes: MAX_CREATE_DIRECTORY_RESPONSE_BYTES,
    }
}

fn prepare_create_directory(
    anchored: AnchoredPath,
    dry_run: bool,
) -> Result<PreparedCreateDirectoryMutation, AppError> {
    let started = Instant::now();
    let (parent_relative, directory_name) = split_parent_and_name(&anchored.relative_path)?;
    let root_fd = open_root_directory(&anchored.root_path)?;
    let root_metadata = descriptor_fs::fstat(&root_fd).map_err(descriptor_error)?;
    if !FileType::from_raw_mode(root_metadata.st_mode).is_dir() {
        return Err(path_rejected(
            anchored.display_path.to_string_lossy().as_ref(),
        ));
    }
    let mut normalized_components = Vec::new();
    for component in anchored.relative_path.components() {
        let Component::Normal(component) = component else {
            return Err(path_rejected(
                anchored.display_path.to_string_lossy().as_ref(),
            ));
        };
        normalized_components.push(component.as_bytes());
    }
    let grant_target = CreateDirectoryGrantTarget::from_normalized_components(
        root_metadata.st_dev,
        root_metadata.st_ino,
        normalized_components,
    )
    .map_err(|_| {
        AppError::Io(std::io::Error::other(
            "create directory authorization target is invalid",
        ))
    })?;
    let parent_fd = open_mutation_parent_directory(root_fd, &parent_relative)?;

    match descriptor_fs::statat(&parent_fd, &directory_name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) if FileType::from_raw_mode(metadata.st_mode).is_symlink() => {
            return Err(path_rejected(
                anchored.display_path.to_string_lossy().as_ref(),
            ));
        }
        Ok(_) => return Err(AppError::PathAlreadyExists),
        Err(rustix::io::Errno::NOENT) => {}
        Err(error) => return Err(descriptor_error(error)),
    }

    Ok(PreparedCreateDirectoryMutation {
        result: create_directory_result(&anchored, dry_run),
        parent_fd,
        directory_name,
        grant_target,
        started,
    })
}

fn copy_file_result(
    source: &AnchoredPath,
    destination: &AnchoredPath,
    dry_run: bool,
    size_bytes: usize,
) -> CopyFileResult {
    CopyFileResult {
        source_path: source.display_path.to_string_lossy().to_string(),
        destination_path: destination.display_path.to_string_lossy().to_string(),
        dry_run,
        size_bytes,
        mode: "0600".to_owned(),
        max_file_bytes: MAX_COPY_FILE_BYTES,
        max_response_bytes: MAX_COPY_FILE_RESPONSE_BYTES,
    }
}

fn copy_source_parent_error(error: AppError) -> AppError {
    match error {
        AppError::PathNotFound => AppError::CopySourceNotFound,
        other => other,
    }
}

fn copy_destination_parent_error(error: AppError) -> AppError {
    match error {
        AppError::PathNotFound => AppError::CopyDestinationParentNotFound,
        other => other,
    }
}

fn copy_source_size(metadata: &descriptor_fs::Stat) -> Result<u64, AppError> {
    u64::try_from(metadata.st_size).map_err(|_| {
        AppError::Io(std::io::Error::other(
            "copy source reported an invalid size",
        ))
    })
}

fn copy_file_identity_and_contract_match(
    metadata: &descriptor_fs::Stat,
    expected_device: u64,
    expected_inode: u64,
    expected_size: usize,
) -> bool {
    FileType::from_raw_mode(metadata.st_mode).is_file()
        && metadata.st_dev == expected_device
        && metadata.st_ino == expected_inode
        && (metadata.st_mode & 0o7777) == COPY_FILE_MODE
        && u64::try_from(metadata.st_size).ok() == Some(expected_size as u64)
}

fn stat_modified_time(metadata: &descriptor_fs::Stat) -> Option<String> {
    let seconds = u64::try_from(metadata.st_mtime).ok()?;
    let modified = UNIX_EPOCH.checked_add(Duration::from_secs(seconds))?;
    let datetime: chrono::DateTime<chrono::Utc> = modified.into();
    Some(datetime.to_rfc3339())
}

fn write_policy_error_to_app_error(error: WritePolicyError) -> AppError {
    match error {
        WritePolicyError::PayloadTooLarge { bytes, max_bytes } => AppError::WritePayloadTooLarge {
            size: usize_to_u64(bytes),
            max_size: usize_to_u64(max_bytes),
        },
    }
}

fn usize_to_u64(value: usize) -> u64 {
    value as u64
}

fn unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(u64::MAX, |duration| duration.as_secs())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub modified: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDirResult {
    pub path: String,
    pub entries: Vec<FileInfo>,
    pub truncated: bool,
    pub max_entries: usize,
    pub max_response_bytes: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReadFileResult {
    pub path: String,
    pub content: String,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadBinaryFileResult {
    pub encoding: String,
    pub data: String,
    pub size_bytes: usize,
    pub max_file_bytes: usize,
    pub max_response_bytes: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadBinaryRangeResult {
    pub encoding: String,
    pub data: String,
    pub offset_bytes: u64,
    pub size_bytes: usize,
    pub file_size_bytes: u64,
    pub eof: bool,
    pub max_read_bytes: usize,
    pub max_file_bytes: usize,
    pub max_response_bytes: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadTextRangeResult {
    pub content: String,
    pub offset_bytes: u64,
    pub next_offset_bytes: u64,
    pub size_bytes: usize,
    pub file_size_bytes: u64,
    pub eof: bool,
    pub max_read_bytes: usize,
    pub max_file_bytes: usize,
    pub max_response_bytes: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HashFileResult {
    pub algorithm: String,
    pub digest: String,
    pub size_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathMetadataKind {
    RegularFile,
    Directory,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathMetadataResult {
    pub path: String,
    pub kind: PathMetadataKind,
    pub size_bytes: Option<u64>,
    pub modified: Option<String>,
    pub max_response_bytes: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDirectoryResult {
    pub path: String,
    pub dry_run: bool,
    pub mode: String,
    pub max_response_bytes: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyFileResult {
    pub source_path: String,
    pub destination_path: String,
    pub dry_run: bool,
    pub size_bytes: usize,
    pub mode: String,
    pub max_file_bytes: usize,
    pub max_response_bytes: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindPathFilter {
    #[default]
    Any,
    RegularFile,
    Directory,
}

impl FindPathFilter {
    const fn matches(self, kind: FindPathKind) -> bool {
        match self {
            Self::Any => true,
            Self::RegularFile => matches!(kind, FindPathKind::RegularFile),
            Self::Directory => matches!(kind, FindPathKind::Directory),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindPathKind {
    RegularFile,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindPathMatch {
    pub path: String,
    pub kind: FindPathKind,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindPathsResult {
    pub path: String,
    pub matches: Vec<FindPathMatch>,
    pub truncated: bool,
    pub entries_examined: usize,
    pub skipped_invalid_utf8_entries: usize,
    pub skipped_unsafe_entries: usize,
    pub skipped_unreadable_entries: usize,
    pub query_bytes: usize,
    pub kind_filter: FindPathFilter,
    pub max_depth: u32,
    pub max_entries: usize,
    pub max_matches: usize,
    pub max_response_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTextMatch {
    pub path: String,
    pub line_number: usize,
    pub column_byte: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTextResult {
    pub path: String,
    pub matches: Vec<SearchTextMatch>,
    pub truncated: bool,
    pub entries_examined: usize,
    pub files_scanned: usize,
    pub bytes_scanned: usize,
    pub skipped_oversized_files: usize,
    pub skipped_invalid_utf8_files: usize,
    pub skipped_unsafe_entries: usize,
    pub skipped_unreadable_entries: usize,
    pub query_bytes: usize,
    pub max_depth: u32,
    pub max_entries: usize,
    pub max_files: usize,
    pub max_file_bytes: usize,
    pub max_total_bytes: usize,
    pub max_matches: usize,
    pub max_response_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::{symlink, PermissionsExt};

    use crate::audit::{AuditDecision, AuditMode};
    use crate::write_policy::DEFAULT_MAX_WRITE_BYTES;

    fn assert_rejected(result: Result<PathBuf, AppError>) {
        assert!(
            matches!(result, Err(AppError::PathTraversal { .. })),
            "expected safe-root rejection"
        );
    }

    #[test]
    fn base64_encoder_matches_rfc_4648_canonical_vectors() {
        for (plain, expected) in [
            (b"".as_slice(), ""),
            (b"f".as_slice(), "Zg=="),
            (b"fo".as_slice(), "Zm8="),
            (b"foo".as_slice(), "Zm9v"),
            (b"foob".as_slice(), "Zm9vYg=="),
            (b"fooba".as_slice(), "Zm9vYmE="),
            (b"foobar".as_slice(), "Zm9vYmFy"),
            (&[0x00, 0xff, 0x10, 0x80], "AP8QgA=="),
        ] {
            let encoded = encode_base64(plain);
            assert_eq!(encoded, expected);
            assert_eq!(encoded.len(), base64_encoded_len(plain.len()).unwrap());
        }
        assert_eq!(
            base64_encoded_len(MAX_BINARY_READ_BYTES),
            Some(MAX_BINARY_READ_BASE64_BYTES)
        );
        assert_eq!(
            base64_encoded_len(MAX_BINARY_RANGE_BYTES),
            Some(MAX_BINARY_RANGE_BASE64_BYTES)
        );
        assert_eq!(base64_encoded_len(usize::MAX), None);
    }

    #[test]
    fn bounded_reader_rejects_runtime_growth_without_returning_partial_data() {
        let error = read_bounded_bytes(std::io::Cursor::new(vec![0x5a; 65]), 64).unwrap_err();
        assert!(matches!(
            error,
            AppError::FileTooLarge {
                size: 65,
                max_size: 64
            }
        ));
        assert_eq!(
            read_bounded_bytes(std::io::Cursor::new(vec![0xa5; 64]), 64).unwrap(),
            vec![0xa5; 64]
        );
    }

    #[tokio::test]
    async fn binary_read_returns_canonical_content_without_path_metadata() {
        let root = tempfile::tempdir().unwrap();
        let binary_path = root.path().join("payload.bin");
        let empty_path = root.path().join("empty.bin");
        std::fs::write(&binary_path, [0x00, 0xff, 0x10, 0x80]).unwrap();
        std::fs::write(&empty_path, []).unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        let binary = tools
            .read_binary_file(binary_path.to_string_lossy().to_string())
            .await
            .unwrap();
        assert_eq!(binary.encoding, "base64");
        assert_eq!(binary.data, "AP8QgA==");
        assert_eq!(binary.size_bytes, 4);
        assert_eq!(binary.max_file_bytes, MAX_BINARY_READ_BYTES);
        assert_eq!(binary.max_response_bytes, MAX_BINARY_READ_RESPONSE_BYTES);
        let serialized = serde_json::to_value(binary).unwrap();
        assert_eq!(
            serialized.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec![
                "data",
                "encoding",
                "maxFileBytes",
                "maxResponseBytes",
                "sizeBytes"
            ]
        );
        assert!(!serialized.to_string().contains("payload.bin"));

        let empty = tools
            .read_binary_file(empty_path.to_string_lossy().to_string())
            .await
            .unwrap();
        assert!(empty.data.is_empty());
        assert_eq!(empty.size_bytes, 0);
    }

    #[tokio::test]
    async fn binary_range_returns_canonical_slice_and_explicit_eof_without_path_metadata() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("range.bin");
        std::fs::write(&path, [0x00, 0xff, 0x80, b'a', b'\n', 0x01, 0xfe]).unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        let range = tools
            .read_binary_range(path.to_string_lossy().to_string(), 2, 4)
            .await
            .unwrap();
        assert_eq!(range.encoding, "base64");
        assert_eq!(range.data, "gGEKAQ==");
        assert_eq!(range.offset_bytes, 2);
        assert_eq!(range.size_bytes, 4);
        assert_eq!(range.file_size_bytes, 7);
        assert!(!range.eof);
        assert_eq!(range.max_read_bytes, MAX_BINARY_RANGE_BYTES);
        assert_eq!(range.max_file_bytes, MAX_BINARY_RANGE_FILE_BYTES);
        assert_eq!(range.max_response_bytes, MAX_BINARY_RANGE_RESPONSE_BYTES);
        let serialized = serde_json::to_value(range).unwrap();
        assert_eq!(
            serialized.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec![
                "data",
                "encoding",
                "eof",
                "fileSizeBytes",
                "maxFileBytes",
                "maxReadBytes",
                "maxResponseBytes",
                "offsetBytes",
                "sizeBytes"
            ]
        );
        assert!(!serialized.to_string().contains("range.bin"));

        let short_final = tools
            .read_binary_range(path.to_string_lossy().to_string(), 5, 10)
            .await
            .unwrap();
        assert_eq!(short_final.data, "Af4=");
        assert_eq!(short_final.size_bytes, 2);
        assert_eq!(short_final.file_size_bytes, 7);
        assert!(short_final.eof);

        let eof = tools
            .read_binary_range(path.to_string_lossy().to_string(), 7, 1)
            .await
            .unwrap();
        assert!(eof.data.is_empty());
        assert_eq!(eof.size_bytes, 0);
        assert!(eof.eof);

        assert!(matches!(
            tools
                .read_binary_range(path.to_string_lossy().to_string(), 8, 1)
                .await,
            Err(AppError::InvalidBinaryRange)
        ));
        for length in [0, MAX_BINARY_RANGE_BYTES + 1] {
            assert!(matches!(
                tools
                    .read_binary_range(path.to_string_lossy().to_string(), 0, length)
                    .await,
                Err(AppError::InvalidBinaryRange)
            ));
        }
    }

    #[tokio::test]
    async fn binary_range_enforces_exact_range_and_file_limits() {
        let root = tempfile::tempdir().unwrap();
        let exact_range = root.path().join("exact-range.bin");
        let exact_file = root.path().join("exact-file.bin");
        let oversized_file = root.path().join("oversized-file.bin");
        std::fs::write(&exact_range, vec![0xa5; MAX_BINARY_RANGE_BYTES]).unwrap();
        File::create(&exact_file)
            .unwrap()
            .set_len(MAX_BINARY_RANGE_FILE_BYTES as u64)
            .unwrap();
        File::create(&oversized_file)
            .unwrap()
            .set_len((MAX_BINARY_RANGE_FILE_BYTES + 1) as u64)
            .unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        let range = tools
            .read_binary_range(
                exact_range.to_string_lossy().to_string(),
                0,
                MAX_BINARY_RANGE_BYTES,
            )
            .await
            .unwrap();
        assert_eq!(range.size_bytes, MAX_BINARY_RANGE_BYTES);
        assert_eq!(range.data.len(), MAX_BINARY_RANGE_BASE64_BYTES);
        assert!(range.eof);

        let exact_eof = tools
            .read_binary_range(
                exact_file.to_string_lossy().to_string(),
                MAX_BINARY_RANGE_FILE_BYTES as u64,
                1,
            )
            .await
            .unwrap();
        assert_eq!(exact_eof.size_bytes, 0);
        assert!(exact_eof.eof);

        assert!(matches!(
            tools
                .read_binary_range(oversized_file.to_string_lossy().to_string(), 0, 1)
                .await,
            Err(AppError::FileTooLarge { size, max_size })
                if size == (MAX_BINARY_RANGE_FILE_BYTES + 1) as u64
                    && max_size == MAX_BINARY_RANGE_FILE_BYTES as u64
        ));
    }

    #[test]
    fn binary_range_rejects_concurrent_size_change_without_returning_partial_data() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("changing.bin");
        std::fs::write(&path, b"12345678").unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let anchored = tools.anchor(path.to_string_lossy().as_ref()).unwrap();
        let file = open_verified_regular_file(&anchored, MAX_BINARY_RANGE_FILE_BYTES).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(9)
            .unwrap();

        assert!(matches!(
            read_verified_binary_range(file, 0, 4),
            Err(AppError::FileChangedDuringRead)
        ));
    }

    #[tokio::test]
    async fn text_range_pages_only_on_utf8_boundaries_and_reports_exact_offsets() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("range.txt");
        std::fs::write(&path, "aé🙂z").unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let path = path.to_string_lossy().to_string();

        let first = tools
            .read_text_range(path.clone(), 0, MIN_TEXT_RANGE_BYTES)
            .await
            .unwrap();
        assert_eq!(first.content, "aé");
        assert_eq!(first.offset_bytes, 0);
        assert_eq!(first.next_offset_bytes, 3);
        assert_eq!(first.size_bytes, 3);
        assert_eq!(first.file_size_bytes, 8);
        assert!(!first.eof);
        assert_eq!(first.max_read_bytes, MAX_TEXT_RANGE_BYTES);
        assert_eq!(first.max_file_bytes, MAX_TEXT_RANGE_FILE_BYTES);
        assert_eq!(first.max_response_bytes, MAX_TEXT_RANGE_RESPONSE_BYTES);

        let second = tools
            .read_text_range(path.clone(), first.next_offset_bytes, MIN_TEXT_RANGE_BYTES)
            .await
            .unwrap();
        assert_eq!(second.content, "🙂");
        assert_eq!(second.next_offset_bytes, 7);
        assert!(!second.eof);

        let final_page = tools
            .read_text_range(path.clone(), second.next_offset_bytes, MIN_TEXT_RANGE_BYTES)
            .await
            .unwrap();
        assert_eq!(final_page.content, "z");
        assert_eq!(final_page.next_offset_bytes, 8);
        assert!(final_page.eof);

        let eof = tools
            .read_text_range(path.clone(), 8, MIN_TEXT_RANGE_BYTES)
            .await
            .unwrap();
        assert!(eof.content.is_empty());
        assert_eq!(eof.next_offset_bytes, 8);
        assert!(eof.eof);

        assert!(matches!(
            tools.read_text_range(path, 2, MIN_TEXT_RANGE_BYTES).await,
            Err(AppError::InvalidTextRange)
        ));
    }

    #[tokio::test]
    async fn text_range_rejects_invalid_arguments_encoding_and_file_size() {
        let root = tempfile::tempdir().unwrap();
        let invalid = root.path().join("invalid.txt");
        let truncated = root.path().join("truncated.txt");
        let exact_file = root.path().join("exact-file.txt");
        let oversized_file = root.path().join("oversized-file.txt");
        std::fs::write(&invalid, [b'a', 0xff]).unwrap();
        std::fs::write(&truncated, [b'a', 0xf0, 0x9f]).unwrap();
        File::create(&exact_file)
            .unwrap()
            .set_len(MAX_TEXT_RANGE_FILE_BYTES as u64)
            .unwrap();
        File::create(&oversized_file)
            .unwrap()
            .set_len((MAX_TEXT_RANGE_FILE_BYTES + 1) as u64)
            .unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        for path in [invalid, truncated] {
            assert!(matches!(
                tools
                    .read_text_range(path.to_string_lossy().to_string(), 0, MIN_TEXT_RANGE_BYTES,)
                    .await,
                Err(AppError::InvalidFileEncoding)
            ));
        }

        let exact_eof = tools
            .read_text_range(
                exact_file.to_string_lossy().to_string(),
                MAX_TEXT_RANGE_FILE_BYTES as u64,
                MIN_TEXT_RANGE_BYTES,
            )
            .await
            .unwrap();
        assert!(exact_eof.content.is_empty());
        assert!(exact_eof.eof);

        assert!(matches!(
            tools
                .read_text_range(
                    oversized_file.to_string_lossy().to_string(),
                    0,
                    MIN_TEXT_RANGE_BYTES,
                )
                .await,
            Err(AppError::FileTooLarge { size, max_size })
                if size == (MAX_TEXT_RANGE_FILE_BYTES + 1) as u64
                    && max_size == MAX_TEXT_RANGE_FILE_BYTES as u64
        ));

        for max_bytes in [MIN_TEXT_RANGE_BYTES - 1, MAX_TEXT_RANGE_BYTES + 1] {
            assert!(matches!(
                tools
                    .read_text_range(exact_file.to_string_lossy().to_string(), 0, max_bytes,)
                    .await,
                Err(AppError::InvalidTextRange)
            ));
        }
        assert!(matches!(
            tools
                .read_text_range(
                    exact_file.to_string_lossy().to_string(),
                    MAX_TEXT_RANGE_FILE_BYTES as u64 + 1,
                    MIN_TEXT_RANGE_BYTES,
                )
                .await,
            Err(AppError::InvalidTextRange)
        ));
    }

    #[tokio::test]
    async fn text_range_accepts_the_exact_content_limit_and_redacts_path_metadata() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("private-range.txt");
        std::fs::write(&path, "x".repeat(MAX_TEXT_RANGE_BYTES)).unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        let result = tools
            .read_text_range(path.to_string_lossy().to_string(), 0, MAX_TEXT_RANGE_BYTES)
            .await
            .unwrap();
        assert_eq!(result.content.len(), MAX_TEXT_RANGE_BYTES);
        assert_eq!(result.next_offset_bytes, MAX_TEXT_RANGE_BYTES as u64);
        assert!(result.eof);
        let serialized = serde_json::to_value(result).unwrap();
        assert_eq!(
            serialized.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec![
                "content",
                "eof",
                "fileSizeBytes",
                "maxFileBytes",
                "maxReadBytes",
                "maxResponseBytes",
                "nextOffsetBytes",
                "offsetBytes",
                "sizeBytes"
            ]
        );
        assert!(!serialized.to_string().contains("private-range.txt"));
    }

    #[test]
    fn text_range_rejects_concurrent_size_change_without_partial_content() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("changing.txt");
        std::fs::write(&path, b"12345678").unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let anchored = tools.anchor(path.to_string_lossy().as_ref()).unwrap();
        let file = open_verified_regular_file(&anchored, MAX_TEXT_RANGE_FILE_BYTES).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(9)
            .unwrap();

        assert!(matches!(
            read_verified_text_range(file, 0, MIN_TEXT_RANGE_BYTES),
            Err(AppError::FileChangedDuringRead)
        ));
    }

    #[test]
    fn sanitize_rejects_empty_relative_nul_and_parent_components() {
        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        assert_rejected(tools.sanitize(""));
        assert_rejected(tools.sanitize("   "));
        assert_rejected(tools.sanitize("relative.txt"));
        assert_rejected(tools.sanitize("relative/child.txt"));
        assert_rejected(tools.sanitize("/tmp/bad\0name"));

        let parent_reference = root.path().join("..").join("outside.txt");
        assert_rejected(tools.sanitize(parent_reference.to_string_lossy().as_ref()));
    }

    #[test]
    fn sanitize_rejects_new_file_when_existing_parent_is_outside_safe_root() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let target = other.path().join("outside.txt");

        assert_rejected(tools.sanitize(target.to_string_lossy().as_ref()));
    }

    #[test]
    fn sanitize_rejects_new_file_when_parent_does_not_exist() {
        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let target = root.path().join("missing-parent").join("file.txt");

        assert_rejected(tools.sanitize(target.to_string_lossy().as_ref()));
    }

    #[test]
    fn sanitize_allows_new_file_under_existing_safe_root_parent() {
        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let target = root.path().join("new-file.txt");

        let sanitized = tools.sanitize(target.to_string_lossy().as_ref()).unwrap();
        let expected = root.path().canonicalize().unwrap().join("new-file.txt");

        assert_eq!(sanitized, expected);
        assert!(!target.exists());
    }

    #[test]
    fn write_file_audit_defaults_to_dry_run_without_sensitive_fields() {
        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let event = tools.audit_write_decision(1_725_000_000, 14, None);

        assert_eq!(event.timestamp_unix_seconds, 1_725_000_000);
        assert_eq!(event.tool_name, "write_file");
        assert_eq!(event.gate_name, "filesystem_write");
        assert_eq!(event.mode, AuditMode::DryRun);
        assert_eq!(event.decision, AuditDecision::Allowed);
        assert_eq!(event.reason_code, "dry_run_preview");
        assert_eq!(event.metadata["content_bytes"], 14);
        assert_eq!(
            event.metadata["max_bytes"],
            WritePolicy::default().max_write_bytes() as u64
        );

        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value.get("path"), None);
        assert_eq!(value.get("content"), None);
        assert_eq!(value.get("file_content"), None);
    }

    #[test]
    fn write_file_audit_tracks_explicit_mutation_without_runtime_surface_change() {
        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let event = tools.audit_write_decision(2, 38, Some(false));

        assert_eq!(event.mode, AuditMode::Mutating);
        assert_eq!(event.decision, AuditDecision::Allowed);
        assert_eq!(event.reason_code, "explicit_mutation");
        assert_eq!(event.metadata["content_bytes"], 38);
    }

    #[test]
    fn write_policy_error_maps_to_payload_too_large_app_error() {
        let error = write_policy_error_to_app_error(WritePolicyError::PayloadTooLarge {
            bytes: 17,
            max_bytes: 16,
        });

        assert!(matches!(
            error,
            AppError::WritePayloadTooLarge {
                size: 17,
                max_size: 16
            }
        ));
    }

    #[tokio::test]
    async fn read_file_rejects_existing_file_outside_safe_root() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let other_file = other.path().join("outside.txt");
        tokio::fs::write(&other_file, "outside").await.unwrap();

        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let result = tools
            .read_file(other_file.to_string_lossy().to_string())
            .await;

        assert!(matches!(result, Err(AppError::PathTraversal { .. })));
    }

    #[tokio::test]
    async fn write_file_rejects_outside_root_even_with_explicit_mutation() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let target = other.path().join("outside.txt");

        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let result = tools
            .write_file(
                target.to_string_lossy().to_string(),
                "should not write".to_string(),
                Some(false),
            )
            .await;

        assert!(matches!(result, Err(AppError::PathTraversal { .. })));
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn write_file_rejects_missing_parent_even_with_explicit_mutation() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("missing-parent").join("file.txt");

        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let result = tools
            .write_file(
                target.to_string_lossy().to_string(),
                "should not write".to_string(),
                Some(false),
            )
            .await;

        assert!(matches!(result, Err(AppError::PathTraversal { .. })));
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn write_file_rejects_oversized_dry_run_payload_before_path_resolution() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("outside.txt");
        let oversized_content = "x".repeat(DEFAULT_MAX_WRITE_BYTES + 1);

        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let result = tools
            .write_file(
                target.to_string_lossy().to_string(),
                oversized_content,
                None,
            )
            .await;

        assert!(matches!(
            result,
            Err(AppError::WritePayloadTooLarge {
                size,
                max_size
            }) if size == (DEFAULT_MAX_WRITE_BYTES + 1) as u64
                && max_size == DEFAULT_MAX_WRITE_BYTES as u64
        ));
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn write_file_rejects_oversized_explicit_mutation_without_creating_file() {
        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let target = root.path().join("oversized.txt");
        let oversized_content = "x".repeat(DEFAULT_MAX_WRITE_BYTES + 1);

        let result = tools
            .write_file(
                target.to_string_lossy().to_string(),
                oversized_content,
                Some(false),
            )
            .await;

        assert!(matches!(
            result,
            Err(AppError::WritePayloadTooLarge {
                size,
                max_size
            }) if size == (DEFAULT_MAX_WRITE_BYTES + 1) as u64
                && max_size == DEFAULT_MAX_WRITE_BYTES as u64
        ));
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn write_file_defaults_to_dry_run_without_creating_file() {
        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let target = root.path().join("default_dry_run.txt");

        let result = tools
            .write_file(
                target.to_string_lossy().to_string(),
                "should not be written".to_string(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(result, "DRY-RUN");
        assert!(!target.exists());
    }

    #[test]
    fn armed_temp_file_cleanup_removes_file_on_drop() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("armed.tmp");
        std::fs::write(&path, "temporary").unwrap();
        let root_fd = open_root_directory(root.path()).unwrap();

        {
            let _cleanup = DescriptorTempFileCleanup::new(&root_fd, OsString::from("armed.tmp"));
        }

        assert!(!path.exists());
    }

    #[test]
    fn disarmed_temp_file_cleanup_preserves_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("disarmed.tmp");
        std::fs::write(&path, "temporary").unwrap();
        let root_fd = open_root_directory(root.path()).unwrap();

        {
            let mut cleanup =
                DescriptorTempFileCleanup::new(&root_fd, OsString::from("disarmed.tmp"));
            cleanup.disarm();
        }

        assert!(path.exists());
    }

    #[test]
    fn directory_cleanup_removes_only_the_created_identity() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("created.tmp");
        let parked = root.path().join("created.parked");
        std::fs::create_dir(&original).unwrap();
        let root_fd = open_root_directory(root.path()).unwrap();
        let original_metadata =
            descriptor_fs::statat(&root_fd, "created.tmp", AtFlags::SYMLINK_NOFOLLOW).unwrap();

        {
            let mut cleanup =
                DescriptorDirectoryCleanup::new(&root_fd, OsString::from("created.tmp"));
            cleanup.set_expected_identity(original_metadata.st_dev, original_metadata.st_ino);
            std::fs::rename(&original, &parked).unwrap();
            std::fs::create_dir(&original).unwrap();
        }

        assert!(original.is_dir());
        assert!(parked.is_dir());
    }

    #[test]
    fn directory_cleanup_without_captured_identity_preserves_unknown_directory() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("unknown.tmp");
        std::fs::create_dir(&path).unwrap();
        let root_fd = open_root_directory(root.path()).unwrap();

        {
            let _cleanup = DescriptorDirectoryCleanup::new(&root_fd, OsString::from("unknown.tmp"));
        }

        assert!(path.is_dir());
    }

    #[test]
    fn directory_cleanup_removes_matching_empty_directory() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("created.tmp");
        std::fs::create_dir(&path).unwrap();
        let root_fd = open_root_directory(root.path()).unwrap();
        let metadata =
            descriptor_fs::statat(&root_fd, "created.tmp", AtFlags::SYMLINK_NOFOLLOW).unwrap();

        {
            let mut cleanup =
                DescriptorDirectoryCleanup::new(&root_fd, OsString::from("created.tmp"));
            cleanup.set_expected_identity(metadata.st_dev, metadata.st_ino);
        }

        assert!(!path.exists());
    }

    #[tokio::test]
    async fn write_file_requires_explicit_false_to_mutate_safe_rooted_file() {
        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let target = root.path().join("explicit_write.txt");

        let result = tools
            .write_file(
                target.to_string_lossy().to_string(),
                "written only when explicitly requested".to_string(),
                Some(false),
            )
            .await
            .unwrap();

        assert_eq!(result, "Wrote 38 bytes");
        assert_eq!(
            tokio::fs::read_to_string(&target).await.unwrap(),
            "written only when explicitly requested"
        );
        assert_eq!(
            std::fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[tokio::test]
    async fn create_directory_defaults_to_validated_dry_run() {
        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let target = root.path().join("preview-directory");

        let result = tools
            .create_directory(target.to_string_lossy().to_string(), None)
            .await
            .unwrap();

        assert_eq!(result.path, target.to_string_lossy());
        assert!(result.dry_run);
        assert_eq!(result.mode, "0700");
        assert_eq!(
            result.max_response_bytes,
            MAX_CREATE_DIRECTORY_RESPONSE_BYTES
        );
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn create_directory_requires_explicit_false_and_publishes_mode_0700() {
        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let target = root.path().join("created-directory");

        let result = tools
            .create_directory(target.to_string_lossy().to_string(), Some(false))
            .await
            .unwrap();

        assert!(!result.dry_run);
        assert_eq!(result.mode, "0700");
        let metadata = std::fs::symlink_metadata(&target).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.permissions().mode() & 0o777, CREATE_DIRECTORY_MODE);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn create_directory_rejects_existing_and_missing_parent_targets() {
        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let existing_file = root.path().join("existing-file");
        let existing_directory = root.path().join("existing-directory");
        std::fs::write(&existing_file, "unchanged").unwrap();
        std::fs::create_dir(&existing_directory).unwrap();

        for target in [&existing_file, &existing_directory] {
            let result = tools
                .create_directory(target.to_string_lossy().to_string(), Some(false))
                .await;
            assert!(matches!(result, Err(AppError::PathAlreadyExists)));
        }

        let missing_parent = root.path().join("missing-parent").join("child");
        let result = tools
            .create_directory(missing_parent.to_string_lossy().to_string(), Some(false))
            .await;
        assert!(matches!(result, Err(AppError::PathNotFound)));
        assert!(!missing_parent.exists());
        assert_eq!(std::fs::read_to_string(existing_file).unwrap(), "unchanged");
    }

    #[tokio::test]
    async fn create_directory_rejects_root_outside_and_symlink_boundaries() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let linked_parent = root.path().join("linked-parent");
        symlink(outside.path(), &linked_parent).unwrap();
        let linked_target = root.path().join("linked-target");
        symlink(outside.path().join("redirected"), &linked_target).unwrap();

        for target in [
            root.path().to_path_buf(),
            outside.path().join("outside-created"),
            linked_parent.join("child"),
            linked_target,
        ] {
            let result = tools
                .create_directory(target.to_string_lossy().to_string(), Some(false))
                .await;
            assert!(matches!(result, Err(AppError::PathTraversal { .. })));
        }

        assert!(!outside.path().join("outside-created").exists());
        assert!(!outside.path().join("child").exists());
        assert!(!outside.path().join("redirected").exists());
    }

    #[tokio::test]
    async fn create_directory_grant_stays_consumed_after_post_authorization_failure() {
        const TEST_KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let target = root.path().join("post-authorization-failure");
        let session_id = uuid::Uuid::new_v4().to_string();
        let authority = crate::create_directory_grant::CreateDirectoryGrantAuthority::from_hex_key(
            "test-key-1",
            TEST_KEY,
            "test-static-principal",
        )
        .unwrap();
        let prepared = tools
            .prepare_create_directory_mutation(target.to_string_lossy().to_string())
            .await
            .unwrap();
        let token = authority
            .issue_at(
                &session_id,
                &prepared.grant_target,
                unix_timestamp_seconds(),
            )
            .unwrap();

        let first_attempt = prepared.execute_authorized(|grant_target| {
            authority.consume_at(
                Some(&token),
                &session_id,
                grant_target,
                unix_timestamp_seconds(),
            )?;
            std::fs::create_dir(&target).unwrap();
            Ok(())
        });
        assert!(matches!(
            first_attempt,
            Err(AuthorizedCreateDirectoryError::Filesystem(
                AppError::PathAlreadyExists
            ))
        ));

        std::fs::remove_dir(&target).unwrap();
        let replay = tools
            .prepare_create_directory_mutation(target.to_string_lossy().to_string())
            .await
            .unwrap()
            .execute_authorized(|grant_target| {
                authority.consume_at(
                    Some(&token),
                    &session_id,
                    grant_target,
                    unix_timestamp_seconds(),
                )
            });
        assert!(matches!(
            replay,
            Err(AuthorizedCreateDirectoryError::Authorization(
                CreateDirectoryGrantError::Replayed
            ))
        ));
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn copy_file_defaults_to_dry_run_and_explicit_copy_is_binary_exact_and_private() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.bin");
        let preview = root.path().join("preview.bin");
        let destination = root.path().join("destination.bin");
        let empty_source = root.path().join("empty-source.bin");
        let empty_destination = root.path().join("empty-destination.bin");
        let bytes = [0, 1, 2, 0xff, b'\n', 0, 0x80];
        std::fs::write(&source, bytes).unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o777)).unwrap();
        std::fs::write(&empty_source, []).unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        let dry_run = tools
            .copy_file(
                source.to_string_lossy().to_string(),
                preview.to_string_lossy().to_string(),
                None,
            )
            .await
            .unwrap();
        assert!(dry_run.dry_run);
        assert_eq!(dry_run.size_bytes, bytes.len());
        assert_eq!(dry_run.mode, "0600");
        assert_eq!(dry_run.max_file_bytes, MAX_COPY_FILE_BYTES);
        assert_eq!(dry_run.max_response_bytes, MAX_COPY_FILE_RESPONSE_BYTES);
        assert!(!preview.exists());

        let copied = tools
            .copy_file(
                source.to_string_lossy().to_string(),
                destination.to_string_lossy().to_string(),
                Some(false),
            )
            .await
            .unwrap();
        assert!(!copied.dry_run);
        assert_eq!(copied.source_path, source.to_string_lossy());
        assert_eq!(copied.destination_path, destination.to_string_lossy());
        assert_eq!(std::fs::read(&destination).unwrap(), bytes);
        assert_eq!(
            std::fs::symlink_metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            COPY_FILE_MODE
        );

        let empty = tools
            .copy_file(
                empty_source.to_string_lossy().to_string(),
                empty_destination.to_string_lossy().to_string(),
                Some(false),
            )
            .await
            .unwrap();
        assert_eq!(empty.size_bytes, 0);
        assert_eq!(std::fs::read(&empty_destination).unwrap(), Vec::<u8>::new());
    }

    #[tokio::test]
    async fn copy_file_accepts_exact_limit_and_rejects_one_byte_over() {
        let root = tempfile::tempdir().unwrap();
        let exact = root.path().join("exact.bin");
        let oversized = root.path().join("oversized.bin");
        let exact_destination = root.path().join("exact-copy.bin");
        let oversized_destination = root.path().join("oversized-copy.bin");
        std::fs::write(&exact, vec![0x5a; MAX_COPY_FILE_BYTES]).unwrap();
        std::fs::write(&oversized, vec![0x5b; MAX_COPY_FILE_BYTES + 1]).unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        let result = tools
            .copy_file(
                exact.to_string_lossy().to_string(),
                exact_destination.to_string_lossy().to_string(),
                Some(false),
            )
            .await
            .unwrap();
        assert_eq!(result.size_bytes, MAX_COPY_FILE_BYTES);
        assert_eq!(
            std::fs::metadata(exact_destination).unwrap().len(),
            1_048_576
        );

        let result = tools
            .copy_file(
                oversized.to_string_lossy().to_string(),
                oversized_destination.to_string_lossy().to_string(),
                Some(false),
            )
            .await;
        assert!(matches!(
            result,
            Err(AppError::FileTooLarge { size, max_size })
                if size == (MAX_COPY_FILE_BYTES + 1) as u64
                    && max_size == MAX_COPY_FILE_BYTES as u64
        ));
        assert!(!oversized_destination.exists());
    }

    #[tokio::test]
    async fn copy_file_rejects_missing_same_existing_and_unsupported_objects() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.txt");
        let existing = root.path().join("existing.txt");
        let directory_source = root.path().join("directory-source");
        std::fs::write(&source, "source").unwrap();
        std::fs::write(&existing, "unchanged").unwrap();
        std::fs::create_dir(&directory_source).unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        let missing_source = tools
            .copy_file(
                root.path().join("missing").to_string_lossy().to_string(),
                root.path().join("unused").to_string_lossy().to_string(),
                Some(false),
            )
            .await;
        assert!(matches!(missing_source, Err(AppError::CopySourceNotFound)));

        let missing_parent = tools
            .copy_file(
                source.to_string_lossy().to_string(),
                root.path()
                    .join("missing-parent")
                    .join("copy")
                    .to_string_lossy()
                    .to_string(),
                Some(false),
            )
            .await;
        assert!(matches!(
            missing_parent,
            Err(AppError::CopyDestinationParentNotFound)
        ));

        let same = tools
            .copy_file(
                source.to_string_lossy().to_string(),
                source.to_string_lossy().to_string(),
                Some(false),
            )
            .await;
        assert!(matches!(same, Err(AppError::CopySourceDestinationSame)));

        let existing_result = tools
            .copy_file(
                source.to_string_lossy().to_string(),
                existing.to_string_lossy().to_string(),
                Some(false),
            )
            .await;
        assert!(matches!(existing_result, Err(AppError::PathAlreadyExists)));
        assert_eq!(std::fs::read_to_string(existing).unwrap(), "unchanged");

        let directory_result = tools
            .copy_file(
                directory_source.to_string_lossy().to_string(),
                root.path()
                    .join("directory-copy")
                    .to_string_lossy()
                    .to_string(),
                Some(false),
            )
            .await;
        assert!(matches!(
            directory_result,
            Err(AppError::UnsupportedPathType)
        ));
    }

    #[tokio::test]
    async fn copy_file_rejects_outside_and_symlink_boundaries_and_allows_cross_root_copy() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let source = first.path().join("source.txt");
        let cross_root_destination = second.path().join("copied.txt");
        let source_link = first.path().join("source-link");
        let destination_link = first.path().join("destination-link");
        let linked_parent = first.path().join("linked-parent");
        std::fs::write(&source, "cross-root").unwrap();
        symlink(&source, &source_link).unwrap();
        symlink(outside.path().join("destination"), &destination_link).unwrap();
        symlink(outside.path(), &linked_parent).unwrap();
        let tools = FileSystemTools::new(vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
        ]);

        for (copy_source, copy_destination) in [
            (
                outside.path().join("outside-source"),
                first.path().join("copy-a"),
            ),
            (source.clone(), outside.path().join("outside-destination")),
            (source_link, first.path().join("copy-b")),
            (source.clone(), destination_link),
            (source.clone(), linked_parent.join("copy-c")),
        ] {
            let result = tools
                .copy_file(
                    copy_source.to_string_lossy().to_string(),
                    copy_destination.to_string_lossy().to_string(),
                    Some(false),
                )
                .await;
            assert!(matches!(result, Err(AppError::PathTraversal { .. })));
        }

        let result = tools
            .copy_file(
                source.to_string_lossy().to_string(),
                cross_root_destination.to_string_lossy().to_string(),
                Some(false),
            )
            .await
            .unwrap();
        assert!(!result.dry_run);
        assert_eq!(
            std::fs::read_to_string(cross_root_destination).unwrap(),
            "cross-root"
        );
        assert!(!outside.path().join("outside-destination").exists());
        assert!(!outside.path().join("destination").exists());
        assert!(!outside.path().join("copy-c").exists());
    }

    #[test]
    fn copied_file_cleanup_removes_only_the_captured_regular_file_identity() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("copy.tmp");
        let parked = root.path().join("copy.parked");
        std::fs::write(&original, "original").unwrap();
        let root_fd = open_root_directory(root.path()).unwrap();
        let metadata =
            descriptor_fs::statat(&root_fd, "copy.tmp", AtFlags::SYMLINK_NOFOLLOW).unwrap();

        {
            let mut cleanup =
                DescriptorCopiedFileCleanup::new(&root_fd, OsString::from("copy.tmp"));
            cleanup.set_expected_identity(metadata.st_dev, metadata.st_ino);
            std::fs::rename(&original, &parked).unwrap();
            std::fs::write(&original, "replacement").unwrap();
        }

        assert_eq!(std::fs::read_to_string(original).unwrap(), "replacement");
        assert_eq!(std::fs::read_to_string(parked).unwrap(), "original");
    }

    #[test]
    fn copied_file_cleanup_removes_matching_file_and_preserves_unknown_identity() {
        let root = tempfile::tempdir().unwrap();
        let matching = root.path().join("matching.tmp");
        let unknown = root.path().join("unknown.tmp");
        std::fs::write(&matching, "matching").unwrap();
        std::fs::write(&unknown, "unknown").unwrap();
        let root_fd = open_root_directory(root.path()).unwrap();
        let metadata =
            descriptor_fs::statat(&root_fd, "matching.tmp", AtFlags::SYMLINK_NOFOLLOW).unwrap();

        {
            let mut cleanup =
                DescriptorCopiedFileCleanup::new(&root_fd, OsString::from("matching.tmp"));
            cleanup.set_expected_identity(metadata.st_dev, metadata.st_ino);
        }
        {
            let _cleanup =
                DescriptorCopiedFileCleanup::new(&root_fd, OsString::from("unknown.tmp"));
        }

        assert!(!matching.exists());
        assert!(unknown.exists());
    }

    #[test]
    fn held_source_descriptor_prevents_copy_redirection_after_path_exchange() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let parked = root.path().join("source-parked");
        let outside_source = outside.path().join("outside-source");
        std::fs::write(&source, "inside-source").unwrap();
        std::fs::write(&outside_source, "outside-source").unwrap();
        let root_fd = open_root_directory(root.path()).unwrap();
        let source_fd = descriptor_fs::openat(
            &root_fd,
            "source",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        std::fs::rename(&source, &parked).unwrap();
        symlink(&outside_source, &source).unwrap();

        let mut content = String::new();
        File::from(source_fd).read_to_string(&mut content).unwrap();
        assert_eq!(content, "inside-source");
        assert_eq!(
            std::fs::read_to_string(outside_source).unwrap(),
            "outside-source"
        );
    }

    #[test]
    fn held_hash_descriptor_prevents_redirection_after_path_exchange() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let parked = root.path().join("source-parked");
        let outside_source = outside.path().join("outside-source");
        std::fs::write(&source, b"inside-hash").unwrap();
        std::fs::write(&outside_source, b"outside-secret").unwrap();
        let root_fd = open_root_directory(root.path()).unwrap();
        let source_fd = descriptor_fs::openat(
            &root_fd,
            "source",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        std::fs::rename(&source, &parked).unwrap();
        symlink(&outside_source, &source).unwrap();

        let mut bytes = Vec::new();
        File::from(source_fd).read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"inside-hash");
        assert_eq!(Sha256::digest(&bytes), Sha256::digest(b"inside-hash"));
        assert_eq!(
            std::fs::read_to_string(outside_source).unwrap(),
            "outside-secret"
        );
    }

    #[test]
    fn held_binary_read_descriptor_prevents_redirection_after_path_exchange() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let parked = root.path().join("source-parked");
        let outside_source = outside.path().join("outside-source");
        std::fs::write(&source, b"inside-binary").unwrap();
        std::fs::write(&outside_source, b"outside-secret").unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let anchored = tools.anchor(source.to_string_lossy().as_ref()).unwrap();
        let file = open_verified_regular_file(&anchored, MAX_BINARY_READ_BYTES).unwrap();

        std::fs::rename(&source, &parked).unwrap();
        symlink(&outside_source, &source).unwrap();

        let bytes = read_bounded_bytes(file.file, MAX_BINARY_READ_BYTES).unwrap();
        assert_eq!(bytes, b"inside-binary");
        assert_eq!(std::fs::read(outside_source).unwrap(), b"outside-secret");
    }

    #[test]
    fn held_binary_range_descriptor_prevents_redirection_after_path_exchange() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let parked = root.path().join("source-parked");
        let outside_source = outside.path().join("outside-source");
        std::fs::write(&source, b"inside-binary").unwrap();
        std::fs::write(&outside_source, b"outside-secret").unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let anchored = tools.anchor(source.to_string_lossy().as_ref()).unwrap();
        let file = open_verified_regular_file(&anchored, MAX_BINARY_RANGE_FILE_BYTES).unwrap();

        std::fs::rename(&source, &parked).unwrap();
        symlink(&outside_source, &source).unwrap();

        let range = read_verified_binary_range(file, 7, 6).unwrap();
        assert_eq!(range.bytes, b"binary");
        assert!(range.eof);
        assert_eq!(std::fs::read(outside_source).unwrap(), b"outside-secret");
    }

    #[test]
    fn held_text_range_descriptor_prevents_redirection_after_path_exchange() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let parked = root.path().join("source-parked");
        let outside_source = outside.path().join("outside-source");
        std::fs::write(&source, b"inside-text").unwrap();
        std::fs::write(&outside_source, b"outside-secret").unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let anchored = tools.anchor(source.to_string_lossy().as_ref()).unwrap();
        let file = open_verified_regular_file(&anchored, MAX_TEXT_RANGE_FILE_BYTES).unwrap();

        std::fs::rename(&source, &parked).unwrap();
        symlink(&outside_source, &source).unwrap();

        let range = read_verified_text_range(file, 7, MIN_TEXT_RANGE_BYTES).unwrap();
        assert_eq!(range.content, "text");
        assert!(range.eof);
        assert_eq!(std::fs::read(outside_source).unwrap(), b"outside-secret");
    }

    #[test]
    fn held_destination_parent_descriptor_prevents_copy_redirection() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let parent = root.path().join("parent");
        let parked = root.path().join("parent-parked");
        std::fs::create_dir(&parent).unwrap();
        let root_fd = open_root_directory(root.path()).unwrap();
        let parent_fd = open_mutation_parent_directory(root_fd, Path::new("parent")).unwrap();
        std::fs::rename(&parent, &parked).unwrap();
        symlink(outside.path(), &parent).unwrap();

        let staged = descriptor_fs::openat(
            &parent_fd,
            "copy.tmp",
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .unwrap();
        File::from(staged).write_all(b"inside-copy").unwrap();
        descriptor_fs::renameat_with(
            &parent_fd,
            "copy.tmp",
            &parent_fd,
            "copy",
            RenameFlags::NOREPLACE,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(parked.join("copy")).unwrap(),
            "inside-copy"
        );
        assert!(!outside.path().join("copy").exists());
    }

    #[test]
    fn copy_no_replace_publication_rejects_concurrent_final_destination() {
        let root = tempfile::tempdir().unwrap();
        let root_fd = open_root_directory(root.path()).unwrap();
        std::fs::write(root.path().join("prepared.tmp"), "prepared").unwrap();
        std::fs::write(root.path().join("destination"), "concurrent").unwrap();

        let result = descriptor_fs::renameat_with(
            &root_fd,
            "prepared.tmp",
            &root_fd,
            "destination",
            RenameFlags::NOREPLACE,
        );

        assert_eq!(result, Err(rustix::io::Errno::EXIST));
        assert_eq!(
            std::fs::read_to_string(root.path().join("prepared.tmp")).unwrap(),
            "prepared"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("destination")).unwrap(),
            "concurrent"
        );
    }

    #[test]
    fn held_parent_descriptor_prevents_directory_creation_redirection() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let parent = root.path().join("parent");
        let parked = root.path().join("parent-parked");
        std::fs::create_dir(&parent).unwrap();

        let root_fd = open_root_directory(root.path()).unwrap();
        let parent_fd = open_mutation_parent_directory(root_fd, Path::new("parent")).unwrap();
        std::fs::rename(&parent, &parked).unwrap();
        symlink(outside.path(), &parent).unwrap();

        descriptor_fs::mkdirat(&parent_fd, "created", Mode::RUSR | Mode::WUSR | Mode::XUSR)
            .unwrap();
        descriptor_fs::fsync(&parent_fd).unwrap();

        assert!(parked.join("created").is_dir());
        assert!(!outside.path().join("created").exists());
    }

    #[test]
    fn no_replace_publication_rejects_concurrent_final_target() {
        let root = tempfile::tempdir().unwrap();
        let root_fd = open_root_directory(root.path()).unwrap();
        descriptor_fs::mkdirat(
            &root_fd,
            "prepared.tmp",
            Mode::RUSR | Mode::WUSR | Mode::XUSR,
        )
        .unwrap();
        std::fs::create_dir(root.path().join("target")).unwrap();

        let result = descriptor_fs::renameat_with(
            &root_fd,
            "prepared.tmp",
            &root_fd,
            "target",
            RenameFlags::NOREPLACE,
        );

        assert_eq!(result, Err(rustix::io::Errno::EXIST));
        assert!(root.path().join("prepared.tmp").is_dir());
        assert!(root.path().join("target").is_dir());
    }

    #[tokio::test]
    async fn path_metadata_reports_only_bounded_file_and_directory_fields() {
        let root = tempfile::tempdir().unwrap();
        let file_path = root.path().join("visible.txt");
        std::fs::write(&file_path, "five!").unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        let file = tools
            .path_metadata(file_path.to_string_lossy().to_string())
            .await
            .unwrap();
        assert_eq!(file.path, file_path.to_string_lossy());
        assert_eq!(file.kind, PathMetadataKind::RegularFile);
        assert_eq!(file.size_bytes, Some(5));
        assert_eq!(file.max_response_bytes, MAX_PATH_METADATA_RESPONSE_BYTES);
        chrono::DateTime::parse_from_rfc3339(file.modified.as_deref().unwrap()).unwrap();

        let empty_path = root.path().join("empty.txt");
        std::fs::write(&empty_path, []).unwrap();
        let empty = tools
            .path_metadata(empty_path.to_string_lossy().to_string())
            .await
            .unwrap();
        assert_eq!(empty.kind, PathMetadataKind::RegularFile);
        assert_eq!(empty.size_bytes, Some(0));

        let directory = tools
            .path_metadata(root.path().to_string_lossy().to_string())
            .await
            .unwrap();
        assert_eq!(directory.path, root.path().to_string_lossy());
        assert_eq!(directory.kind, PathMetadataKind::Directory);
        assert_eq!(directory.size_bytes, None);

        let serialized = serde_json::to_value(file).unwrap();
        assert_eq!(
            serialized.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec!["kind", "maxResponseBytes", "modified", "path", "sizeBytes"]
        );
        for forbidden in ["inode", "device", "uid", "gid", "mode", "access", "five!"] {
            assert!(!serialized
                .to_string()
                .to_ascii_lowercase()
                .contains(forbidden));
        }
    }

    #[tokio::test]
    async fn path_metadata_rejects_outside_missing_symlink_and_socket_targets() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.txt");
        std::fs::write(&outside_file, "outside").unwrap();
        symlink(&outside_file, root.path().join("link.txt")).unwrap();
        let fifo_path = root.path().join("runtime.fifo");
        let root_fd = open_root_directory(root.path()).unwrap();
        descriptor_fs::mkfifoat(&root_fd, "runtime.fifo", Mode::RUSR | Mode::WUSR).unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        assert!(matches!(
            tools
                .path_metadata(outside_file.to_string_lossy().to_string())
                .await,
            Err(AppError::PathTraversal { .. })
        ));
        assert!(matches!(
            tools
                .path_metadata(root.path().join("missing").to_string_lossy().to_string())
                .await,
            Err(AppError::PathNotFound)
        ));
        assert!(matches!(
            tools
                .path_metadata(
                    root.path()
                        .join("missing-parent")
                        .join("child")
                        .to_string_lossy()
                        .to_string()
                )
                .await,
            Err(AppError::PathNotFound)
        ));
        assert!(matches!(
            tools
                .path_metadata(root.path().join("link.txt").to_string_lossy().to_string())
                .await,
            Err(AppError::PathTraversal { .. })
        ));
        assert!(matches!(
            tools
                .path_metadata(fifo_path.to_string_lossy().to_string())
                .await,
            Err(AppError::UnsupportedPathType)
        ));
    }

    #[test]
    fn held_metadata_descriptor_prevents_final_object_exchange_redirection() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let child = root.path().join("child");
        let parked = root.path().join("parked");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(child.join("target.txt"), "safe").unwrap();
        std::fs::write(outside.path().join("target.txt"), "outside-secret").unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let requested = child.join("target.txt");
        let anchored = tools.anchor(requested.to_string_lossy().as_ref()).unwrap();
        let root_fd = open_root_directory(&anchored.root_path).unwrap();
        let target_fd = open_metadata_descriptor(root_fd, &anchored.relative_path).unwrap();

        std::fs::rename(&child, &parked).unwrap();
        symlink(outside.path(), &child).unwrap();

        let metadata = descriptor_fs::fstat(&target_fd).unwrap();
        assert_eq!(metadata.st_size, 4);
        assert_eq!(
            std::fs::read_to_string(parked.join("target.txt")).unwrap(),
            "safe"
        );
    }

    #[test]
    fn descriptor_walk_rejects_parent_swapped_to_symlink_after_validation() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let child = root.path().join("child");
        let parked = root.path().join("parked");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(outside.path().join("secret.txt"), "outside-secret").unwrap();
        let requested = child.join("secret.txt");
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        let anchored = tools.anchor(requested.to_string_lossy().as_ref()).unwrap();
        std::fs::rename(&child, &parked).unwrap();
        symlink(outside.path(), &child).unwrap();

        let (parent_relative, _) = split_parent_and_name(&anchored.relative_path).unwrap();
        let root_fd = open_root_directory(&anchored.root_path).unwrap();
        let result = open_descendant_directory(root_fd, &parent_relative);

        assert!(matches!(result, Err(AppError::PathTraversal { .. })));
    }

    #[test]
    fn held_parent_descriptor_prevents_write_redirection_after_swap() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let child = root.path().join("child");
        let parked = root.path().join("parked");
        std::fs::create_dir(&child).unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
        let requested = child.join("result.txt");
        let anchored = tools.anchor(requested.to_string_lossy().as_ref()).unwrap();
        let (parent_relative, file_name) = split_parent_and_name(&anchored.relative_path).unwrap();
        let root_fd = open_root_directory(&anchored.root_path).unwrap();
        let parent_fd = open_descendant_directory(root_fd, &parent_relative).unwrap();

        std::fs::rename(&child, &parked).unwrap();
        symlink(outside.path(), &child).unwrap();

        let temp_name = OsString::from(".result.test.tmp");
        let temp_fd = descriptor_fs::openat(
            &parent_fd,
            &temp_name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .unwrap();
        let mut cleanup = DescriptorTempFileCleanup::new(&parent_fd, temp_name.clone());
        let mut file = File::from(temp_fd);
        file.write_all(b"safe-write").unwrap();
        file.sync_all().unwrap();
        drop(file);
        descriptor_fs::renameat(&parent_fd, &temp_name, &parent_fd, &file_name).unwrap();
        cleanup.disarm();
        descriptor_fs::fsync(&parent_fd).unwrap();

        assert!(!outside.path().join("result.txt").exists());
        assert_eq!(
            std::fs::read_to_string(parked.join("result.txt")).unwrap(),
            "safe-write"
        );
    }

    #[test]
    fn search_query_contract_accepts_exact_limit_and_rejects_unsafe_shapes() {
        assert!(validate_search_query(&"q".repeat(MAX_SEARCH_QUERY_BYTES)).is_ok());
        for query in [
            String::new(),
            "q".repeat(MAX_SEARCH_QUERY_BYTES + 1),
            "two\nlines".to_string(),
            "carriage\rreturn".to_string(),
            "nul\0byte".to_string(),
        ] {
            assert!(matches!(
                validate_search_query(&query),
                Err(AppError::InvalidSearchQuery)
            ));
        }
    }

    #[tokio::test]
    async fn search_text_returns_deterministic_locations_without_echoing_content() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(
            root.path().join("z.txt"),
            "needle first\nnone\nneedle and needle",
        )
        .unwrap();
        std::fs::write(root.path().join("a.txt"), "prefix needle").unwrap();
        std::fs::write(nested.join("b.txt"), "needle nested").unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        let shallow = tools
            .search_text(
                root.path().to_string_lossy().to_string(),
                "needle".to_string(),
                Some(1),
            )
            .await
            .unwrap();
        assert_eq!(shallow.matches.len(), 4);
        assert!(shallow
            .matches
            .iter()
            .all(|matched| !matched.path.ends_with("nested/b.txt")));

        let result = tools
            .search_text(
                root.path().to_string_lossy().to_string(),
                "needle".to_string(),
                Some(2),
            )
            .await
            .unwrap();
        let locations = result
            .matches
            .iter()
            .map(|matched| {
                (
                    matched
                        .path
                        .strip_prefix(root.path().to_string_lossy().as_ref())
                        .unwrap()
                        .to_string(),
                    matched.line_number,
                    matched.column_byte,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            locations,
            vec![
                ("/a.txt".to_string(), 1, 8),
                ("/nested/b.txt".to_string(), 1, 1),
                ("/z.txt".to_string(), 1, 1),
                ("/z.txt".to_string(), 3, 1),
                ("/z.txt".to_string(), 3, 12),
            ]
        );
        assert!(!result.truncated);
        assert_eq!(result.query_bytes, 6);
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(!serialized.contains("prefix needle"));
        assert!(!serialized.contains("needle nested"));
        assert!(!serialized.contains("needle and needle"));
    }

    #[tokio::test]
    async fn search_text_rejects_outside_root_and_skips_symlink_escape() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "needle outside").unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        assert!(matches!(
            tools
                .search_text(
                    outside.path().to_string_lossy().to_string(),
                    "needle".to_string(),
                    None,
                )
                .await,
            Err(AppError::PathTraversal { .. })
        ));
        let result = tools
            .search_text(
                root.path().to_string_lossy().to_string(),
                "needle".to_string(),
                None,
            )
            .await
            .unwrap();
        assert!(result.matches.is_empty());
        assert_eq!(result.skipped_unsafe_entries, 1);
    }

    #[tokio::test]
    async fn search_text_skips_oversized_and_invalid_utf8_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("oversized.txt"),
            vec![b'x'; MAX_SEARCH_FILE_BYTES + 1],
        )
        .unwrap();
        std::fs::write(root.path().join("binary.dat"), [0xff, 0xfe, 0xfd]).unwrap();
        std::fs::write(root.path().join("valid.txt"), "needle").unwrap();
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        let result = tools
            .search_text(
                root.path().to_string_lossy().to_string(),
                "needle".to_string(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.files_scanned, 2);
        assert_eq!(result.skipped_oversized_files, 1);
        assert_eq!(result.skipped_invalid_utf8_files, 1);
        assert!(result.truncated);
    }

    #[tokio::test]
    async fn search_text_enforces_match_and_aggregate_byte_budgets() {
        let match_root = tempfile::tempdir().unwrap();
        std::fs::write(
            match_root.path().join("matches.txt"),
            "x ".repeat(MAX_SEARCH_MATCHES + 1),
        )
        .unwrap();
        let match_tools = FileSystemTools::new(vec![match_root.path().to_path_buf()]);
        let matches = match_tools
            .search_text(
                match_root.path().to_string_lossy().to_string(),
                "x".to_string(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(matches.matches.len(), MAX_SEARCH_MATCHES);
        assert!(matches.truncated);

        let byte_root = tempfile::tempdir().unwrap();
        for index in 0..9 {
            std::fs::write(
                byte_root.path().join(format!("{index}.txt")),
                vec![b'a'; MAX_SEARCH_FILE_BYTES],
            )
            .unwrap();
        }
        let byte_tools = FileSystemTools::new(vec![byte_root.path().to_path_buf()]);
        let bytes = byte_tools
            .search_text(
                byte_root.path().to_string_lossy().to_string(),
                "needle".to_string(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(bytes.bytes_scanned, MAX_SEARCH_TOTAL_BYTES);
        assert_eq!(bytes.files_scanned, 8);
        assert!(bytes.truncated);
    }

    #[tokio::test]
    async fn search_text_truncates_long_path_matches_to_response_budget() {
        let root = tempfile::tempdir().unwrap();
        let component = "d".repeat(250);
        let mut directory = root.path().to_path_buf();
        for suffix in ['a', 'b', 'c', 'd'] {
            directory.push(format!("{component}{suffix}"));
            std::fs::create_dir(&directory).unwrap();
        }
        for index in 0..MAX_SEARCH_MATCHES {
            std::fs::write(directory.join(format!("{index:03}.txt")), "needle").unwrap();
        }
        let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);

        let result = tools
            .search_text(
                root.path().to_string_lossy().to_string(),
                "needle".to_string(),
                Some(MAX_SEARCH_DEPTH),
            )
            .await
            .unwrap();
        assert!(result.truncated);
        assert!(result.matches.len() < MAX_SEARCH_MATCHES);
        assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_SEARCH_STRUCTURED_BYTES);
    }

    #[test]
    fn held_directory_descriptor_prevents_search_redirection_after_swap() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let child = root.path().join("child");
        let parked = root.path().join("parked");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(child.join("data.txt"), "needle safe").unwrap();
        std::fs::write(outside.path().join("data.txt"), "needle outside").unwrap();
        let child_fd = open_root_directory(&child).unwrap();

        std::fs::rename(&child, &parked).unwrap();
        symlink(outside.path(), &child).unwrap();

        let pending = SearchPendingEntry {
            name: OsString::from("data.txt"),
            display_path: child.join("data.txt"),
            file_type: FileType::RegularFile,
            size: 11,
        };
        let mut state = SearchState::new("needle");
        scan_search_file(&child_fd, &pending, &mut state).unwrap();

        assert_eq!(state.matches.len(), 1);
        assert_eq!(state.bytes_scanned, 11);
        assert!(!state.matches[0].path.contains("outside"));
    }
}
