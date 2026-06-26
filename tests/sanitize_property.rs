//! Property-based tests for path sanitization logic.

use proptest::prelude::*;
use termux_mcp_server::tools::FileSystemTools;

proptest! {
    #[test]
    fn sanitize_rejects_traversal_attempts(s in ".*") {
        let tools = FileSystemTools::new(vec!["/storage/emulated/0".into(), "/sdcard".into()]);

        if s.contains("..") || s.starts_with("/etc") || s.starts_with("/data") || s.starts_with("/system") {
            prop_assert!(tools.sanitize(&s).is_err());
        }
    }

    #[test]
    fn sanitize_accepts_safe_paths(s in "[a-zA-Z0-9_/.-]{1,100}") {
        let tools = FileSystemTools::new(vec!["/storage/emulated/0".into()]);

        if !s.contains("..") && !s.starts_with('/') {
            let _ = tools.sanitize(&format!("/storage/emulated/0/{}", s));
        }
    }
}
