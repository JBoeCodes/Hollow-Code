// OR2: lane preflight. A planted claim on src-tauri/src/memory_guard.rs blocks
// a second lane that names it, a neighbour in the same folder only warns, a
// claim older than 72 hours is flagged stale and no longer blocks, and the
// verdict never reads a wall clock (`now` is passed in).
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { STALE_HOURS, normalizePath, parseClaim, preflight, readClaims, render } from './lane-preflight.mjs';

const script = fileURLToPath(new URL('./lane-preflight.mjs', import.meta.url));
const NOW = '2026-11-17T12:00:00Z';
const GUARD = 'src-tauri/src/memory_guard.rs';
const NEIGHBOUR = 'src-tauri/src/memory_maintenance.rs';

const claim = (over = {}) => ({ id: 'lane-d-1', lane: 'D', claimedAt: '2026-11-17T08:00:00Z', status: 'live', paths: [GUARD], ...over });

test('the same file in another live claim blocks, and a same-folder neighbour only warns', () => {
  const planted = [claim()];
  const same = preflight({ id: 'lane-r2-4', lane: 'R2', paths: [GUARD], claims: planted, now: NOW });
  assert.equal(same.verdict, 'block');
  assert.deepEqual(same.blocks, [{ path: GUARD, claim: 'lane-d-1', lane: 'D', theirs: GUARD }]);
  assert.equal(same.exitCode, 1);

  const near = preflight({ id: 'lane-r2-4', lane: 'R2', paths: [NEIGHBOUR], claims: planted, now: NOW });
  assert.equal(near.verdict, 'warn');
  assert.equal(near.exitCode, 0);
  assert.deepEqual(near.warnings, [{ path: NEIGHBOUR, claim: 'lane-d-1', lane: 'D', theirs: GUARD, why: 'same folder' }]);

  const both = preflight({ id: 'lane-r2-4', lane: 'R2', paths: [NEIGHBOUR, GUARD], claims: planted, now: NOW });
  assert.equal(both.verdict, 'block');
  assert.equal(both.warnings.length, 1);
});

test('an unrelated claim is clear', () => {
  const r = preflight({ id: 'x', lane: 'R2', paths: ['src/chat/ChatBody.tsx'], claims: [claim()], now: NOW });
  assert.equal(r.verdict, 'clear');
  assert.equal(r.exitCode, 0);
  assert.deepEqual([r.blocks, r.warnings, r.stale], [[], [], []]);
});

test('a claim older than 72 hours is stale: flagged, and neither blocks nor warns', () => {
  assert.equal(STALE_HOURS, 72);
  const old = claim({ claimedAt: '2026-11-14T11:00:00Z' }); // 73 h before NOW
  const r = preflight({ id: 'x', lane: 'R2', paths: [GUARD, NEIGHBOUR], claims: [old], now: NOW });
  assert.equal(r.verdict, 'clear');
  assert.deepEqual(r.stale, [{ claim: 'lane-d-1', lane: 'D', hours: 73 }]);
  // Exactly 72 h is still live; the bound is "older than".
  const edge = claim({ claimedAt: '2026-11-14T12:00:00Z' });
  assert.equal(preflight({ id: 'x', lane: 'R2', paths: [GUARD], claims: [edge], now: NOW }).verdict, 'block');
  // The window is a parameter.
  assert.equal(preflight({ id: 'x', lane: 'R2', paths: [GUARD], claims: [claim()], now: NOW, staleHours: 2 }).verdict, 'clear');
});

test('the same lane or claim id re-claiming, and released claims, never conflict', () => {
  const mine = preflight({ id: 'lane-d-1', lane: 'D', paths: [GUARD], claims: [claim()], now: NOW });
  assert.equal(mine.verdict, 'clear');
  const sameLane = preflight({ id: 'lane-d-2', lane: 'D', paths: [GUARD], claims: [claim()], now: NOW });
  assert.equal(sameLane.verdict, 'clear');
  for (const status of ['released', 'done', 'merged', 'cancelled']) {
    assert.equal(preflight({ id: 'x', lane: 'R2', paths: [GUARD], claims: [claim({ status })], now: NOW }).verdict, 'clear', status);
  }
});

test('a folder claim warns for files inside it, and a module file warns for its folder', () => {
  const folder = claim({ paths: ['src-tauri/src/memory/'] });
  const r = preflight({ id: 'x', lane: 'R2', paths: ['src-tauri/src/memory/index.rs'], claims: [folder], now: NOW });
  assert.equal(r.verdict, 'warn');
  assert.equal(r.warnings[0].why, 'inside a claimed folder');
  const mod = claim({ paths: ['src-tauri/src/memory.rs'] });
  const m = preflight({ id: 'x', lane: 'R2', paths: ['src-tauri/src/memory/index.rs'], claims: [mod], now: NOW });
  assert.equal(m.verdict, 'warn');
  assert.equal(m.warnings[0].why, 'module and its folder');
  // Two folder claims that are the same folder block: it is the same path.
  assert.equal(preflight({ id: 'x', lane: 'R2', paths: ['src-tauri/src/memory'], claims: [folder], now: NOW }).verdict, 'block');
});

