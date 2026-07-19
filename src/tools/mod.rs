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
    CopyFileResult, CreateDirectoryResult, FileSystemTools, FindPathFilter, FindPathKind,
    FindPathMatch, FindPathsResult, HashFileResult, PathMetadataKind, PathMetadataResult,
    COPY_FILE_MODE, CREATE_DIRECTORY_MODE, MAX_BINARY_RANGE_BASE64_BYTES, MAX_BINARY_RANGE_BYTES,
    MAX_BINARY_RANGE_FILE_BYTES, MAX_BINARY_RANGE_RESPONSE_BYTES, MAX_BINARY_READ_BASE64_BYTES,
    MAX_BINARY_READ_BYTES, MAX_BINARY_READ_RESPONSE_BYTES, MAX_COPY_FILE_BYTES,
    MAX_COPY_FILE_RESPONSE_BYTES, MAX_CREATE_DIRECTORY_RESPONSE_BYTES, MAX_FIND_DEPTH,
    MAX_FIND_ENTRIES, MAX_FIND_MATCHES, MAX_FIND_QUERY_BYTES, MAX_FIND_RESPONSE_BYTES,
    MAX_HASH_FILE_BYTES, MAX_HASH_FILE_RESPONSE_BYTES, MAX_LIST_ENTRIES, MAX_LIST_RESPONSE_BYTES,
    MAX_PATH_METADATA_RESPONSE_BYTES, MAX_READ_BYTES, MAX_READ_RESPONSE_BYTES, MAX_SEARCH_DEPTH,
    MAX_SEARCH_ENTRIES, MAX_SEARCH_FILES, MAX_SEARCH_FILE_BYTES, MAX_SEARCH_MATCHES,
    MAX_SEARCH_QUERY_BYTES, MAX_SEARCH_RESPONSE_BYTES, MAX_SEARCH_TOTAL_BYTES, MIN_FIND_DEPTH,
    MIN_SEARCH_DEPTH,
};

#[cfg(feature = "mcp-runtime")]
pub(crate) use filesystem::{AuthorizedCreateDirectoryError, PreparedCreateDirectoryMutation};

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
