#![cfg(unix)]

mod support;

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::thread;
use std::time::Duration;

use support::{
    TestCommandExt, TestDirectory, agent_flock, critical_section_command, send_signal,
    wait_for_path, wait_for_pid,
};

#[test]
fn separate_worktrees_cannot_enter_the_same_guarded_section_together() {
    let root = TestDirectory::new("same-group");
    let worktree_a = root.path().join("worktree-a");
    let worktree_b = root.path().join("worktree-b");
    fs::create_dir(&worktree_a).expect("first worktree should be created");
    fs::create_dir(&worktree_b).expect("second worktree should be created");

    let mut first = critical_section_command(root.path(), &worktree_a, "memory", "a")
        .spawn_guarded()
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
    let mut holder = holder.spawn_guarded().expect("lock holder should start");
    let guarded_pid = wait_for_pid(&guarded_pid_path);
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
    let mut waiter = waiter.spawn_guarded().expect("waiter should start");

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
fn different_lock_groups_can_run_concurrently() {
    let root = TestDirectory::new("different-groups");
    let worktree_a = root.path().join("worktree-a");
    let worktree_b = root.path().join("worktree-b");
    fs::create_dir(&worktree_a).expect("first worktree should be created");
    fs::create_dir(&worktree_b).expect("second worktree should be created");

    let mut first = critical_section_command(root.path(), &worktree_a, "memory", "holder")
        .spawn_guarded()
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
    let mut holder = holder.spawn_guarded().expect("holder should start");
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
