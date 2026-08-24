#!/usr/bin/env node
'use strict';

const { spawn } = require('node:child_process');

const platformPackages = {
  'darwin-arm64': '@koenvg/agent-flock-darwin-arm64',
  'darwin-x64': '@koenvg/agent-flock-darwin-x64',
  'linux-arm64': '@koenvg/agent-flock-linux-arm64',
  'linux-x64': '@koenvg/agent-flock-linux-x64',
};

function fail(message) {
  console.error(`agent-flock: ${message}`);
  process.exitCode = 1;
}

function resolveBinary() {
  const platform = `${process.platform}-${process.arch}`;
  const packageName = platformPackages[platform];
  if (!packageName) {
    fail(`no native binary is published for ${platform}`);
    return null;
  }

  try {
    return require.resolve(`${packageName}/bin/agent-flock`, { paths: [__dirname] });
  } catch {
    fail(
      `native package is missing for ${platform}; reinstall agent-flock with optional dependencies enabled`,
    );
    return null;
  }
}

function main() {
  const binary = resolveBinary();
  if (!binary) return;

  const child = spawn(binary, process.argv.slice(2), {
    cwd: process.cwd(),
    env: process.env,
    stdio: 'inherit',
  });
  let finished = false;
  let receivedSignal = null;
  const forwardedSignals = ['SIGHUP', 'SIGINT', 'SIGQUIT', 'SIGTERM'];
  const forward = Object.fromEntries(
    forwardedSignals.map((signal) => [
      signal,
      () => {
        if (!finished) {
          receivedSignal ??= signal;
          child.kill(signal);
        }
      },
    ]),
  );
  for (const signal of forwardedSignals) process.on(signal, forward[signal]);

  child.once('error', (error) => {
    finished = true;
    fail(`failed to start native binary: ${error.message}`);
  });
  child.once('exit', (code, signal) => {
    finished = true;
    for (const forwardedSignal of forwardedSignals) {
      process.removeListener(forwardedSignal, forward[forwardedSignal]);
    }

    const terminatingSignal = signal ?? receivedSignal;
    if (terminatingSignal) {
      process.kill(process.pid, terminatingSignal);
      return;
    }
    process.exitCode = code ?? 1;
  });
}

main();
