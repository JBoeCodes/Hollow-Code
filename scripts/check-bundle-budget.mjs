#!/usr/bin/env node
// Raw-size caps on the production bundle. Run after `npm run build`; CI runs it
// in the frontend job right after the build.
//
// Every named-chunk budget must match at least one built asset. Before
// 2026-10-02 the script still capped `Vault-*.js` and `Graph-*.js`, chunks that
// stopped existing when f8427bb1 deleted those pages, so two of its four checks
// passed by matching nothing. A budget whose chunk is gone now fails the run
// (rename or drop the budget in the same change that moves the chunk).
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { launchSet } from './perf-report.mjs';

export const KB = 1024;
export const IMAGE_EXTS = ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.avif'];

export const BUDGETS = [
  // Every CSS asset. Raised 340 → 344 with the 2026-08 overhaul (schedule
  // picker, Notion sync chip, capture slot, overflow menus); the wave also
  // deleted dead Today CSS, so this is the new honest floor, not drift.
  { name: 'css', ext: '.css', limit: 344 * KB },
  // The shell chunk. 229 KB after next-level P21 (2026-10-01): anything not on
  // screen at launch goes behind `OnceOpened`, not a static import in App.tsx.
  // Raised 240 → 340 when the browser mock left the production build (merged
  // plan slice 6). No code was added to the launch path: SmokeHarness used to
  // share ~99 KB of shell modules (store/runtime, navigation, HtmlDoc, chat
  // helpers) with App, so rollup put them in a separate eager chunk; with the
  // harness gone they fold into App. Launch JS went from index 147.5 + App
  // 228.9 + shared 99.3 KB to index 30.2 + App 334.0 KB.
  // Lowered 340 → 330 by overnight P3 (2026-10-12): the confirm dialog and the
  // activity dock load when first shown, and the launch path imports the small
  // streak and task-lock helpers instead of whole modules (338.6 → 320.8 KB).
  // Raised 330 → 340 by consolidation D/E (2026-10-13): the shell now hosts
  // the destinations that replaced six overlays (Your work's view strip,
  // Review's rail row, the one Ask button, origin labels for same-named
  // workspaces, visible "⋯" menus). All of it is on screen at launch; the
  // views themselves stay lazy. 326.7 → 336.8 KB; launch set 747 of 800 KB.
  { name: 'app', prefix: 'App-', ext: '.js', limit: 340 * KB },
  // The lazy CodeMirror editor (src/editor/MarkdownEditor.tsx), the largest
  // chunk (541.6 KB on 2026-10-02). It took over the editor weight the old
  // 650 KB `Vault-*` cap used to guard.
  { name: 'editor', prefix: 'MarkdownEditor-', ext: '.js', limit: 600 * KB },
  // The workspace map, Runbooks, welcome tour, Present and What's new chunks
  // left with those screens (overnight 11-08 S4).
  // Mission Control (overnight 10-11 B2): cards and a CSS-only orbit, no
  // canvas or library. 22 → 26 KB with run 15 P3 (25.1 KB): src/missionControl.ts
  // (about 4.4 KB) was its own chunk while the Main agent drawer's turn list
  // also imported it; with the drawer removed it is inlined here. No code was
  // added: opening Mission Control loads the same bytes in one file, not two.
  // 26 → 30 KB with P1a to P1c (2026-11-15): Pause and Resume cards, the
  // paused row and Running's Pause all and Resume at controls (26.7 KB after
  // P1a; the resume-at picker itself is its own lazy chunk).
  { name: 'mission-control', prefix: 'MissionControl-', ext: '.js', limit: 30 * KB },
  // The triage inbox chunk went with Needs you Incoming (overnight 11-08 S3).
  // Every bundled image (five asks B4). The Home artwork was a 3.19 MB PNG
  // decoded at every launch; it ships as a 791 KB JPEG of the same pixels now
  // (docs/DOORWAY_LANDSCAPE.md). Re-encode before raising this.
  { name: 'image', exts: IMAGE_EXTS, limit: 1024 * KB },
  // The rail's 18–22 px mark: a 96 px copy (7.5 KB), not the 1254 px,
  // 688 KB master the native icons are made from (docs/BRAND_DOORWAY.md).
  { name: 'rail-mark', prefix: 'hollow-mark-96-', exts: ['.png'], limit: 16 * KB },
];

