'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const test = require('node:test');

const repositoryRoot = path.resolve(__dirname, '..');
const platforms = require('./platforms.json');
const rootPackage = require('../package.json');

test('platform matrix defines launcher packages and root optional dependencies', () => {
  const expectedOptionalDependencies = {};
  for (const platform of platforms) {
    assert.equal(platform.id, `${platform.os}-${platform.cpu}`);
    assert.equal(platform.package, `${rootPackage.name}-${platform.id}`);
    expectedOptionalDependencies[platform.package] = rootPackage.version;
  }

  assert.deepEqual(rootPackage.optionalDependencies, expectedOptionalDependencies);
});

test('npm package assembly creates a self-contained launcher and platform packages', (context) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'agent-flock-package-test-'));
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const artifacts = path.join(root, 'artifacts');
  const output = path.join(root, 'npm');
  const launcherOutput = path.join(root, 'launcher');

  for (const platform of platforms) {
    const artifactDirectory = path.join(artifacts, platform.id);
    fs.mkdirSync(artifactDirectory, { recursive: true });
    fs.writeFileSync(path.join(artifactDirectory, 'agent-flock'), `binary:${platform.id}`);
  }

  const result = spawnSync(
    process.execPath,
    [
      path.join(repositoryRoot, 'npm', 'assemble-platform-packages.mjs'),
      '--artifacts',
      artifacts,
      '--output',
      output,
      '--launcher-output',
      launcherOutput,
    ],
    { encoding: 'utf8' },
  );
  assert.equal(result.status, 0, result.stderr);

  const launcher = path.join(launcherOutput, 'npm', 'agent-flock.cjs');
  const launcherSource = fs.readFileSync(launcher, 'utf8');
  assert.doesNotMatch(launcherSource, /platforms\.json/);
  for (const platform of platforms) {
    assert.ok(
      launcherSource.includes(`${JSON.stringify(platform.id)}: ${JSON.stringify(platform.package)}`),
    );
  }

  const launcherManifest = JSON.parse(
    fs.readFileSync(path.join(launcherOutput, 'package.json'), 'utf8'),
  );
  assert.deepEqual(launcherManifest, rootPackage);

  for (const platform of platforms) {
    const packageRoot = path.join(output, platform.id);
    const manifest = JSON.parse(fs.readFileSync(path.join(packageRoot, 'package.json'), 'utf8'));
    assert.equal(manifest.name, platform.package);
    assert.equal(manifest.version, rootPackage.version);
    assert.deepEqual(manifest.os, [platform.os]);
    assert.deepEqual(manifest.cpu, [platform.cpu]);
    assert.deepEqual(manifest.files, ['bin/agent-flock']);
    assert.equal(
      fs.readFileSync(path.join(packageRoot, 'bin', 'agent-flock'), 'utf8'),
      `binary:${platform.id}`,
    );
    assert.ok(fs.statSync(path.join(packageRoot, 'bin', 'agent-flock')).mode & 0o111);
  }
});
