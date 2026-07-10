use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_termux-mcp-server"))
}

#[test]
fn version_is_exact_and_does_not_require_runtime_configuration() {
    let output = binary().arg("--version").output().unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("termux-mcp-server {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn help_is_successful_and_describes_supported_commands() {
    let output = binary().arg("--help").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage:"));
    assert!(stdout.contains("termux-mcp-server --version"));
    assert!(stdout.contains("termux-mcp-server --help"));
    assert!(output.stderr.is_empty());
}

#[test]
fn unknown_and_extra_arguments_fail_before_server_startup() {
    for arguments in [vec!["--unknown"], vec!["--version", "extra"]] {
        let output = binary().args(arguments).output().unwrap();

        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("unsupported command-line arguments"));
        assert!(!stderr.contains("MCP__AUTH__STATIC_TOKEN"));
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_argument_fails_without_echoing_raw_input() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let argument = OsString::from_vec(vec![0xff, b'x']);
    let output = binary().arg(argument).output().unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unsupported command-line arguments"));
    assert!(!stderr.contains('�'));
}
