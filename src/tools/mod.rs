//! Tool module placeholders for the current HTTP health-check service.
//!
//! MCP tool transport integration is intentionally tracked separately. Until a
//! patched and compatible rmcp integration is restored, the vulnerable rmcp-backed
//! tool modules remain out of the compiled module tree.

use std::path::PathBuf;

#[derive(Clone)]
pub struct FileSystemTools {
    safe_roots: Vec<PathBuf>,
}

impl FileSystemTools {
    pub fn new(safe_roots: Vec<PathBuf>) -> Self {
        Self { safe_roots }
    }

    pub fn safe_roots(&self) -> &[PathBuf] {
        &self.safe_roots
    }
}

#[derive(Clone, Default)]
pub struct SystemTools;
