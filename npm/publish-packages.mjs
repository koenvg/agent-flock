#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

function requiredPathOption(name) {
  const index = process.argv.indexOf(name);
  if (index === -1 || !process.argv[index + 1]) {
    throw new Error(`${name} is required`);
  }
  return path.resolve(process.argv[index + 1]);
}

function publishPackage(packageDirectory) {
  const { name, version } = JSON.parse(
    fs.readFileSync(path.join(packageDirectory, 'package.json'), 'utf8'),
  );
  const packageSpec = `${name}@${version}`;
  const view = spawnSync('npm', ['view', packageSpec, 'version'], { stdio: 'ignore' });

  if (view.status === 0) {
    console.log(`${packageSpec} is already published`);
    return 0;
  }

  const publish = spawnSync('npm', ['publish', packageDirectory, '--access', 'public'], {
    stdio: 'inherit',
  });
  if (publish.error) {
    throw publish.error;
  }
  return publish.status ?? 1;
}

function platformPackageDirectories(root) {
  const entries = fs
    .readdirSync(root)
    .filter((entry) => !entry.startsWith('.'))
    .sort()
    .map((entry) => path.join(root, entry));
  if (entries.length === 0) {
    throw new Error(`no platform packages found in ${root}`);
  }
  return entries;
}

function main() {
  const platformPackages = requiredPathOption('--platform-packages');
  const launcherPackage = requiredPathOption('--launcher-package');

  for (const packageDirectory of platformPackageDirectories(platformPackages)) {
    const status = publishPackage(packageDirectory);
    if (status !== 0) {
      return status;
    }
  }

  return publishPackage(launcherPackage);
}

try {
  process.exitCode = main();
} catch (error) {
  console.error(`agent-flock publishing: ${error.message}`);
  process.exitCode = 1;
}
