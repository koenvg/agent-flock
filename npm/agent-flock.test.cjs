'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const test = require('node:test');

const platformPackages = {
  'darwin-arm64': '@agent-flock/darwin-arm64',
  'darwin-x64': '@agent-flock/darwin-x64',
  'linux-arm64': '@agent-flock/linux-arm64',
  'linux-x64': '@agent-flock/linux-x64',
};

function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'agent-flock-npm-test-'));
  const launcher = path.join(root, 'agent-flock.cjs');
  fs.copyFileSync(path.join(__dirname, 'agent-flock.cjs'), launcher);
  return { root, launcher };
}

function installFakeBinary(root, contents) {
  const packageName = platformPackages[`${process.platform}-${process.arch}`];
  assert.ok(packageName, 'test host must be in the supported npm platform matrix');
  const packageRoot = path.join(root, 'node_modules', ...packageName.split('/'));
  const binary = path.join(packageRoot, 'bin', 'agent-flock');
  fs.mkdirSync(path.dirname(binary), { recursive: true });
  fs.writeFileSync(
    path.join(packageRoot, 'package.json'),
    JSON.stringify({ name: packageName, version: '0.1.0' }),
  );
  fs.writeFileSync(binary, contents, { mode: 0o755 });
}

test('npm launcher forwards arguments, stdio, cwd, environment, and exit code', (context) => {
  const { root, launcher } = fixture();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  installFakeBinary(
    root,
    '#!/bin/sh\nprintf "cwd=%s\\nenv=%s\\n" "$PWD" "$LAUNCHER_TEST_VALUE"\nprintf "<%s>\\n" "$@"\nprintf "launcher-stderr\\n" >&2\nexit 23\n',
  );

  const result = spawnSync(process.execPath, [launcher, 'two words', '*.rs', 'semi;colon'], {
    cwd: root,
    env: { ...process.env, LAUNCHER_TEST_VALUE: 'from-parent' },
    encoding: 'utf8',
  });

  assert.equal(result.status, 23);
  assert.equal(
    result.stdout,
    `cwd=${fs.realpathSync(root)}\nenv=from-parent\n<two words>\n<*.rs>\n<semi;colon>\n`,
  );
  assert.equal(result.stderr, 'launcher-stderr\n');
});

test('npm launcher reports a missing platform binary without an install script fallback', (context) => {
  const { root, launcher } = fixture();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));

  const result = spawnSync(process.execPath, [launcher, '--version'], {
    cwd: root,
    encoding: 'utf8',
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /native package is missing/);
  assert.match(result.stderr, /optional dependencies/);
});

test('npm launcher forwards targeted termination and preserves signal status', async (context) => {
  const { root, launcher } = fixture();
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const ready = path.join(root, 'ready');
  const interrupted = path.join(root, 'interrupted');
  installFakeBinary(
    root,
    '#!/bin/sh\ntrap \'printf interrupted > "$2"; exit 0\' TERM\nprintf ready > "$1"\nwhile :; do sleep 0.05; done\n',
  );

  const child = require('node:child_process').spawn(process.execPath, [launcher, ready, interrupted], {
    cwd: root,
    stdio: 'ignore',
  });
  const deadline = Date.now() + 5000;
  while (!fs.existsSync(ready) && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  assert.ok(fs.existsSync(ready), 'native process did not become ready');

  child.kill('SIGTERM');
  const result = await new Promise((resolve) => {
    child.once('exit', (code, signal) => resolve({ code, signal }));
  });

  assert.equal(result.signal, 'SIGTERM');
  assert.equal(result.code, null);
  assert.ok(fs.existsSync(interrupted), 'native process did not receive SIGTERM');
});