// What a cold launch of the main window loads, JS + CSS raw bytes (five asks
// B4): index + App + App's preload group + their static imports, as
// scripts/perf-report.mjs computes it. 691.5 KB on 2026-10-10. Anything not
// on screen at launch belongs behind a lazy import, not in this set.
export const LAUNCH_LIMIT = 800 * KB;

export function listAssets(dir) {
  return readdirSync(dir)
    .map(name => ({ name, size: statSync(join(dir, name)).size }))
    .filter(f => /\.(js|css|md)$/.test(f.name) || IMAGE_EXTS.some(e => f.name.endsWith(e)))
    .sort((a, b) => b.size - a.size);
}

export function fmt(bytes) {
  return `${(bytes / KB).toFixed(1)} KB`;
}

const matches = (rule, name) => (rule.exts ?? [rule.ext]).some(e => name.endsWith(e)) && (!rule.prefix || name.startsWith(rule.prefix));

// Returns the list of failures (empty when every budget holds).
export function checkBudgets(assets, budgets = BUDGETS) {
  const failures = [];
  for (const rule of budgets) {
    const hits = assets.filter(f => matches(rule, f.name));
    if (rule.prefix && hits.length === 0) {
      failures.push(`${rule.name} budget matches no asset (${rule.prefix}*${rule.ext ?? rule.exts.join('|')}): the chunk was renamed or deleted, so update the budget`);
    }
    for (const f of hits) {
      if (f.size > rule.limit) failures.push(`${f.name} ${fmt(f.size)} > ${rule.name} budget ${fmt(rule.limit)}`);
    }
  }
  return failures;
}

/** The launch-set check over a built dist/ (index.html next to assets/).
 * Returns { bytes, files, failures }. */
export function checkLaunch(distDir, limit = LAUNCH_LIMIT) {
  const assets = join(distDir, 'assets');
  const set = launchSet(readFileSync(join(distDir, 'index.html'), 'utf8'), name => readFileSync(join(assets, name), 'utf8'));
  const files = [...set.js, ...set.css];
  const bytes = files.reduce((n, f) => n + statSync(join(assets, f)).size, 0);
  const failures = bytes > limit ? [`launch set ${fmt(bytes)} > launch budget ${fmt(limit)} (${files.length} files: ${files.join(', ')})`] : [];
  return { bytes, files, failures };
}

function main(argv) {
  const root = fileURLToPath(new URL('..', import.meta.url));
  const assetsDir = argv[0] ?? join(root, 'dist', 'assets');
  let assets;
  try {
    assets = listAssets(assetsDir);
  } catch {
    console.error(`bundle-budget: ${assetsDir} not found. Run npm run build first.`);
    process.exit(1);
  }

  console.log('bundle-budget: largest assets');
  for (const f of assets.slice(0, 12)) console.log(`  ${fmt(f.size).padStart(9)}  ${f.name}`);

  const failures = checkBudgets(assets);
  const dist = join(assetsDir, '..');
  if (existsSync(join(dist, 'index.html'))) {
    const launch = checkLaunch(dist);
    console.log(`bundle-budget: launch set ${fmt(launch.bytes)} in ${launch.files.length} files (budget ${fmt(LAUNCH_LIMIT)})`);
    failures.push(...launch.failures);
  } else {
    console.log('bundle-budget: launch check skipped (no index.html next to the assets folder)');
  }
  if (failures.length) {
    console.error('\nbundle-budget: failed');
    for (const f of failures) console.error(`  ${f}`);
    process.exit(1);
  }
  console.log('bundle-budget: ok');
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  main(process.argv.slice(2));
}
