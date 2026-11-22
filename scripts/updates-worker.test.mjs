// The updates Worker (infra/updates-worker, row U3), run in Node against an
// in-memory stand-in for its R2 bucket binding. No network, no wrangler: the
// Worker's modules are plain ESM and Node 22+ has Request, Response and
// crypto.subtle. The clock is a counter the tests move by hand.
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { createHandler, parseRoute, parseRange, resolveRange, BODIES } from '../infra/updates-worker/src/worker.js';
import { hashToken, timingSafeEqual, findPerson, parsePeople, bearerToken, DEFAULT_PEOPLE_TTL_MS } from '../infra/updates-worker/src/auth.js';
import { parseManifest, pointsAtOwnArchive } from '../infra/updates-worker/src/manifest.js';
import { escapeHtml } from '../infra/updates-worker/src/install-page.js';
import { publishOrder, manifestPrerequisites } from '../infra/updates-worker/src/publish-order.js';
import { BUILD_FILES, buildKey, buildRecordKey, channelKey } from '../infra/updates-worker/src/layout.js';

const HOST = 'https://updates.test';
const ALICE = 'A'.repeat(40) + 'a_-'; // both channels
const BOB = 'B'.repeat(43); // revoked
const CAROL = 'C'.repeat(43); // main only
const PAT = 'D'.repeat(43); // no channels
const STRANGER = 'Z'.repeat(43); // not in people.json
const SIG = 'dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZQ==';
const TARBALL = 'tarball-bytes-0123456789';

async function peopleJson(overrides = {}) {
  return JSON.stringify({
    people: [
      { id: 'p-alice', name: 'Alice', tokenHash: await hashToken(ALICE), revoked: false, createdAt: '2026-11-15T10:00:00Z' },
      { id: 'p-bob', name: 'Bob', tokenHash: await hashToken(BOB), channels: ['main', 'nightly'], revoked: true, createdAt: '2026-11-15T10:00:00Z' },
      { id: 'p-carol', name: 'Carol', tokenHash: await hashToken(CAROL), channels: ['main'], revoked: false, createdAt: '2026-11-15T10:00:00Z' },
      { id: 'p-pat', name: 'Pat', tokenHash: await hashToken(PAT), channels: [], revoked: false, createdAt: '2026-11-15T10:00:00Z' },
    ],
    ...overrides,
  });
}

function manifest(version, channel, extra = {}) {
  return JSON.stringify({
    version,
    notes: 'What is new',
    pub_date: '2026-11-15T12:00:00Z',
    channel,
    commit: 'a'.repeat(40),
    buildId: `build-${version}`,
    schema: { sessions: 42, tasks: 7 },
    platforms: { 'darwin-aarch64': { signature: SIG, url: `https://elsewhere.example/builds/${version}/Hollow.app.tar.gz` } },
    ...extra,
  });
}

function buildFiles(version) {
  return {
    [`builds/${version}/Hollow.app.tar.gz`]: TARBALL,
    [`builds/${version}/Hollow.app.tar.gz.sig`]: `${SIG}\n`,
    [`builds/${version}/Hollow.dmg`]: 'dmg-bytes',
    [`builds/${version}/notes.json`]: '{"entries":[]}',
    // Stored for promotion (row U8), never served.
    [`builds/${version}/manifest.json`]: manifest(version, 'nightly'),
  };
}

/** An in-memory R2 bucket binding: get (with range and onlyIf etagMatches),
 *  head, and a call log. Each write gets a new etag. `afterHead` runs once
 *  after the next head (to overwrite a file between head and get). */
function fakeBucket(files) {
  const enc = new TextEncoder();
  let writes = 0;
  const etags = new Map();
  const store = new Map();
  const put = (k, v) => { store.set(k, enc.encode(v)); etags.set(k, `e${++writes}`); };
  for (const [k, v] of Object.entries(files)) put(k, v);
  const object = (key, bytes, size = bytes.length) => ({
    key,
    size,
    etag: etags.get(key),
    httpEtag: `"${etags.get(key)}"`,
    body: new Blob([bytes]).stream(),
    text: async () => new TextDecoder().decode(bytes),
    arrayBuffer: async () => bytes.slice().buffer,
  });
  return {
    store,
    calls: [],
    fail: false,
    afterHead: null,
    set(key, value) { put(key, value); },
    async get(key, options) {
      this.calls.push(['get', key]);
      if (this.fail) throw new Error(`storage failed for ${key}`);
      const bytes = store.get(key);
      if (!bytes) return null;
      if (options?.onlyIf?.etagMatches !== undefined && options.onlyIf.etagMatches !== etags.get(key)) {
        const { body, ...meta } = object(key, bytes);
        return meta; // R2: a failed precondition returns the object without a body
      }
      if (options?.range) {
        const { offset, length } = options.range;
        return object(key, bytes.slice(offset, offset + length), bytes.length);
      }
      return object(key, bytes);
    },
    async head(key) {
      this.calls.push(['head', key]);
      if (this.fail) throw new Error(`storage failed for ${key}`);
      const bytes = store.get(key);
      const result = bytes ? { ...object(key, bytes), body: undefined } : null;
      const hook = this.afterHead;
      this.afterHead = null;
      if (hook) hook(this);
      return result;
    },
  };
}

