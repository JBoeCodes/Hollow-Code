// BG3: the hashed identifier denylist. Plants a listed name and checks the
// guard blocks it without printing it, that an empty or missing list fails
// closed, and that this repo's tracked files hold no listed identifier.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import {
  DENYLIST, MAX_BYTES, hashNames, hashPhrase, loadDenylist, parseDenylist, scanFiles, scanText, trackedFiles,
} from './identifier-guard.mjs';

const REPO = join(fileURLToPath(new URL('.', import.meta.url)), '..');
// Built at run time so no file of the repo holds the canary in plaintext.
// (Shingles ignore punctuation, so two adjacent quoted words would match too.)
const CANARY = 'canaryzxqv'.replace('y', 'y ');

test('a listed one-word and two-word name is found by line and column, case and punctuation aside', () => {
  const hashes = new Set([hashPhrase('quillfeather'), hashPhrase('mara quillfeather')]);
  const text = ['nothing here', 'Contact: Mara  QUILLFEATHER, for the lot', 'x quillfeathers y'].join('\n');
  const hits = scanText(text, hashes);
  assert.deepEqual(hits.map(h => [h.line, h.column]), [[2, 10], [2, 16]]);
  assert.equal(hits.every(h => h.hash.length === 64), true);
  // A different word is not a hit.
  assert.deepEqual(scanText('quillfeathers', hashes), []);
});

test('a name a hard wrap split over two lines, or over a comment marker, still matches; a blank line does not join', () => {
  const hashes = new Set([hashPhrase('mara quillfeather')]);
  assert.equal(scanText('asked Mara\nQuillfeather to send it', hashes).length, 1);
  assert.equal(scanText('// asked Mara\n// Quillfeather to send it', hashes).length, 1);
  assert.equal(scanText('> Mara\n> Quillfeather', hashes).length, 1);
  assert.deepEqual(scanText('Mara\n\nQuillfeather', hashes), [], 'two paragraphs are not one name');
  // The hit is placed at the first word.
  assert.deepEqual(scanText('x\nsaid Mara\nQuillfeather', hashes).map(h => [h.line, h.column]), [[2, 6]]);
});

test('accents and composed forms fold to one word, and columns come from the original text', () => {
  const hashes = new Set([hashPhrase('josé garcía'), hashPhrase('istanbul')]);
  const composed = 'Jos\u00e9 Garc\u00eda';
  const decomposed = 'Jose\u0301 Garci\u0301a';
  assert.equal(scanText(composed, hashes).length, 1);
  assert.equal(scanText(decomposed, hashes).length, 1, 'a name pasted decomposed matches the composed list entry');
  // A capital dotted I lowercases to two code units; the column is still the original offset.
  const dotted = new Set([hashPhrase('i\u0307stanbul')]);
  assert.deepEqual(scanText('x \u0130stanbul', dotted).map(h => [h.line, h.column]), [[1, 3]]);
  assert.deepEqual(hashNames('Jose\u0301 Garci\u0301a').hashes, [hashPhrase('josé garcía')]);
});

test('a short single word is never looked up, a two-word shingle is', () => {
  const hashes = new Set([hashPhrase('al'), hashPhrase('al b')]);
  assert.deepEqual(scanText('al', hashes), []);
  assert.equal(scanText('Al B', hashes).length, 1);
});

test('the committed list is hashes only and carries the canary', () => {
  const text = readFileSync(DENYLIST, 'utf8');
  for (const line of text.split('\n')) {
    const body = line.replace(/#.*$/, '').trim();
    if (body) assert.match(body, /^[0-9a-f]{64}$/);
  }
  const hashes = loadDenylist();
  assert.ok(hashes.has(hashPhrase(CANARY)));
});

test('a planted listed name blocks a file without printing it', () => {
  const hashes = loadDenylist();
  const files = { 'a.md': Buffer.from('fine\n'), 'fixture.json': Buffer.from(`{"contact": "${CANARY}"}\n`) };
  const { hits } = scanFiles(Object.keys(files), f => files[f], hashes);
  assert.equal(hits.length, 1);
  assert.match(hits[0], /^fixture\.json:1:\d+ [0-9a-f]{8}$/);
  assert.equal(hits[0].toLowerCase().includes('canary'), false);
});

test('binary and oversize files are not read as text; oversize is reported', () => {
  const hashes = loadDenylist();
  const bin = Buffer.concat([Buffer.from([0, 1, 2]), Buffer.from(CANARY)]);
  const big = Buffer.alloc(MAX_BYTES + 1, 97);
  const files = { 'a.bin': bin, 'big.txt': big };
  const out = scanFiles(Object.keys(files), f => files[f], hashes);
  assert.deepEqual(out.hits, []);
  assert.equal(out.scanned, 0);
  assert.equal(out.skipped.length, 1, 'an oversize file is named, not passed in silence');
  assert.equal(out.binary, 1, 'a binary file is counted, not scanned');
  assert.match(out.skipped.join('\n'), /big\.txt \(over/);
  // A UTF-16 text file with its mark is decoded and scanned, not skipped as binary.
  const utf16 = Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(`note: ${CANARY}\n`, 'utf16le')]);
  const wide = scanFiles(['wide.txt'], () => utf16, hashes);
  assert.equal(wide.hits.length, 1);
  assert.equal(wide.binary, 0);
  // A file that can't be read is named; one that is gone from the working tree is not.
  const gone = scanFiles(['unreadable.md', 'deleted.md'], f => (f === 'deleted.md' ? undefined : null), hashes);
  assert.deepEqual(gone.skipped, ['unreadable.md (could not be read)']);
});

test('an empty, comment-only, malformed or missing list fails closed', () => {
  assert.throws(() => parseDenylist(''), /fails closed/);
  assert.throws(() => parseDenylist('# only a comment\n\n'), /fails closed/);
  assert.throws(() => parseDenylist('not-a-hash'), /not a SHA-256/);
  assert.throws(() => loadDenylist(join(REPO, 'scripts', 'no-such-list.sha256')), /fails closed/);
  assert.equal(parseDenylist(`${hashPhrase('x y')} # note\n`).size, 1);
});

test('--hash turns names into hashes and refuses short or long ones', () => {
  const { hashes, refused } = hashNames(`${CANARY}\n\n# comment\nab\nOne Two Three\nlonely\n`);
  assert.deepEqual(hashes, [hashPhrase(CANARY), hashPhrase('lonely')]);
  assert.equal(refused.length, 2);
  const cli = spawnSync(process.execPath, [join(REPO, 'scripts/identifier-guard.mjs'), '--hash'], { input: `${CANARY}\n`, encoding: 'utf8' });
  assert.equal(cli.stdout.trim(), hashPhrase(CANARY));
});

test('no tracked file of this repo holds a listed identifier', () => {
  const hashes = loadDenylist();
  const files = trackedFiles(REPO);
  assert.ok(files.length > 500, `scanned ${files.length} files`);
  const read = f => {
    try {
      return readFileSync(join(REPO, f));
    } catch (e) {
      return e.code === 'ENOENT' ? undefined : null;
    }
  };
  const { hits, skipped, scanned } = scanFiles(files, read, hashes);
  assert.deepEqual(skipped, []);
  assert.ok(scanned > 500, `scanned ${scanned} of ${files.length}`);
  assert.deepEqual(hits, []);
});
