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

fn assert_symbol_is_closed(stderr: &str, symbol: &str, context: &str) {
    assert!(
        stderr.contains(symbol)
            && (stderr.contains("private")
                || stderr.contains("unresolved import")
                || stderr.contains("no ")),
        "{context} failed for the wrong reason:\n{stderr}"
    );
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
        assert_symbol_is_closed(&stderr, expected_symbol, name);
    }
}

#[test]
fn one_public_builder_compiles_and_every_legacy_router_entry_is_closed() {
    let probe = tempfile::tempdir().unwrap();
    fs::create_dir(probe.path().join("src")).unwrap();
    let package_path = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::write(
        probe.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"secure-router-api-probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\ntermux-mcp-server = {{ path = {package_path:?}, features = [\"command-execution\", \"android-volume-control\"] }}\n\n[workspace]\n"
        ),
    )
    .unwrap();

    write_probe_source(
        probe.path(),
        r#"
use std::path::PathBuf;
use termux_mcp_server::{
    auth::McpAuthPolicy,
    mcp_transport::McpRouterBuilder,
    request_limits::McpRequestLimits,
    transport_security::TransportSecurityPolicy,
};

fn main() {
    let _ = McpRouterBuilder::new(
        "127.0.0.1",
        McpAuthPolicy::static_bearer("compile-probe-token").unwrap(),
        McpRequestLimits::from_seconds(1, 1, 1024).unwrap(),
        TransportSecurityPolicy::localhost(8000, false).unwrap(),
        vec![PathBuf::from("/compile-only-safe-root")],
    );
}
"#,
    );
    let valid = run_cargo(
        probe.path(),
        &probe.path().join("target-valid"),
        &["check", "--quiet", "--offline", "--jobs", "1"],
    );
    assert!(
        valid.status.success(),
        "the one public MCP builder failed to compile for a dependency consumer:\n{}",
        String::from_utf8_lossy(&valid.stderr)
    );

    let rejected = [
        "McpRouterProtection",
        "McpCapabilityAuthorities",
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
    ];

    for symbol in rejected {
        write_probe_source(
            probe.path(),
            &format!(
                "use termux_mcp_server::mcp_transport::{symbol};\n\nfn main() {{ let _ = {symbol}; }}\n"
            ),
        );
        let output = run_cargo(
            probe.path(),
            &probe.path().join("target-closed"),
            &["check", "--quiet", "--offline", "--jobs", "1"],
        );
        assert!(
            !output.status.success(),
            "{symbol} unexpectedly remained public"
        );
        assert_symbol_is_closed(
            &String::from_utf8_lossy(&output.stderr),
            symbol,
            "legacy router constructor",
        );
    }

    write_probe_source(
        probe.path(),
        r#"
use termux_mcp_server::mcp_transport::McpRouterBuilder;

fn cannot_enable_command(builder: McpRouterBuilder) {
    let _ = builder.with_command_execution_enabled(true);
}

fn main() {}
"#,
    );
    let command = run_cargo(
        probe.path(),
        &probe.path().join("target-command"),
        &["check", "--quiet", "--offline", "--jobs", "1"],
    );
    assert!(!command.status.success(), "downstream command enablement compiled");
    assert_symbol_is_closed(
        &String::from_utf8_lossy(&command.stderr),
        "with_command_execution_enabled",
        "builder command enablement",
    );
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
};

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
    assert!(stderr.contains("consumer/src/main.rs"));
    assert_symbol_is_closed(
        &stderr,
        "binary_server_router_with_filesystem_authorities_and_options",
        "selected-workspace filesystem router",
    );
    assert_symbol_is_closed(
        &stderr,
        "binary_server_router_with_capability_authorities_and_options",
        "selected-workspace capability router",
    );
}
