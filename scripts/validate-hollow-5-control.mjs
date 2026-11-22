import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkVersions } from './version.mjs';

const root = resolve(fileURLToPath(new URL('..', import.meta.url)));
const read = (path) => readFileSync(resolve(root, path), 'utf8');

const packageManifest = JSON.parse(read('package.json'));

// One version source: package.json, which tauri.conf.json references and
// scripts/version.mjs mirrors into the rest.
const versionProblems = checkVersions(root);
if (versionProblems.length) {
  throw new Error(
    `release version drift: ${versionProblems.join('; ')} (fix with node scripts/version.mjs bump <x.y.z>)`,
  );
}

console.log(`release version: ${packageManifest.version}`);
