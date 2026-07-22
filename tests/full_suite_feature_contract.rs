#![cfg(feature = "full-suite")]

#[test]
fn full_suite_enables_every_governed_constituent_feature() {
    assert!(cfg!(feature = "mcp-runtime"));
    assert!(cfg!(feature = "android-battery-status"));
    assert!(cfg!(feature = "android-volume-status"));
    assert!(cfg!(feature = "android-volume-control"));
    assert!(cfg!(feature = "command-execution"));
}