async function setup({ people, files, ...options } = {}) {
  const clock = { t: 1_000_000 };
  const logs = [];
  const bucket = fakeBucket({
    'people.json': people ?? (await peopleJson()),
    'channels/main.json': manifest('0.6.100', 'main'),
    'channels/nightly.json': manifest('0.6.105', 'nightly'),
    ...buildFiles('0.6.100'),
    ...buildFiles('0.6.105'),
    ...files,
  });
  const handler = createHandler({ now: () => clock.t, log: line => logs.push(line), ...options });
  const call = (path, { token, method = 'GET', headers = {} } = {}) => {
    const h = new Headers(headers);
    if (token !== undefined) h.set('Authorization', `Bearer ${token}`);
    return handler.fetch(new Request(`${HOST}${path}`, { method, headers: h }), { UPDATES: bucket });
  };
  return { bucket, clock, logs, call, handler };
}

async function snapshot(res) {
  return { status: res.status, body: await res.text(), headers: [...res.headers.entries()].sort() };
}

test('a valid token gets the channel manifest, pointed at this host', async () => {
  const { call } = await setup();
  const res = await call('/channels/nightly.json', { token: ALICE });
  assert.equal(res.status, 200);
  assert.equal(res.headers.get('content-type'), 'application/json; charset=utf-8');
  assert.equal(res.headers.get('cache-control'), 'no-store');
  assert.equal(res.headers.get('x-content-type-options'), 'nosniff');
  const body = await res.json();
  assert.equal(body.version, '0.6.105');
  assert.equal(body.channel, 'nightly');
  assert.deepEqual(body.schema, { sessions: 42, tasks: 7 });
  assert.deepEqual(body.platforms, { 'darwin-aarch64': { signature: SIG, url: `${HOST}/builds/0.6.105/Hollow.app.tar.gz` } });
});

test('revoked, unknown and malformed tokens all get the same 401', async () => {
  const { call } = await setup();
  const variants = [
    { token: BOB },
    { token: STRANGER },
    {},
    { headers: { Authorization: `Basic ${ALICE}` } },
    { headers: { Authorization: `Bearer  ${ALICE}` } },
    // (A trailing space is not tested: fetch's Headers trims header values.)
    { headers: { Authorization: `Bearer ${ALICE}x` } },
    { headers: { Authorization: `Bearer ${ALICE.slice(1)}` } },
    { headers: { Authorization: 'Bearer ' } },
    { headers: { Authorization: `bearer ${ALICE}` } },
  ];
  const results = [];
  for (const v of variants) results.push(await snapshot(await call('/channels/main.json', v)));
  assert.equal(results[0].status, 401);
  assert.equal(results[0].body, BODIES.unauthorized);
  for (const r of results) assert.deepEqual(r, results[0]);
  for (const r of results) assert.ok(!r.body.includes(ALICE) && !r.body.includes(BOB));
});

test('a channel the person is not allowed gets 403; absent channels means both', async () => {
  const { call } = await setup();
  assert.equal((await call('/channels/nightly.json', { token: CAROL })).status, 403);
  assert.equal((await call('/channels/main.json', { token: CAROL })).status, 200);
  assert.equal((await call('/channels/main.json', { token: ALICE })).status, 200);
  assert.equal((await call('/channels/main.json', { token: PAT })).status, 403);
  assert.equal((await call('/builds/0.6.100/Hollow.app.tar.gz', { token: PAT })).status, 403);
  assert.equal((await call(`/builds/0.6.100/Hollow.dmg?token=${PAT}`)).status, 403);
  const install = await call(`/install/${PAT}`);
  assert.equal(install.status, 403, 'an install link for someone with no channels');
  assert.equal(await install.text(), BODIES.forbidden);
});

