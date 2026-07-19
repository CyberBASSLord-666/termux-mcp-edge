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

#[cfg(any(feature = "mcp-runtime", feature = "android-volume-control"))]
fn assert_signed_capability_byte(payload_hex: &str, byte_offset: usize, expected: &str) {
    let hex_offset = byte_offset.checked_mul(2).unwrap();
    assert_eq!(&payload_hex[hex_offset..hex_offset + 2], expected);
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
    assert!(stdout.contains("termux-mcp-server --issue-copy-file-grant"));
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
    for command in [
        "--issue-create-directory-grant",
        "--issue-copy-file-grant",
        "--issue-write-file-grant",
    ] {
        let output = isolated_binary().arg(command).output().unwrap();

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
fn configured_copy_issuer(
    root: &std::path::Path,
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Command {
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let mut command = isolated_binary();
    command
        .arg("--issue-copy-file-grant")
        .env("MCP__AUTH__STATIC_TOKEN", "private-copy-cli-principal")
        .env("MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY", "false")
        .env("MCP__FILE__SAFE_ROOTS", root)
        .env("MCP__FILE__COPY_FILE_MUTATION_ENABLED", "true")
        .env("MCP__CAPABILITY__KEY_ID", "copy-cli-test-1")
        .env("MCP__CAPABILITY__HMAC_KEY_HEX", KEY)
        .env(
            "MCP__CAPABILITY__SESSION_ID",
            "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
        )
        .env("MCP__CAPABILITY__COPY_FILE_SOURCE", source)
        .env("MCP__CAPABILITY__COPY_FILE_DESTINATION", destination);
    command
}

#[cfg(feature = "mcp-runtime")]
fn configured_write_issuer(
    root: &std::path::Path,
    target: &std::path::Path,
    content_file: &std::path::Path,
    disposition: &str,
) -> Command {
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let mut command = isolated_binary();
    command
        .arg("--issue-write-file-grant")
        .env("MCP__AUTH__STATIC_TOKEN", "private-write-cli-principal")
        .env("MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY", "false")
        .env("MCP__FILE__SAFE_ROOTS", root)
        .env("MCP__FILE__WRITE_MUTATION_ENABLED", "true")
        .env("MCP__CAPABILITY__KEY_ID", "write-cli-test-1")
        .env("MCP__CAPABILITY__HMAC_KEY_HEX", KEY)
        .env(
            "MCP__CAPABILITY__SESSION_ID",
            "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
        )
        .env("MCP__CAPABILITY__WRITE_FILE_TARGET", target)
        .env("MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE", content_file)
        .env("MCP__CAPABILITY__WRITE_FILE_DISPOSITION", disposition);
    command
}

#[cfg(feature = "mcp-runtime")]
#[test]
fn exact_copy_cli_issuer_outputs_one_private_source_destination_bound_grant() {
    use termux_mcp_server::{
        copy_file_grant::{
            CopyFileGrantAuthority, CopyFileGrantError, MAX_COPY_FILE_GRANT_HEADER_BYTES,
        },
        tools::FileSystemTools,
    };

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const SESSION: &str = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("private-copy-cli-source.bin");
    let destination = root.path().join("private-copy-cli-destination.bin");
    let other_destination = root.path().join("private-copy-cli-other.bin");
    std::fs::write(&source, b"private-copy\0\xff-content").unwrap();

    let output = configured_copy_issuer(root.path(), &source, &destination)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let grant = stdout.trim_end();
    assert!(grant.len() <= MAX_COPY_FILE_GRANT_HEADER_BYTES);
    let segments = grant.split('.').collect::<Vec<_>>();
    assert_eq!(segments.len(), 4);
    assert_eq!(segments[0], "v1");
    assert_eq!(segments[1], "copy-cli-test-1");
    assert_eq!(segments[2].len(), 130);
    assert_signed_capability_byte(segments[2], 16, "04");
    assert_eq!(segments[3].len(), 64);
    assert!(segments[2..].iter().all(|segment| segment
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))));
    for private in [
        "private-copy-cli-principal",
        "private-copy-cli-source",
        "private-copy-cli-destination",
        "private-copy",
        SESSION,
    ] {
        assert!(!stdout.contains(private));
    }
    assert!(!destination.exists());

    let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()]).unwrap();
    let exact_target = tools
        .copy_file_grant_target(
            source.to_string_lossy().as_ref(),
            destination.to_string_lossy().as_ref(),
        )
        .unwrap();
    let authority =
        CopyFileGrantAuthority::from_hex_key("copy-cli-test-1", KEY, "private-copy-cli-principal")
            .unwrap();
    authority
        .consume(Some(grant), SESSION, &exact_target)
        .unwrap();

    let other_target = tools
        .copy_file_grant_target(
            source.to_string_lossy().as_ref(),
            other_destination.to_string_lossy().as_ref(),
        )
        .unwrap();
    assert_eq!(
        authority
            .consume(Some(grant), SESSION, &other_target)
            .unwrap_err(),
        CopyFileGrantError::BindingMismatch
    );
}

