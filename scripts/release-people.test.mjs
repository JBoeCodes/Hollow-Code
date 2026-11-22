// Tests for scripts/release/people.mjs (updates row U9) against an in-memory
// bucket with the uploader's interface. No network, no R2, no real clock.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { checkName, main, run, readLimited, HIDDEN_CHAR } from './release/people.mjs';
import { parsePeople } from '../infra/updates-worker/src/auth.js';

const hashOf = (token) => createHash('sha256').update(token, 'utf8').digest('hex');
const HASH_A = hashOf('A'.repeat(43));
const HASH_B = hashOf('B'.repeat(43));
const NOW = new Date('2026-11-15T12:34:56.789Z');

const EMPTY_FILE = '{"people":[]}\n';
const preconditionFailed = (key) => Object.assign(new Error(`R2 PUT ${key} answered 412 (PreconditionFailed)`), { status: 412 });

/** An in-memory bucket with R2's conditional PUT: every write gets a new
 *  ETag; `ifMatch` must equal the current one, `ifNoneMatch: '*'` needs no
 *  object. `initial` null: no people.json. */
function fakeBucket(initial = EMPTY_FILE) {
  const store = new Map();
  const etags = new Map();
  let n = 0;
  const set = (key, text) => { store.set(key, text); etags.set(key, `"e${++n}"`); };
  if (initial !== null) set('people.json', initial);
  const puts = [];
  return {
    store,
    puts,
    etags,
    set,
    async get(key) { return store.has(key) ? store.get(key) : null; },
    async getWithEtag(key) { return store.has(key) ? { text: store.get(key), etag: etags.get(key) } : null; },
    async put(key, bytes, opts = {}) {
      if (opts.ifMatch !== undefined && etags.get(key) !== opts.ifMatch) throw preconditionFailed(key);
      if (opts.ifNoneMatch === '*' && store.has(key)) throw preconditionFailed(key);
      puts.push({ key, text: Buffer.from(bytes).toString('utf8'), opts });
      set(key, Buffer.from(bytes).toString('utf8'));
    },
  };
}

function deps(bucket, { stdin = '', ids = ['p-0000000000a1', 'p-0000000000b2'] } = {}) {
  const queue = [...ids];
  return { bucket, readStdin: async () => stdin, now: () => NOW, randomId: () => queue.shift() };
}

/** Runs main with captured output. */
async function cli(argv, io) {
  let out = '';
  let err = '';
  const code = await main(argv, { ...io, stdout: { write: s => { out += s; } }, stderr: { write: s => { err += s; } } });
  return { code, out, err };
}

test('add reads the hash from stdin, writes one valid people.json and shows no hash', async () => {
  const bucket = fakeBucket();
  const r = await cli(['add', '  Sam  ', 'nightly', 'main'], { bucket, readStdin: async () => `${HASH_A}\n`, now: () => NOW, randomId: () => 'p-0000000000a1' });
  assert.equal(r.code, 0, r.err);
  assert.deepEqual(JSON.parse(r.out), { id: 'p-0000000000a1', name: 'Sam', channels: ['main', 'nightly'], revoked: false, createdAt: '2026-11-15T12:34:56Z' });
  assert.equal(bucket.puts.length, 1, 'one PUT');
  assert.equal(bucket.puts[0].key, 'people.json');
  assert.equal(bucket.puts[0].opts.contentType, 'application/json; charset=utf-8');
  const parsed = parsePeople(bucket.store.get('people.json'));
  assert.ok(parsed.ok);
  assert.equal(parsed.people[0].tokenHash, HASH_A, 'the bucket keeps the hash');
  assert.ok(!r.out.includes(HASH_A) && !r.err.includes(HASH_A), 'the output never shows it');
});