test('a missing or inconsistent build makes the channel answer like an empty one', async () => {
  const empty = await setup({ files: {} });
  empty.bucket.store.delete('channels/nightly.json');
  const expected = await snapshot(await empty.call('/channels/nightly.json', { token: ALICE }));
  assert.equal(expected.status, 404);
  assert.equal(expected.body, BODIES.notFound);

  const breakages = [
    b => b.store.delete('builds/0.6.105/Hollow.app.tar.gz'),
    b => b.store.delete('builds/0.6.105/Hollow.app.tar.gz.sig'),
    b => b.store.delete('builds/0.6.105/notes.json'),
    b => b.set('builds/0.6.105/Hollow.app.tar.gz', ''),
    b => b.set('builds/0.6.105/Hollow.app.tar.gz.sig', 'another signature'),
    b => b.set('channels/nightly.json', manifest('0.6.105', 'main')),
    b => b.set('channels/nightly.json', manifest('0.6.105', 'nightly', { platforms: { 'darwin-aarch64': { signature: SIG, url: 'https://x.example/builds/0.6.100/Hollow.app.tar.gz' } } })),
    b => b.set('channels/nightly.json', manifest('0.6.105', 'nightly', { platforms: { 'darwin-aarch64': { signature: SIG, url: 'https://x.example/builds/0.6.105/Hollow.app.tar.gz?x=1' } } })),
    b => b.set('channels/nightly.json', manifest('0.6.105', 'nightly', { platforms: {} })),
    b => b.set('channels/nightly.json', manifest('0.6.105', 'nightly', { schema: { sessions: -1 } })),
    b => b.set('channels/nightly.json', manifest('../people', 'nightly')),
    b => b.set('channels/nightly.json', '{"version":'),
  ];
  for (const [i, breakIt] of breakages.entries()) {
    const { bucket, call } = await setup();
    breakIt(bucket);
    assert.deepEqual(await snapshot(await call('/channels/nightly.json', { token: ALICE })), expected, `breakage ${i}`);
  }
});

test('path traversal and anything off the routes is 404, and people.json is never served', async () => {
  const { call, bucket } = await setup();
  const paths = [
    '/people.json',
    '/channels/../people.json',
    '/channels/%2e%2e/people.json',
    '/builds/0.6.100/../../people.json',
    '/builds/0.6.100/%2e%2e/%2e%2e/people.json',
    '/builds/0.6.100/..%2F..%2Fpeople.json',
    '/builds/0.6.100%2FHollow.dmg',
    '/builds/0.6.100/Hollow.dmg%00',
    '/builds/0.6.100/Hollow.dmg/',
    '/builds//0.6.100/Hollow.dmg',
    '/builds/0.6.100\\..\\people.json',
    '/builds/0.6.0100/Hollow.dmg',
    '/builds/0.7.1/Hollow.dmg',
    '/builds/0.6.100/other.bin',
    '/builds/0.6.100/notes.JSON',
    // The stored build record (row U8) is never served.
    '/builds/0.6.100/manifest.json',
    '/channels/beta.json',
    '/channels/MAIN.json',
    '/channels/main.json/',
    '/channels',
    '/',
    '/install/',
    `/install/${ALICE}/x`,
  ];
  for (const path of paths) {
    const res = await call(path, { token: ALICE });
    assert.equal(res.status, 404, path);
    assert.equal(await res.text(), BODIES.notFound, path);
  }
  assert.ok(!bucket.calls.some(([, key]) => key !== 'people.json' && /people|\.\./.test(key)));
  assert.equal(parseRoute('/builds/0.6.1/../people.json'), null);
  assert.equal(parseRoute('/builds/./Hollow.dmg'), null);
  assert.deepEqual(parseRoute('/builds/0.6.1/Hollow.dmg'), { kind: 'build', version: '0.6.1', file: 'Hollow.dmg' });
});

test('a ?token= query is accepted on the DMG route only', async () => {
  const { call } = await setup();
  const dmg = await call(`/builds/0.6.100/Hollow.dmg?token=${ALICE}`);
  assert.equal(dmg.status, 200);
  assert.equal(await dmg.text(), 'dmg-bytes');
  assert.equal(dmg.headers.get('content-type'), 'application/x-apple-diskimage');
  assert.equal(dmg.headers.get('cache-control'), 'no-store', 'a response under ?token= is never stored');
  const dmgHeader = await call('/builds/0.6.100/Hollow.dmg', { token: ALICE });
  assert.equal(dmgHeader.headers.get('cache-control'), 'private, max-age=3600');

  const refused = [
    [`/channels/main.json?token=${ALICE}`, {}],
    [`/builds/0.6.100/Hollow.app.tar.gz?token=${ALICE}`, {}],
    [`/builds/0.6.100/notes.json?token=${ALICE}`, {}],
    [`/channels/main.json?token=${ALICE}`, { token: ALICE }], // even beside a valid header
    [`/install/${ALICE}?token=${ALICE}`, {}],
    [`/builds/0.6.100/Hollow.dmg?token=${ALICE}`, { token: ALICE }], // both at once
    [`/builds/0.6.100/Hollow.dmg?token=${ALICE}&token=${ALICE}`, {}],
    [`/builds/0.6.100/Hollow.dmg?token=${BOB}`, {}],
    [`/builds/0.6.100/Hollow.dmg?token=short`, {}],
  ];
  for (const [path, opts] of refused) {
    const res = await call(path, opts);
    assert.equal(res.status, 401, path);
    assert.equal(await res.text(), BODIES.unauthorized, path);
  }
});

