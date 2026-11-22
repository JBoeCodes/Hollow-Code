// Hollow updates Worker (design sections 4 and 11, row U3). Serves, from the
// R2 bucket bound as `UPDATES`:
//
//   GET|HEAD /channels/<main|nightly>.json         bearer token, channel allowed
//   GET|HEAD /builds/<0.6.n>/<file>                bearer token (DMG: or ?token=)
//   GET|HEAD /install/<token>                      the install page
//   GET|HEAD /me                                   bearer token: the person's name and channels (U9)
//
// Everything else is 404, `people.json` included. Request text never becomes
// an R2 key: routes match fixed patterns and keys are rebuilt from the
// matched parts (layout.js). A bad, unknown or revoked token gets one fixed
// 401 whichever check failed. Nothing logs a token, a hash or a URL.

import { CHANNELS, buildKey, buildPath, isBuildFile, isChannel, isVersion } from './layout.js';
import { bearerToken, createPeopleCache, findPerson, hashToken, isToken, timingSafeEqual, DEFAULT_PEOPLE_TTL_MS } from './auth.js';
import { completeManifest, servedManifest } from './manifest.js';
import { installPage } from './install-page.js';

/**
 * @typedef {{ size: number, etag?: string, httpEtag?: string, body?: ReadableStream | null, text(): Promise<string>, arrayBuffer(): Promise<ArrayBuffer> }} R2ObjectLike
 *   An R2 object. A `get` whose `onlyIf` fails resolves to one with no `body`.
 * @typedef {{
 *   get(key: string, options?: { range?: { offset: number, length: number }, onlyIf?: { etagMatches?: string } }): Promise<R2ObjectLike | null>,
 *   head(key: string): Promise<R2ObjectLike | null>,
 * }} BucketLike
 * @typedef {{ UPDATES?: BucketLike }} Env
 * @typedef {{ kind: 'channel', channel: string }
 *   | { kind: 'build', version: string, file: string }
 *   | { kind: 'install', token: string }
 *   | { kind: 'me' }} Route
 */

const CHANNEL_PATH = /^\/channels\/([a-z]+)\.json$/;
const BUILD_PATH = /^\/builds\/([0-9.]+)\/([A-Za-z.]+)$/;
const INSTALL_PATH = /^\/install\/([A-Za-z0-9_-]+)$/;

const CONTENT_TYPES = /** @type {Record<string, string>} */ ({
  'Hollow.app.tar.gz': 'application/gzip',
  'Hollow.app.tar.gz.sig': 'text/plain; charset=utf-8',
  'Hollow.dmg': 'application/x-apple-diskimage',
  'notes.json': 'application/json; charset=utf-8',
});

/** Bodies are fixed strings so no response can carry request text. */
export const BODIES = Object.freeze({
  unauthorized: 'Unauthorized\n',
  forbidden: 'Forbidden\n',
  notFound: 'Not found\n',
  method: 'Method not allowed\n',
  range: 'Range not satisfiable\n',
  unavailable: 'Temporarily unavailable\n',
});

/**
 * The route a path names, or null. The path must already be exactly one of
 * the shapes above: no percent escapes, backslashes, dot segments or empty
 * segments, and every part checked against its allowlist.
 * @param {string} pathname
 * @returns {Route | null}
 */
export function parseRoute(pathname) {
  if (typeof pathname !== 'string' || /[%\\]|\/\/|\/\.\.?(?:\/|$)/.test(pathname)) return null;
  if (pathname === '/me') return { kind: 'me' };
  let m = CHANNEL_PATH.exec(pathname);
  if (m) return isChannel(m[1]) ? { kind: 'channel', channel: m[1] } : null;
  m = BUILD_PATH.exec(pathname);
  if (m) return isVersion(m[1]) && isBuildFile(m[2]) ? { kind: 'build', version: m[1], file: m[2] } : null;
  m = INSTALL_PATH.exec(pathname);
  if (m) return isToken(m[1]) ? { kind: 'install', token: m[1] } : null;
  return null;
}

/**
 * A single `bytes=` range, or null when absent, multi-part or malformed (the
 * whole file is sent then, as RFC 9110 allows).
 * @param {string | null} header
 * @returns {{ start: number, end: number | null } | { suffix: number } | null}
 */
export function parseRange(header) {
  if (typeof header !== 'string') return null;
  const m = /^bytes=(\d{0,15})-(\d{0,15})$/.exec(header.trim());
  if (!m || (m[1] === '' && m[2] === '')) return null;
  if (m[1] === '') return { suffix: Number(m[2]) };
  const start = Number(m[1]);
  const end = m[2] === '' ? null : Number(m[2]);
  if (end !== null && end < start) return null;
  return { start, end };
}

/**
 * Resolves a parsed range against a size: the byte span to send, or
 * 'unsatisfiable'.
 * @param {{ start: number, end: number | null } | { suffix: number }} range
 * @param {number} size
 * @returns {{ offset: number, length: number } | 'unsatisfiable'}
 */
