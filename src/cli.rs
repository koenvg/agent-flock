use std::ffi::OsString;

pub const HELP: &str = "agent-flock serializes opted-in commands across processes and Git worktrees.\n\nUSAGE:\n    agent-flock [--lock <name>] -- <command> [args...]\n\nOPTIONS:\n    --lock <name>  Resource group to lock [default: default]\n    --help         Print help\n    --version      Print version\n";

pub enum Action {
    Run(Invocation),
    Help,
    Version,
}

pub struct Invocation {
    pub lock_name: String,
    pub command: OsString,
    pub arguments: Vec<OsString>,
}

pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Action, String> {
    let mut arguments = arguments.into_iter();
    let mut lock_name = None;

    loop {
        let Some(argument) = arguments.next() else {
            return Err("expected `--` before the command".into());
        };

        if argument == "--" {
            break;
        }

        if argument == "--help" {
            return Ok(Action::Help);
        }
        if argument == "--version" {
            return Ok(Action::Version);
        }

        if argument == "--lock" {
            if lock_name.is_some() {
                return Err("`--lock` may only be specified once".into());
            }
            let value = arguments
                .next()
                .ok_or_else(|| "`--lock` requires a name".to_owned())?;
            let value = value
                .into_string()
                .map_err(|_| "lock names must be valid UTF-8".to_owned())?;
            if value.is_empty() {
                return Err("lock names cannot be empty".into());
            }
            lock_name = Some(value);
            continue;
        }

        return Err(format!("unknown option `{}`", argument.to_string_lossy()));
    }

    let command = arguments
        .next()
        .ok_or_else(|| "expected a command after `--`".to_owned())?;

    Ok(Action::Run(Invocation {
        lock_name: lock_name.unwrap_or_else(|| "default".into()),
        command,
        arguments: arguments.collect(),
    }))
}
