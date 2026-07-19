#![cfg(feature = "command-execution")]

use std::{fs, path::Path, process::Command};

fn write_probe_source(root: &Path, source: &str) {
    fs::write(root.join("src/main.rs"), source).unwrap();
}

fn run_cargo(root: &Path, subcommand: &str) -> std::process::Output {
    Command::new(env!("CARGO"))
        .arg(subcommand)
        .arg("--quiet")
        .arg("--offline")
        .current_dir(root)
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env("CARGO_TERM_COLOR", "never")
        .env_remove("CARGO_PRIMARY_PACKAGE")
        .output()
        .unwrap()
}

#[test]
fn dependency_consumers_cannot_forge_command_execution_authority() {
    let probe = tempfile::tempdir().unwrap();
    fs::create_dir(probe.path().join("src")).unwrap();
    let package_path = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::write(
        probe.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"command-api-probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\ntermux-mcp-server = {{ path = {:?}, features = [\"command-execution\"] }}\n\n[workspace]\n",
            package_path
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
            "forged primary-server authority",
            r#"
use std::num::NonZeroU8;
use termux_mcp_server::mcp_transport::ServerCommandAuthority;

fn main() {
    let _ = ServerCommandAuthority {
        _private: NonZeroU8::MIN,
    };
}
"#,
            "_private",
        ),
    ];

    for (name, source, expected_symbol) in rejected {
        write_probe_source(probe.path(), source);
        let output = run_cargo(probe.path(), "check");
        assert!(
            !output.status.success(),
            "{name} unexpectedly compiled as a dependency consumer"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(expected_symbol) && stderr.contains("private"),
            "{name} failed for the wrong reason:\n{stderr}"
        );
    }

    write_probe_source(
        probe.path(),
        r#"
use termux_mcp_server::mcp_transport::ServerCommandAuthority;

fn main() {
    assert!(
        ServerCommandAuthority::for_primary_package().is_none(),
        "dependency builds must not acquire primary-server command authority"
    );
}
"#,
    );
    let output = run_cargo(probe.path(), "run");
    assert!(
        output.status.success(),
        "dependency authority probe failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
