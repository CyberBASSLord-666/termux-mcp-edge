#![cfg(feature = "command-execution")]

use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{Mutex, MutexGuard, OnceLock},
    thread,
    time::{Duration, Instant},
};

use rustix::io::Errno;
use rustix::process::{kill_process_group, test_kill_process, Pid, Signal};

const NESTED_CARGO_TIMEOUT: Duration = Duration::from_secs(120);
const NESTED_CARGO_SUITE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(20);
const MAX_RETAINED_CAPTURE_BYTES: u64 = 1024 * 1024;

static NESTED_CARGO_TEST_LOCK: Mutex<()> = Mutex::new(());
static NESTED_CARGO_SUITE_DEADLINE: OnceLock<Instant> = OnceLock::new();

struct ProcessGroupGuard {
    process_group: Pid,
    armed: bool,
}

impl ProcessGroupGuard {
    fn new(process_group: Pid) -> Self {
        Self {
            process_group,
            armed: true,
        }
    }

    fn terminate(&self) -> rustix::io::Result<()> {
        kill_process_group(self.process_group, Signal::KILL)
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.terminate();
        }
    }
}

fn nested_cargo_test_guard() -> MutexGuard<'static, ()> {
    NESTED_CARGO_TEST_LOCK
        .lock()
        .expect("a preceding nested Cargo public-API probe failed")
}

fn remaining_nested_cargo_timeout() -> Duration {
    let deadline = NESTED_CARGO_SUITE_DEADLINE
        .get_or_init(|| Instant::now() + NESTED_CARGO_SUITE_TIMEOUT);
    let remaining = deadline.saturating_duration_since(Instant::now());
    assert!(
        !remaining.is_zero(),
        "nested Cargo public-API probes exceeded their aggregate timeout"
    );
    remaining.min(NESTED_CARGO_TIMEOUT)
}

fn capture_sizes_within_limit(
    label: &str,
    stdout: &fs::File,
    stderr: &fs::File,
) -> Result<(), String> {
    for (stream, file) in [("stdout", stdout), ("stderr", stderr)] {
        let bytes = file
            .metadata()
            .map_err(|error| format!("{label} {stream} capture metadata failed: {error}"))?
            .len();
        if bytes > MAX_RETAINED_CAPTURE_BYTES {
            return Err(format!(
                "{label} {stream} exceeded the {MAX_RETAINED_CAPTURE_BYTES}-byte diagnostic limit"
            ));
        }
    }
    Ok(())
}

fn capture_file(label: &str, file: &mut fs::File) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("{label} capture seek failed: {error}"))?;
    let retained_limit = MAX_RETAINED_CAPTURE_BYTES + 1;
    let mut output = Vec::with_capacity(
        usize::try_from(retained_limit).expect("capture bound must fit usize"),
    );
    file.take(retained_limit)
        .read_to_end(&mut output)
        .map_err(|error| format!("{label} capture failed: {error}"))?;
    if output.len() as u64 > MAX_RETAINED_CAPTURE_BYTES {
        return Err(format!(
            "{label} capture exceeded the {MAX_RETAINED_CAPTURE_BYTES}-byte diagnostic limit"
        ));
    }
    Ok(output)
}

fn checked_group_termination(
    label: &str,
    process_group: &ProcessGroupGuard,
) -> Result<(), String> {
    match process_group.terminate() {
        Ok(()) | Err(Errno::SRCH) => Ok(()),
        Err(error) => Err(format!("{label} process-group termination failed: {error}")),
    }
}

fn read_published_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(value) = fs::read_to_string(path)
            && let Ok(pid) = value.trim().parse()
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "process fixture did not publish its descendant identifier"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn process_exists(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
        return false;
    };
    match test_kill_process(pid) {
        Ok(()) => true,
        Err(error) => error != Errno::SRCH,
    }
}

fn assert_process_gone(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while process_exists(pid) {
        assert!(
            Instant::now() < deadline,
            "command-runner descendant {pid} survived cleanup"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn run_bounded_command(
    label: &str,
    mut command: Command,
    timeout: Duration,
) -> Result<Output, String> {
    let mut stdout = tempfile::tempfile()
        .map_err(|error| format!("{label} stdout capture setup failed: {error}"))?;
    let mut stderr = tempfile::tempfile()
        .map_err(|error| format!("{label} stderr capture setup failed: {error}"))?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            stdout
                .try_clone()
                .map_err(|error| format!("{label} stdout capture clone failed: {error}"))?,
        ))
        .stderr(Stdio::from(
            stderr
                .try_clone()
                .map_err(|error| format!("{label} stderr capture clone failed: {error}"))?,
        ))
        .process_group(0);

    let mut child = command
        .spawn()
        .map_err(|error| format!("{label} spawn failed: {error}"))?;
    let Some(process_group) = i32::try_from(child.id())
        .ok()
        .and_then(Pid::from_raw)
    else {
        let _ = child.kill();
        child
            .wait()
            .map_err(|error| format!("{label} invalid-process-id reap failed: {error}"))?;
        return Err(format!("{label} returned an invalid process id"));
    };
    let mut process_group = ProcessGroupGuard::new(process_group);
    let deadline = Instant::now() + timeout;

    let status = loop {
        if let Err(error) = capture_sizes_within_limit(label, &stdout, &stderr) {
            let group_cleanup = checked_group_termination(label, &process_group);
            let _ = child.kill();
            child
                .wait()
                .map_err(|wait_error| format!("{label} output-limit reap failed: {wait_error}"))?;
            group_cleanup?;
            process_group.disarm();
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                checked_group_termination(label, &process_group)?;
                break status;
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(CHILD_POLL_INTERVAL),
                );
            }
            Ok(None) => {
                let group_cleanup = checked_group_termination(label, &process_group);
                let _ = child.kill();
                let status = child
                    .wait()
                    .map_err(|error| format!("{label} reap after timeout failed: {error}"))?;
                group_cleanup?;
                process_group.disarm();
                let stdout = capture_file("stdout", &mut stdout)?;
                let stderr = capture_file("stderr", &mut stderr)?;
                return Err(format!(
                    "{label} timed out after {} seconds (status: {status}; stdout: {}; stderr: {})",
                    timeout.as_secs_f64(),
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr),
                ));
            }
            Err(error) => {
                let group_cleanup = checked_group_termination(label, &process_group);
                let _ = child.kill();
                child.wait().map_err(|wait_error| {
                    format!("{label} reap after status-check failure failed: {wait_error}")
                })?;
                group_cleanup?;
                process_group.disarm();
                return Err(format!("{label} status check failed: {error}"));
            }
        }
    };

    process_group.disarm();
    Ok(Output {
        status,
        stdout: capture_file("stdout", &mut stdout)?,
        stderr: capture_file("stderr", &mut stderr)?,
    })
}

