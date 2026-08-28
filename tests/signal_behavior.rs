#![cfg(unix)]

mod support;

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use support::{TestCommandExt, TestDirectory, agent_flock, send_signal, wait_for_pid};

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
             : > \"$1\"; sleep 0.1; printf '%s' \"$$\" > \"$1\"; \
             while :; do sleep 0.05; done",
        )
        .arg("holder")
        .arg(&guarded_pid_path)
        .arg(&interrupted_path);
    let mut holder = holder
        .spawn_guarded()
        .expect("interruptible holder should start");
    let guarded_pid = wait_for_pid(&guarded_pid_path);

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
    let mut holder = holder.spawn_guarded().expect("holder should start");
    let guarded_pid = wait_for_pid(&guarded_pid_path);

    let mut waiter = agent_flock();
    waiter
        .env("AGENT_FLOCK_LOCK_DIR", &lock_directory)
        .args(["--lock", "memory", "--", "sh", "-c"])
        .arg("printf ran > \"$1\"")
        .arg("waiter")
        .arg(&command_ran_path)
        .stderr(Stdio::null());
    let mut waiter = waiter.spawn_guarded().expect("waiter should start");
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
fn guarded_children_are_cleaned_up_during_unwind() {
    let root = TestDirectory::new("child-cleanup");
    let guarded_pid_path = root.path().join("guarded.pid");

    let panic = std::panic::catch_unwind(|| {
        let mut child = agent_flock();
        child
            .env("AGENT_FLOCK_LOCK_DIR", root.path().join("locks"))
            .args(["--", "sh", "-c"])
            .arg("printf '%s' \"$$\" > \"$1\"; while :; do sleep 1; done")
            .arg("guarded-child")
            .arg(&guarded_pid_path);
        let _child = child.spawn_guarded().expect("guarded child should start");
        let guarded_pid = wait_for_pid(&guarded_pid_path);

        assert_eq!(guarded_pid, 0, "trigger cleanup during assertion unwind");
    });

    assert!(panic.is_err(), "test assertion should have panicked");
    let guarded_pid = wait_for_pid(&guarded_pid_path);
    let result = unsafe { libc::kill(guarded_pid as libc::pid_t, 0) };
    assert_eq!(result, -1, "guarded child should no longer be running");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
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
