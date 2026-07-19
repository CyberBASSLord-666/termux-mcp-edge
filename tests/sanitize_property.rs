//! Property coverage for filesystem safe-root acceptance and rejection.

#![cfg(all(unix, feature = "mcp-runtime"))]

use proptest::prelude::*;
use termux_mcp_server::{error::AppError, tools::FileSystemTools};

proptest! {
    #[test]
    fn sanitize_accepts_existing_children_of_the_safe_root(
        name in "[a-zA-Z0-9_-]{1,48}"
    ) {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join(format!("{name}.txt"));
        std::fs::write(&child, b"safe").unwrap();
        let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()]).expect("test safe root must validate");

        let sanitized = tools.sanitize(child.to_string_lossy().as_ref()).unwrap();

        prop_assert_eq!(sanitized, child.canonicalize().unwrap());
    }

    #[test]
    fn sanitize_rejects_relative_and_parent_escape_paths(
        name in "[a-zA-Z0-9_-]{1,48}"
    ) {
        let root = tempfile::tempdir().unwrap();
        let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()]).expect("test safe root must validate");
        let relative = format!("relative/{name}.txt");
        let parent_escape = root.path().join("..").join(format!("{name}.txt"));

        let relative_result = tools.sanitize(&relative);
        let parent_result = tools.sanitize(parent_escape.to_string_lossy().as_ref());

        prop_assert!(
            matches!(relative_result, Err(AppError::PathTraversal { .. })),
            "relative path should fail safe-root validation"
        );
        prop_assert!(
            matches!(parent_result, Err(AppError::PathTraversal { .. })),
            "parent escape should fail safe-root validation"
        );
    }
}