fn write_probe_source(root: &Path, source: &str) {
    fs::write(root.join("src/main.rs"), source).unwrap();
}

fn run_cargo(
    label: &str,
    root: &Path,
    target: &Path,
    arguments: &[&str],
) -> std::process::Output {
    // These temporary downstream-consumer probes intentionally have no committed
    // lockfile. They remain offline and run beneath the root suite's locked Cargo
    // invocation; no probe output is packaged or used as a release artifact.
    let mut command = Command::new(env!("CARGO"));
    command
        .args(arguments)
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_TERM_COLOR", "never");
    run_bounded_command(label, command, remaining_nested_cargo_timeout())
        .unwrap_or_else(|error| panic!("{error}"))
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
    let _nested_cargo_guard = nested_cargo_test_guard();
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
        let label = format!("nested Cargo authority probe ({name})");
        let output = run_cargo(
            &label,
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
    let _nested_cargo_guard = nested_cargo_test_guard();
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
        let label = format!("nested Cargo legacy-router probe ({symbol})");
        let output = run_cargo(
            &label,
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
    let _nested_cargo_guard = nested_cargo_test_guard();
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
        "nested Cargo selected-workspace valid probe",
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
        "nested Cargo selected-workspace rejection probe",
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

#[test]
fn bounded_command_runner_kills_timed_out_process_group() {
    let fixture = tempfile::tempdir().unwrap();
    let timeout_pid = fixture.path().join("timeout-descendant-pid");
    let timeout_pid_text = timeout_pid.to_string_lossy();
    assert!(!timeout_pid_text.contains('\''));

    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(format!(
        "(trap '' HUP TERM; exec /bin/sleep 30) &\n\
         printf '%s\\n' \"$!\" >'{timeout_pid_text}'\n\
         wait"
    ));

    let started_at = Instant::now();
    let worker = thread::spawn(move || {
        run_bounded_command(
            "blocking process-group fixture",
            command,
            Duration::from_secs(5),
        )
    });
    let timeout_pid = read_published_pid(&timeout_pid);
    let error = worker
        .join()
        .expect("blocking process-group fixture thread panicked")
        .unwrap_err();
    assert!(error.contains("timed out after"));
    assert!(
        started_at.elapsed() < Duration::from_secs(8),
        "bounded command runner exceeded its timeout allowance"
    );
    assert_process_gone(timeout_pid);

    let exit_pid = fixture.path().join("normal-exit-descendant-pid");
    let exit_pid_text = exit_pid.to_string_lossy();
    assert!(!exit_pid_text.contains('\''));
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(format!(
        "(trap '' HUP TERM; exec /bin/sleep 30) &\n\
         printf '%s\\n' \"$!\" >'{exit_pid_text}'\n\
         exit 0"
    ));
    let started_at = Instant::now();
    let output = run_bounded_command(
        "normal-exit process-group fixture",
        command,
        Duration::from_secs(2),
    )
    .unwrap();
    assert!(output.status.success());
    assert!(
        started_at.elapsed() < Duration::from_secs(5),
        "normal child exit left the command runner blocked on a descendant"
    );
    let exit_pid = read_published_pid(&exit_pid);
    assert_process_gone(exit_pid);

    let overflow_pid = fixture.path().join("overflow-descendant-pid");
    let overflow_pid_text = overflow_pid.to_string_lossy();
    assert!(!overflow_pid_text.contains('\''));
    let overflow_start = fixture.path().join("start-overflow");
    let overflow_start_text = overflow_start.to_string_lossy();
    assert!(!overflow_start_text.contains('\''));
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(format!(
        "(trap '' HUP TERM\n\
         while [ ! -f '{overflow_start_text}' ]; do /bin/sleep 0.01; done\n\
         exec /usr/bin/yes) &\n\
         printf '%s\\n' \"$!\" >'{overflow_pid_text}'\n\
         wait"
    ));
    let started_at = Instant::now();
    let worker = thread::spawn(move || {
        run_bounded_command(
            "overflowing process-group fixture",
            command,
            Duration::from_secs(10),
        )
    });
    let overflow_pid = read_published_pid(&overflow_pid);
    fs::write(&overflow_start, b"start").unwrap();
    let error = worker
        .join()
        .expect("overflowing process-group fixture thread panicked")
        .unwrap_err();
    assert!(error.contains("exceeded the"));
    assert!(
        started_at.elapsed() < Duration::from_secs(5),
        "output overflow did not fail before the command deadline"
    );
    assert_process_gone(overflow_pid);
}
