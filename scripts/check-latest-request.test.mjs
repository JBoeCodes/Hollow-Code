import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { MIGRATED, ROOT, handRolledGuards } from './check-latest-request.mjs';

test('every migrated view exists', () => {
  assert.deepEqual(MIGRATED.filter(f => !existsSync(join(ROOT, f))), []);
});

test('X21b: the stores and the three pure factories are checked too', () => {
  for (const f of [
    'src/needsYou.ts', 'src/paletteSearch.ts', 'src/settings/controllers/settingsNavigation.ts', 'src/pages/Skills.tsx',
    'src/store/chatRuntime.ts',
    'src/store/mainAgent.ts', 'src/store/prepPack.ts', 'src/store/runQueue.ts', 'src/store/triage.ts',
    'src/store/workspaces.ts',
  ]) assert.ok(MIGRATED.includes(f), f);
  // Run 15 P4 deleted From other apps' page and store (two migrated files);
  // One Ask (2026-11-03) deleted Ask Hollow's store.
  // Overnight 11-08 S5 deleted the Votes list (FeedbackPanel); the 11-10
  // sweep deleted its unused store (store/feedback.ts).
  assert.equal(MIGRATED.length, 20);
  assert.equal(new Set(MIGRATED).size, MIGRATED.length);
});

test('the repository passes: no migrated view hand-rolls a request counter', () => {
  assert.deepEqual(handRolledGuards(), []);
});

test('a `= ++identifier` guard is reported, with or without spaces; ids and latestRequest are not', () => {
  const files = {
    'a.tsx': 'const mine = ++request;\nif (mine !== request) return;',
    'b.tsx': 'const path=props.path, token=++generation;',
    'c.tsx': 'const ticket = reads.start();\nif (!reads.isLatest(ticket)) return;',
    'd.tsx': 'const aiKey=`${++editorSeq}|`;\nfailed++;\ncount += 1;',
    'f.ts': 'return ++chatAttachmentSequence;\nnext.unshift({ touchedAt: ++artifactSeq });\ngitFresh.set(id, ++gitFreshVersion);\nsaveEpoch += 1;',
    'e.tsx': 'let x = 0; x = ++ $seq;',
  };
  assert.deepEqual(handRolledGuards(Object.keys(files), f => files[f]), [
    'a.tsx:1: = ++request',
    'b.tsx:1: =++generation',
    'e.tsx:1: = ++ $seq',
  ]);
});

test('lane H review: postfix and `+= 1` counters are reported too; comparisons, arrows and bare bumps are not', () => {
  const files = {
    'g.tsx': 'const mine = gen++;\nconst ticket = (load += 1);\nlet t; t = seq += 1;',
    'h.ts': 'if (mine === ++gen) x();\nif (a !== b++) y();\nconst next = () => ++seq;\nsaveEpoch += 1;\nn++;\nconst total = a + 1;\nok = count >= 1;',
  };
  assert.deepEqual(handRolledGuards(Object.keys(files), f => files[f]), [
    'g.tsx:1: = gen++',
    'g.tsx:2: = (load += 1',
    'g.tsx:3: = seq += 1',
  ]);
});
