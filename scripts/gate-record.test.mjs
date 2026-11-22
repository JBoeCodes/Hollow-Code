// The gate record (overnight 10-12/13, S12): a passing `npm run gate` is kept
// in the repository's common git directory, so removing the worktree that ran
// it doesn't lose the proof (src-tauri/src/install_ready.rs reads it).
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, utimesSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { gateRecord, prLine, startPath, writeRecord } from './gate-record.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const FP = 'a'.repeat(64);
const provenance = { sourceFingerprint: FP, commit: 'c'.repeat(40), checkout: '/w/lane', generatedAt: '2026-10-13T06:00:00.000Z', inputCount: 3, dirty: false, version: '0.5.15' };
const current = { sourceFingerprint: FP, commit: 'c'.repeat(40) };

test('only a clean tree, the same at start, build and end, inside npm run gate, is recorded', () => {
  const start = { ...current, dirty: false };
  const base = { inGate: true, start, provenance, current, dirty: false, now: '2026-10-13T06:10:00.000Z' };
  const ok = gateRecord(base);
  assert.equal(ok.skip, undefined);
  assert.equal(ok.record.gate, 'passed');
  assert.equal(ok.record.sourceFingerprint, FP);
  assert.equal(ok.record.commit, provenance.commit);
  assert.equal(ok.record.checkout, '/w/lane');
  assert.equal(ok.record.recordedAt, '2026-10-13T06:10:00.000Z');
  assert.equal(ok.record.dirty, false);
  const skip = over => gateRecord({ ...base, ...over }).skip;
  assert.match(skip({ inGate: false }), /only `npm run gate`/);
  assert.match(skip({ start: null }), /start wasn't recorded/);
  assert.match(skip({ provenance: null }), /build-provenance/);
  assert.match(skip({ dirty: true }), /uncommitted/);
  assert.match(skip({ provenance: { ...provenance, dirty: true } }), /uncommitted/);
  assert.match(skip({ start: { ...start, dirty: true } }), /uncommitted changes when the gate started/);
  // An edit committed while the frontend tests ran: the build and the end agree, the start doesn't.
  const later = { sourceFingerprint: 'b'.repeat(64), commit: 'd'.repeat(40) };
  assert.match(skip({ provenance: { ...provenance, ...later }, current: later }), /changed during the gate/);
  assert.match(skip({ current: { ...current, sourceFingerprint: 'b'.repeat(64) } }), /changed during the gate/);
  assert.match(skip({ current: { ...current, commit: 'd'.repeat(40) }, start: { ...start, commit: 'd'.repeat(40) } }), /HEAD moved/);
  const bad = { sourceFingerprint: 'nope' };
  assert.match(skip({ provenance: { ...provenance, ...bad }, current: { ...current, ...bad }, start: { ...start, ...bad } }), /fingerprint/);
});

test('each checkout has its own start snapshot', () => {
  assert.notEqual(startPath('/r', '/w/a'), startPath('/r', '/w/b'));
  assert.match(startPath('/r', '/w/a'), /^\/r\/start\/[0-9a-f]{16}\.json$/);
});

test('records are named by fingerprint and the oldest beyond the limit go', () => {
  const dir = mkdtempSync(join(tmpdir(), 'gate-records-'));
  try {
    writeFileSync(join(dir, 'README'), 'kept');
    for (const [i, c] of ['1', '2', '3'].entries()) {
      const p = writeRecord(dir, { ...provenance, sourceFingerprint: c.repeat(64), gate: 'passed' }, 10);
      utimesSync(p, 1000 + i, 1000 + i);
    }
    const path = writeRecord(dir, { ...provenance, gate: 'passed' }, 2);
    assert.equal(path, join(dir, `${FP}.json`));
    assert.equal(JSON.parse(readFileSync(path, 'utf8')).sourceFingerprint, FP);
    assert.deepEqual(readdirSync(dir).sort(), ['3'.repeat(64) + '.json', 'README', `${FP}.json`].sort());
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('the PR line names the commit and the whole fingerprint', () => {
  const line = prLine({ ...provenance, gate: 'passed' });
  assert.ok(line.includes(FP));
  assert.ok(line.includes('cccccccc'));
  assert.match(line, /npm run gate/);
});

test('the gate starts and ends with the record, and --print hashes without writing', () => {
  const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
  // Since OF11 the chain runs inside gate-run.mjs (scripts/gate-run.test.mjs pins the wrapper).
  const chain = pkg.scripts.gate.match(/^node scripts\/gate-run\.mjs '([^']+)'$/)?.[1] ?? pkg.scripts.gate;
  assert.match(chain, /^node scripts\/gate-record\.mjs --start && /);
  assert.match(chain, /&& node scripts\/gate-record\.mjs$/);
  const provenancePath = join(root, 'src-tauri/build-provenance.json');
  const before = existsSync(provenancePath) ? statSync(provenancePath).mtimeMs : null;
  const r = spawnSync(process.execPath, [join(root, 'scripts/source-provenance.mjs'), '--print'], { encoding: 'utf8' });
  assert.equal(r.status, 0, r.stderr);
  const printed = JSON.parse(r.stdout);
  assert.match(printed.sourceFingerprint, /^[0-9a-f]{64}$/);
  assert.match(printed.commit, /^[0-9a-f]{40}$/);
  assert.equal(existsSync(provenancePath) ? statSync(provenancePath).mtimeMs : null, before);
});

test('run by hand, outside npm run gate, it records nothing and exits 0', () => {
  const env = { ...process.env };
  delete env.npm_lifecycle_event;
  const r = spawnSync(process.execPath, [join(root, 'scripts/gate-record.mjs')], { encoding: 'utf8', env });
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /not recorded/);
});
