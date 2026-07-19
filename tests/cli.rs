use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_termux-mcp-server"))
}

fn isolated_binary() -> Command {
    let mut command = binary();
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("MCP__") {
            command.env_remove(name);
        }
    }
    command
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
    assert!(stdout.contains("termux-mcp-server --issue-create-directory-grant"));
    assert!(stdout.contains("termux-mcp-server --issue-write-file-grant"));
    assert!(stdout.contains("termux-mcp-server --issue-android-volume-grant"));
    assert!(output.stderr.is_empty());
}

#[cfg(not(feature = "android-volume-control"))]
#[test]
fn volume_grant_issuance_fails_closed_without_the_compiled_capability() {
    let output = isolated_binary()
        .arg("--issue-android-volume-grant")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("requires a binary built with the android-volume-control feature"));
}

#[cfg(not(feature = "mcp-runtime"))]
#[test]
fn grant_issuance_fails_closed_without_the_compiled_runtime() {
    for argument in ["--issue-create-directory-grant", "--issue-write-file-grant"] {
        let output = isolated_binary().arg(argument).output().unwrap();

        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("requires a binary built with the mcp-runtime feature"));
    }
}

#[cfg(feature = "mcp-runtime")]
fn configured_issuer(root: &std::path::Path, target: &std::path::Path) -> Command {
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let mut command = isolated_binary();
    command
        .arg("--issue-create-directory-grant")
        .env("MCP__AUTH__STATIC_TOKEN", "private-cli-principal")
        .env("MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY", "false")
        .env("MCP__FILE__SAFE_ROOTS", root)
        .env("MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED", "true")
        .env("MCP__CAPABILITY__KEY_ID", "cli-test-1")
        .env("MCP__CAPABILITY__HMAC_KEY_HEX", KEY)
        .env(
            "MCP__CAPABILITY__SESSION_ID",
            "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
        )
        .env("MCP__CAPABILITY__CREATE_DIRECTORY_TARGET", target);
    command
}

#[cfg(feature = "mcp-runtime")]
fn configured_write_issuer(
    root: &std::path::Path,
    target: &std::path::Path,
    content_sha256: &str,
) -> Command {
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let mut command = isolated_binary();
    command
        .arg("--issue-write-file-grant")
        .env("MCP__AUTH__STATIC_TOKEN", "private-write-cli-principal")
        .env("MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY", "false")
        .env("MCP__FILE__SAFE_ROOTS", root)
        .env("MCP__FILE__WRITE_MUTATION_ENABLED", "true")
        .env("MCP__CAPABILITY__KEY_ID", "write-cli-1")
        .env("MCP__CAPABILITY__HMAC_KEY_HEX", KEY)
        .env(
            "MCP__CAPABILITY__SESSION_ID",
            "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
        )
        .env("MCP__CAPABILITY__WRITE_FILE_TARGET", target)
        .env("MCP__CAPABILITY__WRITE_FILE_CONTENT_SHA256", content_sha256);
    command
}

#[cfg(feature = "android-volume-control")]
fn configured_volume_issuer(stream: &str, level: &str) -> Command {
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let mut command = isolated_binary();
    command
        .arg("--issue-android-volume-grant")
        .env("MCP__AUTH__STATIC_TOKEN", "private-volume-cli-principal")
        .env("MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY", "false")
        .env("MCP__ANDROID__VOLUME_CONTROL_ENABLED", "true")
        .env("MCP__CAPABILITY__KEY_ID", "volume-cli-1")
        .env("MCP__CAPABILITY__HMAC_KEY_HEX", KEY)
        .env(
            "MCP__CAPABILITY__SESSION_ID",
            "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
        )
        .env("MCP__CAPABILITY__VOLUME_STREAM", stream)
        .env("MCP__CAPABILITY__VOLUME_LEVEL", level);
    command
}