test('the install page escapes what it shows and its DMG link works with the same token', async () => {
  const people = JSON.stringify({ people: [{ id: 'p-x', name: '<img src=x onerror="alert(1)">&\'`', tokenHash: await hashToken(ALICE), revoked: false, createdAt: '' }] });
  const { call } = await setup({ people });
  const res = await call(`/install/${ALICE}`);
  assert.equal(res.status, 200);
  assert.equal(res.headers.get('content-type'), 'text/html; charset=utf-8');
  assert.equal(res.headers.get('cache-control'), 'no-store');
  assert.equal(res.headers.get('referrer-policy'), 'no-referrer');
  assert.match(res.headers.get('content-security-policy'), /default-src 'none'/);
  const html = await res.text();
  assert.ok(!html.includes('<img'), 'name is escaped');
  assert.ok(html.includes('&#60;img src&#61;x onerror&#61;&#34;alert(1)&#34;&#62;&#38;&#39;&#96;'));
  assert.ok(!html.includes('<script'));
  assert.ok(html.includes(`href="hollow://updates/connect?token&#61;${ALICE}"`));
  assert.ok(html.includes('Connect Hollow'));
  const m = /href="(\/builds\/0\.6\.100\/Hollow\.dmg\?token&#61;[^"]+)"/.exec(html);
  assert.ok(m, 'main channel DMG link');
  const href = m[1].replace(/&#61;/g, '=');
  const dmg = await call(href);
  assert.equal(dmg.status, 200);
  assert.equal(await dmg.text(), 'dmg-bytes');
  assert.equal(escapeHtml('a"b<c>&\'`='), 'a&#34;b&#60;c&#62;&#38;&#39;&#96;&#61;');
});

test('a bad or revoked install link gets a plain 401 that does not echo the token', async () => {
  const { call } = await setup();
  for (const token of [BOB, STRANGER]) {
    const res = await call(`/install/${token}`);
    assert.equal(res.status, 401);
    assert.equal(res.headers.get('content-type'), 'text/plain; charset=utf-8');
    const body = await res.text();
    assert.equal(body, BODIES.unauthorized);
    assert.ok(!body.includes(token));
  }
  const odd = await call('/install/%3Cscript%3E');
  assert.equal(odd.status, 404);
  assert.ok(!(await odd.text()).includes('script'));
  // An install path with a header is refused too: the page takes its token from the path only.
  assert.equal((await call(`/install/${ALICE}`, { token: ALICE })).status, 401);
});

test('the install page offers no download when no complete build has a DMG', async () => {
  const { call, bucket } = await setup();
  bucket.store.delete('builds/0.6.100/Hollow.dmg');
  const html = await (await call(`/install/${CAROL}`)).text();
  assert.ok(!html.includes('Hollow.dmg?'));
  assert.ok(html.includes('No build is published'));
  assert.ok(html.includes('hollow://updates/connect'));
});

