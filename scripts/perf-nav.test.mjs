// Profiling night 11-20: the perf scripts' selectors (scripts/perf-nav.mjs) still exist in the
// source. The browser scripts are not in the gate, so they rotted through the 11-17 shell redesign
// without a failure; this is the gate's half of keeping them honest.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { HOME_READY, PAGE_TITLE, RAIL_ROWS, TASK_ROW } from './perf-nav.mjs';

const read = rel => readFileSync(fileURLToPath(new URL(`../${rel}`, import.meta.url)), 'utf8');

test('the sidebar landmarks the perf scripts click exist', () => {
  const rail = read('src/components/Rail.tsx');
  assert.match(rail, /aria-label="Project navigation"/);
  assert.match(rail, /aria-label="Main"/);
  assert.ok(RAIL_ROWS.includes('Project navigation'));
  const top = read('src/components/TopBar.tsx');
  assert.match(top, /aria-label="Search chats"/);
  assert.match(top, /'Show sidebar' : 'Hide sidebar'/);
  const search = read('src/components/RailSearch.tsx');
  assert.match(search, /aria-label="Search chats"/);
  assert.match(search, /work-rail-search-chat/);
  assert.match(search, /Search everything/);
});

test('Home, the page title and the task rows keep the names the scripts wait for', () => {
  assert.match(read('src/components/StudioHome.tsx'), /aria-label="Start a conversation"/);
  assert.ok(HOME_READY.includes('Start a conversation'));
  assert.match(read('src/components/PageHeader.tsx'), /<h1 class="page-title">/);
  assert.equal(PAGE_TITLE, 'h1.page-title');
  assert.match(read('src/pages/tasks/IssueList.tsx'), /class="issue-row/);
  assert.equal(TASK_ROW, '.issue-row');
  for (const page of ['Library', 'Tasks']) assert.match(read(`src/pages/${page}.tsx`), new RegExp(`PageHeader title="${page}"`));
  assert.match(read('src/pages/Workflows.tsx'), /title="Automations"/);
  assert.match(read('src/components/CommandPalette.tsx'), /Command palette/);
});
