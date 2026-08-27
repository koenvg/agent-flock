'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const test = require('node:test');

const publishScript = path.join(__dirname, 'publish-packages.mjs');

function writePackage(directory, name, version = '1.2.3') {
  fs.mkdirSync(directory, { recursive: true });
  fs.writeFileSync(path.join(directory, 'package.json'), JSON.stringify({ name, version }));
}

function fixture(context) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'agent-flock-publish-test-'));
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));

  const platformPackages = path.join(root, 'platform-packages');
  const launcherPackage = path.join(root, 'launcher-package');
  writePackage(path.join(platformPackages, 'zeta'), '@koenvg/platform-zeta');
  writePackage(path.join(platformPackages, 'alpha'), '@koenvg/platform-alpha');
  writePackage(launcherPackage, '@koenvg/launcher');

  const bin = path.join(root, 'bin');
  const log = path.join(root, 'npm-calls.jsonl');
  fs.mkdirSync(bin);
  fs.writeFileSync(
    path.join(bin, 'npm'),
    `#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.FAKE_NPM_LOG, JSON.stringify(args) + '\\n');
if (args[0] === 'view') {
  const existing = JSON.parse(process.env.FAKE_NPM_EXISTING || '[]');
  process.exit(existing.includes(args[1]) ? 0 : 1);
}
if (args[0] === 'publish') {
  const manifest = require(path.join(path.resolve(args[1]), 'package.json'));
  if (manifest.name === process.env.FAKE_NPM_FAIL_PUBLISH) process.exit(42);
  process.exit(0);
}
process.exit(2);
`,
    { mode: 0o755 },
  );

  return { root, platformPackages, launcherPackage, bin, log };
}

function runPublisher(fixturePaths, environment = {}) {
  return spawnSync(
    process.execPath,
    [
      publishScript,
      '--platform-packages',
      fixturePaths.platformPackages,
      '--launcher-package',
      fixturePaths.launcherPackage,
    ],
    {
      encoding: 'utf8',
      env: {
        ...process.env,
        PATH: `${fixturePaths.bin}${path.delimiter}${process.env.PATH}`,
        FAKE_NPM_LOG: fixturePaths.log,
        ...environment,
      },
    },
  );
}

function npmCalls(log) {
  return fs
    .readFileSync(log, 'utf8')
    .trim()
    .split('\n')
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

test('publisher skips existing packages and publishes missing platforms before the launcher', (context) => {
  const paths = fixture(context);
  const result = runPublisher(paths, {
    FAKE_NPM_EXISTING: JSON.stringify(['@koenvg/platform-alpha@1.2.3']),
  });

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /@koenvg\/platform-alpha@1\.2\.3 is already published/);
  assert.deepEqual(npmCalls(paths.log), [
    ['view', '@koenvg/platform-alpha@1.2.3', 'version'],
    ['view', '@koenvg/platform-zeta@1.2.3', 'version'],
    ['publish', path.join(paths.platformPackages, 'zeta'), '--access', 'public'],
    ['view', '@koenvg/launcher@1.2.3', 'version'],
    ['publish', paths.launcherPackage, '--access', 'public'],
  ]);
});

test('publisher stops before the launcher when a platform publish fails', (context) => {
  const paths = fixture(context);
  const result = runPublisher(paths, {
    FAKE_NPM_FAIL_PUBLISH: '@koenvg/platform-alpha',
  });

  assert.equal(result.status, 42);
  assert.deepEqual(npmCalls(paths.log), [
    ['view', '@koenvg/platform-alpha@1.2.3', 'version'],
    ['publish', path.join(paths.platformPackages, 'alpha'), '--access', 'public'],
  ]);
});
