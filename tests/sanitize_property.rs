//! Configuration-facing tests for quarantined filesystem tool state.

use std::path::PathBuf;

use proptest::prelude::*;
use termux_mcp_server::tools::FileSystemTools;

proptest! {
    #[test]
    fn filesystem_tools_preserve_safe_root_order(paths in proptest::collection::vec("/[a-zA-Z0-9_/.-]{1,64}", 1..8)) {
        let roots: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        let tools = FileSystemTools::new(roots.clone());

        prop_assert_eq!(tools.safe_roots(), roots.as_slice());
    }
}
