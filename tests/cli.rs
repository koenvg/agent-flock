#![cfg(unix)]

use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "agent-flock-test-{}-{label}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("test directory should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for {path:?}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn agent_flock() -> Command {
    Command::new(env!("CARGO_BIN_EXE_agent-flock"))
}

static DEFAULT_LOCK_DIRECTORY_MUTEX: Mutex<()> = Mutex::new(());

fn lock_default_directory_tests() -> std::sync::MutexGuard<'static, ()> {
    DEFAULT_LOCK_DIRECTORY_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct DefaultLockDirectoryFixture {
    path: PathBuf,
    backup: Option<PathBuf>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl DefaultLockDirectoryFixture {
    fn new() -> Self {
        let lock = lock_default_directory_tests();
        let effective_user = unsafe { libc::geteuid() };
        let path = PathBuf::from("/tmp").join(format!("agent-flock-{effective_user}"));
        let backup = match fs::symlink_metadata(&path) {
            Ok(_) => {
                let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let backup = PathBuf::from("/tmp").join(format!(
                    "agent-flock-test-backup-{}-{}-{sequence}",
                    std::process::id(),
                    effective_user
                ));
                fs::rename(&path, &backup).expect("existing default lock path should be moved");
                Some(backup)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("default lock path should be inspectable: {error}"),
        };

        Self {
            path,
            backup,
            _lock: lock,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DefaultLockDirectoryFixture {
    fn drop(&mut self) {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.is_dir() => {
                let _ = fs::remove_dir_all(&self.path);
            }
            Ok(_) => {
                let _ = fs::remove_file(&self.path);
            }
            Err(_) => {}
        }

        if let Some(backup) = &self.backup {
            fs::rename(backup, &self.path).expect("default lock path should be restored");
        }
    }
}

fn run_with_default_lock_directory() -> std::process::Output {
    agent_flock()
        .env_remove("AGENT_FLOCK_LOCK_DIR")
        .args(["--", "sh", "-c", "exit 99"])
        .output()
        .expect("agent-flock should start")
}

fn assert_lock_directory_failure(output: std::process::Output, expected: &str) {
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(
        stderr.contains(expected),
        "expected stderr to contain {expected:?}, got {stderr:?}"
    );
}

#[test]
fn rejects_an_empty_configured_lock_directory() {
    let output = agent_flock()
        .env("AGENT_FLOCK_LOCK_DIR", "")
        .args(["--", "sh", "-c", "exit 99"])
        .output()
        .expect("agent-flock should start");

    assert_lock_directory_failure(output, "AGENT_FLOCK_LOCK_DIR must not be empty");
}

#[test]
fn rejects_a_symlink_at_the_default_lock_path() {
    let fixture = DefaultLockDirectoryFixture::new();
    let target = TestDirectory::new("default-lock-symlink-target");
    symlink(target.path(), fixture.path()).expect("default lock path symlink should be created");

    let output = run_with_default_lock_directory();

    assert_lock_directory_failure(output, "default lock path is a symbolic link");
}

#[test]
fn rejects_a_file_at_the_default_lock_path() {
    let fixture = DefaultLockDirectoryFixture::new();
    fs::write(fixture.path(), b"not a directory")
        .expect("default lock path file should be created");

    let output = run_with_default_lock_directory();

    assert_lock_directory_failure(output, "default lock path is not a directory");
}

#[test]
fn creates_the_default_lock_directory_with_private_permissions() {
    let fixture = DefaultLockDirectoryFixture::new();

    let output = run_with_default_lock_directory();

    assert_eq!(output.status.code(), Some(99));
    let metadata = fs::metadata(fixture.path()).expect("default lock directory should exist");
    assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
}

#[test]
fn rejects_a_default_lock_directory_owned_by_another_user_when_privileged() {
    let effective_user = unsafe { libc::geteuid() };
    if effective_user != 0 {
        return;
    }

    let fixture = DefaultLockDirectoryFixture::new();
    fs::create_dir(fixture.path()).expect("default lock directory should be created");
    let path = CString::new(fixture.path().as_os_str().as_bytes())
        .expect("default lock path should not contain a null byte");
    let result = unsafe { libc::chown(path.as_ptr(), 1, u32::MAX) };
    assert_eq!(
        result, 0,
        "test should change the default lock directory owner"
    );

    let output = run_with_default_lock_directory();

    assert_lock_directory_failure(output, "default lock directory is owned by user 1");
}

#[test]
fn forwards_the_guarded_commands_exit_code() {
    let root = TestDirectory::new("exit-code");
    let status = agent_flock()
        .env("AGENT_FLOCK_LOCK_DIR", root.path().join("locks"))
        .args(["--", "sh", "-c", "exit 37"])
        .status()
        .expect("agent-flock should start");

    assert_eq!(status.code(), Some(37));
}

#[test]
fn forwards_arguments_without_shell_reparsing() {
    let root = TestDirectory::new("arguments");
    let output = agent_flock()
        .env("AGENT_FLOCK_LOCK_DIR", root.path().join("locks"))
        .args([
            "--",
            "sh",
            "-c",
            "printf '<%s>\\n' \"$@\"",
            "argument-printer",
            "two words",
            "*.rs",
            "semi;colon",
            "quote\"mark",
        ])
        .output()
        .expect("agent-flock should start");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        "<two words>\n<*.rs>\n<semi;colon>\n<quote\"mark>\n"
    );
}

fn critical_section_command(
    root: &Path,
    worktree: &Path,
    lock_name: &str,
    participant: &str,
) -> Command {
    let script = r#"
critical=$1
log=$2
participant=$3
if ! mkdir "$critical" 2>/dev/null; then
  printf 'overlap:%s\n' "$participant" >> "$log"
  exit 91
fi
printf 'enter:%s\n' "$participant" >> "$log"
sleep 0.25
printf 'exit:%s\n' "$participant" >> "$log"
rmdir "$critical"
"#;

    let mut command = agent_flock();
    command
        .current_dir(worktree)
        .env("AGENT_FLOCK_LOCK_DIR", root.join("locks"))
        .arg("--lock")
        .arg(lock_name)
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg(script)
        .arg("critical-section")
        .arg(root.join("critical"))
        .arg(root.join("events.log"))
        .arg(participant);
    command
}

#[test]
fn separate_worktrees_cannot_enter_the_same_guarded_section_together() {
    let root = TestDirectory::new("same-group");
    let worktree_a = root.path().join("worktree-a");
    let worktree_b = root.path().join("worktree-b");
    fs::create_dir(&worktree_a).expect("first worktree should be created");
    fs::create_dir(&worktree_b).expect("second worktree should be created");

    let mut first = critical_section_command(root.path(), &worktree_a, "memory", "a")
        .spawn()
        .expect("first agent-flock process should start");
    wait_for_path(&root.path().join("critical"));

    let second_status = critical_section_command(root.path(), &worktree_b, "memory", "b")
        .status()
        .expect("second agent-flock process should start");
    let first_status = first
        .wait()
        .expect("first agent-flock process should finish");

    assert!(first_status.success());
    assert!(second_status.success());
    let events = fs::read_to_string(root.path().join("events.log"))
        .expect("critical-section events should be recorded");
    assert_eq!(events, "enter:a\nexit:a\nenter:b\nexit:b\n");
}

fn send_signal(pid: u32, signal: &str) {
    let status = Command::new("kill")
        .args([signal, &pid.to_string()])
        .status()
        .expect("kill should start");
    assert!(status.success(), "kill should deliver {signal} to {pid}");
}

#[test]
fn crash_recovery_needs_no_stale_timeout_and_tracks_the_orphaned_command() {
    let root = TestDirectory::new("wrapper-crash");
    let lock_directory = root.path().join("locks");
    let guarded_pid_path = root.path().join("guarded.pid");
    let acquired_path = root.path().join("acquired");

    let mut holder = agent_flock();
    holder
        .env("AGENT_FLOCK_LOCK_DIR", &lock_directory)
        .args(["--lock", "memory", "--", "sh", "-c"])
        .arg("printf '%s' \"$$\" > \"$1\"; exec sleep 60")
        .arg("holder")
        .arg(&guarded_pid_path);
    let mut holder = holder.spawn().expect("lock holder should start");
    wait_for_path(&guarded_pid_path);

    let guarded_pid: u32 = fs::read_to_string(&guarded_pid_path)
        .expect("guarded pid should be readable")
        .parse()
        .expect("guarded pid should be numeric");
    send_signal(holder.id(), "-KILL");
    let holder_status = holder.wait().expect("killed wrapper should be reaped");
    assert_eq!(holder_status.signal(), Some(9));

    let mut waiter = agent_flock();
    waiter
        .env("AGENT_FLOCK_LOCK_DIR", &lock_directory)
        .args(["--lock", "memory", "--", "sh", "-c"])
        .arg("printf acquired > \"$1\"")
        .arg("waiter")
        .arg(&acquired_path);
    let mut waiter = waiter.spawn().expect("waiter should start");

    thread::sleep(Duration::from_millis(300));
    let acquired_before_guarded_exit = acquired_path.exists();
    send_signal(guarded_pid, "-KILL");
    let waiter_status = waiter.wait().expect("waiter should finish after recovery");

    assert!(waiter_status.success());
    assert!(
        !acquired_before_guarded_exit,
        "waiter acquired while the orphaned guarded command was still running"
    );
    assert!(acquired_path.exists());
}

#[test]
fn interruption_reaches_the_guarded_command_and_releases_the_lock() {
    let root = TestDirectory::new("interruption");
    let lock_directory = root.path().join("locks");
    let guarded_pid_path = root.path().join("guarded.pid");
    let interrupted_path = root.path().join("interrupted");

    let mut holder = agent_flock();
    holder
        .env("AGENT_FLOCK_LOCK_DIR", &lock_directory)
        .args(["--lock", "memory", "--", "sh", "-c"])
        .arg(
            "trap 'printf interrupted > \"$2\"; exit 0' TERM; \
             printf '%s' \"$$\" > \"$1\"; while :; do sleep 0.05; done",
        )
        .arg("holder")
        .arg(&guarded_pid_path)
        .arg(&interrupted_path);
    let mut holder = holder.spawn().expect("interruptible holder should start");
    wait_for_path(&guarded_pid_path);
    let guarded_pid: u32 = fs::read_to_string(&guarded_pid_path)
        .expect("guarded pid should be readable")
        .parse()
        .expect("guarded pid should be numeric");

    send_signal(holder.id(), "-TERM");
    let holder_status = holder.wait().expect("interrupted wrapper should finish");

    let deadline = Instant::now() + Duration::from_secs(1);
    while !interrupted_path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let guarded_command_was_interrupted = interrupted_path.exists();
    if !guarded_command_was_interrupted {
        let _ = Command::new("kill")
            .args(["-KILL", &guarded_pid.to_string()])
            .status();
    }

    assert_eq!(holder_status.signal(), Some(15));
    assert!(
        guarded_command_was_interrupted,
        "guarded command did not receive SIGTERM"
    );

    let release_status = agent_flock()
        .env("AGENT_FLOCK_LOCK_DIR", &lock_directory)
        .args(["--lock", "memory", "--", "sh", "-c", "exit 0"])
        .status()
        .expect("lock should be reusable after interruption");
    assert!(release_status.success());
}

#[test]
fn different_lock_groups_can_run_concurrently() {
    let root = TestDirectory::new("different-groups");
    let worktree_a = root.path().join("worktree-a");
    let worktree_b = root.path().join("worktree-b");
    fs::create_dir(&worktree_a).expect("first worktree should be created");
    fs::create_dir(&worktree_b).expect("second worktree should be created");

    let mut first = critical_section_command(root.path(), &worktree_a, "memory", "holder")
        .spawn()
        .expect("first resource group should start");
    wait_for_path(&root.path().join("critical"));

    let second_status = agent_flock()
        .current_dir(&worktree_b)
        .env("AGENT_FLOCK_LOCK_DIR", root.path().join("locks"))
        .args(["--lock", "network", "--", "sh", "-c"])
        .arg("if mkdir \"$1\" 2>/dev/null; then rmdir \"$1\"; exit 92; fi")
        .arg("concurrency-check")
        .arg(root.path().join("critical"))
        .status()
        .expect("second resource group should start");
    let first_status = first.wait().expect("first resource group should finish");

    assert!(first_status.success());
    assert!(
        second_status.success(),
        "different resource groups were unexpectedly serialized"
    );
}

#[test]
fn preserves_cwd_environment_and_stdio() {
    let root = TestDirectory::new("process-context");
    let worktree = root.path().join("worktree");
    fs::create_dir(&worktree).expect("worktree should be created");

    let mut command = agent_flock();
    command
        .current_dir(&worktree)
        .env("AGENT_FLOCK_LOCK_DIR", root.path().join("locks"))
        .env("AGENT_FLOCK_TEST_VALUE", "from-parent")
        .args(["--", "sh", "-c"])
        .arg(
            "read input; printf 'cwd=%s\\nenv=%s\\nstdin=%s\\n' \
             \"$PWD\" \"$AGENT_FLOCK_TEST_VALUE\" \"$input\"; \
             printf 'stderr-marker\\n' >&2",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("agent-flock should start");
    child
        .stdin
        .as_mut()
        .expect("wrapper stdin should be piped")
        .write_all(b"from-stdin\n")
        .expect("stdin should be writable");
    let output = child.wait_with_output().expect("agent-flock should finish");

    let canonical_worktree = fs::canonicalize(&worktree).expect("worktree should canonicalize");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        format!(
            "cwd={}\nenv=from-parent\nstdin=from-stdin\n",
            canonical_worktree.display()
        )
    );
    assert_eq!(output.stderr, b"stderr-marker\n");
}

#[test]
fn default_lock_directory_is_stable_across_temp_directories() {
    let _lock = lock_default_directory_tests();
    let root = TestDirectory::new("stable-default-directory");
    let worktree_a = root.path().join("worktree-a");
    let worktree_b = root.path().join("worktree-b");
    let temp_a = root.path().join("temp-a");
    let temp_b = root.path().join("temp-b");
    for directory in [&worktree_a, &worktree_b, &temp_a, &temp_b] {
        fs::create_dir(directory).expect("test directory should be created");
    }
    let lock_name = format!("stable-default-directory-{}", std::process::id());

    let mut first = critical_section_command(root.path(), &worktree_a, &lock_name, "a");
    first
        .env_remove("AGENT_FLOCK_LOCK_DIR")
        .env("TMPDIR", &temp_a);
    let mut first = first.spawn().expect("first process should start");
    wait_for_path(&root.path().join("critical"));

    let mut second = critical_section_command(root.path(), &worktree_b, &lock_name, "b");
    second
        .env_remove("AGENT_FLOCK_LOCK_DIR")
        .env("TMPDIR", &temp_b);
    let second_status = second.status().expect("second process should start");
    let first_status = first.wait().expect("first process should finish");

    assert!(first_status.success());
    assert!(second_status.success());
    let events = fs::read_to_string(root.path().join("events.log"))
        .expect("critical-section events should be recorded");
    assert_eq!(events, "enter:a\nexit:a\nenter:b\nexit:b\n");
}

#[test]
fn interruption_stops_a_waiter_without_running_its_command() {
    let root = TestDirectory::new("waiting-interruption");
    let lock_directory = root.path().join("locks");
    let guarded_pid_path = root.path().join("guarded.pid");
    let command_ran_path = root.path().join("command-ran");

    let mut holder = agent_flock();
    holder
        .env("AGENT_FLOCK_LOCK_DIR", &lock_directory)
        .args(["--lock", "memory", "--", "sh", "-c"])
        .arg("printf '%s' \"$$\" > \"$1\"; exec sleep 60")
        .arg("holder")
        .arg(&guarded_pid_path);
    let mut holder = holder.spawn().expect("holder should start");
    wait_for_path(&guarded_pid_path);
    let guarded_pid: u32 = fs::read_to_string(&guarded_pid_path)
        .expect("guarded pid should be readable")
        .parse()
        .expect("guarded pid should be numeric");

    let mut waiter = agent_flock();
    waiter
        .env("AGENT_FLOCK_LOCK_DIR", &lock_directory)
        .args(["--lock", "memory", "--", "sh", "-c"])
        .arg("printf ran > \"$1\"")
        .arg("waiter")
        .arg(&command_ran_path)
        .stderr(Stdio::null());
    let mut waiter = waiter.spawn().expect("waiter should start");
    thread::sleep(Duration::from_millis(200));
    assert!(!command_ran_path.exists());

    send_signal(waiter.id(), "-TERM");
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut waiter_status_before_release = None;
    while Instant::now() < deadline {
        if let Some(status) = waiter.try_wait().expect("waiter status should be readable") {
            waiter_status_before_release = Some(status);
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    send_signal(guarded_pid, "-KILL");
    let _ = holder.wait();
    let interrupted_while_waiting = waiter_status_before_release.is_some();
    let waiter_status = match waiter_status_before_release {
        Some(status) => status,
        None => waiter.wait().expect("waiter should eventually finish"),
    };

    assert!(
        interrupted_while_waiting,
        "interrupted waiter remained blocked on the lock"
    );
    assert_eq!(waiter_status.signal(), Some(15));
    assert!(!command_ran_path.exists());
}

#[test]
fn waiting_status_is_concise_and_immediate_acquisition_is_quiet() {
    let root = TestDirectory::new("waiting-status");
    let lock_directory = root.path().join("locks");
    let ready_path = root.path().join("ready");

    let immediate = agent_flock()
        .env("AGENT_FLOCK_LOCK_DIR", &lock_directory)
        .args(["--lock", "quiet", "--", "sh", "-c", "exit 0"])
        .output()
        .expect("immediate command should run");
    assert!(immediate.status.success());
    assert!(immediate.stderr.is_empty());

    let mut holder = agent_flock();
    holder
        .env("AGENT_FLOCK_LOCK_DIR", &lock_directory)
        .args(["--lock", "busy", "--", "sh", "-c"])
        .arg("printf ready > \"$1\"; sleep 0.3")
        .arg("holder")
        .arg(&ready_path);
    let mut holder = holder.spawn().expect("holder should start");
    wait_for_path(&ready_path);

    let waiter = agent_flock()
        .env("AGENT_FLOCK_LOCK_DIR", &lock_directory)
        .args(["--lock", "busy", "--", "sh", "-c", "exit 0"])
        .output()
        .expect("waiter should run");
    assert!(holder.wait().expect("holder should finish").success());
    assert!(waiter.status.success());

    let stderr = String::from_utf8(waiter.stderr).expect("stderr should be UTF-8");
    let lines: Vec<_> = stderr.lines().collect();
    assert_eq!(lines.len(), 2, "unexpected waiting output: {stderr:?}");
    assert_eq!(lines[0], "agent-flock: waiting for lock \"busy\"");
    assert!(
        lines[1].starts_with("agent-flock: acquired lock \"busy\" after ")
            && lines[1].ends_with('s'),
        "unexpected acquisition output: {}",
        lines[1]
    );
}

#[test]
fn help_and_version_do_not_require_a_guarded_command() {
    let help = agent_flock()
        .arg("--help")
        .output()
        .expect("help should run");
    assert!(help.status.success());
    let help_stdout = String::from_utf8(help.stdout).expect("help should be UTF-8");
    assert!(help_stdout.contains("agent-flock [--lock <name>] -- <command> [args...]"));
    assert!(help_stdout.contains("--lock <name>"));

    let version = agent_flock()
        .arg("--version")
        .output()
        .expect("version should run");
    assert!(version.status.success());
    assert_eq!(version.stdout, b"agent-flock 0.1.0\n");
}

#[test]
fn preserves_signal_termination_from_the_guarded_command() {
    let root = TestDirectory::new("child-signal");
    let status = agent_flock()
        .env("AGENT_FLOCK_LOCK_DIR", root.path().join("locks"))
        .args(["--", "sh", "-c", "kill -TERM $$"])
        .status()
        .expect("agent-flock should run a signal-terminated command");

    assert_eq!(status.signal(), Some(15));
}
