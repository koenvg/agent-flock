# Agent Flock

`agent-flock` serializes opted-in development commands across processes and Git worktrees. It is a small Rust CLI distributed as native binaries, including through npm.

Git worktrees isolate files, not RAM. If several coding agents run type checks, tests, linters, or builds at once, their combined memory use can push the host into swap. Task-runner concurrency settings cannot help when each agent starts a separate task-runner process.

## Install

Install the npm package in a JavaScript or TypeScript project:

```sh
npm install --save-dev agent-flock
```

The npm package installs one native binary for the current platform through an optional platform package. It does not compile Rust or download files in an install script.

For development from this repository:

```sh
cargo install --path . --locked
```

The release binary is also usable in Rust, Go, Python, Java, and other projects. Nothing in the locking protocol depends on Node.js or an agent framework.

## Usage

```sh
agent-flock -- npm run typecheck
agent-flock --lock high-memory -- cargo test
agent-flock --lock local-postgres -- ./scripts/integration-tests
```

The `--` separator is required. Everything after it is passed directly to the guarded program without shell reparsing.

```text
agent-flock [--lock <name>] -- <command> [args...]
```

The default lock name is `default`. Commands using the same name run one at a time. Commands using different names may run concurrently.

Name a lock after the shared bottleneck, not after one command. For example, `high-memory` can cover type checks, tests, linters, and builds because they compete for the same RAM. `local-postgres` can cover commands that need exclusive access to one development database.

### package.json example

Route each memory-heavy script through the same resource group:

```json
{
  "devDependencies": {
    "agent-flock": "^0.1.0"
  },
  "scripts": {
    "typecheck": "agent-flock --lock high-memory -- tsc --noEmit",
    "test": "agent-flock --lock high-memory -- vitest run",
    "lint": "agent-flock --lock high-memory -- eslint .",
    "build": "agent-flock --lock high-memory -- vite build"
  }
}
```

Use another name only for a different bottleneck. Here, Vitest joins the shared high-memory queue, while the database reset is serialized with other commands that use one local PostgreSQL instance:

```json
{
  "scripts": {
    "test": "agent-flock --lock high-memory -- vitest run",
    "database:reset": "agent-flock --lock local-postgres -- ./scripts/reset-test-database"
  }
}
```

## Lock identity and location

On macOS and Linux, the default directory is:

```text
/tmp/agent-flock-<effective-user-id>
```

This location is outside every worktree and does not depend on `cwd`, `TMPDIR`, or the repository path. All processes for the same OS account therefore use the same namespace. The tool does not coordinate different OS accounts.

A lock filename is `v1-<sha256>.lock`. The digest covers a protocol-version prefix and the exact UTF-8 lock name. This keeps paths bounded and avoids collisions caused by sanitizing resource names.

Set `AGENT_FLOCK_LOCK_DIR` to use another directory. Every cooperating process must use the same value. Use a local filesystem. Advisory lock behavior on network filesystems differs by operating system and mount configuration.

The empty lock files remain after commands finish. File existence does not mean the lock is held.

## Waiting behavior

Immediate acquisition is silent. On contention, the CLI prints two lines to stderr:

```text
agent-flock: waiting for lock "high-memory"
agent-flock: acquired lock "high-memory" after 4.2s
```

It checks the lock every 100 milliseconds but does not print polling messages. Waiting has no timeout.

## Command and signal behavior

The native CLI starts the requested executable directly and preserves:

- each argument as a separate argument
- the current working directory
- the current environment
- stdin, stdout, and stderr
- the command's exit code
- termination by `SIGHUP`, `SIGINT`, `SIGQUIT`, or `SIGTERM`

When the wrapper receives one of those signals, it forwards the signal to the guarded process, waits for it to finish, closes its copy of the lock, and terminates with the same signal. Signals aimed only at the wrapper are forwarded to the direct child. Descendant handling still depends on that program's own process model.

On Unix, the CLI also passes the locked file descriptor to the guarded process. If the wrapper is killed with `SIGKILL` or crashes, the guarded process and descendants that retain the descriptor continue holding the lock. The kernel releases it when the final holder exits or closes the descriptor.

A program that deliberately closes inherited nonstandard file descriptors removes that extra crash protection. The wrapper still holds the lock during normal operation.

## Crash recovery and stale locks

`agent-flock` uses the operating system's advisory whole-file lock through Rust's standard library. On Unix this currently maps to `flock(2)`. The kernel releases locks when their last file descriptor closes, including after process termination.

There is no stale timeout, heartbeat, or stale-lock deletion. An ordinary command can run for hours without becoming stale, and crash recovery does not wait for a timer. A leftover lock file is harmless because ownership lives in the kernel.

The original design called for `proper-lockfile` unless repository constraints justified another choice. `proper-lockfile` is a JavaScript library and would require Node.js at runtime. A native Rust binary makes the wrapper useful outside Node projects. Kernel-managed locks also avoid `proper-lockfile`'s heartbeat and stale-window tradeoff, so this implementation deliberately does not use it.

## What it does not do

`agent-flock` does not:

- impose a memory or CPU limit
- discover heavy commands automatically
- coordinate commands that bypass the wrapper
- make Git worktrees resource-isolated
- choose the right resource groups for a project
- provide fairness guarantees between waiters

All participants must cooperate by using the same lock name and lock directory.

## Comparison with other controls

### Unix `flock`

`flock` provides the same basic kernel primitive on systems that ship the command. `agent-flock` adds a stable per-user named-lock strategy, npm distribution, concise waiting output, direct argument forwarding, signal handling, and the inherited-descriptor crash behavior. macOS does not ship the common Linux `flock` command by default.

### Task-runner concurrency flags

Task-runner flags coordinate tasks inside one invocation. They do not coordinate separate agents, terminals, CI jobs, or worktrees that each start their own invocation.

### Node `--max-old-space-size`

This flag caps one Node.js V8 heap. It does not cap native allocations, child processes, non-Node tools, or the combined memory used by several commands. It can complement `agent-flock`, but it solves a different problem.

### Linux cgroups and `systemd-run`

Cgroups can enforce real memory and CPU limits. They are stronger than serialization, but they require Linux-specific setup and can terminate work when a limit is reached. `agent-flock` works on macOS and Linux without elevated privileges. Use cgroups when enforcement matters more than portability.

## Platform support

Version 0.1 publishes native npm packages for:

- macOS arm64
- macOS x64
- Linux arm64 using a static musl build
- Linux x64 using a static musl build

Windows is not supported in version 0.1 and no Windows npm binary is published. Rust's standard library has Windows file locking, but Unix signal forwarding and inherited lock-descriptor behavior do not map directly to Windows process handles. A later Windows implementation must define and test those weaker process-tree guarantees rather than claim POSIX behavior.

## Development

The repository requires Rust 1.89 or newer and Node.js 18 or newer.

```sh
npm run format
npm run lint
npm run typecheck
npm test
npm run build
```

`npm/assemble-platform-packages.mjs` turns release artifacts into the four platform packages. It expects one binary at `<artifacts>/<platform-id>/agent-flock` and writes publishable packages to an empty output directory:

```sh
node npm/assemble-platform-packages.mjs \
  --artifacts dist/artifacts \
  --output dist/npm
```

Publish the platform packages before publishing the root `agent-flock` package so every exact-version optional dependency is available.
