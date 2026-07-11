//! Tool implementations for the default and staged MCP build postures.
//!
//! The optional `mcp-runtime` feature compiles the safe-rooted filesystem tools
//! used by the current internal staged transport. The default build retains only
//! the lightweight configuration holder. `SystemTools` remains inert; no command
//! execution or broad host-control surface is compiled here.

#[cfg(feature = "mcp-runtime")]
mod filesystem;

#[cfg(not(feature = "mcp-runtime"))]
use std::path::PathBuf;

#[cfg(feature = "mcp-runtime")]
pub use filesystem::{
    FileSystemTools, MAX_LIST_ENTRIES, MAX_LIST_RESPONSE_BYTES, MAX_READ_BYTES,
    MAX_READ_RESPONSE_BYTES,
};

#[cfg(not(feature = "mcp-runtime"))]
#[derive(Clone)]
pub struct FileSystemTools {
    safe_roots: Vec<PathBuf>,
}

#[cfg(not(feature = "mcp-runtime"))]
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
