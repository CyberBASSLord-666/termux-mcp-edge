use std::fs;
use std::path::Path;

#[test]
fn repository_ships_only_the_canonical_project_service() {
    assert!(
        !Path::new("scripts/runit/mcp-server/run").exists(),
        "the retired mcp-server runner must not be shipped"
    );

    let deploy = fs::read_to_string("scripts/termux_deploy.sh")
        .expect("canonical deployment manager must be readable");
    assert!(deploy.contains("SERVICE_NAME=\"mcp_runtime\""));
    assert!(!deploy.contains("SERVICE_NAME=\"mcp-server\""));
}

#[test]
fn retirement_helper_is_fail_closed_and_preserves_configuration() {
    let helper = fs::read_to_string("scripts/retire_legacy_runit.sh")
        .expect("legacy retirement helper must be readable");

    for required in [
        "sv down",
        "sv status",
        "legacy service did not reach a confirmed down state",
        "legacy service path must not be a symlink",
        "preserved legacy token file",
    ] {
        assert!(helper.contains(required), "missing retirement contract: {required}");
    }

    assert!(!helper.contains("rm -f -- \"$LEGACY_TOKEN_FILE\""));
    assert!(!helper.contains("rm -rf -- \"$CANONICAL_SERVICE_DIR\""));
}

#[test]
fn migration_documentation_names_the_supported_service_and_config() {
    let guide = fs::read_to_string("docs/legacy-runit-migration.md")
        .expect("legacy migration guide must be readable");

    assert!(guide.contains("mcp_runtime"));
    assert!(guide.contains("scripts/termux_deploy.sh"));
    assert!(guide.contains(".config/termux-mcp-edge/runtime.env"));
    assert!(guide.contains("test ! -e \"$PREFIX/var/service/mcp-server\""));
}