test('add validates before writing: bad hash, bad name, no channel, duplicates, invalid file', async () => {
  const cases = [
    { argv: ['add', 'Sam', 'main'], stdin: 'not-a-hash', code: 1, words: /token hash/ },
    { argv: ['add', 'Sam', 'main'], stdin: HASH_A.toUpperCase(), code: 1, words: /token hash/ },
    { argv: ['add', 'Sam', 'main'], stdin: `${HASH_A}${HASH_A}`, code: 1, words: /token hash/ },
    { argv: ['add', 'Sam'], stdin: HASH_A, code: 2, words: /at least one channel/ },
    { argv: ['add', 'Sam', 'beta'], stdin: HASH_A, code: 2, words: /unknown channel/ },
    { argv: ['add', 'Sam', 'main', 'main'], stdin: HASH_A, code: 2, words: /named twice/ },
    { argv: ['add', '   ', 'main'], stdin: HASH_A, code: 2, words: /name is needed/ },
    { argv: ['add', 'I\nan', 'main'], stdin: HASH_A, code: 2, words: /control/ },
    { argv: ['add', 'x'.repeat(81), 'main'], stdin: HASH_A, code: 2, words: /at most 80/ },
  ];
  for (const c of cases) {
    const bucket = fakeBucket();
    const r = await cli(c.argv, { bucket, readStdin: async () => c.stdin, now: () => NOW, randomId: () => 'p-0000000000a1' });
    assert.equal(r.code, c.code, `${c.argv.join(' ')}: ${r.err}`);
    assert.match(r.err, c.words);
    assert.equal(bucket.puts.length, 0, 'nothing written');
  }
  // The same hash twice.
  const bucket = fakeBucket();
  await run(['add', 'Sam', 'main'], deps(bucket, { stdin: HASH_A }));
  await assert.rejects(run(['add', 'Sam again', 'main'], deps(bucket, { stdin: HASH_A })), /already in people.json/);
  assert.equal(bucket.puts.length, 1);
  // An invalid file in the bucket is never written over.
  const broken = fakeBucket('{"people": [{"id": "x"}]}');
  await assert.rejects(run(['add', 'Sam', 'main'], deps(broken, { stdin: HASH_A })), /invalid/);
  assert.equal(broken.puts.length, 0);
});

test('a hash in the arguments is refused, whatever the command', async () => {
  for (const argv of [['add', HASH_A, 'main'], ['add', 'Sam', 'main', HASH_A], ['revoke', HASH_A], ['set-channels', HASH_A.toUpperCase(), 'main']]) {
    const bucket = fakeBucket();
    const r = await cli(argv, { bucket, readStdin: async () => HASH_B, now: () => NOW, randomId: () => 'p-0000000000a1' });
    assert.equal(r.code, 2);
    assert.match(r.err, /goes on stdin/);
    assert.ok(!r.err.includes(HASH_A) && !r.err.toLowerCase().includes(HASH_A));
    assert.equal(bucket.puts.length, 0);
  }
});

test('revoke keeps the entry and its hash, and sets revoked', async () => {
  const bucket = fakeBucket();
  const sam = await run(['add', 'Sam', 'main', 'nightly'], deps(bucket, { stdin: HASH_A, ids: ['p-0000000000a1'] }));
  await run(['add', 'Sam', 'main'], deps(bucket, { stdin: HASH_B, ids: ['p-0000000000b2'] }));
  const revoked = await run(['revoke', sam.id], deps(bucket));
  assert.equal(revoked.revoked, true);
  const parsed = parsePeople(bucket.store.get('people.json'));
  assert.ok(parsed.ok);
  assert.equal(parsed.people.length, 2, 'never deleted');
  assert.equal(parsed.people[0].tokenHash, HASH_A);
  assert.equal(parsed.people[0].revoked, true);
  assert.equal(parsed.people[1].revoked, false, 'only that person');
  const puts = bucket.puts.length;
  await run(['revoke', sam.id], deps(bucket));
  assert.equal(bucket.puts.length, puts, 'revoking twice writes nothing');
  await assert.rejects(run(['revoke', 'p-ffffffffffff'], deps(bucket)), /no person has that id/);
  await assert.rejects(run(['revoke', '../people'], deps(bucket)), /not a person id/);
});

test('set-channels replaces the allowed channels; none is allowed; revoked people are refused', async () => {
  const bucket = fakeBucket();
  const sam = await run(['add', 'Sam', 'main'], deps(bucket, { stdin: HASH_A }));
  assert.deepEqual((await run(['set-channels', sam.id, 'nightly', 'main'], deps(bucket))).channels, ['main', 'nightly']);
  assert.deepEqual((await run(['set-channels', sam.id], deps(bucket))).channels, []);
  assert.deepEqual(parsePeople(bucket.store.get('people.json')).people[0].channels, []);
  await assert.rejects(run(['set-channels', sam.id, 'beta'], deps(bucket)), /unknown channel/);
  await run(['revoke', sam.id], deps(bucket));
  await assert.rejects(run(['set-channels', sam.id, 'main'], deps(bucket)), /revoked/);
});

