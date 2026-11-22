#!/usr/bin/env node
// Review F05 (2026-10-13): every native event subscription goes through
// src/nativeEvents.ts, which has a browser-mock bus. A direct import of
// `@tauri-apps/api/event` elsewhere would bring back the full-app mock's
// `transformCallback` TypeError. Tests may import it (they mock it).
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..');
const ADAPTER = 'src/nativeEvents.ts';

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) walk(full, out);
    else if (/\.(ts|tsx)$/.test(name) && !/\.test\.tsx?$/.test(name)) out.push(full);
  }
  return out;
}

/** Files (relative to the repo) that import Tauri's event module directly. */
export function directEventImports(files, read = f => readFileSync(f, 'utf8'), root = ROOT) {
  return files
    .map(f => relative(root, f))
    .filter(rel => rel !== ADAPTER)
    .filter(rel => /from\s+['"]@tauri-apps\/api\/event['"]/.test(read(join(root, rel))));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const bad = directEventImports(walk(join(ROOT, 'src')));
  if (bad.length) {
    console.error(`Import listen/onNative from src/nativeEvents.ts, not @tauri-apps/api/event:\n  ${bad.join('\n  ')}`);
    process.exit(1);
  }
  console.log('native events: every subscription goes through src/nativeEvents.ts');
}
export { walk, ROOT };
