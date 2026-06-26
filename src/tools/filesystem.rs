//! Filesystem tools with safe-root enforcement, bounded traversal, and metrics.

use std::collections::VecDeque;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use metrics::{counter, histogram};
use rmcp::tool;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::error::AppError;

const DEFAULT_LIST_DEPTH: u32 = 1;
const MAX_LIST_DEPTH: u32 = 5;
const MAX_LIST_ENTRIES: usize = 4_096;

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
            let file_name = candidate.file_name().ok_or_else(|| AppError::PathTraversal {
                attempted: input.to_string(),
            })?;
            let canonical_parent = parent.canonicalize().map_err(|_| AppError::PathTraversal {
                attempted: input.to_string(),
            })?;
            canonical_parent.join(file_name)
        };

        if self.safe_roots.iter().any(|root| resolved.starts_with(root)) {
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
                let metadata = entry.metadata().await?;
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

#[tool]
impl FileSystemTools {
    #[tool(description = "List a safe-rooted directory with bounded breadth-first traversal and metrics")]
    pub async fn list_directory(
        &self,
        path: String,
        max_depth: Option<u32>,
    ) -> Result<ListDirResult, AppError> {
        let start = Instant::now();
        let safe_path = self.sanitize(&path)?;
        let depth = max_depth.unwrap_or(DEFAULT_LIST_DEPTH).clamp(1, MAX_LIST_DEPTH);

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

    #[tool(description = "Read a UTF-8 file from a configured safe root with byte and latency metrics")]
    pub async fn read_file(&self, path: String) -> Result<ReadFileResult, AppError> {
        let start = Instant::now();
        let safe_path = self.sanitize(&path)?;
        let content = fs::read_to_string(&safe_path).await?;
        let size = content.len();

        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.fs.read.latency_seconds").record(duration);
        counter!("mcp.fs.read.bytes_total").increment(size as u64);

        Ok(ReadFileResult {
            path: safe_path.to_string_lossy().to_string(),
            content,
            size,
        })
    }

    #[tool(description = "Atomically write a UTF-8 file under a configured safe root; supports dry-run mode")]
    pub async fn write_file(
        &self,
        path: String,
        content: String,
        dry_run: Option<bool>,
    ) -> Result<String, AppError> {
        let start = Instant::now();
        let safe_path = self.sanitize(&path)?;

        if dry_run.unwrap_or(false) {
            counter!("mcp.fs.write.dry_runs_total").increment(1);
            return Ok("DRY-RUN".to_string());
        }

        let parent = safe_path.parent().ok_or_else(|| AppError::PathTraversal {
            attempted: path.clone(),
        })?;
        let file_name = safe_path
            .file_name()
            .ok_or_else(|| AppError::PathTraversal {
                attempted: path.clone(),
            })?
            .to_string_lossy();
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
