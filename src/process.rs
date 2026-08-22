use crate::cli::Invocation;
use crate::lock::LockGuard;
use crate::signals::SignalMonitor;
use std::io;
use std::process::Command;
use std::thread;
use std::time::Duration;

pub enum CommandOutcome {
    Exit(i32),
    Signal(i32),
}

pub fn run(
    invocation: &Invocation,
    lock: &LockGuard,
    signals: &SignalMonitor,
) -> io::Result<CommandOutcome> {
    let mut command = Command::new(&invocation.command);
    command.args(&invocation.arguments);
    let mut child = lock.spawn(&mut command)?;
    let mut received_signal = None;
    let mut delivery_error = None;

    loop {
        if let Some(signal) = signals.take() {
            received_signal.get_or_insert(signal);
            if unsafe { libc::kill(child.id() as libc::pid_t, signal) } == -1 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::NotFound {
                    delivery_error.get_or_insert(error);
                }
            }
        }

        if let Some(status) = child.try_wait()? {
            if let Some(error) = delivery_error {
                return Err(error);
            }

            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(signal) = status.signal().or(received_signal) {
                    return Ok(CommandOutcome::Signal(signal));
                }
            }

            return Ok(CommandOutcome::Exit(status.code().unwrap_or(1)));
        }

        thread::sleep(Duration::from_millis(20));
    }
}
