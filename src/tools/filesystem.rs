//! Filesystem tools with safe-root enforcement, bounded traversal, and metrics.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use metrics::{counter, histogram};
use rustix::fs::{self as descriptor_fs, AtFlags, Dir, FileType, Mode, OFlags};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::audit::AuditEvent;
use crate::error::AppError;
use crate::write_policy::{WritePolicy, WritePolicyError};

const DEFAULT_LIST_DEPTH: u32 = 1;
const MAX_LIST_DEPTH: u32 = 5;
pub const MAX_LIST_ENTRIES: usize = 4_096;
pub const MAX_LIST_RESPONSE_BYTES: usize = 262_144;
pub const MAX_READ_BYTES: usize = 1_048_576;
pub const MAX_READ_RESPONSE_BYTES: usize = 1_114_112;
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
const MAX_SEARCH_STRUCTURED_BYTES: usize = MAX_SEARCH_RESPONSE_BYTES - 1_024;

struct DescriptorTempFileCleanup<'a> {
    parent: &'a OwnedFd,
    name: OsString,
    armed: bool,
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
            display_path: root_path.join(relative_path),
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
            let (parent_relative, file_name) = split_parent_and_name(&anchored.relative_path)?;
            let root_fd = open_root_directory(&anchored.root_path)?;
            let parent_fd = open_descendant_directory(root_fd, &parent_relative)?;
            let metadata = descriptor_fs::statat(&parent_fd, &file_name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(descriptor_error)?;
            if FileType::from_raw_mode(metadata.st_mode).is_symlink() {
                return Err(path_rejected(
                    anchored.display_path.to_string_lossy().as_ref(),
                ));
            }
            let file_fd = descriptor_fs::openat(
                &parent_fd,
                &file_name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(descriptor_error)?;
            let file = File::from(file_fd);
            let mut limited_reader = file.take((MAX_READ_BYTES + 1) as u64);
            let mut bytes = Vec::with_capacity(MAX_READ_BYTES.min(64 * 1_024));
            limited_reader.read_to_end(&mut bytes)?;
            let bytes_read = bytes.len();

            if bytes_read > MAX_READ_BYTES {
                counter!("mcp.fs.read.rejected_too_large_total").increment(1);
                return Err(AppError::FileTooLarge {
                    size: bytes_read as u64,
                    max_size: MAX_READ_BYTES as u64,
                });
            }

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

fn split_parent_and_name(relative_path: &Path) -> Result<(PathBuf, OsString), AppError> {
    let Some(name) = relative_path.file_name() else {
        return Err(path_rejected(relative_path.to_string_lossy().as_ref()));
    };
    let parent = relative_path.parent().unwrap_or_else(|| Path::new(""));
    Ok((parent.to_path_buf(), name.to_os_string()))
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
