#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const npmDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(npmDirectory, '..');
const platforms = JSON.parse(fs.readFileSync(path.join(npmDirectory, 'platforms.json'), 'utf8'));
const rootPackage = JSON.parse(fs.readFileSync(path.join(repositoryRoot, 'package.json'), 'utf8'));

function option(name) {
  const index = process.argv.indexOf(name);
  if (index === -1 || !process.argv[index + 1]) {
    throw new Error(`${name} is required`);
  }
  return path.resolve(process.argv[index + 1]);
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
  if (fs.existsSync(output) && fs.readdirSync(output).length > 0) {
    throw new Error(`output directory is not empty: ${output}`);
  }
  fs.mkdirSync(output, { recursive: true });

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

  console.log(`assembled ${platforms.length} platform packages in ${output}`);
}

try {
  main();
} catch (error) {
  console.error(`agent-flock packaging: ${error.message}`);
  process.exitCode = 1;
}