#[cfg(feature = "mcp-runtime")]
#[test]
fn copy_cli_issuer_fails_closed_without_gate_or_for_invalid_private_inputs() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("private-copy-cli-denied-source");
    let destination = root.path().join("private-copy-cli-denied-destination");
    std::fs::write(&source, "private-copy-denied-content").unwrap();

    let disabled = isolated_binary()
        .arg("--issue-copy-file-grant")
        .env("MCP__AUTH__STATIC_TOKEN", "private-copy-cli-principal")
        .env("MCP__FILE__SAFE_ROOTS", root.path())
        .env(
            "MCP__CAPABILITY__SESSION_ID",
            "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
        )
        .env("MCP__CAPABILITY__COPY_FILE_SOURCE", &source)
        .env("MCP__CAPABILITY__COPY_FILE_DESTINATION", &destination)
        .output()
        .unwrap();
    assert!(!disabled.status.success());
    assert!(disabled.stdout.is_empty());
    let disabled_stderr = String::from_utf8(disabled.stderr).unwrap();
    assert!(disabled_stderr.contains("copy_file mutation gate is disabled"));

    let missing_source = root.path().join("private-copy-cli-missing-source");
    let invalid = configured_copy_issuer(root.path(), &missing_source, &destination)
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
    let invalid_stderr = String::from_utf8(invalid.stderr).unwrap();
    assert!(invalid_stderr.contains("copy_file grant target validation failed"));

    for stderr in [disabled_stderr, invalid_stderr] {
        for private in [
            "private-copy-cli-principal",
            "private-copy-cli-denied",
            "private-copy-denied-content",
            "private-copy-cli-missing",
            "0194f9f9",
            "0123456789abcdef",
        ] {
            assert!(!stderr.contains(private));
        }
    }
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
    assert_signed_capability_byte(segments[2], 64, "03");
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
    assert_signed_capability_byte(segments[2], 64, "01");
    assert_eq!(segments[3].len(), 64);
    assert!(segments[2..].iter().all(|segment| segment
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))));
    assert!(!stdout.contains("private-cli-principal"));
    assert!(!stdout.contains("private-cli-target"));
    assert!(!target.exists());

    let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");
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

#[cfg(all(feature = "mcp-runtime", unix))]
#[test]
fn exact_write_cli_issuer_outputs_one_private_content_and_target_bound_grant() {
    use std::os::unix::fs::PermissionsExt;

    use termux_mcp_server::{
        tools::FileSystemTools,
        write_file_grant::{
            WriteFileDisposition, WriteFileGrantAuthority, WriteFileGrantError,
            MAX_WRITE_FILE_GRANT_HEADER_BYTES,
        },
    };

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const SESSION: &str = "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("private-write-cli-target.txt");
    let other_target = root.path().join("private-write-cli-other.txt");
    let content_file = root.path().join("private-write-cli-content.txt");
    let content = b"private-write-cli-content";
    std::fs::write(&content_file, content).unwrap();
    std::fs::set_permissions(&content_file, std::fs::Permissions::from_mode(0o600)).unwrap();

    let output = configured_write_issuer(root.path(), &target, &content_file, "create")
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
    assert_eq!(segments[1], "write-cli-test-1");
    assert_eq!(segments[2].len(), 130);
    assert_signed_capability_byte(segments[2], 16, "02");
    assert_eq!(segments[3].len(), 64);
    assert!(segments[2..].iter().all(|segment| segment
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))));
    for private in [
        "private-write-cli-principal",
        "private-write-cli-target",
        "private-write-cli-content",
        SESSION,
    ] {
        assert!(!stdout.contains(private));
    }
    assert!(!target.exists());

    let tools = FileSystemTools::try_new(vec![root.path().to_path_buf()])
        .expect("test safe root must validate");
    let binding = tools
        .write_file_grant_target(
            target.to_string_lossy().as_ref(),
            content,
            WriteFileDisposition::Create,
        )
        .unwrap();
    let authority = WriteFileGrantAuthority::from_hex_key(
        "write-cli-test-1",
        KEY,
        "private-write-cli-principal",
    )
    .unwrap();
    authority.consume(Some(grant), SESSION, &binding).unwrap();

    let other_binding = tools
        .write_file_grant_target(
            other_target.to_string_lossy().as_ref(),
            content,
            WriteFileDisposition::Create,
        )
        .unwrap();
    let other_authority = WriteFileGrantAuthority::from_hex_key(
        "write-cli-test-1",
        KEY,
        "private-write-cli-principal",
    )
    .unwrap();
    assert_eq!(
        other_authority
            .consume(Some(grant), SESSION, &other_binding)
            .unwrap_err(),
        WriteFileGrantError::BindingMismatch
    );
}