test('list shows id, name, channels, revoked and createdAt, never a hash; a hand-written entry reads too', async () => {
  const bucket = fakeBucket(JSON.stringify({ people: [{ id: 'p-sam', name: 'Sam', tokenHash: HASH_A, revoked: false }] }));
  await run(['add', 'Sam', 'nightly'], deps(bucket, { stdin: HASH_B }));
  const r = await cli(['list'], { bucket });
  assert.equal(r.code, 0, r.err);
  const listed = JSON.parse(r.out);
  assert.deepEqual(listed, {
    people: [
      { id: 'p-sam', name: 'Sam', channels: ['main', 'nightly'], revoked: false, createdAt: '' },
      { id: 'p-0000000000a1', name: 'Sam', channels: ['nightly'], revoked: false, createdAt: '2026-11-15T12:34:56Z' },
    ],
  });
  assert.ok(!r.out.includes(HASH_A) && !r.out.includes(HASH_B) && !r.out.includes('tokenHash'));
  assert.deepEqual(JSON.parse((await cli(['list'], { bucket: fakeBucket(null) })).out), { people: [] }, 'a missing file lists as empty');
});

test('errors from the bucket print the uploader\'s fixed words only', async () => {
  const bucket = { async getWithEtag() { throw new Error(`R2 GET people.json answered 403 (AccessDenied)`); }, async put() {} };
  const r = await cli(['list'], { bucket });
  assert.equal(r.code, 1);
  assert.equal(r.err, 'people: R2 GET people.json answered 403 (AccessDenied)\n');
  const other = { async getWithEtag() { throw new TypeError(`something with ${HASH_A}`); }, async put() {} };
  const r2 = await cli(['list'], { bucket: other });
  assert.equal(r2.err, 'people: failed\n', 'an unknown error is not repeated');
});

test('stdin is capped', async () => {
  async function* chunks() { yield Buffer.alloc(2048, 0x61); }
  await assert.rejects(readLimited(chunks()), /too long/);
  async function* ok() { yield Buffer.from(`${HASH_A}\n`); }
  assert.equal(await readLimited(ok()), `${HASH_A}\n`);
});

test('names keep their letters', () => {
  assert.equal(checkName(' Zoë Ångström '), 'Zoë Ångström');
  assert.throws(() => checkName('a\u2028b'), /control/);
});

test('names with format characters are refused before trimming (the BOM, zero widths, bidi overrides)', () => {
  // JS trim strips U+FEFF (Rust's doesn't): refused, so both sides agree.
  for (const name of ['\uFEFFSam', 'Sam\uFEFF', 'S\u200Bam', 'S\u200Dam', '\u202EmaS', 'Sam\u2066', 'Sa\u00ADm', '\u0085Sam']) {
    assert.throws(() => checkName(name), /invisible formatting/, JSON.stringify(name));
  }
});

test('every write is conditional on the ETag read; a change in between is refused, not lost', async () => {
  const bucket = fakeBucket();
  const before = bucket.etags.get('people.json');
  const sam = await run(['add', 'Sam', 'main'], deps(bucket, { stdin: HASH_A }));
  assert.deepEqual(bucket.puts[0].opts.ifMatch, before);
  assert.equal(bucket.puts[0].opts.ifNoneMatch, undefined);
  // Another writer changes the file between this run's read and its write.
  const racing = {
    ...bucket,
    async getWithEtag(key) {
      const found = await bucket.getWithEtag(key);
      bucket.set(key, found.text); // rewritten: same text, a new ETag
      return found;
    },
  };
  for (const argv of [['revoke', sam.id], ['set-channels', sam.id, 'nightly']]) {
    const puts = bucket.puts.length;
    const r = await cli(argv, { bucket: racing });
    assert.equal(r.code, 1);
    assert.equal(r.err, 'people: people.json changed while this ran, so nothing was written; list people and try again\n');
    assert.equal(bucket.puts.length, puts, 'nothing written');
  }
  const r = await cli(['add', 'Sam', 'main'], { bucket: racing, readStdin: async () => HASH_B, now: () => NOW, randomId: () => 'p-0000000000b2' });
  assert.match(r.err, /changed while this ran/);
});

