#!/usr/bin/env node
// Teammates' access to updates (updates design section 11, row U9): edits
// `people.json` in the R2 bucket, which the updates Worker reads.
//
//   node scripts/release/people.mjs add [--init] <name> <channel...>   token hash on stdin
//   node scripts/release/people.mjs init                       an empty people.json
//   node scripts/release/people.mjs revoke <id>
//   node scripts/release/people.mjs set-channels <id> [<channel>...]   none: no channel
//   node scripts/release/people.mjs list
//
// Each prints one line of JSON on stdout: `add` the new person, `revoke` and
// `set-channels` the changed person, `list` `{people: [...]}`. A person is
// shown as {id, name, channels, revoked, createdAt}: never a token hash.
// Errors are one line on stderr (`people: ...`), exit 1; bad usage exit 2.
//
// Secrets. The token is made on the publisher's Mac (Hollow's People card)
// and never reaches this script: only its hash does, and only on stdin
// (never argv, a file or a log line; an argument shaped like a hash is
// refused). Nothing prints a hash. The R2 credentials come from the
// environment (`r2-uploader.mjs`), as for the release builder: on the Mac
// mini the app runs this through ssh after sourcing the builder's env file.
//
// Writes. Read people.json and its ETag, check it with the Worker's own
// parser, change the raw entry, check the whole result again (all or
// nothing, as the Worker reads it), then one conditional PUT: `If-Match` on
// the ETag read, so a change made by another writer in between is refused
// (412) instead of lost; `If-None-Match: *` when creating the file. A missing
// file is created only by `init` or `add --init` (a wrong bucket setting must
// not quietly start a new list). A revoke only sets `revoked: true`: entries
// are never deleted, so the file is the audit trail.
import { randomBytes } from 'node:crypto';
import { realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createR2Uploader } from './r2-uploader.mjs';
import { parsePeople } from '../../infra/updates-worker/src/auth.js';
import { CHANNELS, PEOPLE_KEY, isChannel } from '../../infra/updates-worker/src/layout.js';

export const HASH_RE = /^[0-9a-f]{64}$/;
export const ID_RE = /^p-[0-9a-f]{12}$/;
export const MAX_NAME = 80;
const MAX_STDIN = 1024;

/** A failure with words for a person (exit 1), or bad usage (exit 2). */
export class PeopleError extends Error {
  constructor(message, code = 1) {
    super(message);
    this.code = code;
  }
}

/** Control, format (bidi overrides, zero widths, the BOM) and separator
 *  characters: the same set as the app's `people::hidden_char`. */
export const HIDDEN_CHAR = /[\p{Cc}\p{Cf}\u2028\u2029]/u;

/** A display name: trimmed, 1 to MAX_NAME characters, no control, format or
 *  line-separator characters anywhere. Checked before trimming: JS's trim
 *  strips U+FEFF and Rust's doesn't, so the app and this script would
 *  otherwise disagree on a name holding one. */
export function checkName(raw) {
  if (typeof raw === 'string' && HIDDEN_CHAR.test(raw)) {
    throw new PeopleError('a name cannot hold control or invisible formatting characters', 2);
  }
  const name = typeof raw === 'string' ? raw.trim() : '';
  if (name.length === 0) throw new PeopleError('a name is needed', 2);
  if ([...name].length > MAX_NAME) throw new PeopleError(`a name is at most ${MAX_NAME} characters`, 2);
  return name;
}

/** Channels from argv: known, no repeats, in CHANNELS order. */
export function checkChannels(list) {
  for (const c of list) if (!isChannel(c)) throw new PeopleError(`unknown channel (use ${CHANNELS.join(' or ')})`, 2);
  if (new Set(list).size !== list.length) throw new PeopleError('a channel is named twice', 2);
  return CHANNELS.filter(c => list.includes(c));
}

/** The token hash from stdin's text: one line of 64 lowercase hex. */
export function checkHash(text) {
  const hash = typeof text === 'string' ? text.trim() : '';
  if (!HASH_RE.test(hash)) throw new PeopleError('stdin must hold the token hash: 64 lowercase hex characters');
  return hash;
}

/** A person as shown: never the hash. */
export function shown(entry) {
  return {
    id: entry.id,
    name: entry.name,
    channels: entry.channels === undefined ? [...CHANNELS] : CHANNELS.filter(c => entry.channels.includes(c)),
    revoked: entry.revoked,
    createdAt: entry.createdAt ?? '',
  };
}

export const MISSING = 'people.json is not in the bucket; check HOLLOW_R2_BUCKET, or start a new list with `init` (or `add --init`)';

/** The bucket's people.json as raw data, checked, with its ETag. A missing
 *  file is `{data: null}`. */
async function readPeople(bucket) {
  const found = await bucket.getWithEtag(PEOPLE_KEY);
  if (found === null) return { data: null, etag: null };
  if (!parsePeople(found.text).ok) {
    throw new PeopleError('people.json in the bucket is invalid, so nobody is let in; fix it by hand before changing it here');
  }
  if (typeof found.etag !== 'string' || found.etag === '') {
    throw new PeopleError('the bucket gave people.json no ETag, so a change could not be made safely; nothing was written');
  }
  return { data: JSON.parse(found.text), etag: found.etag };
}

/** Check the whole new file as the Worker will read it, then one PUT,
 *  conditional on the file being as read (`etag` null: on its not existing). */
