import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { BUDGETS, KB, LAUNCH_LIMIT, checkBudgets, checkLaunch } from './check-bundle-budget.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const script = join(root, 'scripts', 'check-bundle-budget.mjs');

function sourceBasenames(dir, out = new Set()) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) sourceBasenames(join(dir, entry.name), out);
    else out.add(entry.name.replace(/\.(tsx?|jsx?|png|jpe?g)$/, ''));
  }
  return out;
}

const healthy = () => [
  { name: 'index-abc.css', size: 200 * KB },
  { name: 'App-abc.css', size: 84 * KB },
  { name: 'App--abc.js', size: 321 * KB },
  { name: 'MarkdownEditor-abc.js', size: 542 * KB },
  { name: 'MissionControl-abc.js', size: 12 * KB },
  { name: 'index-abc.js', size: 150 * KB },
  { name: 'hollow-doorway-landscape-abc.jpg', size: 791 * KB },
  { name: 'hollow-mark-96-abc.png', size: 8 * KB },
];

test('every named-chunk budget names a module that still exists in src/', () => {
  // Vite names a lazy chunk after its module, so `Vault-*`/`Graph-*` budgets
  // outliving src/pages/Vault.tsx and Graph.tsx checked nothing.
  const names = sourceBasenames(join(root, 'src'));
  for (const rule of BUDGETS.filter(b => b.prefix && !b.asset)) {
    const module = rule.prefix.replace(/-$/, '');
    assert.ok(names.has(module), `${rule.name} budget (${rule.prefix}*) has no src module named ${module}`);
  }
});

test('a healthy build passes', () => {
  assert.deepEqual(checkBudgets(healthy()), []);
});

test('a named budget whose chunk is gone fails instead of passing silently', () => {
  const assets = healthy().filter(f => !f.name.startsWith('MarkdownEditor-'));
  const failures = checkBudgets(assets);
  assert.equal(failures.length, 1);
  assert.match(failures[0], /editor budget matches no asset/);
});

test('oversized CSS and App chunks fail', () => {
  const assets = healthy();
  assets[0].size = 345 * KB;
  assets[2].size = 341 * KB;
  const failures = checkBudgets(assets);
  assert.equal(failures.length, 2);
  assert.match(failures[0], /index-abc\.css .* > css budget/);
  assert.match(failures[1], /App--abc\.js .* > app budget/);
});

test('the CLI reads the assets folder it is given and exits non-zero on a breach', () => {
  const dir = mkdtempSync(join(tmpdir(), 'bundle-budget-'));
  try {
    writeFileSync(join(dir, 'App--x.js'), Buffer.alloc(10 * KB));
    writeFileSync(join(dir, 'MarkdownEditor-x.js'), Buffer.alloc(10 * KB));
    writeFileSync(join(dir, 'MissionControl-x.js'), Buffer.alloc(10 * KB));
    writeFileSync(join(dir, 'index-x.css'), Buffer.alloc(10 * KB));
    writeFileSync(join(dir, 'hollow-mark-96-x.png'), Buffer.alloc(4 * KB));
    assert.match(execFileSync(process.execPath, [script, dir], { encoding: 'utf8' }), /bundle-budget: ok/);

    writeFileSync(join(dir, 'App--x.js'), Buffer.alloc(341 * KB));
    assert.throws(
      () => execFileSync(process.execPath, [script, dir], { stdio: 'pipe' }),
      err => err.status === 1 && /app budget/.test(String(err.stderr)),
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a bundled image over 1 MB fails, so the 3.19 MB Home PNG cannot come back (five asks B4)', () => {
  const assets = healthy();
  assets.find(f => f.name.endsWith('.jpg')).size = 3116 * KB;
  const failures = checkBudgets(assets);
  assert.equal(failures.length, 1);
  assert.match(failures[0], /hollow-doorway-landscape-abc\.jpg .* > image budget/);
});

test('the rail mark stays a small copy, not the 688 KB icon master', () => {
  const assets = healthy();
  assets.push({ name: 'hollow-doorway-studio-v2-abc.png', size: 672 * KB });
  assets.find(f => f.name.startsWith('hollow-mark-96-')).size = 672 * KB;
  const failures = checkBudgets(assets);
  assert.equal(failures.length, 1);
  assert.match(failures[0], /hollow-mark-96-abc\.png .* > rail-mark budget/);
  assert.match(checkBudgets(healthy().filter(f => !f.name.startsWith('hollow-mark-96-')))[0], /rail-mark budget matches no asset/);
});

test('the launch set counts index, App, its preload group and static imports, not lazy chunks', () => {
  const dist = mkdtempSync(join(tmpdir(), 'bundle-launch-'));
  try {
    const assets = join(dist, 'assets');
    mkdirSync(assets);
    writeFileSync(join(dist, 'index.html'), '<script type="module" crossorigin src="/assets/index-a.js"></script>\n<link rel="stylesheet" crossorigin href="/assets/index-a.css">');
    writeFileSync(join(assets, 'index-a.js'), 'const __vite__mapDeps=(i,m=__vite__mapDeps,d=(m.f||(m.f=["assets/App-a.js","assets/ui-a.js","assets/App-a.css","assets/Lazy-a.js"])))=>i.map(i=>d[i]);\nj(()=>import("./App-a.js").then(s=>s.kf),__vite__mapDeps([0,1,2])).then(s=>{});j(()=>import("./Lazy-a.js"),__vite__mapDeps([3]))');
    writeFileSync(join(assets, 'index-a.css'), 'a'.repeat(100));
    writeFileSync(join(assets, 'App-a.js'), 'import{a as b}from"./ui-a.js";import"./api-a.js";const x=()=>import("./Settings-a.js");' + ' '.repeat(1000));
    writeFileSync(join(assets, 'App-a.css'), 'b'.repeat(200));
    writeFileSync(join(assets, 'ui-a.js'), 'export const a=1;');
    writeFileSync(join(assets, 'api-a.js'), 'export const c=1;');
    writeFileSync(join(assets, 'Settings-a.js'), ' '.repeat(50 * KB));
    writeFileSync(join(assets, 'Lazy-a.js'), ' '.repeat(50 * KB));
    const ok = checkLaunch(dist);
    assert.deepEqual(ok.files.sort(), ['App-a.css', 'App-a.js', 'api-a.js', 'index-a.css', 'index-a.js', 'ui-a.js']);
    assert.deepEqual(ok.failures, []);
    assert.equal(LAUNCH_LIMIT, 800 * KB);
    // No preload group (Vite omits it when App has no deps): App still counts.
    writeFileSync(join(assets, 'index-a.js'), 'import("./App-a.js").then(s=>s.App)');
    assert.ok(checkLaunch(dist).files.includes('App-a.js'));
    writeFileSync(join(assets, 'index-a.js'), 'import("./Other-a.js")');
    assert.throws(() => checkLaunch(dist), /imports no App-\*\.js chunk/);
    writeFileSync(join(dist, 'index.html'), '<div id="root"></div>');
    assert.throws(() => checkLaunch(dist), /names no \/assets\/\*\.js entry/);
    writeFileSync(join(dist, 'index.html'), '<script type="module" crossorigin src="/assets/index-a.js"></script>');
    writeFileSync(join(assets, 'index-a.js'), 'import("./App-a.js")');
    const over = checkLaunch(dist, 1 * KB);
    assert.match(over.failures[0], /launch set .* > launch budget 1\.0 KB/);
  } finally {
    rmSync(dist, { recursive: true, force: true });
  }
});