test('a missing people.json is created only by init or add --init, and never over an existing one', async () => {
  for (const argv of [['add', 'Sam', 'main'], ['revoke', 'p-0000000000a1'], ['set-channels', 'p-0000000000a1', 'main']]) {
    const bucket = fakeBucket(null);
    const r = await cli(argv, { bucket, readStdin: async () => HASH_A, now: () => NOW, randomId: () => 'p-0000000000a1' });
    assert.equal(r.code, 1, argv.join(' '));
    assert.match(r.err, /not in the bucket; check HOLLOW_R2_BUCKET/);
    assert.equal(bucket.puts.length, 0);
  }
  const bucket = fakeBucket(null);
  const sam = await run(['add', '--init', 'Sam', 'main'], deps(bucket, { stdin: HASH_A }));
  assert.equal(sam.name, 'Sam');
  assert.equal(bucket.puts[0].opts.ifNoneMatch, '*', 'created only if still absent');
  assert.equal(parsePeople(bucket.store.get('people.json')).people.length, 1);
  // --init on an existing file just adds (conditional on its ETag).
  await run(['add', '--init', 'Sam', 'main'], deps(bucket, { stdin: HASH_B, ids: ['p-0000000000b2'] }));
  assert.equal(parsePeople(bucket.store.get('people.json')).people.length, 2);
  assert.ok(bucket.puts[1].opts.ifMatch);

  const fresh = fakeBucket(null);
  assert.deepEqual(await run(['init'], deps(fresh)), { people: [] });
  assert.equal(fresh.store.get('people.json'), EMPTY_FILE.replace('{"people":[]}', '{\n  "people": []\n}'));
  assert.equal(fresh.puts[0].opts.ifNoneMatch, '*');
  await assert.rejects(run(['init'], deps(fresh)), /already in the bucket/);
  assert.equal(fresh.puts.length, 1);
  // A file that appears between init's read and its write is not overwritten.
  const late = fakeBucket(null);
  const racing = { ...late, async getWithEtag(key) { const f = await late.getWithEtag(key); late.set(key, EMPTY_FILE); return f; } };
  await assert.rejects(run(['init'], deps(racing)), /changed while this ran/);
  assert.equal(late.puts.length, 0);
});

test('the script never prints a hash: no log or write names tokenHash or the hash', () => {
  const source = readFileSync(new URL('./release/people.mjs', import.meta.url), 'utf8');
  for (const line of source.split('\n')) {
    if (/console\.|\.write\(/.test(line)) assert.ok(!/tokenHash|hash\b/i.test(line), line);
  }
  assert.ok(!/console\.log/.test(source), 'nothing logs');
  assert.ok(!/argv[^\n]*tokenHash|tokenHash[^\n]*argv/.test(source), 'the hash never comes from argv');
});

test('the hidden-character set matches the fixture the Rust side pins (hand copy of Cc and Cf)', () => {
  const fixture = JSON.parse(readFileSync(new URL('./hidden-chars.fixture.json', import.meta.url), 'utf8'));
  const inFixture = (c) => fixture.ranges.some(([lo, hi]) => c >= lo && c <= hi);
  // Every character the fixture lists is refused here, on any Node.
  for (const [lo, hi] of fixture.ranges) {
    for (let c = lo; c <= hi; c++) assert.ok(HIDDEN_CHAR.test(String.fromCodePoint(c)), `U+${c.toString(16)} is in the fixture but not refused`);
  }
  // On the fixture's Unicode version the two agree exactly; a newer Node may
  // know more format characters, which is the cue to update the fixture and
  // update_client::people::hidden_char together (this part is skipped then).
  if (process.versions.unicode === fixture.unicode) {
    for (let c = 0; c <= 0x10FFFF; c++) {
      if (c >= 0xD800 && c <= 0xDFFF) continue;
      assert.equal(HIDDEN_CHAR.test(String.fromCodePoint(c)), inFixture(c), `U+${c.toString(16)}`);
    }
  }
});