#[cfg(feature = "android-volume-control")]
#[test]
fn exact_volume_cli_issuer_outputs_one_private_target_bound_grant() {
    use std::time::{SystemTime, UNIX_EPOCH};

    use termux_mcp_server::{
        android_volume_control::AndroidVolumeStreamName,
        android_volume_grant::{
            AndroidVolumeGrantAuthority, AndroidVolumeGrantError, AndroidVolumeGrantTarget,
            MAX_ANDROID_VOLUME_GRANT_HEADER_BYTES,
        },
    };

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const SESSION: &str = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    let output = configured_volume_issuer("music", "9").output().unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let grant = stdout.trim_end();
    assert!(grant.len() <= MAX_ANDROID_VOLUME_GRANT_HEADER_BYTES);
    let segments = grant.split('.').collect::<Vec<_>>();
    assert_eq!(segments.len(), 4);
    assert_eq!(segments[0], "v1");
    assert_eq!(segments[1], "volume-cli-1");
    assert_eq!(segments[2].len(), 182);
    assert_eq!(&segments[2][128..130], "03");
    assert_eq!(segments[3].len(), 64);
    assert!(segments[2..].iter().all(|segment| segment
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))));
    for private_value in ["private-volume-cli-principal", SESSION, "music"] {
        assert!(!stdout.contains(private_value));
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let target = AndroidVolumeGrantTarget::new(AndroidVolumeStreamName::Music, 9).unwrap();
    let authority = AndroidVolumeGrantAuthority::from_hex_key(
        "volume-cli-1",
        KEY,
        "private-volume-cli-principal",
    )
    .unwrap();
    authority
        .consume_at(Some(grant), SESSION, target, now)
        .unwrap();

    let other_authority = AndroidVolumeGrantAuthority::from_hex_key(
        "volume-cli-1",
        KEY,
        "private-volume-cli-principal",
    )
    .unwrap();
    let other_target = AndroidVolumeGrantTarget::new(AndroidVolumeStreamName::Ring, 9).unwrap();
    assert_eq!(
        other_authority
            .consume_at(Some(grant), SESSION, other_target, now)
            .unwrap_err(),
        AndroidVolumeGrantError::BindingMismatch
    );
}

#[cfg(all(feature = "android-volume-control", unix))]
#[test]
fn volume_cli_issuer_loads_private_literal_config_without_shell_evaluation() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let config_file = root.path().join("runtime.env");
    std::fs::write(
        &config_file,
        format!(
            "MCP__AUTH__STATIC_TOKEN=literal-private-volume-principal\n\
             MCP__ANDROID__VOLUME_CONTROL_ENABLED=true\n\
             MCP__CAPABILITY__KEY_ID=literal-volume-1\n\
             MCP__CAPABILITY__HMAC_KEY_HEX={}\n",
            "0123456789abcdef".repeat(4),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&config_file, std::fs::Permissions::from_mode(0o600)).unwrap();

    let output = isolated_binary()
        .arg("--issue-android-volume-grant")
        .env("MCP__CAPABILITY__CONFIG_FILE", &config_file)
        .env(
            "MCP__CAPABILITY__SESSION_ID",
            "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
        )
        .env("MCP__CAPABILITY__VOLUME_STREAM", "notification")
        .env("MCP__CAPABILITY__VOLUME_LEVEL", "6")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let grant = String::from_utf8(output.stdout).unwrap();
    assert!(grant.starts_with("v1.literal-volume-1."));
    assert_eq!(grant.lines().count(), 1);
    assert!(!grant.contains("literal-private-volume-principal"));
    assert!(!grant.contains("notification"));
}

#[cfg(feature = "android-volume-control")]
#[test]
fn volume_cli_issuer_fails_closed_and_never_reflects_private_inputs() {
    const SESSION: &str = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    const PRINCIPAL: &str = "private-volume-cli-principal";
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let disabled = isolated_binary()
        .arg("--issue-android-volume-grant")
        .env("MCP__AUTH__STATIC_TOKEN", PRINCIPAL)
        .env("MCP__CAPABILITY__SESSION_ID", SESSION)
        .env("MCP__CAPABILITY__VOLUME_STREAM", "music")
        .env("MCP__CAPABILITY__VOLUME_LEVEL", "9")
        .output()
        .unwrap();
    assert!(!disabled.status.success());
    assert!(disabled.stdout.is_empty());
    let disabled_stderr = String::from_utf8(disabled.stderr).unwrap();
    assert!(disabled_stderr.contains("volume control gate is disabled"));

    let invalid_stream = configured_volume_issuer("private-invalid-stream", "9")
        .output()
        .unwrap();
    assert!(!invalid_stream.status.success());
    assert!(invalid_stream.stdout.is_empty());
    let invalid_stream_stderr = String::from_utf8(invalid_stream.stderr).unwrap();
    assert!(invalid_stream_stderr.contains("grant stream validation failed"));

    let invalid_level = configured_volume_issuer("music", "-1").output().unwrap();
    assert!(!invalid_level.status.success());
    assert!(invalid_level.stdout.is_empty());
    let invalid_level_stderr = String::from_utf8(invalid_level.stderr).unwrap();
    assert!(invalid_level_stderr.contains("grant target validation failed"));

    for stderr in [disabled_stderr, invalid_stream_stderr, invalid_level_stderr] {
        for private_value in [PRINCIPAL, SESSION, KEY, "private-invalid-stream"] {
            assert!(!stderr.contains(private_value));
        }
    }
}

