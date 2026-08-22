mod cli;
mod lock;
mod process;
mod signals;

use process::CommandOutcome;
use std::env;
use std::process::exit;

fn main() {
    let invocation = match cli::parse(env::args_os().skip(1)) {
        Ok(cli::Action::Run(invocation)) => invocation,
        Ok(cli::Action::Help) => {
            print!("{}", cli::HELP);
            return;
        }
        Ok(cli::Action::Version) => {
            println!("agent-flock {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Err(error) => {
            eprintln!("agent-flock: {error}");
            eprintln!("usage: agent-flock [--lock <name>] -- <command> [args...]");
            exit(2);
        }
    };
    let signals = signals::SignalMonitor::install().unwrap_or_else(|error| {
        eprintln!("agent-flock: failed to install signal handlers: {error}");
        exit(1);
    });
    let lock_guard = match lock::acquire(&invocation.lock_name, &signals) {
        Ok(lock_guard) => lock_guard,
        Err(lock::AcquireError::Interrupted(signal)) => signals.terminate(signal),
        Err(lock::AcquireError::Io(error)) => {
            eprintln!(
                "agent-flock: failed to acquire lock \"{}\": {error}",
                invocation.lock_name
            );
            exit(1);
        }
    };

    let outcome = process::run(&invocation, &lock_guard, &signals);
    drop(lock_guard);

    match outcome {
        Ok(CommandOutcome::Exit(code)) => exit(code),
        Ok(CommandOutcome::Signal(signal)) => signals.terminate(signal),
        Err(error) => {
            eprintln!("agent-flock: failed to run command: {error}");
            exit(1);
        }
    }
}