export function resolveRange(range, size) {
  if (size <= 0) return 'unsatisfiable';
  if ('suffix' in range) {
    if (range.suffix === 0) return 'unsatisfiable';
    const length = Math.min(range.suffix, size);
    return { offset: size - length, length };
  }
  if (range.start >= size) return 'unsatisfiable';
  const end = range.end === null ? size - 1 : Math.min(range.end, size - 1);
  return { offset: range.start, length: end - range.start + 1 };
}

/** @param {Record<string, string>} extra */
function baseHeaders(extra) {
  return new Headers({ 'X-Content-Type-Options': 'nosniff', 'Referrer-Policy': 'no-referrer', ...extra });
}

/**
 * @param {Request} request @param {number} status @param {string} body
 * @param {Record<string, string>} [extra]
 */
function textResponse(request, status, body, extra = {}) {
  const headers = baseHeaders({ 'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'no-store', ...extra });
  return new Response(request.method === 'HEAD' ? null : body, { status, headers });
}

/** The one 401 every token failure gets. @param {Request} request */
function unauthorized(request) {
  return textResponse(request, 401, BODIES.unauthorized, { 'WWW-Authenticate': 'Bearer realm="hollow-updates"' });
}

/**
 * @param {{ now?: () => number, ttlMs?: number, equal?: (a: string, b: string) => boolean, log?: (line: string) => void }} [options]
 *   `now` and `ttlMs` drive the people.json cache (default 60 s); `equal` is
 *   the hash comparison (tests pass a spy); `log` gets one fixed line per
 *   storage failure, never request text.
 */
export function createHandler({ now = Date.now, ttlMs = DEFAULT_PEOPLE_TTL_MS, equal = timingSafeEqual, log = line => console.error(line) } = {}) {
  const people = createPeopleCache({ ttlMs, now, log });

  /** @param {BucketLike} bucket @param {string} token */
  async function authenticate(bucket, token) {
    const tokenHash = await hashToken(token);
    const list = await people.get(bucket);
    if (!list.ok) return null;
    return findPerson(list.people, tokenHash, equal);
  }

  /**
   * The token for a request: the bearer header, or on the DMG route only a
   * single `?token=`. Both at once, or a query token anywhere else, is a
   * failure (null).
   * @param {Request} request @param {URL} url @param {Route} route
   */
  function requestToken(request, url, route) {
    const header = request.headers.get('Authorization');
    const queryTokens = url.searchParams.getAll('token');
    if (queryTokens.length === 0) return bearerToken(header);
    const dmg = route.kind === 'build' && route.file === 'Hollow.dmg';
    if (!dmg || header !== null || queryTokens.length !== 1 || !isToken(queryTokens[0])) return null;
    return queryTokens[0];
  }

  /** @param {Request} request @param {BucketLike} bucket @param {string} channel */
  async function serveChannel(request, bucket, channel) {
    const manifest = await completeManifest(bucket, channel);
    if (!manifest) return textResponse(request, 404, BODIES.notFound);
    const body = JSON.stringify(servedManifest(manifest, new URL(request.url).origin));
    const headers = baseHeaders({ 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store' });
    return new Response(request.method === 'HEAD' ? null : body, { status: 200, headers });
  }

  /**
   * @param {Request} request @param {BucketLike} bucket @param {string} version @param {string} file
   * @param {boolean} viaQuery the token came from `?token=`: nothing may store the response
   */
  async function serveBuild(request, bucket, version, file, viaQuery) {
    const key = buildKey(version, file);
    /** @type {Record<string, string>} */
    const extra = {
      'Content-Type': CONTENT_TYPES[file],
      'Cache-Control': viaQuery ? 'no-store' : 'private, max-age=3600',
      'Accept-Ranges': 'bytes',
    };
    if (file === 'Hollow.dmg' || file === 'Hollow.app.tar.gz') extra['Content-Disposition'] = `attachment; filename="${file}"`;
    const withObject = (/** @type {R2ObjectLike} */ object, /** @type {Record<string, string>} */ more = {}) => {
      const headers = baseHeaders({ ...extra, ...more });
      if (object.httpEtag) headers.set('ETag', object.httpEtag);
      return headers;
    };

    if (request.method === 'HEAD') {
      const object = await bucket.head(key);
      if (!object) return textResponse(request, 404, BODIES.notFound);
      return new Response(null, { status: 200, headers: withObject(object, { 'Content-Length': String(object.size) }) });
    }

    // If-Range can't be checked cheaply here, so a conditional range gets the
    // whole file, which RFC 9110 allows.
    const range = request.headers.has('If-Range') ? null : parseRange(request.headers.get('Range'));
    if (!range) {
      const object = await bucket.get(key);
      if (!object) return textResponse(request, 404, BODIES.notFound);
      return new Response(object.body ?? null, { status: 200, headers: withObject(object, { 'Content-Length': String(object.size) }) });
    }

    // The span is worked out from a head, then read with `onlyIf` that same
    // etag, so a file overwritten in between can't mix two versions' bytes
    // or sizes. Every header comes from the object actually read. One retry
    // covers a single overwrite; a second mismatch is a 503.
    for (let attempt = 0; attempt < 2; attempt++) {
      const head = await bucket.head(key);
      if (!head) return textResponse(request, 404, BODIES.notFound);
      const span = resolveRange(range, head.size);
      if (span === 'unsatisfiable') return textResponse(request, 416, BODIES.range, { 'Content-Range': `bytes */${head.size}` });
      if (typeof head.etag !== 'string' || head.etag === '') break;
      const object = await bucket.get(key, { range: span, onlyIf: { etagMatches: head.etag } });
      if (!object) return textResponse(request, 404, BODIES.notFound);
      if (!object.body || object.etag !== head.etag || object.size !== head.size) continue;
      const contentRange = `bytes ${span.offset}-${span.offset + span.length - 1}/${object.size}`;
      return new Response(object.body, {
        status: 206,
        headers: withObject(object, { 'Content-Length': String(span.length), 'Content-Range': contentRange }),
      });
    }
    log('updates-worker: build changed during a ranged read (or has no etag)');
    return textResponse(request, 503, BODIES.unavailable, { 'Retry-After': '1' });
  }

  /**
   * `/me` (row U9): who the token belongs to and which channels it may use,
   * so the app can limit its channel picker. Built from fixed fields only:
   * never the id, the hash, `revoked` or anyone else's entry. `channels` is
   * in CHANNELS order; `[]` means none.
   * @param {Request} request @param {{ name: string, channels: string[] }} person
   */
  function serveMe(request, person) {
    const body = JSON.stringify({ name: person.name, channels: CHANNELS.filter(c => person.channels.includes(c)) });
    const headers = baseHeaders({ 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store' });
    return new Response(request.method === 'HEAD' ? null : body, { status: 200, headers });
  }

  /** @param {Request} request @param {BucketLike} bucket @param {string} token */
  async function serveInstall(request, bucket, token) {
    const person = await authenticate(bucket, token);
    if (!person) return unauthorized(request);
    // No channels: the same 403 the other routes give, not a page with a link that would 403.
    const channel = CHANNELS.find(c => person.channels.includes(c));
    if (!channel) return textResponse(request, 403, BODIES.forbidden);
    /** @type {string | null} */
    let version = null;
    /** @type {string | null} */
    let dmgPath = null;
    const manifest = await completeManifest(bucket, channel);
    if (manifest) {
      const dmg = await bucket.head(buildKey(manifest.version, 'Hollow.dmg'));
      if (dmg && dmg.size > 0) {
        version = manifest.version;
        dmgPath = buildPath(manifest.version, 'Hollow.dmg');
      }
    }
    const headers = baseHeaders({
      'Content-Type': 'text/html; charset=utf-8',
      'Cache-Control': 'no-store',
      'Content-Security-Policy': "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
      'X-Frame-Options': 'DENY',
      'X-Robots-Tag': 'noindex, nofollow',
    });
    const body = installPage({ token, name: person.name, version, dmgPath });
    return new Response(request.method === 'HEAD' ? null : body, { status: 200, headers });
  }

  return {
    /** @param {Request} request @param {Env} env */
    async fetch(request, env) {
      if (request.method !== 'GET' && request.method !== 'HEAD') {
        return textResponse(request, 405, BODIES.method, { Allow: 'GET, HEAD' });
      }
      const url = new URL(request.url);
      const route = parseRoute(url.pathname);
      if (!route) return textResponse(request, 404, BODIES.notFound);
      const bucket = env?.UPDATES;
      if (!bucket) {
        log('updates-worker: no UPDATES bucket binding');
        return textResponse(request, 503, BODIES.unavailable);
      }
      try {
        if (route.kind === 'install') {
          if (url.searchParams.has('token') || request.headers.has('Authorization')) return unauthorized(request);
          return await serveInstall(request, bucket, route.token);
        }
        const token = requestToken(request, url, route);
        if (!token) return unauthorized(request);
        const person = await authenticate(bucket, token);
        if (!person) return unauthorized(request);
        if (route.kind === 'me') return serveMe(request, person);
        if (route.kind === 'channel') {
          if (!person.channels.includes(route.channel)) return textResponse(request, 403, BODIES.forbidden);
          return await serveChannel(request, bucket, route.channel);
        }
        if (person.channels.length === 0) return textResponse(request, 403, BODIES.forbidden);
        return await serveBuild(request, bucket, route.version, route.file, url.searchParams.has('token'));
      } catch {
        // The error itself is not logged: an R2 message can quote the key.
        log(`updates-worker: storage error on a ${route.kind} request`);
        return textResponse(request, 503, BODIES.unavailable);
      }
    },
  };
}

export default createHandler();
