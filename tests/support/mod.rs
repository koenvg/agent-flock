#![allow(dead_code)]

use std::fs;
use std::io;
use std::ops::{Deref, DerefMut};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const PROCESS_CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);
const READINESS_TIMEOUT: Duration = Duration::from_secs(5);

static NEXT_TEMP_PATH: AtomicU64 = AtomicU64::new(0);

pub struct TestChild {
    child: Option<Child>,
    process_group: libc::pid_t,
}

impl TestChild {
    fn new(child: Child) -> Self {
        let process_group = child.id() as libc::pid_t;
        Self {
            child: Some(child),
            process_group,
        }
    }

    pub fn wait_with_output(mut self) -> io::Result<Output> {
        self.child
            .take()
            .expect("test child should be present")
            .wait_with_output()
    }

    fn child(&self) -> &Child {
        self.child.as_ref().expect("test child should be present")
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("test child should be present")
    }
}

impl Deref for TestChild {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        self.child()
    }
}

impl DerefMut for TestChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.child_mut()
    }
}

impl Drop for TestChild {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };

        let _ = unsafe { libc::kill(-self.process_group, libc::SIGTERM) };
        let deadline = Instant::now() + PROCESS_CLEANUP_TIMEOUT;
        while process_group_exists(self.process_group) && Instant::now() < deadline {
            let _ = child.try_wait();
            thread::sleep(Duration::from_millis(10));
        }

        if process_group_exists(self.process_group) {
            let _ = unsafe { libc::kill(-self.process_group, libc::SIGKILL) };
        }
        let _ = child.wait();
    }
}

pub trait TestCommandExt {
    fn spawn_guarded(&mut self) -> io::Result<TestChild>;
}

impl TestCommandExt for Command {
    fn spawn_guarded(&mut self) -> io::Result<TestChild> {
        self.process_group(0);
        self.spawn().map(TestChild::new)
    }
}

fn process_group_exists(process_group: libc::pid_t) -> bool {
    if unsafe { libc::kill(-process_group, 0) } == 0 {
        return true;
    }

    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub struct TestDirectory(PathBuf);

impl TestDirectory {
    pub fn new(label: &str) -> Self {
        let path = unique_path_in(&std::env::temp_dir(), label);
        fs::create_dir(&path).expect("test directory should be created");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn unique_path_in(parent: &Path, label: &str) -> PathBuf {
    let sequence = NEXT_TEMP_PATH.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        "agent-flock-test-{}-{label}-{sequence}",
        std::process::id()
    ))
}

pub fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + READINESS_TIMEOUT;
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for {path:?}");
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn wait_for_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + READINESS_TIMEOUT;
    loop {
        match fs::read_to_string(path) {
            Ok(contents) => match contents.parse() {
                Ok(pid) => return pid,
                Err(error) if Instant::now() >= deadline => {
                    panic!("timed out waiting for a complete PID in {path:?}: {error}")
                }
                Err(_) => {}
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to read PID from {path:?}: {error}"),
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for PID file {path:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn agent_flock() -> Command {
    Command::new(env!("CARGO_BIN_EXE_agent-flock"))
}

pub fn send_signal(pid: u32, signal: &str) {
    let status = Command::new("kill")
        .args([signal, &pid.to_string()])
        .status()
        .expect("kill should start");
    assert!(status.success(), "kill should deliver {signal} to {pid}");
}
