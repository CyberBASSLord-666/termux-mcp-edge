#![cfg(feature = "command-execution")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

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
        .env_remove("CARGO_PRIMARY_PACKAGE")
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
            "[package]\nname = \"command-api-probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\ntermux-mcp-server = {{ path = {package_path:?}, features = [\"command-execution\"] }}\n\n[workspace]\n"
        ),
    )
    .unwrap();

    let rejected = [
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
            "private",
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
            "private",
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
            "private",
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
            "private",
        ),
        (
            "copy router command flag",
            r#"
use termux_mcp_server::mcp_transport::protected_router_with_copy_file_authority;

fn main() {
    let _ = protected_router_with_copy_file_authority(
        todo!(),
        todo!(),
        todo!(),
        false,
        false,
        true,
        todo!(),
    );
}
"#,
            "protected_router_with_copy_file_authority",
            "arguments",
        ),
        (
            "all-filesystem router command flag",
            r#"
use termux_mcp_server::mcp_transport::protected_router_with_all_filesystem_authorities;

fn main() {
    let _ = protected_router_with_all_filesystem_authorities(
        todo!(),
        todo!(),
        todo!(),
        false,
        false,
        true,
        None,
        None,
        None,
    );
}
"#,
            "protected_router_with_all_filesystem_authorities",
            "arguments",
        ),
    ];

    for (name, source, expected_symbol, expected_reason) in rejected {
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
            stderr.contains(expected_symbol) && stderr.contains(expected_reason),
            "{name} failed for the wrong reason:\n{stderr}"
        );
    }
}

#[test]
fn selected_workspace_consumer_cannot_reach_removed_command_authority() {
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
        "[package]\nname = \"command-workspace-consumer\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\ntermux-mcp-server = { path = \"../server\", features = [\"command-execution\"] }\n",
    )
    .unwrap();
    write_probe_source(
        &consumer,
        r#"
use termux_mcp_server::mcp_transport;

fn main() {
    let _ = std::mem::size_of::<mcp_transport::McpTransportOptions>();
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
            "termux-mcp-server/command-execution",
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
use termux_mcp_server::mcp_transport;

fn main() {
    let _ = mcp_transport::ServerCommandAuthority::for_primary_package;
    let _ = mcp_transport::protected_primary_server_router_with_filesystem_authorities_and_options;
    let _ = mcp_transport::binary_server_router_with_filesystem_authorities_and_options;
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
            "termux-mcp-server/command-execution",
        ],
    );
    assert!(
        !rejected.status.success(),
        "a selected-workspace consumer unexpectedly reached removed command authority"
    );
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        stderr.contains("consumer/src/main.rs")
            && stderr.contains("ServerCommandAuthority")
            && stderr.contains(
                "protected_primary_server_router_with_filesystem_authorities_and_options"
            )
            && stderr.contains("binary_server_router_with_filesystem_authorities_and_options")
            && (stderr.contains("could not find")
                || stderr.contains("cannot find")
                || stderr.contains("unresolved")),
        "selected-workspace probe failed for the wrong reason:\n{stderr}"
    );
}
