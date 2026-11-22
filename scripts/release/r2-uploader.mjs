// The release builder's uploader for Cloudflare R2 (updates design section 4,
// row U2), over R2's S3-compatible API with AWS Signature Version 4, using
// node's crypto and fetch only (no dependency). `fetch` is injectable: the
// tests never make a network call.
//
// Interface (the one scripts/release/nightly.mjs and its fake bucket share):
//   put(key, bytes, { contentType, ifMatch, ifNoneMatch })   PUT one object
//                                      (a failed condition throws, status 412)
//   get(key)                           the object's text, or null on 404
//   getWithEtag(key)                   { text, etag }, or null on 404
//   list(prefix)                       every key under prefix (ListObjectsV2, all pages)
//   delete(key)                        DELETE one object (404 counts as done)
//
// Environment (values are never printed, logged or put in an error):
//   HOLLOW_R2_ACCESS_KEY_ID, HOLLOW_R2_SECRET_ACCESS_KEY
//   HOLLOW_R2_ENDPOINT   https://<account id>.r2.cloudflarestorage.com
//   HOLLOW_R2_BUCKET     default hollow-updates
// Requests are path-style (`<endpoint>/<bucket>/<key>`), region `auto`.
import { createHash, createHmac } from 'node:crypto';

export const REGION = 'auto';
export const SERVICE = 's3';
const EMPTY_SHA256 = createHash('sha256').update('').digest('hex');

const sha256Hex = (data) => createHash('sha256').update(data).digest('hex');
const hmac = (key, data) => createHmac('sha256', key).update(data).digest();

/** RFC 3986 percent-encoding as SigV4 wants it; `/` kept when `keepSlash`. */
export function uriEncode(text, keepSlash = false) {
  let out = '';
  for (const byte of Buffer.from(String(text), 'utf8')) {
    const c = String.fromCharCode(byte);
    if (/[A-Za-z0-9\-._~]/.test(c) || (keepSlash && c === '/')) out += c;
    else out += `%${byte.toString(16).toUpperCase().padStart(2, '0')}`;
  }
  return out;
}

/** `20130524T000000Z` for a Date. */
export function amzDate(date) {
  return date.toISOString().replace(/[-:]/g, '').replace(/\.\d{3}Z$/, 'Z');
}

/**
 * Signs one request. `headers` must already hold every header to sign
 * (lowercase names); `host`, `x-amz-date` and `x-amz-content-sha256` are
 * added here. Returns the headers to send, `Authorization` included.
 * Pure, so the AWS documentation's examples test it.
 */
export function signRequest({ method, host, path, query = {}, headers = {}, payloadHash, accessKeyId, secretAccessKey, region = REGION, service = SERVICE, date }) {
  const stamp = amzDate(date);
  const day = stamp.slice(0, 8);
  const all = { ...headers, host, 'x-amz-date': stamp, 'x-amz-content-sha256': payloadHash };
  const names = Object.keys(all).map(n => n.toLowerCase()).sort();
  const lower = Object.fromEntries(Object.entries(all).map(([k, v]) => [k.toLowerCase(), v]));
  const canonicalHeaders = names.map(n => `${n}:${String(lower[n]).trim().replace(/\s+/g, ' ')}\n`).join('');
  const signedHeaders = names.join(';');
  const canonicalQuery = Object.keys(query).sort().map(k => `${uriEncode(k)}=${uriEncode(query[k])}`).join('&');
  const canonicalRequest = [method, uriEncode(path, true), canonicalQuery, canonicalHeaders, signedHeaders, payloadHash].join('\n');
  const scope = `${day}/${region}/${service}/aws4_request`;
  const toSign = ['AWS4-HMAC-SHA256', stamp, scope, sha256Hex(canonicalRequest)].join('\n');
  const key = hmac(hmac(hmac(hmac(`AWS4${secretAccessKey}`, day), region), service), 'aws4_request');
  const signature = createHmac('sha256', key).update(toSign).digest('hex');
  const out = { ...all };
  out.authorization = `AWS4-HMAC-SHA256 Credential=${accessKeyId}/${scope}, SignedHeaders=${signedHeaders}, Signature=${signature}`;
  return out;
}

/** Which R2 settings are missing from `env` (names only). */
export function missingR2Config(env = process.env) {
  return ['HOLLOW_R2_ACCESS_KEY_ID', 'HOLLOW_R2_SECRET_ACCESS_KEY', 'HOLLOW_R2_ENDPOINT']
    .filter(k => typeof env[k] !== 'string' || env[k].trim() === '');
}

const xmlText = (s) => s.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&amp;/g, '&');

/** The S3 error `<Code>` of a response body, or nothing; never the message (it can echo request text). */
function errorCode(body) {
  const m = /<Code>([A-Za-z0-9.]{1,64})<\/Code>/.exec(body ?? '');
  return m ? ` (${m[1]})` : '';
}