test('paths are compared after normalising', () => {
  assert.equal(normalizePath('./src-tauri//src\\memory_guard.rs'), GUARD);
  assert.equal(normalizePath('src-tauri/src/memory/'), 'src-tauri/src/memory/');
  const r = preflight({ id: 'x', lane: 'R2', paths: ['./src-tauri/src/memory_guard.rs'], claims: [claim()], now: NOW });
  assert.equal(r.verdict, 'block');
});

test('claims parse from JSON and from markdown front matter; a bad file is reported, not fatal', () => {
  const json = parseClaim('a.json', JSON.stringify({ id: 'a', lane: 'M', claimed_at: '2026-11-17T09:00:00Z', paths: [GUARD] }));
  assert.deepEqual(json, { id: 'a', lane: 'M', claimedAt: '2026-11-17T09:00:00Z', status: 'live', paths: [GUARD] });
  const md = parseClaim('b.md', ['---', 'id: b', 'lane: K', 'claimed_at: 2026-11-17T10:00:00Z', 'status: live', 'paths:', `  - ${GUARD}`, '  - scripts/x.mjs', '---', 'Body text with paths: not-read'].join('\n'));
  assert.deepEqual(md.paths, [GUARD, 'scripts/x.mjs']);
  assert.equal(md.lane, 'K');
  const inline = parseClaim('c.md', ['---', 'id: c', 'paths: a/b.rs, c/d.rs', 'claimed_at: 2026-11-17T10:00:00Z', '---'].join('\n'));
  assert.deepEqual(inline.paths, ['a/b.rs', 'c/d.rs']);
  assert.throws(() => parseClaim('d.json', '{not json'), /d\.json/);
  assert.throws(() => parseClaim('e.json', JSON.stringify({ id: 'e', paths: [] })), /paths/);
  assert.throws(() => parseClaim('f.json', JSON.stringify({ id: 'f', paths: ['a'], claimed_at: 'yesterday' })), /claimed_at/);
});

test('readClaims reads a folder of claims and lists the unreadable ones', () => {
  const dir = mkdtempSync(join(tmpdir(), 'preflight-'));
  try {
    writeFileSync(join(dir, 'one.json'), JSON.stringify(claim()));
    writeFileSync(join(dir, 'two.md'), `---\nid: two\nlane: K\nclaimed_at: 2026-11-17T10:00:00Z\npaths: scripts/x.mjs\n---\n`);
    writeFileSync(join(dir, 'broken.json'), '{');
    writeFileSync(join(dir, 'notes.txt'), 'ignored');
    mkdirSync(join(dir, 'sub'));
    const { claims, problems } = readClaims(dir);
    assert.deepEqual(claims.map((c) => c.id).sort(), ['lane-d-1', 'two']);
    assert.equal(problems.length, 1);
    assert.match(problems[0], /broken\.json/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('the command line exits 1 on a block, 0 with a warning line on a neighbour, and takes the clock as --now', () => {
  const dir = mkdtempSync(join(tmpdir(), 'preflight-'));
  try {
    writeFileSync(join(dir, 'lane-d-1.json'), JSON.stringify(claim()));
    const run = (...args) => spawnSync(process.execPath, [script, '--claims', dir, '--now', NOW, '--lane', 'R2', '--id', 'lane-r2-4', ...args], { encoding: 'utf8' });
    const blocked = run('--paths', GUARD);
    assert.equal(blocked.status, 1);
    assert.match(blocked.stdout, /^BLOCK /m);
    assert.match(blocked.stdout, /memory_guard\.rs.*lane-d-1/);
    const warned = run('--paths', NEIGHBOUR);
    assert.equal(warned.status, 0);
    assert.match(warned.stdout, /^WARN .*memory_maintenance\.rs.*same folder/m);
    const clear = run('--paths', 'docs/x.md');
    assert.equal(clear.status, 0);
    assert.match(clear.stdout, /^clear/m);
    const json = JSON.parse(run('--paths', GUARD, '--json').stdout);
    assert.equal(json.verdict, 'block');
    const stale = spawnSync(process.execPath, [script, '--claims', dir, '--now', '2026-12-01T00:00:00Z', '--lane', 'R2', '--paths', GUARD], { encoding: 'utf8' });
    assert.equal(stale.status, 0);
    assert.match(stale.stdout, /^STALE lane-d-1/m);
    const none = spawnSync(process.execPath, [script, '--claims', dir], { encoding: 'utf8' });
    assert.equal(none.status, 2);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('render prints one line per finding', () => {
  const r = preflight({ id: 'x', lane: 'R2', paths: [GUARD, NEIGHBOUR], claims: [claim(), claim({ id: 'old', claimedAt: '2026-11-01T00:00:00Z' })], now: NOW });
  const lines = render(r);
  assert.equal(lines.filter((l) => l.startsWith('BLOCK')).length, 1);
  assert.equal(lines.filter((l) => l.startsWith('WARN')).length, 1);
  assert.equal(lines.filter((l) => l.startsWith('STALE')).length, 1);
});
