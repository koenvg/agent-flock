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

test('platform package assembly creates publishable packages from release binaries', (context) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'agent-flock-package-test-'));
  context.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const artifacts = path.join(root, 'artifacts');
  const output = path.join(root, 'npm');

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
    ],
    { encoding: 'utf8' },
  );
  assert.equal(result.status, 0, result.stderr);

  const expectedOptionalDependencies = {};
  for (const platform of platforms) {
    expectedOptionalDependencies[platform.package] = rootPackage.version;
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

  assert.deepEqual(rootPackage.optionalDependencies, expectedOptionalDependencies);
});
