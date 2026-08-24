#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_TEMP_PATH: AtomicU64 = AtomicU64::new(0);

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
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for {path:?}");
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