async function writePeople(bucket, data, etag) {
  const text = `${JSON.stringify(data, null, 2)}\n`;
  if (!parsePeople(text).ok) throw new PeopleError('the change would make people.json invalid, so nothing was written');
  const condition = etag === null ? { ifNoneMatch: '*' } : { ifMatch: etag };
  try {
    await bucket.put(PEOPLE_KEY, Buffer.from(text, 'utf8'), { contentType: 'application/json; charset=utf-8', ...condition });
  } catch (e) {
    if (e?.status === 412) {
      throw new PeopleError('people.json changed while this ran, so nothing was written; list people and try again');
    }
    throw e;
  }
}

function findEntry(data, id) {
  // ID_RE is what `add` makes; an id written by hand (`p-sam`) is accepted too.
  if (typeof id !== 'string' || !/^[A-Za-z0-9._-]{1,64}$/.test(id)) {
    throw new PeopleError('that is not a person id', 2);
  }
  const entry = data.people.find(p => p.id === id);
  if (!entry) throw new PeopleError('no person has that id');
  return entry;
}

/**
 * One command. `deps`: `bucket` (put/get, as r2-uploader), `readStdin()`,
 * `now()`, `randomId()`. Returns the value printed as JSON.
 */
export async function run(argv, deps) {
  const [command, ...rest] = argv;
  // A hash belongs on stdin only; one in argv would sit in ps and shell history.
  if (rest.some(a => /^[0-9a-fA-F]{64}$/.test(String(a).trim()))) {
    throw new PeopleError('a token hash goes on stdin, never in the arguments', 2);
  }
  const { bucket } = deps;
  switch (command) {
    case 'list': {
      if (rest.length) throw new PeopleError('list takes no arguments', 2);
      const { data } = await readPeople(bucket);
      return { people: (data?.people ?? []).map(shown) };
    }
    case 'init': {
      if (rest.length) throw new PeopleError('init takes no arguments', 2);
      const { data } = await readPeople(bucket);
      if (data !== null) throw new PeopleError('people.json is already in the bucket; nothing was written');
      await writePeople(bucket, { people: [] }, null);
      return { people: [] };
    }
    case 'add': {
      const init = rest[0] === '--init';
      const [rawName, ...channelArgs] = init ? rest.slice(1) : rest;
      const name = checkName(rawName);
      const channels = checkChannels(channelArgs);
      if (channels.length === 0) throw new PeopleError('add needs at least one channel', 2);
      const tokenHash = checkHash(await deps.readStdin());
      const read = await readPeople(bucket);
      if (read.data === null && !init) throw new PeopleError(MISSING);
      const data = read.data ?? { people: [] };
      if (data.people.some(p => p.tokenHash === tokenHash)) throw new PeopleError('that token is already in people.json; make a new one');
      let id;
      for (let i = 0; i < 8 && (!id || data.people.some(p => p.id === id)); i++) id = deps.randomId();
      if (!ID_RE.test(id) || data.people.some(p => p.id === id)) throw new PeopleError('could not make a new person id');
      const entry = { id, name, tokenHash, channels, revoked: false, createdAt: deps.now().toISOString().replace(/\.\d{3}Z$/, 'Z') };
      data.people.push(entry);
      await writePeople(bucket, data, read.etag);
      return shown(entry);
    }
    case 'revoke': {
      if (rest.length !== 1) throw new PeopleError('revoke takes one person id', 2);
      const { data, etag } = await readPeople(bucket);
      if (data === null) throw new PeopleError(MISSING);
      const entry = findEntry(data, rest[0]);
      if (entry.revoked !== true) {
        entry.revoked = true;
        await writePeople(bucket, data, etag);
      }
      return shown(entry);
    }
    case 'set-channels': {
      const [id, ...channelArgs] = rest;
      const channels = checkChannels(channelArgs);
      const { data, etag } = await readPeople(bucket);
      if (data === null) throw new PeopleError(MISSING);
      const entry = findEntry(data, id);
      if (entry.revoked) throw new PeopleError('that person is revoked; add them again instead');
      entry.channels = channels;
      await writePeople(bucket, data, etag);
      return shown(entry);
    }
    default:
      throw new PeopleError('usage: people.mjs add [--init] <name> <channel...> (hash on stdin) | init | revoke <id> | set-channels <id> [channel...] | list', 2);
  }
}

/** stdin as text, at most MAX_STDIN bytes. */
export async function readLimited(stream, limit = MAX_STDIN) {
  let size = 0;
  const chunks = [];
  for await (const chunk of stream) {
    size += chunk.length;
    if (size > limit) throw new PeopleError('stdin is too long for a token hash');
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString('utf8');
}

/** The CLI: prints one JSON line, or one `people:` line on stderr. Errors
 *  carry no request text, credential or hash (r2-uploader's errors name the
 *  status and S3 code only). */
export async function main(argv = process.argv.slice(2), io = {}) {
  const out = io.stdout ?? process.stdout;
  const err = io.stderr ?? process.stderr;
  try {
    const value = await run(argv, {
      bucket: io.bucket ?? createR2Uploader({ env: io.env ?? process.env }),
      readStdin: io.readStdin ?? (() => readLimited(process.stdin)),
      now: io.now ?? (() => new Date()),
      randomId: io.randomId ?? (() => `p-${randomBytes(6).toString('hex')}`),
    });
    out.write(`${JSON.stringify(value)}\n`);
    return 0;
  } catch (e) {
    const code = e instanceof PeopleError ? e.code : 1;
    const words = e instanceof PeopleError || /^R2 /.test(e?.message ?? '') || /needs HOLLOW_R2|HOLLOW_R2_/.test(e?.message ?? '')
      ? e.message
      : 'failed';
    err.write(`people: ${words}\n`);
    return code;
  }
}

const invoked = (() => {
  try {
    return process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();
if (invoked) process.exitCode = await main();
