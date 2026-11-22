import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, realpathSync, symlinkSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { planRetirement, applyRetirement } from './retire-messaging-data.mjs';

function fixture(t) {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'hollow-messaging-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const vault = join(root, 'vault');
  mkdirSync(vault);
  const put = (path, body) => { mkdirSync(dirname(join(vault, path)), { recursive: true }); writeFileSync(join(vault, path), body); };
  put('.review-queue.json', JSON.stringify({ schemaVersion: 2, items: [
    { id: 'send', kind: 'heartbeat_send', status: 'pending', body: 'private message' },
    { id: 'stale', kind: 'heartbeat_task', sourcePath: 'teams/dms/old.md' },
    { id: 'local', kind: 'heartbeat_task', sourcePath: 'calendar/meeting.md' },
  ] }));
  put('drafts/heartbeat/reply-someone.md', 'Hi Someone,\n\nA message.');
  put('drafts/heartbeat/meeting-prep.md', '# Meeting prep\nDiscuss email integration in the product.');
  put('calendar/meeting.md', '---\nplaudRecordingId: retained\n---\nAuthored meeting body');
  return { root, vault, put };
}
test('dry run preserves all data and selects only message drafts and review records', t => {
  const { vault } = fixture(t);
  const before = readFileSync(join(vault, '.review-queue.json'));
  const plan = planRetirement(vault);
  assert.deepEqual(plan.map(x => x.path).sort(), ['.review-queue.json', 'drafts/heartbeat/reply-someone.md']);
  assert.deepEqual(readFileSync(join(vault, '.review-queue.json')), before);
  assert.deepEqual(JSON.parse(plan[0].after).items.map(x => x.id), ['local']);
});
test('apply preserves Calendar and local prep, records recovery hashes, and is idempotent', t => {
  const { root, vault } = fixture(t);
  const calendar = readFileSync(join(vault, 'calendar/meeting.md'));
  const recovery = applyRetirement(vault, planRetirement(vault), join(root, 'recovery'), () => {});
  assert.equal(existsSync(join(vault, 'drafts/heartbeat/reply-someone.md')), false);
  assert.equal(existsSync(join(vault, 'drafts/heartbeat/meeting-prep.md')), true);
  assert.deepEqual(readFileSync(join(vault, 'calendar/meeting.md')), calendar);
  assert.equal(JSON.parse(readFileSync(join(recovery, 'manifest.json'))).status, 'complete');
  assert.equal(readFileSync(join(recovery, 'before/drafts/heartbeat/reply-someone.md'), 'utf8'), 'Hi Someone,\n\nA message.');
  assert.equal(planRetirement(vault).length, 0);
});
test('concurrent edits stop publication without overwriting the edit or removing drafts', t => {
  const { root, vault, put } = fixture(t);
  const plan = planRetirement(vault);
  put('.review-queue.json', '{"items":[]}');
  assert.throws(() => applyRetirement(vault, plan, join(root, 'recovery'), () => {}), /Concurrent change/);
  assert.equal(readFileSync(join(vault, '.review-queue.json'), 'utf8'), '{"items":[]}');
  assert.equal(existsSync(join(vault, 'drafts/heartbeat/reply-someone.md')), true);
});
test('running app guard prevents any publication', t => {
  const { root, vault } = fixture(t);
  assert.throws(() => applyRetirement(vault, planRetirement(vault), join(root, 'recovery'), () => { throw new Error('running'); }), /running/);
  assert.equal(existsSync(join(root, 'recovery')), false);
});
test('symlinked draft folders are rejected', t => {
  const { root, vault } = fixture(t);
  mkdirSync(join(root, 'external'));
  symlinkSync(join(root, 'external'), join(vault, 'drafts/external'));
  assert.throws(() => planRetirement(vault), /Symlink refused/);
});

test('review routing includes normalized metadata and payload source paths', t => {
  const { vault, put } = fixture(t);
  put('.review-queue.json', JSON.stringify({ schemaVersion: 2, items: [
    { id: 'kind', kind: ' REPLY_DRAFT ' },
    { id: 'risk', kind: 'learning', risk: ' EXTERNAL_SEND ' },
    { id: 'source', kind: 'learning', source: ' Teams ' },
    ...['sourcePath', 'sourceVaultPath', 'replyToVaultPath'].map(key => ({
      id: key, kind: 'heartbeat_task', payload: { [key]: ' vault:./Emails/inbox/old.md ' },
    })),
    { id: 'local', kind: 'meeting_task', payload: { sourcePath: 'calendar/meeting.md' } },
  ] }));
  const planned = planRetirement(vault).find(x => x.path === '.review-queue.json');
  assert.deepEqual(JSON.parse(planned.after).items.map(x => x.id), ['local']);
});

