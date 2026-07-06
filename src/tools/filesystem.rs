//! Filesystem tools with safe-root enforcement, bounded traversal, and metrics.

use std::collections::VecDeque;
use std::path::{Component, Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use metrics::{counter, histogram};
use serde::{Deserialize, Serialize};
use tokio::{fs, io::AsyncReadExt};

use crate::audit::AuditEvent;
use crate::error::AppError;
use crate::write_policy::WritePolicy;

const DEFAULT_LIST_DEPTH: u32 = 1;
const MAX_LIST_DEPTH: u32 = 5;
const MAX_LIST_ENTRIES: usize = 4_096;
const MAX_READ_BYTES: u64 = 1_048_576;

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
    /// The method is intentionally public so integration and property tests can
    /// exercise the same guard used by the MCP tools. It rejects relative paths,
    /// empty paths, NUL bytes, and explicit `..` components before canonicalizing
    /// the path or its nearest existing parent. This lets `write_file` create a
    /// new file while still preventing symlink and parent-directory escapes.
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

    async fn collect_entries_iterative(
        &self,
        root_path: &Path,
        entries: &mut Vec<FileInfo>,
        max_depth: u32,
    ) -> Result<(), AppError> {
        let mut queue = VecDeque::new();
        queue.push_back((root_path.to_path_buf(), 1_u32));

        while let Some((dir_path, depth)) = queue.pop_front() {
            let mut read_dir = fs::read_dir(&dir_path).await?;

            while let Some(entry) = read_dir.next_entry().await? {
                if entries.len() >= MAX_LIST_ENTRIES {
                    counter!("mcp.fs.list.truncated_total").increment(1);
                    return Ok(());
                }

                let entry_path = entry.path();
                let Ok(file_type) = entry.file_type().await else {
                    counter!("mcp.fs.list.skipped_unreadable_entries_total").increment(1);
                    continue;
                };

                if file_type.is_symlink() && !entry_path.exists() {
                    counter!("mcp.fs.list.skipped_unreadable_entries_total").increment(1);
                    continue;
                }

                let Ok(metadata) = entry.metadata().await else {
                    counter!("mcp.fs.list.skipped_unreadable_entries_total").increment(1);
                    continue;
                };
                let is_dir = metadata.is_dir();
                let size = metadata.len();
                let modified = metadata.modified().ok().map(|mtime| {
                    let dt: chrono::DateTime<chrono::Utc> = mtime.into();
                    dt.to_rfc3339()
                });

                // Re-check every child through sanitize. If a symlink resolves
                // outside a safe root, skip it rather than failing the entire
                // listing. That keeps directory listing robust while preserving
                // the sandbox boundary.
                let Ok(safe_child) = self.sanitize(entry_path.to_string_lossy().as_ref()) else {
                    counter!("mcp.fs.list.skipped_unsafe_entries_total").increment(1);
                    continue;
                };

                entries.push(FileInfo {
                    path: safe_child.to_string_lossy().to_string(),
                    size,
                    is_dir,
                    modified,
                });

                if is_dir && depth < max_depth {
                    queue.push_back((safe_child, depth + 1));
                }
            }
        }

        Ok(())
    }

    pub async fn list_directory(
        &self,
        path: String,
        max_depth: Option<u32>,
    ) -> Result<ListDirResult, AppError> {
        let start = Instant::now();
        let safe_path = self.sanitize(&path)?;
        let depth = max_depth
            .unwrap_or(DEFAULT_LIST_DEPTH)
            .clamp(1, MAX_LIST_DEPTH);

        let mut entries = Vec::new();
        self.collect_entries_iterative(&safe_path, &mut entries, depth)
            .await?;

        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.fs.list.latency_seconds").record(duration);
        counter!("mcp.fs.list.calls_total").increment(1);

        Ok(ListDirResult {
            path: safe_path.to_string_lossy().to_string(),
            entries,
        })
    }

    pub async fn read_file(&self, path: String) -> Result<ReadFileResult, AppError> {
        let start = Instant::now();
        let safe_path = self.sanitize(&path)?;
        let file = fs::File::open(&safe_path).await?;
        let mut limited_reader = file.take(MAX_READ_BYTES + 1);
        let mut content = String::new();
        let bytes_read = limited_reader.read_to_string(&mut content).await?;

        if bytes_read as u64 > MAX_READ_BYTES {
            counter!("mcp.fs.read.rejected_too_large_total").increment(1);
            return Err(AppError::FileTooLarge {
                size: bytes_read as u64,
                max_size: MAX_READ_BYTES,
            });
        }

        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.fs.read.latency_seconds").record(duration);
        counter!("mcp.fs.read.bytes_total").increment(bytes_read as u64);

        Ok(ReadFileResult {
            path: safe_path.to_string_lossy().to_string(),
            content,
            size: bytes_read,
        })
    }

    pub async fn write_file(
        &self,
        path: String,
        content: String,
        dry_run: Option<bool>,
    ) -> Result<String, AppError> {
        let start = Instant::now();
        let _audit_event =
            self.audit_write_decision(unix_timestamp_seconds(), content.len(), dry_run);
        let safe_path = self.sanitize(&path)?;
        let parent = safe_path.parent().ok_or_else(|| AppError::PathTraversal {
            attempted: path.clone(),
        })?;

        if !self.safe_roots.iter().any(|root| parent.starts_with(root)) {
            return Err(AppError::PathTraversal {
                attempted: path.clone(),
            });
        }

        let file_name = safe_path
            .file_name()
            .ok_or_else(|| AppError::PathTraversal {
                attempted: path.clone(),
            })?
            .to_string_lossy();

        if dry_run.unwrap_or(true) {
            counter!("mcp.fs.write.dry_runs_total").increment(1);
            return Ok("DRY-RUN".to_string());
        }

        let tmp = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));

        if let Err(err) = fs::write(&tmp, content.as_bytes()).await {
            let _ = fs::remove_file(&tmp).await;
            return Err(err.into());
        }

        if let Err(err) = fs::rename(&tmp, &safe_path).await {
            let _ = fs::remove_file(&tmp).await;
            return Err(err.into());
        }

        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.fs.write.latency_seconds").record(duration);
        counter!("mcp.fs.write.bytes_total").increment(content.len() as u64);

        Ok(format!("Wrote {} bytes", content.len()))
    }
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
pub struct ListDirResult {
    pub path: String,
    pub entries: Vec<FileInfo>,
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

    use crate::audit::{AuditDecision, AuditMode};

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
            tokio::fs::read_to_string(target).await.unwrap(),
            "written only when explicitly requested"
        );
    }
}
