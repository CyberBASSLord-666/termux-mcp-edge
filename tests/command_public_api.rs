#![cfg(feature = "command-execution")]

use std::{fs, path::Path, process::Command};

fn write_probe_source(root: &Path, source: &str) {
    fs::write(root.join("src/main.rs"), source).unwrap();
}

fn run_cargo(root: &Path, target: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO"))
        .args(arguments)
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .unwrap()
}

fn copy_source_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        let destination_path = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_source_tree(&entry.path(), &destination_path);
        } else if file_type.is_file() {
            fs::copy(entry.path(), destination_path).unwrap();
        } else {
            panic!("source fixture contains a symlink or special file");
        }
    }
}

#[test]
fn dependency_consumers_cannot_forge_command_execution_authority() {
    let probe = tempfile::tempdir().unwrap();
    fs::create_dir(probe.path().join("src")).unwrap();
    let package_path = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::write(
        probe.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"command-api-probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\ntermux-mcp-server = {{ path = {package_path:?}, features = [\"command-execution\", \"android-volume-control\"] }}\n\n[workspace]\n"
        ),
    )
    .unwrap();

    let rejected = [
        (
            "bearer principal extraction",
            r#"
use termux_mcp_server::auth::McpAuthPolicy;

fn main() {
    let policy = McpAuthPolicy::static_bearer("opaque-principal").unwrap();
    let McpAuthPolicy { kind } = policy;
    let _ = kind;
}
"#,
            "kind",
        ),
        (
            "forged profile",
            r#"
use std::time::Duration;
use termux_mcp_server::command_policy::CommandProfile;

fn main() {
    let _ = CommandProfile {
        id: "forged",
        ordinal: 99,
        argv: &["--raw"],
        timeout: Duration::from_secs(99),
        max_stdout_bytes: usize::MAX,
        max_stderr_bytes: usize::MAX,
    };
}
"#,
            "CommandProfile",
        ),
        (
            "raw execution client",
            r#"
use termux_mcp_server::command_execution::CommandExecutionClient;

fn main() {
    let _ = std::mem::size_of::<CommandExecutionClient>();
}
"#,
            "command_execution",
        ),
        (
            "resolved profile handle",
            r#"
use termux_mcp_server::command_policy::CommandExecutionPolicy;

fn main() {
    let decision = CommandExecutionPolicy::new().evaluate("server_version", true, true);
    let _ = decision.profile;
}
"#,
            "profile",
        ),
        (
            "binary-only command enablement switch",
            r#"
use termux_mcp_server::mcp_transport::McpRouterBuilder;

fn attempt(builder: McpRouterBuilder) {
    let _ = builder.with_command_execution_enabled(true);
}

fn main() {}
"#,
            "with_command_execution_enabled",
        ),
        (
            "binary-owned command router",
            r#"
use termux_mcp_server::mcp_transport::binary_server_router_with_filesystem_authorities_and_options;

fn main() {
    let _ = binary_server_router_with_filesystem_authorities_and_options;
}
"#,
            "binary_server_router_with_filesystem_authorities_and_options",
        ),
        (
            "binary-owned all-capabilities command router",
            r#"
use termux_mcp_server::mcp_transport::binary_server_router_with_capability_authorities_and_options;

fn main() {
    let _ = binary_server_router_with_capability_authorities_and_options;
}
"#,
            "binary_server_router_with_capability_authorities_and_options",
        ),
        (
            "forged trash grant target",
            r#"
use termux_mcp_server::trash_file_grant::TrashFileGrantTarget;

fn main() {
    let _ = TrashFileGrantTarget {
        root_device: 1,
        root_inode: 2,
        target_digest: [0; 32],
        content_digest: [0; 32],
        identity: unreachable!(),
    };
}
"#,
            "root_device",
        ),
        (
            "crate-private trash transaction types",
            r#"
use termux_mcp_server::tools::{AuthorizedTrashFileError, PreparedTrashFileMutation};

fn main() {
    let _ = std::mem::size_of::<PreparedTrashFileMutation>();
    let _ = std::mem::size_of::<AuthorizedTrashFileError>();
}
"#,
            "PreparedTrashFileMutation",
        ),
    ];

    for (name, source, expected_symbol) in rejected {
        write_probe_source(probe.path(), source);
        let output = run_cargo(
            probe.path(),
            &probe.path().join("target"),
            &["check", "--quiet", "--offline", "--jobs", "1"],
        );
        assert!(
            !output.status.success(),
            "{name} unexpectedly compiled as a dependency consumer"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(expected_symbol)
                && (stderr.contains("private") || stderr.contains("unresolved import")),
            "{name} failed for the wrong reason:\n{stderr}"
        );
    }
}