#[cfg(feature = "mcp-runtime")]
#[test]
fn exact_cli_issuer_outputs_one_private_target_bound_grant() {
    use std::time::{SystemTime, UNIX_EPOCH};

    use termux_mcp_server::{
        create_directory_grant::{
            CreateDirectoryGrantAuthority, CreateDirectoryGrantError,
            MAX_CREATE_DIRECTORY_GRANT_HEADER_BYTES,
        },
        tools::FileSystemTools,
    };

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const SESSION: &str = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("private-cli-target");
    let output = configured_issuer(root.path(), &target).output().unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let grant = stdout.trim_end();
    assert!(grant.len() <= MAX_CREATE_DIRECTORY_GRANT_HEADER_BYTES);
    let segments = grant.split('.').collect::<Vec<_>>();
    assert_eq!(segments.len(), 4);
    assert_eq!(segments[0], "v1");
    assert_eq!(segments[1], "cli-test-1");
    assert_eq!(segments[2].len(), 260);
    assert_eq!(&segments[2][128..130], "01");
    assert_eq!(segments[3].len(), 64);
    assert!(segments[2..].iter().all(|segment| segment
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))));
    assert!(!stdout.contains("private-cli-principal"));
    assert!(!stdout.contains("private-cli-target"));
    assert!(!target.exists());

    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let binding = tools
        .create_directory_grant_target(target.to_string_lossy().as_ref())
        .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let authority =
        CreateDirectoryGrantAuthority::from_hex_key("cli-test-1", KEY, "private-cli-principal")
            .unwrap();
    authority
        .consume_at(Some(grant), SESSION, &binding, now)
        .unwrap();
    let other_target = root.path().join("private-cli-other");
    let other_binding = tools
        .create_directory_grant_target(other_target.to_string_lossy().as_ref())
        .unwrap();
    let other_authority =
        CreateDirectoryGrantAuthority::from_hex_key("cli-test-1", KEY, "private-cli-principal")
            .unwrap();
    assert_eq!(
        other_authority
            .consume_at(Some(grant), SESSION, &other_binding, now)
            .unwrap_err(),
        CreateDirectoryGrantError::BindingMismatch
    );
}

#[cfg(feature = "mcp-runtime")]
#[test]
fn exact_write_cli_issuer_outputs_one_private_operation_bound_grant() {
    use std::time::{SystemTime, UNIX_EPOCH};

    use sha2::{Digest, Sha256};
    use termux_mcp_server::{
        tools::FileSystemTools,
        write_file_grant::{
            content_sha256, WriteFileGrantAuthority, WriteFileGrantError,
            MAX_WRITE_FILE_GRANT_HEADER_BYTES,
        },
    };

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const SESSION: &str = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    const CONTENT: &str = "private write cli content";
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("private-write-cli-target.txt");
    let digest = Sha256::digest(CONTENT.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let output = configured_write_issuer(root.path(), &target, &digest)
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let grant = stdout.trim_end();
    assert!(grant.len() <= MAX_WRITE_FILE_GRANT_HEADER_BYTES);
    let segments = grant.split('.').collect::<Vec<_>>();
    assert_eq!(segments.len(), 4);
    assert_eq!(segments[0], "v1");
    assert_eq!(segments[1], "write-cli-1");
    assert_eq!(segments[2].len(), 260);
    assert_eq!(&segments[2][128..130], "02");
    assert_eq!(segments[3].len(), 64);
    assert!(segments[2..].iter().all(|segment| segment
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))));
    for private in [
        "private-write-cli-principal",
        "private-write-cli-target",
        CONTENT,
        digest.as_str(),
        SESSION,
    ] {
        assert!(!stdout.contains(private));
    }
    assert!(!target.exists());

    let tools = FileSystemTools::new(vec![root.path().to_path_buf()]);
    let binding = tools
        .write_file_grant_target(
            target.to_string_lossy().as_ref(),
            content_sha256(CONTENT.as_bytes()),
        )
        .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let authority =
        WriteFileGrantAuthority::from_hex_key("write-cli-1", KEY, "private-write-cli-principal")
            .unwrap();
    authority
        .consume_at(Some(grant), SESSION, &binding, now)
        .unwrap();

    let other_binding = tools
        .write_file_grant_target(
            target.to_string_lossy().as_ref(),
            content_sha256(b"different content"),
        )
        .unwrap();
    let other_authority =
        WriteFileGrantAuthority::from_hex_key("write-cli-1", KEY, "private-write-cli-principal")
            .unwrap();
    assert_eq!(
        other_authority
            .consume_at(Some(grant), SESSION, &other_binding, now)
            .unwrap_err(),
        WriteFileGrantError::BindingMismatch
    );
}

