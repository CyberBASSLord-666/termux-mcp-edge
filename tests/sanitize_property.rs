//! Property-based tests for path sanitization logic.

use std::sync::OnceLock;

use proptest::prelude::*;
use termux_mcp_server::tools::FileSystemTools;

proptest! {
    #[test]
    fn sanitize_accepts_simple_file_names_inside_the_safe_root(
        name in "[a-zA-Z0-9_.-]{1,64}",
    ) {
        static TEMP_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
        let temp_dir = TEMP_DIR.get_or_init(|| tempfile::tempdir().expect("create temp dir"));
        let root = temp_dir.path().canonicalize().expect("canonicalize temp root");
        let tools = FileSystemTools::new(vec![root.clone()]);
        let candidate = root.join(name);

        prop_assert!(tools.sanitize(candidate.to_string_lossy().as_ref()).is_ok());
    }

    #[test]
    fn sanitize_rejects_relative_paths(path in "[a-zA-Z0-9_/.-]{1,128}") {
        static TEMP_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
        let temp_dir = TEMP_DIR.get_or_init(|| tempfile::tempdir().expect("create temp dir"));
        let root = temp_dir.path().canonicalize().expect("canonicalize temp root");
        let tools = FileSystemTools::new(vec![root]);

        prop_assume!(!path.starts_with('/'));
        prop_assert!(tools.sanitize(&path).is_err());
    }

    #[test]
    fn sanitize_rejects_parent_directory_components(name in "[a-zA-Z0-9_.-]{1,64}") {
        static TEMP_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
        let temp_dir = TEMP_DIR.get_or_init(|| tempfile::tempdir().expect("create temp dir"));
        let root = temp_dir.path().canonicalize().expect("canonicalize temp root");
        let tools = FileSystemTools::new(vec![root.clone()]);
        let candidate = root.join("..").join(name);

        prop_assert!(tools.sanitize(candidate.to_string_lossy().as_ref()).is_err());
    }
}

#[test]
fn sanitize_rejects_common_android_and_linux_escape_targets() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let root = temp_dir.path().canonicalize().expect("canonicalize temp root");
    let tools = FileSystemTools::new(vec![root]);

    for path in ["/etc/passwd", "/data/data", "/system/build.prop", "", "\0"] {
        assert!(tools.sanitize(path).is_err(), "expected rejection for {path:?}");
    }
}