/**
 * The uploader for `env`'s bucket. Throws (names only) when a setting is
 * missing or the endpoint isn't https. Every request has a time limit
 * (AbortSignal.timeout): PUT_TIMEOUT_MS for an upload, REQUEST_TIMEOUT_MS else.
 */
export const PUT_TIMEOUT_MS = 30 * 60_000;
export const REQUEST_TIMEOUT_MS = 2 * 60_000;

export function createR2Uploader({ env = process.env, fetch: fetchImpl = globalThis.fetch, now = () => new Date(), putTimeoutMs = PUT_TIMEOUT_MS, timeoutMs = REQUEST_TIMEOUT_MS } = {}) {
  const missing = missingR2Config(env);
  if (missing.length) throw new Error(`R2 upload needs ${missing.join(', ')}`);
  let endpoint;
  try { endpoint = new URL(env.HOLLOW_R2_ENDPOINT.trim()); } catch { throw new Error('HOLLOW_R2_ENDPOINT is not a URL'); }
  if (endpoint.protocol !== 'https:' || endpoint.pathname !== '/' || endpoint.search || endpoint.username) {
    throw new Error('HOLLOW_R2_ENDPOINT must be https://<account id>.r2.cloudflarestorage.com with no path');
  }
  const bucket = (env.HOLLOW_R2_BUCKET ?? '').trim() || 'hollow-updates';
  if (!/^[a-z0-9][a-z0-9-]{1,61}[a-z0-9]$/.test(bucket)) throw new Error('HOLLOW_R2_BUCKET is not a bucket name');
  const accessKeyId = env.HOLLOW_R2_ACCESS_KEY_ID.trim();
  const secretAccessKey = env.HOLLOW_R2_SECRET_ACCESS_KEY.trim();
  const host = endpoint.host;

  async function send(method, key, { query = {}, body, headers = {} } = {}) {
    const path = key === null ? `/${bucket}` : `/${bucket}/${key}`;
    const payloadHash = body === undefined ? EMPTY_SHA256 : sha256Hex(body);
    const signed = signRequest({ method, host, path, query, headers, payloadHash, accessKeyId, secretAccessKey, date: now() });
    delete signed.host; // fetch sets it from the URL
    const qs = Object.keys(query).sort().map(k => `${uriEncode(k)}=${uriEncode(query[k])}`).join('&');
    const url = `${endpoint.origin}${uriEncode(path, true)}${qs ? `?${qs}` : ''}`;
    let res;
    try {
      res = await fetchImpl(url, { method, headers: signed, body, signal: AbortSignal.timeout(method === 'PUT' ? putTimeoutMs : timeoutMs) });
    } catch (e) {
      throw new Error(`R2 ${method} ${key ?? '(list)'} failed: ${e?.name ?? 'network error'}`);
    }
    return res;
  }
  const fail = async (method, key, res) => {
    const body = await res.text().catch(() => '');
    const error = new Error(`R2 ${method} ${key} answered ${res.status}${errorCode(body)}`);
    error.status = res.status;
    return error;
  };

  return {
    bucket,
    async put(key, bytes, { contentType = 'application/octet-stream', ifMatch, ifNoneMatch } = {}) {
      const body = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes);
      const headers = { 'content-type': contentType };
      if (ifMatch !== undefined) headers['if-match'] = ifMatch;
      if (ifNoneMatch !== undefined) headers['if-none-match'] = ifNoneMatch;
      const res = await send('PUT', key, { body, headers });
      if (!res.ok) throw await fail('PUT', key, res);
    },
    async get(key) {
      const res = await send('GET', key);
      if (res.status === 404) return null;
      if (!res.ok) throw await fail('GET', key, res);
      return await res.text();
    },
    async getWithEtag(key) {
      const res = await send('GET', key);
      if (res.status === 404) return null;
      if (!res.ok) throw await fail('GET', key, res);
      return { text: await res.text(), etag: res.headers.get('etag') };
    },
    async delete(key) {
      const res = await send('DELETE', key);
      if (!res.ok && res.status !== 404) throw await fail('DELETE', key, res);
    },
    async list(prefix) {
      const keys = [];
      let token;
      for (let page = 0; page < 1000; page++) {
        const query = { 'list-type': '2', prefix };
        if (token) query['continuation-token'] = token;
        const res = await send('GET', null, { query });
        if (!res.ok) throw await fail('LIST', prefix, res);
        const xml = await res.text();
        for (const m of xml.matchAll(/<Key>([^<]*)<\/Key>/g)) keys.push(xmlText(m[1]));
        const truncated = /<IsTruncated>true<\/IsTruncated>/.test(xml);
        token = /<NextContinuationToken>([^<]*)<\/NextContinuationToken>/.exec(xml)?.[1];
        if (!truncated) return keys;
        if (!token) throw new Error(`R2 LIST ${prefix} said truncated with no continuation token`);
        token = xmlText(token);
      }
      throw new Error(`R2 LIST ${prefix} has more than 1000 pages`);
    },
  };
}
