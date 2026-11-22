// Tests for scripts/release/r2-uploader.mjs (updates row U2): SigV4 against
// AWS's published examples, and the uploader against a fake fetch. No
// network call is ever made.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { test } from 'node:test';
import { createR2Uploader, missingR2Config, signRequest, uriEncode } from './release/r2-uploader.mjs';

const EMPTY = createHash('sha256').update('').digest('hex');
const AWS = { accessKeyId: 'AKIAIOSFODNN7EXAMPLE', secretAccessKey: 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY', region: 'us-east-1', service: 's3', date: new Date('2013-05-24T00:00:00Z') };
const sig = (h) => /Signature=([0-9a-f]{64})$/.exec(h.authorization)[1];

test('SigV4 matches the AWS S3 documentation examples', () => {
  // "Example: GET Object" (Authenticating Requests: Using the Authorization Header).
  const getObject = signRequest({ ...AWS, method: 'GET', host: 'examplebucket.s3.amazonaws.com', path: '/test.txt', headers: { range: 'bytes=0-9' }, payloadHash: EMPTY });
  assert.equal(sig(getObject), 'f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41');
  assert.match(getObject.authorization, /SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,/);
  // "Example: GET Bucket (List Objects)".
  const list = signRequest({ ...AWS, method: 'GET', host: 'examplebucket.s3.amazonaws.com', path: '/', query: { 'max-keys': '2', prefix: 'J' }, payloadHash: EMPTY });
  assert.equal(sig(list), '34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7');
});

test('uriEncode keeps unreserved characters and slashes only where asked', () => {
  assert.equal(uriEncode('builds/0.6.1/Hollow.app.tar.gz', true), 'builds/0.6.1/Hollow.app.tar.gz');
  assert.equal(uriEncode('a b/c+d~', false), 'a%20b%2Fc%2Bd~');
});

const ENV = { HOLLOW_R2_ACCESS_KEY_ID: 'R2-ACCESS-SECRET-1', HOLLOW_R2_SECRET_ACCESS_KEY: 'R2-SECRET-SECRET-2', HOLLOW_R2_ENDPOINT: 'https://acct.r2.cloudflarestorage.com' };

function fakeFetch(answers) {
  const calls = [];
  const fetch = async (url, init) => {
    calls.push({ url, ...init });
    const a = answers.shift() ?? { status: 200, body: '' };
    return { status: a.status, ok: a.status >= 200 && a.status < 300, text: async () => a.body ?? '' };
  };
  return { fetch, calls };
}

test('put, get, delete and a paged list go to the bucket path with signed headers', async () => {
  const pages = [
    { status: 200, body: '<ListBucketResult><Contents><Key>builds/0.6.1/a</Key></Contents><IsTruncated>true</IsTruncated><NextContinuationToken>tok&amp;1</NextContinuationToken></ListBucketResult>' },
    { status: 200, body: '<ListBucketResult><Contents><Key>builds/0.6.2/b</Key></Contents><IsTruncated>false</IsTruncated></ListBucketResult>' },
  ];
  const { fetch, calls } = fakeFetch([{ status: 200 }, { status: 200, body: '{"v":1}' }, { status: 404 }, { status: 204 }, { status: 404 }, ...pages]);
  const up = createR2Uploader({ env: ENV, fetch, now: () => new Date('2026-11-15T12:00:00Z') });
  await up.put('builds/0.6.1/Hollow.app.tar.gz', Buffer.from('bytes'), { contentType: 'application/gzip' });
  assert.equal(await up.get('channels/nightly.json'), '{"v":1}');
  assert.equal(await up.get('channels/main.json'), null);
  await up.delete('builds/0.6.1/a');
  await up.delete('builds/0.6.1/gone');
  assert.deepEqual(await up.list('builds/'), ['builds/0.6.1/a', 'builds/0.6.2/b']);

  assert.equal(calls[0].url, 'https://acct.r2.cloudflarestorage.com/hollow-updates/builds/0.6.1/Hollow.app.tar.gz');
  assert.equal(calls[0].method, 'PUT');
  assert.equal(calls[0].headers['x-amz-content-sha256'], createHash('sha256').update('bytes').digest('hex'));
  assert.equal(calls[0].headers['content-type'], 'application/gzip');
  assert.match(calls[0].headers.authorization, /^AWS4-HMAC-SHA256 Credential=R2-ACCESS-SECRET-1\/20261115\/auto\/s3\/aws4_request, SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date, Signature=[0-9a-f]{64}$/);
  assert.equal(calls[5].url, 'https://acct.r2.cloudflarestorage.com/hollow-updates?list-type=2&prefix=builds%2F');
  assert.equal(calls[6].url, 'https://acct.r2.cloudflarestorage.com/hollow-updates?continuation-token=tok%261&list-type=2&prefix=builds%2F');
  for (const c of calls) assert.ok(!JSON.stringify(c).includes('R2-SECRET-SECRET-2'), 'the secret key is never sent');
  for (const c of calls) assert.ok(c.signal instanceof AbortSignal, 'every request has a time limit');
});

test('errors name the status and S3 code only, never a credential', async () => {
  const { fetch } = fakeFetch([{ status: 403, body: '<Error><Code>AccessDenied</Code><Message>key R2-ACCESS-SECRET-1 echoed</Message></Error>' }]);
  const up = createR2Uploader({ env: ENV, fetch });
  await assert.rejects(up.put('channels/nightly.json', '{}'), (e) => {
    assert.equal(e.message, 'R2 PUT channels/nightly.json answered 403 (AccessDenied)');
    return true;
  });
  const broken = createR2Uploader({ env: ENV, fetch: async () => { throw new TypeError('fetch failed R2-SECRET-SECRET-2'); } });
  await assert.rejects(broken.get('x'), /^Error: R2 GET x failed: TypeError$/);
});

test('missing or bad settings are refused by name', () => {
  assert.deepEqual(missingR2Config({}), ['HOLLOW_R2_ACCESS_KEY_ID', 'HOLLOW_R2_SECRET_ACCESS_KEY', 'HOLLOW_R2_ENDPOINT']);
  assert.throws(() => createR2Uploader({ env: {} }), /needs HOLLOW_R2_ACCESS_KEY_ID/);
  assert.throws(() => createR2Uploader({ env: { ...ENV, HOLLOW_R2_ENDPOINT: 'http://acct.r2.cloudflarestorage.com' } }), /must be https/);
  assert.throws(() => createR2Uploader({ env: { ...ENV, HOLLOW_R2_BUCKET: 'Bad_Bucket' } }), /not a bucket name/);
});

test('a request that runs past its limit fails with a timeout, naming no credential', async () => {
  const hang = async (url, init) => new Promise((_, reject) => {
    if (init.signal.aborted) { reject(init.signal.reason); return; }
    init.signal.addEventListener('abort', () => reject(init.signal.reason));
  });
  const up = createR2Uploader({ env: ENV, fetch: hang, timeoutMs: 50, putTimeoutMs: 50 });
  await assert.rejects(up.get('channels/nightly.json'), /^Error: R2 GET channels\/nightly\.json failed: TimeoutError$/);
});

test('a conditional put signs If-Match or If-None-Match; getWithEtag returns the ETag; a 412 carries its status', async () => {
  const calls = [];
  const answers = [{ status: 200, body: '{"people":[]}', etag: '"abc"' }, { status: 404 }, { status: 200 }, { status: 412, body: '<Error><Code>PreconditionFailed</Code></Error>' }];
  const fetch = async (url, init) => {
    calls.push({ url, ...init });
    const a = answers.shift();
    return { status: a.status, ok: a.status >= 200 && a.status < 300, text: async () => a.body ?? '', headers: { get: (n) => (n === 'etag' ? a.etag ?? null : null) } };
  };
  const up = createR2Uploader({ env: ENV, fetch, now: () => new Date('2026-11-15T12:00:00Z') });
  assert.deepEqual(await up.getWithEtag('people.json'), { text: '{"people":[]}', etag: '"abc"' });
  assert.equal(await up.getWithEtag('people.json'), null);
  await up.put('people.json', '{}', { contentType: 'application/json', ifMatch: '"abc"' });
  assert.equal(calls[2].headers['if-match'], '"abc"');
  assert.match(calls[2].headers.authorization, /SignedHeaders=content-type;host;if-match;x-amz-content-sha256;x-amz-date,/);
  await assert.rejects(up.put('people.json', '{}', { ifNoneMatch: '*' }), (e) => {
    assert.equal(e.status, 412);
    assert.equal(e.message, 'R2 PUT people.json answered 412 (PreconditionFailed)');
    return true;
  });
  assert.equal(calls[3].headers['if-none-match'], '*');
  assert.ok(!('if-match' in calls[3].headers));
});