#[cfg(feature = "mcp-runtime")]
#[test]
fn write_cli_issuer_fails_closed_without_gate_or_for_invalid_private_inputs() {
    use sha2::{Digest, Sha256};

    const SESSION: &str = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("private-write-denied-target.txt");
    let digest = Sha256::digest(b"content")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    let disabled = isolated_binary()
        .arg("--issue-write-file-grant")
        .env("MCP__AUTH__STATIC_TOKEN", "private-write-cli-principal")
        .env("MCP__FILE__SAFE_ROOTS", root.path())
        .env("MCP__CAPABILITY__SESSION_ID", SESSION)
        .env("MCP__CAPABILITY__WRITE_FILE_TARGET", &target)
        .env("MCP__CAPABILITY__WRITE_FILE_CONTENT_SHA256", &digest)
        .output()
        .unwrap();
    assert!(!disabled.status.success());
    assert!(disabled.stdout.is_empty());
    let disabled_stderr = String::from_utf8(disabled.stderr).unwrap();
    assert!(disabled_stderr.contains("write_file mutation gate is disabled"));

    let invalid_digest = configured_write_issuer(root.path(), &target, "PRIVATE-DIGEST")
        .output()
        .unwrap();
    assert!(!invalid_digest.status.success());
    assert!(invalid_digest.stdout.is_empty());
    let invalid_stderr = String::from_utf8(invalid_digest.stderr).unwrap();
    assert!(invalid_stderr.contains("content digest validation failed"));

    for stderr in [disabled_stderr, invalid_stderr] {
        for private in [
            "private-write-cli-principal",
            "private-write-denied-target",
            "PRIVATE-DIGEST",
            SESSION,
            digest.as_str(),
        ] {
            assert!(!stderr.contains(private));
        }
    }
}

#[cfg(all(feature = "mcp-runtime", unix))]
#[test]
fn cli_issuer_loads_the_private_deployed_literal_config_without_shell_evaluation() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("literal-config-target");
    let config_file = root.path().join("runtime.env");
    std::fs::write(
        &config_file,
        format!(
            "MCP__AUTH__STATIC_TOKEN=literal-private-principal\n\
             MCP__FILE__SAFE_ROOTS={}\n\
             MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true\n\
             MCP__CAPABILITY__KEY_ID=literal-1\n\
             MCP__CAPABILITY__HMAC_KEY_HEX={}\n",
            root.path().display(),
            "0123456789abcdef".repeat(4),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&config_file, std::fs::Permissions::from_mode(0o600)).unwrap();

    let output = isolated_binary()
        .arg("--issue-create-directory-grant")
        .env("MCP__CAPABILITY__CONFIG_FILE", &config_file)
        .env(
            "MCP__CAPABILITY__SESSION_ID",
            "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
        )
        .env("MCP__CAPABILITY__CREATE_DIRECTORY_TARGET", &target)
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let grant = String::from_utf8(output.stdout).unwrap();
    assert!(grant.starts_with("v1.literal-1."));
    assert_eq!(grant.lines().count(), 1);
    assert!(!grant.contains("literal-private-principal"));
    assert!(!grant.contains("literal-config-target"));
    assert!(!target.exists());
}

#[cfg(feature = "mcp-runtime")]
#[test]
fn cli_issuer_fails_closed_without_gate_or_for_invalid_target_without_reflection() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("private-cli-denied-target");

    let disabled = isolated_binary()
        .arg("--issue-create-directory-grant")
        .env("MCP__AUTH__STATIC_TOKEN", "private-cli-principal")
        .env("MCP__FILE__SAFE_ROOTS", root.path())
        .env(
            "MCP__CAPABILITY__SESSION_ID",
            "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
        )
        .env("MCP__CAPABILITY__CREATE_DIRECTORY_TARGET", &target)
        .output()
        .unwrap();
    assert!(!disabled.status.success());
    assert!(disabled.stdout.is_empty());
    let disabled_stderr = String::from_utf8(disabled.stderr).unwrap();
    assert!(disabled_stderr.contains("mutation gate is disabled"));

    std::fs::create_dir(&target).unwrap();
    let invalid = configured_issuer(root.path(), &target).output().unwrap();
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
    let invalid_stderr = String::from_utf8(invalid.stderr).unwrap();
    assert!(invalid_stderr.contains("grant target validation failed"));

    for stderr in [disabled_stderr, invalid_stderr] {
        assert!(!stderr.contains("private-cli-principal"));
        assert!(!stderr.contains("private-cli-denied-target"));
        assert!(!stderr.contains("0194f9f9"));
        assert!(!stderr.contains("0123456789abcdef"));
    }
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
