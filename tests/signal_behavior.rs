#![cfg(unix)]

mod support;

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use support::{TestDirectory, agent_flock, send_signal, wait_for_path};

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
fn preserves_signal_termination_from_the_guarded_command() {
    let root = TestDirectory::new("child-signal");
    let status = agent_flock()
        .env("AGENT_FLOCK_LOCK_DIR", root.path().join("locks"))
        .args(["--", "sh", "-c", "kill -TERM $$"])
        .status()
        .expect("agent-flock should run a signal-terminated command");

    assert_eq!(status.signal(), Some(15));
}
