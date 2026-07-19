//! Tool implementations for the default and staged MCP build postures.
//!
//! The optional `mcp-runtime` feature compiles the safe-rooted filesystem tools
//! used by the current internal staged transport. The default build retains only
//! the lightweight configuration holder. `SystemTools` remains inert; no command
//! execution or broad host-control surface is compiled here.

#[cfg(feature = "mcp-runtime")]
mod filesystem;

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

pub(crate) const WRITE_FILE_QUARANTINE_DIRECTORY: &str = ".termux-mcp-write-quarantine";

pub(crate) fn is_write_quarantine_name(name: &OsStr) -> bool {
    name.as_encoded_bytes()
        .eq_ignore_ascii_case(WRITE_FILE_QUARANTINE_DIRECTORY.as_bytes())
}

/// A non-sensitive reason that a filesystem safe-root configuration was rejected.
///
/// The rejected path is intentionally never retained in this error. Callers may
/// safely surface its `Display` representation without disclosing configured
/// filesystem locations.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum SafeRootConfigurationError {
    #[error("at least one filesystem safe root must be configured")]
    EmptyConfiguration,

    #[error("filesystem safe roots must not contain an empty path")]
    EmptyPath,

    #[error("filesystem safe roots must be absolute paths")]
    RelativePath,

    #[error("filesystem root must not be configured as a safe root")]
    FilesystemRoot,

    #[error("filesystem safe roots must not include a reserved runtime namespace")]
    ReservedNamespace,

    #[error("a configured filesystem safe root cannot be resolved")]
    Unresolved,

    #[error("a configured filesystem safe root is not a directory")]
    NotDirectory,
}

fn canonical_safe_roots(
    safe_roots: Vec<PathBuf>,
) -> Result<Vec<PathBuf>, SafeRootConfigurationError> {
    if safe_roots.is_empty() {
        return Err(SafeRootConfigurationError::EmptyConfiguration);
    }

    let mut canonical_roots = Vec::with_capacity(safe_roots.len());
    for root in safe_roots {
        if root.as_os_str().is_empty() {
            return Err(SafeRootConfigurationError::EmptyPath);
        }
        if !root.is_absolute() {
            return Err(SafeRootConfigurationError::RelativePath);
        }
        if root == Path::new("/") {
            return Err(SafeRootConfigurationError::FilesystemRoot);
        }
        if contains_write_quarantine_component(&root) {
            return Err(SafeRootConfigurationError::ReservedNamespace);
        }

        let canonical = root
            .canonicalize()
            .map_err(|_| SafeRootConfigurationError::Unresolved)?;
        if canonical == Path::new("/") {
            return Err(SafeRootConfigurationError::FilesystemRoot);
        }
        if contains_write_quarantine_component(&canonical) {
            return Err(SafeRootConfigurationError::ReservedNamespace);
        }

        let metadata =
            std::fs::metadata(&canonical).map_err(|_| SafeRootConfigurationError::Unresolved)?;
        if !metadata.is_dir() {
            return Err(SafeRootConfigurationError::NotDirectory);
        }
        canonical_roots.push(canonical);
    }

    canonical_roots.sort_unstable();
    canonical_roots.dedup();
    Ok(canonical_roots)
}

fn contains_write_quarantine_component(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            Component::Normal(name) if is_write_quarantine_name(name)
        )
    })
}

