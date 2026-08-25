#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const npmDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(npmDirectory, '..');
const platforms = JSON.parse(fs.readFileSync(path.join(npmDirectory, 'platforms.json'), 'utf8'));
const rootPackage = JSON.parse(fs.readFileSync(path.join(repositoryRoot, 'package.json'), 'utf8'));
const platformLookupStart = '// BEGIN PLATFORM PACKAGE LOOKUP';
const platformLookupEnd = '// END PLATFORM PACKAGE LOOKUP';

function option(name) {
  const index = process.argv.indexOf(name);
  if (index === -1 || !process.argv[index + 1]) {
    throw new Error(`${name} is required`);
  }
  return path.resolve(process.argv[index + 1]);
}

function createOutputDirectory(output) {
  if (fs.existsSync(output) && fs.readdirSync(output).length > 0) {
    throw new Error(`output directory is not empty: ${output}`);
  }
  fs.mkdirSync(output, { recursive: true });
}

function publishedLauncher() {
  const launcherPath = path.join(npmDirectory, 'agent-flock.cjs');
  const source = fs.readFileSync(launcherPath, 'utf8');
  const lookupStart = source.indexOf(platformLookupStart);
  const lookupEnd = source.indexOf(platformLookupEnd, lookupStart + platformLookupStart.length);
  if (lookupStart === -1 || lookupEnd === -1) {
    throw new Error(`launcher platform lookup markers were not found in ${launcherPath}`);
  }

  const platformPackages = Object.fromEntries(
    platforms.map(({ id, package: packageName }) => [id, packageName]),
  );
  const embeddedLookup = `\nconst platformPackages = ${JSON.stringify(platformPackages, null, 2)};\n`;
  return (
    source.slice(0, lookupStart + platformLookupStart.length) +
    embeddedLookup +
    source.slice(lookupEnd)
  );
}

function assembleLauncherPackage(output) {
  createOutputDirectory(output);
  const launcher = path.join(output, 'npm', 'agent-flock.cjs');
  fs.mkdirSync(path.dirname(launcher), { recursive: true });
  fs.writeFileSync(launcher, publishedLauncher(), { mode: 0o755 });
  for (const filename of ['LICENSE', 'README.md', 'package.json']) {
    fs.copyFileSync(path.join(repositoryRoot, filename), path.join(output, filename));
  }
}

function platformManifest(platform) {
  return {
    name: platform.package,
    version: rootPackage.version,
    description: `Native agent-flock binary for ${platform.id}`,
    license: rootPackage.license,
    repository: rootPackage.repository,
    os: [platform.os],
    cpu: [platform.cpu],
    files: ['bin/agent-flock'],
    publishConfig: { access: 'public', provenance: true },
  };
}

function main() {
  const artifacts = option('--artifacts');
  const output = option('--output');
  const launcherOutput = option('--launcher-output');
  createOutputDirectory(output);
  assembleLauncherPackage(launcherOutput);

  for (const platform of platforms) {
    const source = path.join(artifacts, platform.id, 'agent-flock');
    if (!fs.existsSync(source)) {
      throw new Error(`missing release binary: ${source}`);
    }

    const packageRoot = path.join(output, platform.id);
    const binary = path.join(packageRoot, 'bin', 'agent-flock');
    fs.mkdirSync(path.dirname(binary), { recursive: true });
    fs.copyFileSync(source, binary);
    fs.chmodSync(binary, 0o755);
    fs.writeFileSync(
      path.join(packageRoot, 'package.json'),
      `${JSON.stringify(platformManifest(platform), null, 2)}\n`,
    );
  }

  console.log(
    `assembled launcher in ${launcherOutput} and ${platforms.length} platform packages in ${output}`,
  );
}

try {
  main();
} catch (error) {
  console.error(`agent-flock packaging: ${error.message}`);
  process.exitCode = 1;
}
