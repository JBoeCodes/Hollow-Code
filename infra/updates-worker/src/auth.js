// Per-person tokens (design section 11). A token is 32 random bytes shown to
// the person once, as base64url without padding (43 characters). The bucket
// keeps only `tokenHash`, the lowercase hex SHA-256 of the token's UTF-8 text,
// in people.json. Nothing here logs, and nothing returns or echoes a token.

import { CHANNELS, PEOPLE_KEY, isChannel } from './layout.js';

/** The only token shape accepted anywhere (header, install path, DMG query). */
export const TOKEN_RE = /^[A-Za-z0-9_-]{43}$/;
const HASH_RE = /^[0-9a-f]{64}$/;
const BEARER_RE = /^Bearer ([A-Za-z0-9_-]{43})$/;

/** How long people.json is trusted from memory. A revoke written to the
 *  bucket takes effect for every request that starts after this much time. */
export const DEFAULT_PEOPLE_TTL_MS = 60_000;

/**
 * @typedef {{ id: string, name: string, tokenHash: string, channels: string[], revoked: boolean, createdAt: string }} Person
 * @typedef {{ ok: true, people: Person[] } | { ok: false }} PeopleList
 */

/** @param {unknown} token @returns {token is string} */
export function isToken(token) {
  return typeof token === 'string' && TOKEN_RE.test(token);
}

/**
 * The token from an `Authorization: Bearer <token>` header, or null for a
 * missing, malformed or differently-schemed header. Exactly one space, no
 * trailing text.
 * @param {string | null} header
 */
export function bearerToken(header) {
  if (typeof header !== 'string') return null;
  const m = BEARER_RE.exec(header);
  return m ? m[1] : null;
}

/** Lowercase hex SHA-256 of a token's UTF-8 text.
 *  @param {string} token */
export async function hashToken(token) {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(token));
  let hex = '';
  for (const byte of new Uint8Array(digest)) hex += byte.toString(16).padStart(2, '0');
  return hex;
}

/**
 * Compares two strings without an early exit, so the time taken does not
 * depend on where they first differ. Unequal lengths are unequal, but the
 * loop still runs over the longer one.
 * @param {string} a @param {string} b
 */
export function timingSafeEqual(a, b) {
  const n = Math.max(a.length, b.length);
  let diff = a.length ^ b.length;
  for (let i = 0; i < n; i++) diff |= (a.charCodeAt(i) | 0) ^ (b.charCodeAt(i) | 0);
  return diff === 0;
}

/**
 * The person a token hash belongs to, if they may use updates. Every entry is
 * compared (no early exit), and a revoked person is dropped only after the
 * loop, so an unknown token and a revoked one take the same path.
 * @param {Person[]} people
 * @param {string} tokenHash
 * @param {(a: string, b: string) => boolean} [equal]
 * @returns {Person | null}
 */
export function findPerson(people, tokenHash, equal = timingSafeEqual) {
  /** @type {Person | null} */
  let found = null;
  for (const person of people) {
    if (equal(person.tokenHash, tokenHash) && found === null) found = person;
  }
  if (found === null || found.revoked !== false) return null;
  return found;
}

/**
 * Parses people.json. Strict and all or nothing: any malformed entry, a
 * duplicate id or token hash, or an unknown channel makes the whole list
 * invalid, and an invalid list lets nobody in (fail closed). `revoked` must
 * be the boolean false for a person to be active; `channels` absent means
 * every channel.
 * @param {string} text
 * @returns {PeopleList}
 */
export function parsePeople(text) {
  let data;
  try {
    data = JSON.parse(text);
  } catch {
    return { ok: false };
  }
  if (!data || typeof data !== 'object' || Array.isArray(data) || !Array.isArray(data.people)) return { ok: false };
  /** @type {Person[]} */
  const people = [];
  const ids = new Set();
  const hashes = new Set();
  for (const entry of data.people) {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) return { ok: false };
    const { id, name, tokenHash, channels, revoked, createdAt } = entry;
    if (typeof id !== 'string' || id.length === 0 || ids.has(id)) return { ok: false };
    if (typeof name !== 'string') return { ok: false };
    if (typeof tokenHash !== 'string' || !HASH_RE.test(tokenHash) || hashes.has(tokenHash)) return { ok: false };
    if (typeof revoked !== 'boolean') return { ok: false };
    if (createdAt !== undefined && typeof createdAt !== 'string') return { ok: false };
    let allowed = [...CHANNELS];
    if (channels !== undefined) {
      if (!Array.isArray(channels) || !channels.every(isChannel) || new Set(channels).size !== channels.length) return { ok: false };
      allowed = [...channels];
    }
    ids.add(id);
    hashes.add(tokenHash);
    people.push({ id, name, tokenHash, channels: allowed, revoked, createdAt: createdAt ?? '' });
  }
  return { ok: true, people };
}

/**
 * people.json from the bucket, kept in memory for `ttlMs`. A missing file is
 * an empty list (nobody gets in); a malformed one is cached as invalid for
 * the same time (nobody gets in). A storage error is not cached and rethrown,
 * so the caller answers 503 rather than telling a valid person they were
 * revoked. Concurrent misses each read the bucket: a Worker must not await a
 * promise another request's I/O settles, so no read is shared.
 *
 * An invalid file logs one fixed line per cache fill (no contents).
 *
 * @param {{ ttlMs?: number, now?: () => number, log?: (line: string) => void }} [options]
 */
export function createPeopleCache({ ttlMs = DEFAULT_PEOPLE_TTL_MS, now = Date.now, log = () => {} } = {}) {
  if (!(ttlMs >= 0)) throw new Error('bad ttl');
  /** @type {{ at: number, list: PeopleList } | null} */
  let cached = null;

  /** @param {{ get(key: string): Promise<any> }} bucket @returns {Promise<PeopleList>} */
  async function load(bucket) {
    const object = await bucket.get(PEOPLE_KEY);
    if (!object) return { ok: true, people: [] };
    const list = parsePeople(await object.text());
    if (!list.ok) log('updates-worker: people.json is invalid, nobody is let in');
    return list;
  }

  return {
    /** @param {{ get(key: string): Promise<any> }} bucket @returns {Promise<PeopleList>} */
    async get(bucket) {
      const t = now();
      // A clock that went backwards counts as expired.
      if (cached && t >= cached.at && t - cached.at < ttlMs) return cached.list;
      const list = await load(bucket);
      // Stamped with the time the read started, so a revoke written during
      // the read is still picked up within ttlMs of being written.
      cached = { at: t, list };
      return list;
    },
    /** Drops the cached copy (tests). */
    clear() {
      cached = null;
    },
  };
}