test('historical send arrays and nested retired draft references are filtered', t => {
  const { vault, put } = fixture(t);
  put('drafts/heartbeat/followup-jordan.md', '---\ntype: draft-reply\n---\nAn old message draft.');
  put('.heartbeat-log.jsonl', [
    { id: 'send-array', proposedSends: [{ body: 'Historical reply body' }] },
    { id: 'send-count', proposedSends: 2 },
    { id: 'nested-custom-draft', actionsTaken: ['Wrote drafts/heartbeat/followup-jordan.md'] },
    { id: 'old-draft-link', observations: 'See vault:drafts/heartbeat/reply-missing.md' },
    { id: 'local', proposedSends: [], proposedTasks: 1, actionsTaken: ['Wrote drafts/heartbeat/meeting-prep.md'] },
  ].map(row => JSON.stringify(row)).join('\n'));
  const planned = planRetirement(vault).find(x => x.path === '.heartbeat-log.jsonl');
  assert.deepEqual(planned.after.toString().trim().split('\n').map(line => JSON.parse(line).id), ['local']);
});

test('edits during planning cannot be mistaken for the transformed source snapshot', t => {
  const { root, vault } = fixture(t);
  const queue = join(vault, '.review-queue.json');
  const originalRead = fs.readFileSync;
  let injected = false;
  let concurrent;
  fs.readFileSync = function(path, ...args) {
    const bytes = originalRead(path, ...args);
    if (path === queue && !injected) {
      injected = true;
      const value = JSON.parse(bytes);
      value.items.push({ id: 'concurrent-local-task', kind: 'heartbeat_task' });
      concurrent = JSON.stringify(value);
      writeFileSync(queue, concurrent);
    }
    return bytes;
  };
  syncBuiltinESMExports();
  let plan;
  try {
    plan = planRetirement(vault);
  } finally {
    fs.readFileSync = originalRead;
    syncBuiltinESMExports();
  }
  assert.equal(injected, true);
  assert.throws(() => applyRetirement(vault, plan, join(root, 'recovery'), () => {}), /Concurrent change/);
  assert.equal(readFileSync(queue, 'utf8'), concurrent);
  assert.equal(existsSync(join(vault, 'drafts/heartbeat/reply-someone.md')), true);
});

test('rollback reports partial restoration when it preserves a concurrent edit', t => {
  for (const concurrentEdit of [false, true]) {
    const { root, vault, put } = fixture(t);
    const queue = join(vault, '.review-queue.json');
    const original = readFileSync(queue);
    put('.review-queue.v1.json', original);
    const plan = planRetirement(vault);
    const recoveryRoot = join(root, 'recovery');
    const originalUnlink = fs.unlinkSync;
    const concurrent = '{"schemaVersion":2,"items":[{"id":"new-local-task","kind":"heartbeat_task"}]}';
    fs.unlinkSync = function(path) {
      if (path === join(vault, 'drafts/heartbeat/reply-someone.md')) {
        if (concurrentEdit) writeFileSync(queue, concurrent);
        throw new Error('Injected publication failure');
      }
      return originalUnlink(path);
    };
    syncBuiltinESMExports();
    try {
      assert.throws(() => applyRetirement(vault, plan, recoveryRoot, () => {}), /Injected publication failure/);
    } finally {
      fs.unlinkSync = originalUnlink;
      syncBuiltinESMExports();
    }
    const archive = join(recoveryRoot, fs.readdirSync(recoveryRoot)[0]);
    const manifest = JSON.parse(readFileSync(join(archive, 'manifest.json')));
    assert.equal(manifest.status, concurrentEdit ? 'partial-rollback' : 'rolled-back');
    assert.deepEqual(manifest.rollback.preservedConcurrentPaths, concurrentEdit ? ['.review-queue.json'] : []);
    assert.deepEqual(manifest.rollback.restoredPaths, concurrentEdit ? ['.review-queue.v1.json'] : ['.review-queue.v1.json', '.review-queue.json']);
    assert.deepEqual(manifest.rollback.errors, []);
    assert.deepEqual(readFileSync(queue), concurrentEdit ? Buffer.from(concurrent) : original);
    assert.deepEqual(readFileSync(join(vault, '.review-queue.v1.json')), original);
    assert.deepEqual(readFileSync(join(archive, 'before/.review-queue.json')), original);
    assert.equal(existsSync(join(vault, 'drafts/heartbeat/reply-someone.md')), true);
  }
});
