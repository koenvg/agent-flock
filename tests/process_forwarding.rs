#![cfg(unix)]

mod support;

use std::fs;
use std::io::Write;
use std::process::Stdio;

use support::{TestDirectory, agent_flock};

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
