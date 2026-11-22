// Refuse a node_modules that isn't what `npm ci` would install from
// package-lock.json. CI installs fresh with `npm ci`; the local merge gate
// (`npm run gate`) runs on whatever is already installed, so a package added,
// upgraded or left over by hand could pass locally and fail in CI. Checked:
// - package.json's dependency lists equal the lock's root entry (`npm ci`
//   refuses when they are out of sync);
// - every locked package is installed at exactly its locked version; an
//   optional one may be absent only when its os/cpu excludes this machine;
// - no folder under node_modules (nested ones too) holds a package the lock
//   doesn't name (`npm ci` would remove it, and an import of it would fail).
// Linked workspaces are skipped. It checks; it never installs.
// Overnight S2 (gate gaps), 2026-10-10.
//
//   node scripts/check-installed-lock.mjs     exit 1 listing what differs
import { existsSync, lstatSync, readdirSync, readFileSync, realpathSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const LISTS = ['dependencies', 'devDependencies', 'optionalDependencies', 'peerDependencies'];

/** Where package.json's dependency lists and the lock's root entry differ. */
export function manifestDrift(pkg, lock) {
  const root = lock.packages?.[''] ?? {};
  const problems = [];
  for (const list of LISTS) {
    const want = pkg[list] ?? {};
    const have = root[list] ?? {};
    for (const name of new Set([...Object.keys(want), ...Object.keys(have)])) {
      if (want[name] !== have[name]) {
        problems.push(`package.json ${list}.${name} is ${want[name] ?? 'absent'}, the lock has ${have[name] ?? 'absent'}`);
      }
    }
  }
  return problems;
}

/** Whether a package restricted by `os`/`cpu` (npm's lists, `!x` negations
 *  allowed) is built for another machine. */
export function otherPlatform(entry, platform = process.platform, arch = process.arch) {
  const excludes = (list, value) => {
    if (!Array.isArray(list) || list.length === 0) return false;
    if (list.includes(`!${value}`)) return true;
    const positive = list.filter(x => !x.startsWith('!'));
    return positive.length > 0 && !positive.includes(value);
  };
  return excludes(entry.os, platform) || excludes(entry.cpu, arch);
}

/** What differs between `lock` (package-lock.json, v2/v3) and what
 *  `installed(path)` reports: `null` for an absent package, else its
 *  package.json `version` (`undefined` when it has none). */
export function lockDrift(lock, installed, platform = process.platform, arch = process.arch) {
  const problems = [];
  for (const [path, entry] of Object.entries(lock.packages ?? {})) {
    if (!path || entry.link) continue;
    const version = installed(path);
    if (version === null) {
      const optional = entry.optional || entry.devOptional;
      if (!(optional && otherPlatform(entry, platform, arch))) problems.push(`${path} is missing (locked ${entry.version})`);
      continue;
    }
    if (version !== entry.version) problems.push(`${path} is ${version ?? 'unversioned'}, the lock says ${entry.version ?? 'unversioned'}`);
  }
  return problems;
}

/** Installed package paths (`node_modules/x`, `node_modules/x/node_modules/y`)
 *  the lock doesn't name. `children(path)` lists the package names in
 *  `path/node_modules` ("" is the project root). Only locked packages'
 *  folders are walked, so nothing is visited twice. */
export function extraneous(lock, children) {
  const locked = new Set(Object.keys(lock.packages ?? {}));
  const problems = [];
  // The lock's own root entry is "": walk the root once.
  for (const base of ['', ...[...locked].filter(Boolean)]) {
    if (base && lock.packages[base]?.link) continue;
    for (const name of children(base)) {
      const path = base ? `${base}/node_modules/${name}` : `node_modules/${name}`;
      if (!locked.has(path)) problems.push(`${path} is installed but not in the lock`);
    }
  }
  return problems;
}

/** The package names under `root/<base>/node_modules` (scopes expanded, dot
 *  entries skipped). */
export function childrenAt(root) {
  return base => {
    const dir = join(root, base, 'node_modules');
    if (!existsSync(dir)) return [];
    const names = [];
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.name.startsWith('.')) continue;
      if (entry.name.startsWith('@') && entry.isDirectory()) {
        for (const inner of readdirSync(join(dir, entry.name))) if (!inner.startsWith('.')) names.push(`${entry.name}/${inner}`);
      } else if (entry.isDirectory() || entry.isSymbolicLink()) names.push(entry.name);
    }
    return names;
  };
}

/** The installed version at `root/path`: null when absent, undefined when
 *  its package.json has no version. */
export function installedAt(root) {
  return path => {
    const file = join(root, path, 'package.json');
    if (!existsSync(file)) return null;
    try {
      return JSON.parse(readFileSync(file, 'utf8')).version;
    } catch {
      return null;
    }
  };
}

const isMain = () => {
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
};

if (isMain()) {
  const root = fileURLToPath(new URL('..', import.meta.url));
  const lock = JSON.parse(readFileSync(join(root, 'package-lock.json'), 'utf8'));
  if (!lock.packages) {
    console.error('check-installed-lock: package-lock.json has no "packages" (lockfile v1); run npm install to upgrade it.');
    process.exit(1);
  }
  const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
  const problems = [...manifestDrift(pkg, lock), ...lockDrift(lock, installedAt(root)), ...extraneous(lock, childrenAt(root))];
  if (problems.length) {
    const modules = join(root, 'node_modules');
    const shared = existsSync(modules) && lstatSync(modules).isSymbolicLink()
      ? `\nnode_modules here is a symlink to ${realpathSync(modules)}: other checkouts share that tree, so reinstalling changes theirs too.`
      : '';
    console.error(`check-installed-lock: node_modules differs from package-lock.json (CI's npm ci would install something else):\n  ${problems.slice(0, 20).join('\n  ')}${problems.length > 20 ? `\n  …and ${problems.length - 20} more` : ''}\nRun npm ci (or npm install if you meant to change the lock); in a fresh worktree follow it with npm rebuild --ignore-scripts.${shared}`);
    process.exit(1);
  }
  console.log('check-installed-lock: ok');
}
