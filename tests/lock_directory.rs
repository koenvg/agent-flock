#![cfg(unix)]

mod support;

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::Path;
use std::process::{Command, Output};

use support::{
    TestCommandExt, TestDirectory, agent_flock, critical_section_command, wait_for_path,
};

const DEFAULT_LOCK_PATH_REDIRECT: &str = "AGENT_FLOCK_TEST_DEFAULT_LOCK_PATH";

struct DefaultLockDirectoryFixture {
    _directory_guard: TestDirectory,
    path: std::path::PathBuf,
    interposer: std::path::PathBuf,
}

impl DefaultLockDirectoryFixture {
    fn new() -> Self {
        let root = TestDirectory::new("default-lock-directory");
        let effective_user = unsafe { libc::geteuid() };
        let path = root.path().join(format!("agent-flock-{effective_user}"));
        let interposer = compile_default_lock_directory_interposer(root.path());

        Self {
            _directory_guard: root,
            path,
            interposer,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn command(&self) -> Command {
        let mut command = agent_flock();
        self.isolate(&mut command);
        command
    }

    fn isolate(&self, command: &mut Command) {
        command
            .env_remove("AGENT_FLOCK_LOCK_DIR")
            .env(DEFAULT_LOCK_PATH_REDIRECT, &self.path);

        #[cfg(target_os = "macos")]
        command
            .env("DYLD_INSERT_LIBRARIES", &self.interposer)
            .env("DYLD_FORCE_FLAT_NAMESPACE", "1");

        #[cfg(not(target_os = "macos"))]
        command.env("LD_PRELOAD", &self.interposer);
    }
}

fn compile_default_lock_directory_interposer(directory: &Path) -> std::path::PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/support/default_lock_directory_interposer.c");
    let library = directory.join(if cfg!(target_os = "macos") {
        "default-lock-directory-interposer.dylib"
    } else {
        "default-lock-directory-interposer.so"
    });
    let mut compiler = Command::new("cc");

    #[cfg(target_os = "macos")]
    compiler.args(["-dynamiclib", "-o"]);

    #[cfg(not(target_os = "macos"))]
    compiler.args(["-shared", "-fPIC", "-o"]);

    let output = compiler
        .arg(&library)
        .arg(source)
        .output()
        .expect("C compiler should start");
    assert!(
        output.status.success(),
        "default lock directory interposer should compile: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    library
}

fn run_with_default_lock_directory(fixture: &DefaultLockDirectoryFixture) -> Output {
    fixture
        .command()
        .args(["--", "sh", "-c", "exit 99"])
        .output()
        .expect("agent-flock should start")
}

fn assert_lock_directory_failure(output: Output, expected: &str) {
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
fn rejects_a_symlinked_lock_file_in_a_configured_directory() {
    let root = TestDirectory::new("configured-lock-file-symlink");
    let lock_directory = root.path().join("locks");
    fs::create_dir(&lock_directory).expect("configured lock directory should be created");
    let target = root.path().join("outside-lock-directory");
    fs::write(&target, b"outside").expect("symlink target should be created");
    let lock_path = lock_directory
        .join("v1-32ec1ae9844e3b8c4bfb8a9501835e16eb3e0399d0b6d1e741876964de1da307.lock");
    symlink(&target, lock_path).expect("lock file symlink should be created");

    let output = agent_flock()
        .env("AGENT_FLOCK_LOCK_DIR", &lock_directory)
        .args(["--lock", "memory", "--", "sh", "-c", "exit 99"])
        .output()
        .expect("agent-flock should start");

    assert_lock_directory_failure(output, "failed to acquire lock \"memory\"");
    assert_eq!(
        fs::read(target).expect("symlink target should remain readable"),
        b"outside"
    );
}

#[test]
fn rejects_a_symlink_at_the_default_lock_path() {
    let fixture = DefaultLockDirectoryFixture::new();
    let target = TestDirectory::new("default-lock-symlink-target");
    symlink(target.path(), fixture.path()).expect("default lock path symlink should be created");

    let output = run_with_default_lock_directory(&fixture);

    assert_lock_directory_failure(output, "default lock path is a symbolic link");
}

#[test]
fn rejects_a_file_at_the_default_lock_path() {
    let fixture = DefaultLockDirectoryFixture::new();
    fs::write(fixture.path(), b"not a directory")
        .expect("default lock path file should be created");

    let output = run_with_default_lock_directory(&fixture);

    assert_lock_directory_failure(output, "default lock path is not a directory");
}

#[test]
fn creates_the_default_lock_directory_with_private_permissions() {
    let fixture = DefaultLockDirectoryFixture::new();

    let output = run_with_default_lock_directory(&fixture);

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

    let output = run_with_default_lock_directory(&fixture);

    assert_lock_directory_failure(output, "default lock directory is owned by user 1");
}

#[test]
fn default_lock_directory_is_stable_across_temp_directories() {
    let fixture = DefaultLockDirectoryFixture::new();
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
    fixture.isolate(&mut first);
    first.env("TMPDIR", &temp_a);
    let mut first = first.spawn_guarded().expect("first process should start");
    wait_for_path(&root.path().join("critical"));

    let mut second = critical_section_command(root.path(), &worktree_b, &lock_name, "b");
    fixture.isolate(&mut second);
    second.env("TMPDIR", &temp_b);
    let second_status = second.status().expect("second process should start");
    let first_status = first.wait().expect("first process should finish");

    assert!(first_status.success());
    assert!(second_status.success());
    let events = fs::read_to_string(root.path().join("events.log"))
        .expect("critical-section events should be recorded");
    assert_eq!(events, "enter:a\nexit:a\nenter:b\nexit:b\n");
}
