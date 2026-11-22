import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  AGENTS_DIR,
  ALLOWED,
  ENTRY_MAX_BYTES,
  checkClaudeAnchors,
  checkClaudePaths,
  checkManualAnchors,
  checkManualPaths,
  cleanToken,
  headingAnchors,
  linkingDocs,
  listRepoPaths,
  manualDocs,
  resolveLink,
} from './check-claude-paths.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const agents = () => readFileSync(join(root, 'AGENTS.md'), 'utf8');

function repoOf(files) {
  const dirs = new Set();
  for (const f of files) {
    const parts = f.split('/');
    for (let i = 1; i < parts.length; i++) dirs.add(parts.slice(0, i).join('/'));
  }
  return { files: new Set(files), dirs };
}

const fake = repoOf([
  'README.md',
  'docs/WORKSPACES.md',
  'scripts/version.mjs',
  'src/components/Rail.tsx',
  'src/styles/ui.css',
  'src-tauri/src/chat.rs',
  'src-tauri/src/lib.rs',
  'src-tauri/icons/gen_tray_template.py',
]);
const check = (markdown, allowed = new Map()) => checkClaudePaths({ markdown, repo: fake, allowed });

test('AGENTS.md, docs/agents/*.md and docs/reference/*.md name only repository paths that exist', () => {
  const docs = manualDocs(root);
  assert.ok(Object.keys(docs).length > 10, 'the reference docs are read');
  assert.ok(Object.keys(docs).some((f) => f.startsWith(`${AGENTS_DIR}/`)), 'the area contracts are read');
  assert.deepEqual(checkManualPaths({ docs, repo: listRepoPaths(root) }), []);
});

test('AGENTS.md, README.md, docs/ and the manual link only to manual headings that exist', () => {
  const docs = manualDocs(root);
  const sources = linkingDocs(root);
  assert.deepEqual(checkClaudeAnchors({ markdown: agents(), sources: { ...sources, ...docs } }), []);
  assert.deepEqual(checkManualAnchors({ docs, sources: { ...sources, ...docs } }), []);
});

test('AGENTS.md stays a short entry point that links every docs/agents and reference doc', () => {
  const docs = manualDocs(root);
  const size = Buffer.byteLength(agents());
  assert.ok(size < ENTRY_MAX_BYTES, `AGENTS.md is ${size} bytes; move detail to docs/agents/ or docs/reference/`);
  for (const path of Object.keys(docs)) {
    if (path !== 'AGENTS.md') assert.ok(agents().includes(`](${path})`), `AGENTS.md doesn't link ${path}`);
  }
});

test('root CLAUDE.md only includes AGENTS.md', () => {
  assert.equal(readFileSync(join(root, 'CLAUDE.md'), 'utf8').trim(), '@AGENTS.md');
});

test('a deleted module or a moved file named in AGENTS.md fails', () => {
  const problems = check('Calendar lived in `calendar.rs` and `src/pages/Today.tsx`; see [docs](docs/GONE.md).');
  assert.equal(problems.length, 3, problems.join('\n'));
  assert.match(problems[0], /`calendar\.rs` does not exist/);
  assert.match(problems[1], /`src\/pages\/Today\.tsx` does not exist/);
  assert.match(problems[2], /`docs\/GONE\.md` does not exist/);
});

test('rooted paths must exist exactly; bare and partial source names match a path suffix', () => {
  assert.deepEqual(check('`src-tauri/src/chat.rs`, `chat.rs`, `components/Rail.tsx`, `styles/ui.css`, `lib.rs::setup`'), []);
  // A rooted path is not rescued by a suffix match elsewhere.
  assert.equal(check('`src/chat.rs`').length, 1);
  assert.deepEqual(check('[contract](docs/WORKSPACES.md#build-workspaces), `scripts/`, `README.md`'), []);
});

test('fenced trees match source names and folders by suffix, and ignore prose', () => {
  const tree = ['```', 'src-tauri/', '├── src/lib.rs   setup', '├── icons/       gen_tray_template.py', '(Rail.tsx: the rail; back/forward)', '```'].join('\n');
  assert.deepEqual(check(tree), []);
  assert.equal(check('```\n├── src/missing.rs\n```').length, 1);
});

test('placeholders, globs, URLs, home and absolute paths and runtime files are not repository paths', () => {
  const md = [
    '`vault/tasks/{slug}.md` `workspaces/<id>/` `src/**/*.test.ts` `~/Hollow/Worktrees`',
    '`/Applications/Hollow.app` `budgets.json` `WORKSPACE.md` `.hollow/results.json`',
    '[site](https://example.com/a.ts) `$HOLLOW_SCRATCH_DIR/x.rs` `App-*.js`',
  ].join('\n');
  assert.deepEqual(check(md), []);
});

