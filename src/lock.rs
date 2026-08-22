use crate::signals::SignalMonitor;
use sha2::{Digest, Sha256};
use std::env;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

pub struct LockGuard {
    file: File,
}

pub enum AcquireError {
    Interrupted(i32),
    Io(io::Error),
}

impl fmt::Display for AcquireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interrupted(signal) => write!(formatter, "interrupted by signal {signal}"),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl From<io::Error> for AcquireError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl LockGuard {
    pub fn spawn(&self, command: &mut std::process::Command) -> io::Result<std::process::Child> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;

            let descriptor = self.file.as_raw_fd();
            let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
            if flags == -1 {
                return Err(io::Error::last_os_error());
            }
            if unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } == -1 {
                return Err(io::Error::last_os_error());
            }

            let child = command.spawn();
            unsafe {
                libc::fcntl(descriptor, libc::F_SETFD, flags);
            }
            child
        }

        #[cfg(not(unix))]
        {
            command.spawn()
        }
    }
}

pub fn acquire(name: &str, signals: &SignalMonitor) -> Result<LockGuard, AcquireError> {
    let directory = lock_directory();
    fs::create_dir_all(&directory)?;

    let digest = Sha256::digest([b"agent-flock-lock-v1\0".as_slice(), name.as_bytes()].concat());
    let path = directory.join(format!("v1-{digest:x}.lock"));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;

    match file.try_lock() {
        Ok(()) => {
            if let Some(signal) = signals.take() {
                return Err(AcquireError::Interrupted(signal));
            }
            return Ok(LockGuard { file });
        }
        Err(std::fs::TryLockError::WouldBlock) => {}
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }

    eprintln!("agent-flock: waiting for lock \"{name}\"");
    let started = Instant::now();
    loop {
        if let Some(signal) = signals.take() {
            return Err(AcquireError::Interrupted(signal));
        }
        thread::sleep(Duration::from_millis(100));
        match file.try_lock() {
            Ok(()) => {
                if let Some(signal) = signals.take() {
                    return Err(AcquireError::Interrupted(signal));
                }
                eprintln!(
                    "agent-flock: acquired lock \"{name}\" after {:.1}s",
                    started.elapsed().as_secs_f64()
                );
                return Ok(LockGuard { file });
            }
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
    }
}

fn lock_directory() -> PathBuf {
    if let Some(directory) = env::var_os("AGENT_FLOCK_LOCK_DIR") {
        return PathBuf::from(directory);
    }

    #[cfg(unix)]
    {
        let effective_user = unsafe { libc::geteuid() };
        PathBuf::from("/tmp").join(format!("agent-flock-{effective_user}"))
    }

    #[cfg(not(unix))]
    {
        env::temp_dir().join("agent-flock")
    }
}
