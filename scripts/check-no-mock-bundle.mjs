#!/usr/bin/env node
// Fails a production build that still carries the browser mock (merged plan
// slice 6, hygiene). `src/mockMode.ts` makes `MOCK_BUILD` the literal `false`
// outside `vite dev` and `vite build --mode mock`, so `mockApi.ts`, `mockDemo.ts`
// and their fixtures should be tree-shaken out of `dist/`. These strings exist
// only in those modules; finding one in a shipped chunk means a static import
// or an unguarded branch pulled the mock back in.
//
// Runs at the end of `npm run build` (so CI and every `tauri build` check it).
// `npm run build:mock` deliberately skips it.
import { readdirSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const MOCK_MARKERS = ['__mockApiMisses', '__mockApiCalls', 'mockApi: no fixture', '__mockDemoTasks'];

/** `[{ file, marker }]` for every built `.js` asset under `dir` that carries a mock marker. */
export function findMockMarkers(dir) {
  const hits = [];
  const walk = d => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const path = join(d, e.name);
      if (e.isDirectory()) walk(path);
      else if (/\.m?js$/.test(e.name)) {
        const text = readFileSync(path, 'utf8');
        for (const marker of MOCK_MARKERS) if (text.includes(marker)) hits.push({ file: path, marker });
      }
    }
  };
  walk(dir);
  return hits;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('..', import.meta.url));
  const dist = process.argv[2] ? resolve(process.argv[2]) : join(root, 'dist');
  let hits;
  try {
    hits = findMockMarkers(dist);
  } catch (e) {
    console.error(`check-no-mock-bundle: cannot read ${dist}: ${e.message}`);
    process.exit(1);
  }
  if (hits.length) {
    console.error('check-no-mock-bundle: the production bundle contains the browser mock:');
    for (const h of hits) console.error(`  ${h.file}: ${JSON.stringify(h.marker)}`);
    console.error('Guard the import with MOCK_BUILD (src/mockMode.ts); never import mockApi statically.');
    process.exit(1);
  }
  console.log('check-no-mock-bundle: no browser mock in dist/');
}