#[cfg(all(feature = "mcp-runtime", unix))]
#[test]
fn write_cli_issuer_fails_closed_without_gate_and_for_private_invalid_inputs() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("private-write-cli-denied.txt");
    let content_file = root.path().join("private-write-cli-denied-content.txt");
    std::fs::write(&content_file, "private-write-denied-content").unwrap();
    std::fs::set_permissions(&content_file, std::fs::Permissions::from_mode(0o600)).unwrap();

    let disabled = isolated_binary()
        .arg("--issue-write-file-grant")
        .env("MCP__AUTH__STATIC_TOKEN", "private-write-cli-principal")
        .env("MCP__FILE__SAFE_ROOTS", root.path())
        .env(
            "MCP__CAPABILITY__SESSION_ID",
            "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
        )
        .env("MCP__CAPABILITY__WRITE_FILE_TARGET", &target)
        .env("MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE", &content_file)
        .env("MCP__CAPABILITY__WRITE_FILE_DISPOSITION", "create")
        .output()
        .unwrap();
    assert!(!disabled.status.success());
    assert!(disabled.stdout.is_empty());
    let disabled_stderr = String::from_utf8(disabled.stderr).unwrap();
    assert!(disabled_stderr.contains("write_file mutation gate is disabled"));

    let invalid_disposition =
        configured_write_issuer(root.path(), &target, &content_file, "private-invalid")
            .output()
            .unwrap();
    assert!(!invalid_disposition.status.success());
    assert!(invalid_disposition.stdout.is_empty());
    let invalid_disposition_stderr = String::from_utf8(invalid_disposition.stderr).unwrap();
    assert!(invalid_disposition_stderr.contains("grant disposition validation failed"));

    std::fs::set_permissions(&content_file, std::fs::Permissions::from_mode(0o644)).unwrap();
    let insecure_file = configured_write_issuer(root.path(), &target, &content_file, "create")
        .output()
        .unwrap();
    assert!(!insecure_file.status.success());
    assert!(insecure_file.stdout.is_empty());
    let insecure_file_stderr = String::from_utf8(insecure_file.stderr).unwrap();
    assert!(insecure_file_stderr.contains("inaccessible to group/other"));

    std::fs::set_permissions(&content_file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let target_alias =
        configured_write_issuer(root.path(), &content_file, &content_file, "replace")
            .output()
            .unwrap();
    assert!(!target_alias.status.success());
    assert!(target_alias.stdout.is_empty());
    let target_alias_stderr = String::from_utf8(target_alias.stderr).unwrap();
    assert!(target_alias_stderr.contains("must not alias the replacement target"));

    let config_alias = root.path().join("private-write-cli-runtime.env");
    std::fs::write(
        &config_alias,
        format!(
            "MCP__AUTH__STATIC_TOKEN=private-write-cli-principal\n\
             MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=false\n\
             MCP__FILE__SAFE_ROOTS={}\n\
             MCP__FILE__WRITE_MUTATION_ENABLED=true\n\
             MCP__CAPABILITY__KEY_ID=write-cli-test-1\n\
             MCP__CAPABILITY__HMAC_KEY_HEX=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n",
            root.path().display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&config_alias, std::fs::Permissions::from_mode(0o600)).unwrap();
    let config_alias_output = isolated_binary()
        .arg("--issue-write-file-grant")
        .env("MCP__CAPABILITY__CONFIG_FILE", &config_alias)
        .env(
            "MCP__CAPABILITY__SESSION_ID",
            "0194f9f9-bbbb-7ccc-8ddd-eeeeeeeeeeee",
        )
        .env("MCP__CAPABILITY__WRITE_FILE_TARGET", &target)
        .env("MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE", &config_alias)
        .env("MCP__CAPABILITY__WRITE_FILE_DISPOSITION", "create")
        .output()
        .unwrap();
    assert!(!config_alias_output.status.success());
    assert!(config_alias_output.stdout.is_empty());
    let config_alias_stderr = String::from_utf8(config_alias_output.stderr).unwrap();
    assert!(config_alias_stderr.contains("must not alias the runtime configuration file"));

    for stderr in [
        disabled_stderr,
        invalid_disposition_stderr,
        insecure_file_stderr,
        target_alias_stderr,
        config_alias_stderr,
    ] {
        for private in [
            "private-write-cli-principal",
            "private-write-cli-denied",
            "private-write-denied-content",
            "private-invalid",
            "0194f9f9",
            "0123456789abcdef",
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