#[test]
fn dependency_consumers_cannot_restore_legacy_router_construction_surfaces() {
    let probe = tempfile::tempdir().unwrap();
    fs::create_dir(probe.path().join("src")).unwrap();
    let package_path = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::write(
        probe.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"command-router-arity-probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\ntermux-mcp-server = {{ path = {package_path:?}, features = [\"command-execution\", \"android-volume-control\"] }}\n\n[workspace]\n"
        ),
    )
    .unwrap();

    // These former public entry points could be mixed and matched in ways that
    // omitted a mandatory boundary. They must remain absent now that the sole
    // public entry point is `McpRouterBuilder::try_new`.
    let rejected = [
        "McpTransportState",
        "FilesystemMutationAuthorities",
        "router",
        "router_with_options",
        "router_with_create_directory_authority",
        "router_with_create_directory_authority_and_options",
        "router_with_filesystem_authorities",
        "router_with_filesystem_authorities_and_options",
        "router_with_capability_authorities",
        "router_with_capability_authorities_and_options",
        "router_from_state",
        "binary_server_router_with_filesystem_authorities_and_options",
        "binary_server_router_with_capability_authorities_and_options",
        "protected_router",
        "protected_router_with_options",
        "protected_router_with_create_directory_authority",
        "protected_router_with_create_directory_authority_and_options",
        "protected_router_with_copy_file_authority",
        "protected_router_with_copy_file_authority_and_options",
        "protected_router_with_filesystem_authorities",
        "protected_router_with_filesystem_authorities_and_options",
        "protected_router_with_all_filesystem_authorities",
        "protected_router_with_all_filesystem_authorities_and_options",
        "protected_router_with_capability_authorities",
        "protected_router_with_capability_authorities_and_options",
        "McpRouterProtection",
        "McpTransportOptions",
        "McpCapabilityAuthorities",
    ];

    for symbol in rejected {
        write_probe_source(
            probe.path(),
            &format!(
                "use termux_mcp_server::mcp_transport::{symbol};\n\nfn main() {{\n    let _ = std::mem::size_of::<{symbol}>();\n}}\n"
            ),
        );
        let output = run_cargo(
            probe.path(),
            &probe.path().join("target"),
            &["check", "--quiet", "--offline", "--jobs", "1"],
        );
        assert!(
            !output.status.success(),
            "legacy public construction surface {symbol} unexpectedly compiled"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(symbol)
                && (stderr.contains("unresolved import") || stderr.contains("private")),
            "{symbol} failed for the wrong reason:\n{stderr}"
        );
    }
}

#[test]
fn selected_workspace_consumer_cannot_reach_binary_command_router() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let package = Path::new(env!("CARGO_MANIFEST_DIR"));
    let server = root.join("server");
    let consumer = root.join("consumer");

    fs::create_dir_all(server.join("src")).unwrap();
    fs::create_dir_all(consumer.join("src")).unwrap();
    fs::copy(package.join("Cargo.toml"), server.join("Cargo.toml")).unwrap();
    fs::copy(package.join("README.md"), server.join("README.md")).unwrap();
    fs::copy(package.join("Cargo.lock"), root.join("Cargo.lock")).unwrap();
    copy_source_tree(&package.join("src"), &server.join("src"));

    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"server\", \"consumer\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    fs::write(
        consumer.join("Cargo.toml"),
        "[package]\nname = \"command-workspace-consumer\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\ntermux-mcp-server = { path = \"../server\", features = [\"command-execution\", \"android-volume-control\"] }\n",
    )
    .unwrap();
    write_probe_source(
        &consumer,
        r#"
use termux_mcp_server::mcp_transport;

fn main() {
    let _ = std::mem::size_of::<mcp_transport::McpRouterBuilder>();
}
"#,
    );

    let valid = run_cargo(
        root,
        &root.join("target-valid-workspace"),
        &[
            "check",
            "--quiet",
            "--offline",
            "--jobs",
            "1",
            "--workspace",
            "--features",
            "termux-mcp-server/command-execution,termux-mcp-server/android-volume-control",
        ],
    );
    assert!(
        valid.status.success(),
        "valid selected-workspace consumer failed before the adversarial probe:\n{}",
        String::from_utf8_lossy(&valid.stderr)
    );

    write_probe_source(
        &consumer,
        r#"
use termux_mcp_server::mcp_transport::{
    binary_server_router_with_capability_authorities_and_options,
    binary_server_router_with_filesystem_authorities_and_options,
    McpRouterBuilder,
};

fn attempt(builder: McpRouterBuilder) {
    let _ = builder.with_command_execution_enabled(true);
}

fn main() {
    let _ = binary_server_router_with_filesystem_authorities_and_options;
    let _ = binary_server_router_with_capability_authorities_and_options;
}
"#,
    );

    let rejected = run_cargo(
        root,
        &root.join("target-workspace"),
        &[
            "check",
            "--quiet",
            "--offline",
            "--jobs",
            "1",
            "--workspace",
            "--features",
            "termux-mcp-server/command-execution,termux-mcp-server/android-volume-control",
        ],
    );
    assert!(
        !rejected.status.success(),
        "a selected-workspace consumer unexpectedly reached the binary command router"
    );
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        stderr.contains("consumer/src/main.rs")
            && stderr.contains("binary_server_router_with_filesystem_authorities_and_options")
            && stderr.contains("binary_server_router_with_capability_authorities_and_options")
            && stderr.contains("with_command_execution_enabled")
            && (stderr.contains("private") || stderr.contains("unresolved import")),
        "selected-workspace probe failed for the wrong reason:\n{stderr}"
    );
}
