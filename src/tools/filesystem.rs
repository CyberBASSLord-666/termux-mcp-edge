//! Filesystem tools with safe-root enforcement, bounded traversal, and metrics.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
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

// Leave deterministic room for the JSON-RPC envelope, bounded summary, and a
// normally sized request id. The transport independently enforces the exact
// full-response ceilings above, including caller-controlled ids.
const MAX_LIST_STRUCTURED_BYTES: usize = MAX_LIST_RESPONSE_BYTES - 1_024;

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

#[cfg(test)]
mod tests {
    use super::*;
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
}
