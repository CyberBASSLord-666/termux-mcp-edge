//! Comprehensive FileSystemTools with metrics.

use std::path::{Path, PathBuf};
use std::time::Instant;

use metrics::{counter, histogram};
use rmcp::tool;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::error::AppError;

#[derive(Clone)]
pub struct FileSystemTools {
    safe_roots: Vec<PathBuf>,
}

impl FileSystemTools {
    pub fn new(safe_roots: Vec<PathBuf>) -> Self {
        Self { safe_roots }
    }

    fn sanitize(&self, input: &str) -> Result<PathBuf, AppError> {
        let candidate = Path::new(input);
        let absolute = candidate.canonicalize().unwrap_or_else(|_| candidate.to_path_buf());

        for root in &self.safe_roots {
            if absolute.starts_with(root) {
                return Ok(absolute);
            }
        }
        Err(AppError::PathTraversal { attempted: input.to_string() })
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
    #[tool(description = "List directory (with metrics)")]
    pub async fn list_directory(&self, path: String, max_depth: Option<u32>) -> Result<ListDirResult, AppError> {
        // Start latency timer
        let start = Instant::now();
        // Sanitize the top-level path. This ensures we only operate within the
        // configured safe roots and prevents directory traversal attacks.  The
        // sanitize call canonicalizes the provided path and verifies that the
        // resulting absolute path starts with one of the configured safe roots.
        let safe_path = self.sanitize(&path)?;

        // Determine the recursion depth. If max_depth is None, default to 1
        // (only list the immediate directory).  A depth of N will list all
        // sub‑directories up to N levels deep.
        let depth = max_depth.unwrap_or(1);

        let mut entries = Vec::new();
        // Recursively collect entries.  Each entry is sanitized again to
        // protect against symlink traversal that might escape the safe roots.
        self.collect_entries(&safe_path, &mut entries, depth).await?;

        // Record metrics
        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.fs.list.latency_seconds").record(duration);
        counter!("mcp.fs.list.calls_total").increment(1);

        Ok(ListDirResult {
            path: safe_path.to_string_lossy().to_string(),
            entries,
        })
    }

    /// Recursively collect directory entries.  This helper walks the
    /// filesystem starting at `path`, appending a `FileInfo` record for
    /// each file or subdirectory it encounters.  If `depth` is greater than 1
    /// and the entry is a directory, this function recurses into that
    /// directory.  Every encountered path is sanitized to prevent escaping
    /// the configured safe roots, which also mitigates symlink attacks.
    async fn collect_entries(&self, path: &std::path::Path, entries: &mut Vec<FileInfo>, depth: u32) -> Result<(), AppError> {
        // Attempt to read directory contents.  If the path is not a
        // directory or cannot be read, propagate the IO error back to the
        // caller.  We do not silently ignore errors here because the caller
        // should be made aware of permission or existence issues.
        let mut read_dir = fs::read_dir(path).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            let entry_path = entry.path();
            let metadata = entry.metadata().await?;
            let is_dir = metadata.is_dir();
            let size = metadata.len();

            // Convert the modification time to an RFC3339 string if available.  If
            // the modification time cannot be read (e.g. certain android file
            // systems) we fall back to None.
            let modified = metadata.modified().ok().and_then(|mtime| {
                let dt: chrono::DateTime<chrono::Utc> = mtime.into();
                Some(dt.to_rfc3339())
            });

            // Sanitize the child path relative to the safe roots.  This check
            // prevents following symlinks or encountering hard links that
            // resolve outside of the safe roots.  If sanitization fails, the
            // entry is skipped instead of causing the entire listing to fail.
            if let Ok(safe_child) = self.sanitize(entry_path.to_string_lossy().as_ref()) {
                entries.push(FileInfo {
                    path: safe_child.to_string_lossy().to_string(),
                    size,
                    is_dir,
                    modified,
                });

                // If this entry is a directory and we still have recursion depth
                // remaining, recurse into it.  Decrement the depth to avoid
                // infinite recursion.  Errors during recursion are propagated
                // back to the caller.
                if is_dir && depth > 1 {
                    self.collect_entries(&safe_child, entries, depth - 1).await?;
                }
            }
        }
        Ok(())
    }

    #[tool(description = "Read file with byte metrics")]
    pub async fn read_file(&self, path: String) -> Result<ReadFileResult, AppError> {
        let start = Instant::now();
        let safe_path = self.sanitize(&path)?;
        let content = fs::read_to_string(&safe_path).await?;
        let size = content.len();

        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.fs.read.latency_seconds").record(duration);
        counter!("mcp.fs.read.bytes_total").increment(size as u64);

        Ok(ReadFileResult { path: safe_path.to_string_lossy().to_string(), content, size })
    }

    #[tool(description = "Write file with metrics")]
    pub async fn write_file(&self, path: String, content: String, dry_run: Option<bool>) -> Result<String, AppError> {
        let start = Instant::now();
        let safe_path = self.sanitize(&path)?;

        if dry_run.unwrap_or(false) {
            return Ok("DRY-RUN".to_string());
        }

        let tmp = safe_path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
        fs::write(&tmp, content.as_bytes()).await?;
        fs::rename(&tmp, &safe_path).await?;

        let duration = start.elapsed().as_secs_f64();
        histogram!("mcp.fs.write.latency_seconds").record(duration);
        counter!("mcp.fs.write.bytes_total").increment(content.len() as u64);

        Ok(format!("Wrote {} bytes", content.len()))
    }
}
