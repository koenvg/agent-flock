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
    let directory = prepare_lock_directory(lock_directory()?)?;

    let digest = Sha256::digest([b"agent-flock-lock-v1\0".as_slice(), name.as_bytes()].concat());
    let file_name = format!("v1-{digest:x}.lock");
    let file = open_lock_file(&directory, &file_name)?;

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

struct LockDirectory {
    path: PathBuf,
    #[cfg(unix)]
    expected_owner: Option<libc::uid_t>,
}

struct PreparedLockDirectory {
    path: PathBuf,
    #[cfg(unix)]
    descriptor: Option<File>,
}

fn lock_directory() -> io::Result<LockDirectory> {
    if let Some(directory) = env::var_os("AGENT_FLOCK_LOCK_DIR") {
        if directory.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "AGENT_FLOCK_LOCK_DIR must not be empty",
            ));
        }

        return Ok(LockDirectory {
            path: PathBuf::from(directory),
            #[cfg(unix)]
            expected_owner: None,
        });
    }

    #[cfg(unix)]
    {
        let effective_user = unsafe { libc::geteuid() };
        Ok(LockDirectory {
            path: PathBuf::from("/tmp").join(format!("agent-flock-{effective_user}")),
            expected_owner: Some(effective_user),
        })
    }

    #[cfg(not(unix))]
    {
        Ok(LockDirectory {
            path: env::temp_dir().join("agent-flock"),
        })
    }
}

fn prepare_lock_directory(directory: LockDirectory) -> io::Result<PreparedLockDirectory> {
    #[cfg(unix)]
    if let Some(expected_owner) = directory.expected_owner {
        let descriptor = prepare_default_lock_directory(&directory.path, expected_owner)?;
        return Ok(PreparedLockDirectory {
            path: directory.path,
            descriptor: Some(descriptor),
        });
    }

    fs::create_dir_all(&directory.path)?;
    Ok(PreparedLockDirectory {
        path: directory.path,
        #[cfg(unix)]
        descriptor: None,
    })
}

#[cfg(unix)]
fn prepare_default_lock_directory(
    path: &std::path::Path,
    expected_owner: libc::uid_t,
) -> io::Result<File> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    match builder.create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }

    let path_metadata = fs::symlink_metadata(path)?;
    if path_metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("default lock path is a symbolic link: {}", path.display()),
        ));
    }
    if !path_metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("default lock path is not a directory: {}", path.display()),
        ));
    }

    let descriptor = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = descriptor.metadata()?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("default lock path is not a directory: {}", path.display()),
        ));
    }
    if metadata.uid() != expected_owner {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "default lock directory is owned by user {}, expected effective user {expected_owner}: {}",
                metadata.uid(),
                path.display()
            ),
        ));
    }

    Ok(descriptor)
}

fn open_lock_file(directory: &PreparedLockDirectory, file_name: &str) -> io::Result<File> {
    #[cfg(unix)]
    if let Some(descriptor) = &directory.descriptor {
        use std::ffi::CString;
        use std::os::fd::{AsRawFd, FromRawFd};

        let file_name = CString::new(file_name).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "lock filename contains a null byte",
            )
        })?;
        let file_descriptor = unsafe {
            libc::openat(
                descriptor.as_raw_fd(),
                file_name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if file_descriptor == -1 {
            return Err(io::Error::last_os_error());
        }

        return Ok(unsafe { File::from_raw_fd(file_descriptor) });
    }

    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.path.join(file_name))
}