#[cfg(feature = "mcp-runtime")]
pub use filesystem::{
    CopyFileResult, CreateDirectoryResult, FileSystemTools, FindPathFilter, FindPathKind,
    FindPathMatch, FindPathsResult, HashFileResult, PathMetadataKind, PathMetadataResult,
    ReadTextRangeResult, WriteFileResult, COPY_FILE_MODE, CREATE_DIRECTORY_MODE,
    MAX_BINARY_RANGE_BASE64_BYTES, MAX_BINARY_RANGE_BYTES, MAX_BINARY_RANGE_FILE_BYTES,
    MAX_BINARY_RANGE_RESPONSE_BYTES, MAX_BINARY_READ_BASE64_BYTES, MAX_BINARY_READ_BYTES,
    MAX_BINARY_READ_RESPONSE_BYTES, MAX_COPY_FILE_BYTES, MAX_COPY_FILE_RESPONSE_BYTES,
    MAX_CREATE_DIRECTORY_RESPONSE_BYTES, MAX_FIND_DEPTH, MAX_FIND_ENTRIES, MAX_FIND_MATCHES,
    MAX_FIND_QUERY_BYTES, MAX_FIND_RESPONSE_BYTES, MAX_HASH_FILE_BYTES,
    MAX_HASH_FILE_RESPONSE_BYTES, MAX_LIST_ENTRIES, MAX_LIST_RESPONSE_BYTES,
    MAX_PATH_METADATA_RESPONSE_BYTES, MAX_READ_BYTES, MAX_READ_RESPONSE_BYTES, MAX_SEARCH_DEPTH,
    MAX_SEARCH_ENTRIES, MAX_SEARCH_FILES, MAX_SEARCH_FILE_BYTES, MAX_SEARCH_MATCHES,
    MAX_SEARCH_QUERY_BYTES, MAX_SEARCH_RESPONSE_BYTES, MAX_SEARCH_TOTAL_BYTES,
    MAX_TEXT_RANGE_BYTES, MAX_TEXT_RANGE_ESCAPED_BYTES, MAX_TEXT_RANGE_FILE_BYTES,
    MAX_TEXT_RANGE_RESPONSE_BYTES, MAX_WRITE_FILE_RESPONSE_BYTES, MIN_FIND_DEPTH, MIN_SEARCH_DEPTH,
    MIN_TEXT_RANGE_BYTES, WRITE_FILE_MODE,
};

#[cfg(feature = "mcp-runtime")]
pub(crate) use filesystem::{
    AuthorizedCreateDirectoryError, AuthorizedWriteFileError, PreparedCreateDirectoryMutation,
};

#[cfg(not(feature = "mcp-runtime"))]
#[derive(Clone)]
pub struct FileSystemTools {
    safe_roots: Vec<PathBuf>,
}

#[cfg(not(feature = "mcp-runtime"))]
impl FileSystemTools {
    /// Validate, canonicalize, and deterministically deduplicate safe roots.
    pub fn try_new(safe_roots: Vec<PathBuf>) -> Result<Self, SafeRootConfigurationError> {
        Ok(Self {
            safe_roots: canonical_safe_roots(safe_roots)?,
        })
    }

    pub fn safe_roots(&self) -> &[PathBuf] {
        &self.safe_roots
    }
}

#[derive(Clone, Default)]
pub struct SystemTools;

#[cfg(test)]
mod tests {
    use super::{FileSystemTools, SafeRootConfigurationError, WRITE_FILE_QUARANTINE_DIRECTORY};
    use std::path::PathBuf;

    fn construction_error(safe_roots: Vec<PathBuf>) -> SafeRootConfigurationError {
        match FileSystemTools::try_new(safe_roots) {
            Ok(_) => panic!("invalid safe roots unexpectedly succeeded"),
            Err(error) => error,
        }
    }

    #[test]
    fn safe_roots_reject_empty_configuration() {
        let error = construction_error(Vec::new());
        assert_eq!(error, SafeRootConfigurationError::EmptyConfiguration);
    }

    #[test]
    fn safe_roots_reject_empty_and_relative_paths() {
        let empty = construction_error(vec![PathBuf::new()]);
        assert_eq!(empty, SafeRootConfigurationError::EmptyPath);

        let relative = construction_error(vec![PathBuf::from("relative/root")]);
        assert_eq!(relative, SafeRootConfigurationError::RelativePath);
    }

    #[test]
    fn safe_roots_reject_filesystem_root() {
        let error = construction_error(vec![PathBuf::from("/")]);
        assert_eq!(error, SafeRootConfigurationError::FilesystemRoot);
    }

    #[test]
    fn safe_roots_reject_unresolved_paths_without_disclosing_them() {
        let parent = tempfile::tempdir().unwrap();
        let missing = parent.path().join("private-missing-root");

        let error = construction_error(vec![missing.clone()]);
        assert_eq!(error, SafeRootConfigurationError::Unresolved);
        assert!(!error
            .to_string()
            .contains(missing.to_string_lossy().as_ref()));
    }