test('cleanToken strips prose punctuation, possessives, Rust paths and anchors', () => {
  assert.equal(cleanToken('(Rail.tsx:'), 'Rail.tsx');
  assert.equal(cleanToken('store/topBar.ts\'s'), 'store/topBar.ts');
  assert.equal(cleanToken('lib.rs::setup'), 'lib.rs');
  assert.equal(cleanToken('docs/A.md#x'), 'docs/A.md');
  assert.equal(cleanToken('./config.json'), 'config.json');
});

test('an ALLOWED entry must still be named and must not exist', () => {
  const allowed = new Map([['src/pages/MyPage.tsx', 'example']]);
  assert.deepEqual(check('Create `src/pages/MyPage.tsx`.', allowed), []);
  assert.match(check('Nothing here.', allowed)[0], /no longer named in AGENTS\.md/);
  const exists = new Map([['src-tauri/src/chat.rs', 'stale']]);
  assert.match(check('`src-tauri/src/chat.rs`', exists)[0], /exists now/);
});

test('every ALLOWED entry carries a reason', () => {
  for (const [path, reason] of ALLOWED) assert.ok(reason && reason.length > 10, path);
});

test('heading anchors follow GitHub: punctuation dropped, spaces to hyphens, duplicates numbered', () => {
  const a = headingAnchors('## Build & ship (how)\n### `heartbeat.rs` tick\n## Notes\n## Notes\n```\n# not a heading\n```');
  assert.deepEqual([...a].sort(), ['build--ship-how', 'heartbeatrs-tick', 'notes', 'notes-1']);
  assert.deepEqual(checkClaudeAnchors({ markdown: '## Notes', sources: { 'README.md': 'see AGENTS.md#gone and AGENTS.md#notes' } }), [
    'README.md:1: AGENTS.md has no heading for #gone',
  ]);
  assert.deepEqual(checkClaudeAnchors({ markdown: '## Notes', sources: { 'docs/X.md': 'see [n](../CLAUDE.md#notes)' } }), [
    'docs/X.md:1: CLAUDE.md#notes: CLAUDE.md only includes AGENTS.md; link AGENTS.md or the doc that holds the section',
  ]);
});

test('link targets resolve against the folder of the doc they are in', () => {
  assert.equal(resolveLink('../../src/api.ts', 'docs/reference/x.md'), 'src/api.ts');
  assert.equal(resolveLink('../WORKSPACES.md#build-workspaces', 'docs/reference/x.md'), 'docs/WORKSPACES.md#build-workspaces');
  assert.equal(resolveLink('../history/', 'docs/reference/x.md'), 'docs/history/');
  assert.equal(resolveLink('#x', 'docs/reference/x.md'), '#x');
  assert.equal(resolveLink('https://example.com/a.ts', 'docs/reference/x.md'), 'https://example.com/a.ts');
  const ok = checkManualPaths({ docs: { 'docs/reference/x.md': '[api](../../README.md) [w](../WORKSPACES.md)' }, repo: fake, allowed: new Map() });
  assert.deepEqual(ok, []);
  const bad = checkManualPaths({ docs: { 'docs/reference/x.md': '[w](WORKSPACES.md) `src/gone.ts`' }, repo: fake, allowed: new Map() });
  assert.equal(bad.length, 2, bad.join('\n'));
  assert.ok(bad.some((p) => /^docs\/reference\/x\.md:1: `docs\/reference\/WORKSPACES\.md` does not exist/.test(p)), bad.join('\n'));
});

test('an ALLOWED entry may be named by any manual doc', () => {
  const allowed = new Map([['src/pages/MyPage.tsx', 'example']]);
  const docs = { 'AGENTS.md': 'Entry.', 'docs/reference/common-tasks.md': 'Create `src/pages/MyPage.tsx`.' };
  assert.deepEqual(checkManualPaths({ docs, repo: fake, allowed }), []);
  assert.match(checkManualPaths({ docs: { 'AGENTS.md': 'Entry.' }, repo: fake, allowed })[0], /no longer named/);
});

test('fragment links into a manual doc must name one of its headings', () => {
  const docs = {
    'AGENTS.md': '# AGENTS.md\n## Running\nSee [ship](docs/reference/ship.md#build--ship) and [run](#running).',
    'docs/reference/ship.md': '# Ship\n## Build & ship\nBack to [entry](../../AGENTS.md#running), [self](#gone).',
    'docs/agents/area.md': '# Area\nSee [entry](../../AGENTS.md#nowhere).',
  };
  const sources = { ...docs, 'README.md': '[a](docs/reference/ship.md#nope) [b](docs/WORKSPACES.md#not-checked)', 'docs/X.md': '[c](reference/ship.md#build--ship)' };
  assert.deepEqual(checkManualAnchors({ docs, sources }), [
    'docs/reference/ship.md:3: docs/reference/ship.md has no heading for #gone',
    'docs/agents/area.md:2: AGENTS.md has no heading for #nowhere',
    'README.md:1: docs/reference/ship.md has no heading for #nope',
  ]);
});