test('token hashes are compared in constant time, against every person', async () => {
  const seen = [];
  const equal = (a, b) => {
    seen.push([a, b]);
    return timingSafeEqual(a, b);
  };
  const { call } = await setup({ equal });
  assert.equal((await call('/channels/main.json', { token: ALICE })).status, 200);
  assert.equal(seen.length, 4, 'no early exit after the first match');
  const aliceHash = await hashToken(ALICE);
  assert.ok(seen.every(([, b]) => b === aliceHash));

  assert.equal(timingSafeEqual('abc', 'abc'), true);
  assert.equal(timingSafeEqual('abc', 'abd'), false);
  assert.equal(timingSafeEqual('abc', 'abcd'), false);
  assert.equal(timingSafeEqual('', ''), true);
  assert.equal(timingSafeEqual('', 'a'), false);
  const people = parsePeople(await peopleJson());
  assert.ok(people.ok);
  assert.equal(findPerson(people.people, await hashToken(BOB)), null, 'revoked');
  assert.equal(findPerson(people.people, await hashToken(CAROL)).id, 'p-carol');
  assert.equal(await hashToken('abc'), 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
  assert.equal(bearerToken(`Bearer ${ALICE}`), ALICE);
  assert.equal(bearerToken(null), null);
});

test('HEAD answers like GET with no body', async () => {
  const { call } = await setup();
  const manifestHead = await call('/channels/main.json', { token: ALICE, method: 'HEAD' });
  assert.equal(manifestHead.status, 200);
  assert.equal(manifestHead.headers.get('content-type'), 'application/json; charset=utf-8');
  assert.equal(await manifestHead.text(), '');
  const buildHead = await call('/builds/0.6.100/Hollow.app.tar.gz', { token: ALICE, method: 'HEAD' });
  assert.equal(buildHead.status, 200);
  assert.equal(buildHead.headers.get('content-length'), String(TARBALL.length));
  assert.equal(buildHead.headers.get('accept-ranges'), 'bytes');
  assert.ok(buildHead.headers.get('etag'));
  assert.equal(await buildHead.text(), '');
  const denied = await call('/channels/main.json', { token: BOB, method: 'HEAD' });
  assert.equal(denied.status, 401);
  assert.equal(await denied.text(), '');
  const install = await call(`/install/${ALICE}`, { method: 'HEAD' });
  assert.equal(install.status, 200);
  assert.equal(await install.text(), '');
  assert.equal((await call('/builds/0.6.100/missing', { token: ALICE, method: 'HEAD' })).status, 404);
  assert.equal((await call('/channels/main.json', { token: ALICE, method: 'POST' })).status, 405);
});

test('builds are served whole or by a single byte range', async () => {
  const { call } = await setup();
  const path = '/builds/0.6.100/Hollow.app.tar.gz';
  const full = await call(path, { token: ALICE });
  assert.equal(full.status, 200);
  assert.equal(await full.text(), TARBALL);
  assert.equal(full.headers.get('content-type'), 'application/gzip');
  assert.equal(full.headers.get('cache-control'), 'private, max-age=3600');

  const cases = [
    ['bytes=0-3', 206, 'tarb', 'bytes 0-3/24'],
    ['bytes=20-', 206, '6789', 'bytes 20-23/24'],
    ['bytes=-4', 206, '6789', 'bytes 20-23/24'],
    ['bytes=10-999', 206, TARBALL.slice(10), 'bytes 10-23/24'],
    ['bytes=-999', 206, TARBALL, 'bytes 0-23/24'],
  ];
  for (const [range, status, body, contentRange] of cases) {
    const res = await call(path, { token: ALICE, headers: { Range: range } });
    assert.equal(res.status, status, range);
    assert.equal(res.headers.get('content-range'), contentRange, range);
    assert.equal(res.headers.get('content-length'), String(body.length), range);
    assert.equal(await res.text(), body, range);
  }
  for (const range of ['bytes=24-', 'bytes=-0']) {
    const res = await call(path, { token: ALICE, headers: { Range: range } });
    assert.equal(res.status, 416, range);
    assert.equal(res.headers.get('content-range'), 'bytes */24');
  }
  for (const range of ['bytes=0-1,4-5', 'bytes=5-2', 'items=0-1', 'bytes=-', 'bytes=abc']) {
    const res = await call(path, { token: ALICE, headers: { Range: range } });
    assert.equal(res.status, 200, range);
    assert.equal(await res.text(), TARBALL, range);
  }
  const ifRange = await call(path, { token: ALICE, headers: { Range: 'bytes=0-3', 'If-Range': '"x"' } });
  assert.equal(ifRange.status, 200);
  assert.equal((await call('/builds/0.6.999/Hollow.app.tar.gz', { token: ALICE, headers: { Range: 'bytes=0-3' } })).status, 404);
  assert.deepEqual(parseRange('bytes=1-2'), { start: 1, end: 2 });
  assert.deepEqual(resolveRange({ start: 0, end: null }, 0), 'unsatisfiable');
});

test('a revoke takes effect once the people.json cache expires (fake clock)', async () => {
  const { call, bucket, clock } = await setup();
  assert.equal(DEFAULT_PEOPLE_TTL_MS, 60_000);
  assert.equal((await call('/channels/main.json', { token: CAROL })).status, 200);
  const revoked = JSON.parse(await peopleJson());
  revoked.people[2].revoked = true;
  bucket.set('people.json', JSON.stringify(revoked));
  clock.t += DEFAULT_PEOPLE_TTL_MS - 1;
  assert.equal((await call('/channels/main.json', { token: CAROL })).status, 200, 'still cached inside the TTL');
  clock.t += 1;
  assert.equal((await call('/channels/main.json', { token: CAROL })).status, 401, 'revoked at the TTL');
  // A clock that runs backwards rereads rather than trusting the copy.
  bucket.set('people.json', await peopleJson());
  clock.t -= 10;
  assert.equal((await call('/channels/main.json', { token: CAROL })).status, 200);
  // A shorter TTL is honoured.
  const quick = await setup({ ttlMs: 5_000 });
  assert.equal((await quick.call('/channels/main.json', { token: CAROL })).status, 200);
  quick.bucket.set('people.json', JSON.stringify(revoked));
  quick.clock.t += 5_000;
  assert.equal((await quick.call('/channels/main.json', { token: CAROL })).status, 401);
});

test('a malformed or missing people.json lets nobody in', async () => {
  const alice = await hashToken(ALICE);
  const good = { id: 'p-a', name: 'A', tokenHash: alice, revoked: false, createdAt: '' };
  const broken = [
    'not json',
    'null',
    '[]',
    '{"people":{}}',
    JSON.stringify({ people: [{ ...good, revoked: 'false' }] }),
    JSON.stringify({ people: [{ ...good, revoked: undefined }] }),
    JSON.stringify({ people: [{ ...good, tokenHash: alice.toUpperCase() }] }),
    JSON.stringify({ people: [{ ...good, tokenHash: alice.slice(1) }] }),
    JSON.stringify({ people: [{ ...good, channels: ['main', 'beta'] }] }),
    JSON.stringify({ people: [{ ...good, channels: 'main' }] }),
    JSON.stringify({ people: [{ ...good, id: '' }] }),
    JSON.stringify({ people: [good, { ...good, id: 'p-b' }] }),
    JSON.stringify({ people: [good, { ...good, tokenHash: 'f'.repeat(64) }] }),
    JSON.stringify({ people: [good, null] }),
  ];
  for (const people of broken) {
    assert.equal(parsePeople(people).ok, false, people);
    const { call } = await setup({ people });
    const res = await call('/channels/main.json', { token: ALICE });
    assert.equal(res.status, 401, people);
    assert.equal(await res.text(), BODIES.unauthorized);
  }
  assert.equal(parsePeople(JSON.stringify({ people: [good] })).ok, true);
  const missing = await setup();
  missing.bucket.store.delete('people.json');
  assert.equal((await missing.call('/channels/main.json', { token: ALICE })).status, 401);
});

test('a storage failure is a 503 and logs nothing secret', async () => {
  const { call, bucket, logs } = await setup();
  bucket.fail = true;
  const res = await call('/channels/main.json', { token: ALICE });
  assert.equal(res.status, 503);
  assert.equal(await res.text(), BODIES.unavailable);
  const install = await call(`/install/${ALICE}`);
  assert.equal(install.status, 503);
  assert.equal(logs.length, 2);
  const hash = await hashToken(ALICE);
  for (const line of logs) assert.ok(!line.includes(ALICE) && !line.includes(hash) && !line.includes('people'), line);
  const unbound = await createHandler({ log: line => logs.push(line) }).fetch(new Request(`${HOST}/channels/main.json`), {});
  assert.equal(unbound.status, 503);
});

test('manifest checks', () => {
  assert.ok(parseManifest(manifest('0.6.1', 'main'), 'main'));
  assert.equal(parseManifest(manifest('0.6.1', 'main'), 'nightly'), null);
  assert.equal(parseManifest(manifest('0.6.1', 'main', { commit: 'nope' }), 'main'), null);
  assert.equal(parseManifest(manifest('0.6.1', 'main', { pub_date: 'yesterday' }), 'main'), null);
  assert.ok(pointsAtOwnArchive('/builds/0.6.1/Hollow.app.tar.gz', '0.6.1'));
  assert.equal(pointsAtOwnArchive('http://x.example/builds/0.6.1/Hollow.app.tar.gz', '0.6.1'), false);
  assert.equal(pointsAtOwnArchive('https://x.example/builds/0.6.1/Hollow.dmg', '0.6.1'), false);
});

test('publish order: artifacts, the build record, then notes.json, then the channel manifests last', () => {
  const steps = publishOrder({ version: '0.6.120', channels: ['nightly'] });
  assert.deepEqual(steps, [
    { key: 'builds/0.6.120/Hollow.app.tar.gz', kind: 'artifact' },
    { key: 'builds/0.6.120/Hollow.app.tar.gz.sig', kind: 'artifact' },
    { key: 'builds/0.6.120/Hollow.dmg', kind: 'artifact' },
    { key: 'builds/0.6.120/manifest.json', kind: 'record' },
    { key: 'builds/0.6.120/notes.json', kind: 'notes' },
    { key: 'channels/nightly.json', kind: 'manifest', channel: 'nightly' },
  ]);
  const both = publishOrder({ version: '0.6.120', channels: ['nightly', 'main'] });
  assert.deepEqual(both.slice(-2).map(s => s.key), ['channels/nightly.json', 'channels/main.json']);
  // Every file the Worker needs before serving a manifest comes before it.
  const firstManifest = both.findIndex(s => s.kind === 'manifest');
  for (const key of manifestPrerequisites('0.6.120')) {
    const i = both.findIndex(s => s.key === key);
    assert.ok(i >= 0 && i < firstManifest, key);
  }
  assert.equal(both.filter(s => s.kind !== 'manifest').length, BUILD_FILES.length + 1, 'every build file plus the record');
  assert.equal(buildRecordKey('0.6.120'), 'builds/0.6.120/manifest.json');
  assert.throws(() => buildRecordKey('0.6.1/../x'));
  for (const bad of [
    { version: '0.6.1/../x', channels: ['main'] },
    { version: '0.6.01', channels: ['main'] },
    { version: '0.6.1', channels: [] },
    { version: '0.6.1', channels: ['beta'] },
    { version: '0.6.1', channels: ['main', 'main'] },
  ]) assert.throws(() => publishOrder(bad));
  assert.throws(() => buildKey('0.6.1', '../people.json'));
  assert.throws(() => channelKey('people'));
});

test('a ranged read never mixes a file overwritten between head and get', async () => {
  const { call, bucket, logs } = await setup();
  const path = '/builds/0.6.100/Hollow.app.tar.gz';
  const key = 'builds/0.6.100/Hollow.app.tar.gz';
  const NEWER = 'a-longer-replacement-tarball-body';
  // One overwrite: the first read's precondition fails, the retry reads the new file whole.
  bucket.afterHead = b => b.set(key, NEWER);
  const res = await call(path, { token: ALICE, headers: { Range: 'bytes=0-3' } });
  assert.equal(res.status, 206);
  assert.equal(await res.text(), 'a-lo');
  assert.equal(res.headers.get('content-range'), `bytes 0-3/${NEWER.length}`);
  const etag = res.headers.get('etag');
  const head = await call(path, { token: ALICE, method: 'HEAD' });
  assert.equal(etag, head.headers.get('etag'), 'headers come from the object read');
  assert.ok(bucket.calls.some(([op, k]) => op === 'get' && k === key));

  // Overwritten on every attempt: a 503, never bytes from one version under another's headers.
  let n = 0;
  const churn = b => { b.set(key, `version-${++n}-${'x'.repeat(n)}`); b.afterHead = churn; };
  bucket.afterHead = churn;
  const busy = await call(path, { token: ALICE, headers: { Range: 'bytes=0-3' } });
  bucket.afterHead = null;
  assert.equal(busy.status, 503);
  assert.equal(await busy.text(), BODIES.unavailable);
  assert.equal(logs.length, 1);
  assert.ok(!logs[0].includes(ALICE) && !logs[0].includes('builds/'));
});

test('an invalid people.json logs one fixed line per cache fill', async () => {
  const { call, clock, logs } = await setup({ people: 'not json' });
  for (let i = 0; i < 3; i++) assert.equal((await call('/channels/main.json', { token: ALICE })).status, 401);
  assert.deepEqual(logs, ['updates-worker: people.json is invalid, nobody is let in']);
  clock.t += DEFAULT_PEOPLE_TTL_MS;
  await call('/channels/main.json', { token: ALICE });
  assert.equal(logs.length, 2);
});

test('the served manifest carries only checked fields', async () => {
  const { call, bucket } = await setup();
  bucket.set('channels/main.json', manifest('0.6.100', 'main', {
    extra: '<script>',
    platforms: {
      'darwin-aarch64': { signature: SIG, url: '/builds/0.6.100/Hollow.app.tar.gz', with: 'extra' },
      'windows-x86_64': { signature: 'x', url: 'https://evil.example/x' },
    },
  }));
  const body = await (await call('/channels/main.json', { token: ALICE })).json();
  assert.deepEqual(Object.keys(body).sort(), ['buildId', 'channel', 'commit', 'notes', 'platforms', 'pub_date', 'schema', 'version']);
  assert.deepEqual(body.platforms, { 'darwin-aarch64': { signature: SIG, url: `${HOST}/builds/0.6.100/Hollow.app.tar.gz` } });
  bucket.set('channels/main.json', manifest('0.6.100', 'main', { notes: undefined, commit: undefined, buildId: undefined, schema: undefined, pub_date: undefined }));
  const bare = await (await call('/channels/main.json', { token: ALICE })).json();
  assert.deepEqual(Object.keys(bare).sort(), ['channel', 'platforms', 'version']);
  bucket.set('channels/main.json', manifest('0.6.100', 'main', { schema: { 'bad key!': 1 } }));
  assert.equal((await call('/channels/main.json', { token: ALICE })).status, 404);
});

// ── /me (row U9) ──────────────────────────────────────────────────────────

test('/me answers the person\'s name and allowed channels, and nothing else', async () => {
  const { call } = await setup();
  const alice = await call('/me', { token: ALICE });
  assert.equal(alice.status, 200);
  assert.equal(alice.headers.get('content-type'), 'application/json; charset=utf-8');
  assert.equal(alice.headers.get('cache-control'), 'no-store');
  assert.equal(alice.headers.get('x-content-type-options'), 'nosniff');
  assert.equal(alice.headers.get('referrer-policy'), 'no-referrer');
  const aliceText = await alice.text();
  assert.deepEqual(JSON.parse(aliceText), { name: 'Alice', channels: ['main', 'nightly'] }, 'channels absent means both');
  assert.deepEqual(Object.keys(JSON.parse(aliceText)), ['name', 'channels']);
  assert.deepEqual(await (await call('/me', { token: CAROL })).json(), { name: 'Carol', channels: ['main'] });
  assert.deepEqual(await (await call('/me', { token: PAT })).json(), { name: 'Pat', channels: [] }, '[] is an answer, not a refusal');

  // No hash, id, revoked flag or anyone else's entry, whoever asks.
  const people = JSON.parse(await peopleJson()).people;
  for (const token of [ALICE, CAROL, PAT]) {
    const text = await (await call('/me', { token })).text();
    for (const p of people) {
      assert.ok(!text.includes(p.tokenHash), 'no token hash');
      assert.ok(!text.includes(p.id), 'no id');
    }
    assert.ok(!text.includes(token), 'not the token itself');
    assert.ok(!/revoked|createdAt|tokenHash/.test(text));
  }
});

test('/me orders channels as CHANNELS does, whatever people.json says', async () => {
  const people = JSON.stringify({ people: [{ id: 'p-e', name: 'Eve', tokenHash: await hashToken(ALICE), channels: ['nightly', 'main'], revoked: false }] });
  const { call } = await setup({ people });
  assert.deepEqual(await (await call('/me', { token: ALICE })).json(), { name: 'Eve', channels: ['main', 'nightly'] });
});

test('/me gives revoked, unknown and malformed tokens the one identical 401', async () => {
  const { call } = await setup();
  const variants = [
    { token: BOB },
    { token: STRANGER },
    {},
    { headers: { Authorization: `Basic ${ALICE}` } },
    { headers: { Authorization: `Bearer ${ALICE}x` } },
    { headers: { Authorization: 'Bearer ' } },
  ];
  const reference = await snapshot(await call('/channels/main.json', { token: BOB }));
  for (const v of variants) {
    const r = await snapshot(await call('/me', v));
    assert.deepEqual(r, reference, 'the same answer as every other route\'s 401');
    assert.ok(!r.body.includes(BOB) && !r.body.includes(ALICE));
  }
  // A query token is never accepted here, not even beside a good header.
  assert.deepEqual(await snapshot(await call(`/me?token=${ALICE}`)), reference);
  assert.deepEqual(await snapshot(await call(`/me?token=${ALICE}`, { token: ALICE })), reference);
});

test('/me supports HEAD, refuses other methods and other shapes of the path', async () => {
  const { call } = await setup();
  const head = await call('/me', { token: ALICE, method: 'HEAD' });
  assert.equal(head.status, 200);
  assert.equal(head.headers.get('cache-control'), 'no-store');
  assert.equal(await head.text(), '');
  const headBad = await call('/me', { token: STRANGER, method: 'HEAD' });
  assert.equal(headBad.status, 401);
  assert.equal(await headBad.text(), '');
  assert.equal((await call('/me', { token: ALICE, method: 'POST' })).status, 405);
  for (const path of ['/me/', '/me.json', '/ME', '/me/x', '//me', '/%6de']) {
    assert.equal((await call(path, { token: ALICE })).status, 404, path);
  }
});

test('/me follows a revoke once the people.json cache expires, and fails closed', async () => {
  const { call, bucket, clock } = await setup();
  assert.equal((await call('/me', { token: CAROL })).status, 200);
  const people = JSON.parse(await peopleJson());
  people.people.find(p => p.id === 'p-carol').revoked = true;
  bucket.set('people.json', JSON.stringify(people));
  clock.t += DEFAULT_PEOPLE_TTL_MS;
  assert.equal((await call('/me', { token: CAROL })).status, 401);
  bucket.set('people.json', '{"people": [');
  clock.t += DEFAULT_PEOPLE_TTL_MS;
  assert.equal((await call('/me', { token: ALICE })).status, 401, 'an invalid list lets nobody in');
  bucket.fail = true;
  clock.t += DEFAULT_PEOPLE_TTL_MS;
  assert.equal((await call('/me', { token: ALICE })).status, 503);
});

test('parseRoute knows /me and nothing near it', () => {
  assert.deepEqual(parseRoute('/me'), { kind: 'me' });
  for (const p of ['/me/', '/me?', '/mee', '/me/..', '/./me']) assert.equal(parseRoute(p), null, p);
});