    #[test]
    fn safe_roots_reject_regular_files_without_disclosing_them() {
        let parent = tempfile::tempdir().unwrap();
        let file = parent.path().join("private-file");
        std::fs::write(&file, b"not a directory").unwrap();

        let error = construction_error(vec![file.clone()]);
        assert_eq!(error, SafeRootConfigurationError::NotDirectory);
        assert!(!error.to_string().contains(file.to_string_lossy().as_ref()));
    }

    #[test]
    fn safe_roots_reject_reserved_quarantine_roots_and_overlaps() {
        let parent = tempfile::tempdir().unwrap();
        let quarantine = parent.path().join(WRITE_FILE_QUARANTINE_DIRECTORY);
        std::fs::create_dir(&quarantine).unwrap();

        let direct = construction_error(vec![quarantine.clone()]);
        assert_eq!(direct, SafeRootConfigurationError::ReservedNamespace);

        let overlap = construction_error(vec![parent.path().to_path_buf(), quarantine]);
        assert_eq!(overlap, SafeRootConfigurationError::ReservedNamespace);
    }

    #[test]
    fn safe_roots_reject_mixed_case_reserved_components() {
        let parent = tempfile::tempdir().unwrap();
        let mixed_case = parent.path().join(".TeRmUx-McP-WrItE-qUaRaNtInE");
        std::fs::create_dir(&mixed_case).unwrap();

        let direct = construction_error(vec![mixed_case.clone()]);
        assert_eq!(direct, SafeRootConfigurationError::ReservedNamespace);

        let descendant = mixed_case.join("child");
        std::fs::create_dir(&descendant).unwrap();
        let nested = construction_error(vec![descendant]);
        assert_eq!(nested, SafeRootConfigurationError::ReservedNamespace);
    }

    #[test]
    fn safe_roots_reject_lexical_reserved_components_even_when_resolution_removes_them() {
        let parent = tempfile::tempdir().unwrap();
        let quarantine = parent.path().join(WRITE_FILE_QUARANTINE_DIRECTORY);
        std::fs::create_dir(&quarantine).unwrap();

        let lexical_alias = quarantine.join("..");
        assert_eq!(
            lexical_alias.canonicalize().unwrap(),
            parent.path().canonicalize().unwrap()
        );

        let error = construction_error(vec![lexical_alias]);
        assert_eq!(error, SafeRootConfigurationError::ReservedNamespace);
    }

    #[test]
    fn safe_roots_are_canonical_deduplicated_and_sorted() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let mut expected = vec![
            first.path().canonicalize().unwrap(),
            second.path().canonicalize().unwrap(),
        ];
        expected.sort_unstable();

        let tools = FileSystemTools::try_new(vec![
            second.path().to_path_buf(),
            first.path().join("."),
            first.path().to_path_buf(),
        ])
        .unwrap();

        assert_eq!(tools.safe_roots(), expected);
    }

    #[cfg(unix)]
    #[test]
    fn safe_roots_deduplicate_symlink_aliases() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let alias = parent.path().join("root-alias");
        symlink(root.path(), &alias).unwrap();

        let tools = FileSystemTools::try_new(vec![alias, root.path().to_path_buf()]).unwrap();
        assert_eq!(tools.safe_roots(), &[root.path().canonicalize().unwrap()]);
    }

    #[cfg(unix)]
    #[test]
    fn safe_roots_reject_aliases_that_canonicalize_into_reserved_namespace() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let quarantine = parent.path().join(WRITE_FILE_QUARANTINE_DIRECTORY);
        std::fs::create_dir(&quarantine).unwrap();
        let alias = parent.path().join("apparently-safe-alias");
        symlink(&quarantine, &alias).unwrap();

        let error = construction_error(vec![alias]);
        assert_eq!(error, SafeRootConfigurationError::ReservedNamespace);
    }

    #[cfg(unix)]
    #[test]
    fn safe_roots_reject_aliases_that_canonicalize_to_filesystem_root() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let alias = parent.path().join("root-alias");
        symlink("/", &alias).unwrap();

        let error = construction_error(vec![alias]);
        assert_eq!(error, SafeRootConfigurationError::FilesystemRoot);
    }
}
