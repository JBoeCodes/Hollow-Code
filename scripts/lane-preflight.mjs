// Lane preflight (OR2, 2026-11-17): which files another lane already claimed.
//
// Lanes claim whole rows, and two lanes that touch the same file only find out
// at merge ("main moved: merge, regen, gate again"). So a claim records the
// repo-relative paths it expects to touch, and this runs at claim time against
// the other live claims:
//
//   BLOCK  the same file is in another live claim          (exit 1)
//   WARN   a neighbour is claimed: same folder, a claimed folder that holds the
//          file, or a module file and its folder           (exit 0, advisory)
//   STALE  a claim older than 72 hours: listed, and it no longer blocks or
//          warns (a lane that stopped should not hold files forever)
//
// No model, no network, no wall clock in the verdict: the time is the `--now`
// argument (ISO; the command line falls back to the system clock only when it
// is omitted). Ideas only from mainline's claim check; no code taken.
//
// Claims live in one directory, one file each (any layout of lane worktrees or
// a shared board folder can point at it):
//
//   <claims dir>/<name>.json   {"id","lane","claimed_at","status","paths":[...]}
//   <claims dir>/<name>.md     front matter between two `---` lines:
//                              id: lane-d-1
//                              lane: D
//                              claimed_at: 2026-11-17T08:00:00Z
//                              status: live          (optional; live is the default)
//                              paths:                (a list, or `paths: a, b`)
//                                - src-tauri/src/memory_guard.rs
//
// A claim whose status is released, done, merged or cancelled is ignored, and
// so is any claim with the same `id` or the same `lane` as the new one (a lane
// re-claiming its own files is not a conflict). A folder path ends in `/`.
// Unreadable claim files are listed as problems, never fatal.
//
//   node scripts/lane-preflight.mjs --claims <dir> --paths a,b [--paths x]...
//        [--paths-file <file, one path per line>] [--lane D] [--id lane-d-1]
//        [--now <iso>] [--stale-hours 72] [--json]
//   exit 0 clear or warnings only, 1 blocked, 2 bad arguments.
import { readFileSync, readdirSync } from 'node:fs';
import { basename, extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** A claim older than this many hours is stale. */
export const STALE_HOURS = 72;
const HOUR_MS = 3_600_000;
/** Claim statuses that no longer hold files. */
const ENDED = new Set(['released', 'done', 'merged', 'cancelled', 'canceled', 'abandoned']);

/** `path` repo-relative with `/` separators, no `./`, no doubled slashes; a trailing `/` (a folder) is kept. */
export function normalizePath(path) {
  const slash = String(path ?? '').trim().replace(/\\/g, '/').replace(/\/{2,}/g, '/').replace(/^(\.\/)+/, '');
  return slash;
}

const keyOf = (p) => p.replace(/\/+$/, '');
const isFolder = (p) => p.endsWith('/');
const dirOf = (p) => (keyOf(p).includes('/') ? keyOf(p).slice(0, keyOf(p).lastIndexOf('/')) : '.');

/** How my `mine` path relates to their `theirs`: 'same', a reason string for a neighbour, or null. */
export function relate(mine, theirs) {
  const a = normalizePath(mine);
  const b = normalizePath(theirs);
  if (!keyOf(a) || !keyOf(b)) return null;
  if (keyOf(a) === keyOf(b)) return 'same';
  if (isFolder(b) && keyOf(a).startsWith(`${keyOf(b)}/`)) return 'inside a claimed folder';
  if (isFolder(a) && keyOf(b).startsWith(`${keyOf(a)}/`)) return 'holds a claimed file';
  const stem = (p) => keyOf(p).slice(0, keyOf(p).length - extname(keyOf(p)).length);
  if ((extname(keyOf(b)) && !isFolder(b) && keyOf(a).startsWith(`${stem(b)}/`)) || (extname(keyOf(a)) && !isFolder(a) && keyOf(b).startsWith(`${stem(a)}/`))) return 'module and its folder';
  if (!isFolder(a) && !isFolder(b) && dirOf(a) === dirOf(b)) return 'same folder';
  return null;
}

const when = (v) => {
  const t = typeof v === 'string' ? Date.parse(v) : Number.NaN;
  return Number.isFinite(t) ? t : null;
};

/** One claim from a file's text; throws an Error naming the file when it can't be used. */
export function parseClaim(file, text) {
  const name = basename(file);
  const fail = (why) => {
    throw new Error(`${name}: ${why}`);
  };
  let raw;
  if (extname(file) === '.json') {
    try {
      raw = JSON.parse(text);
    } catch (e) {
      fail(`not valid JSON (${e.message})`);
    }
  } else raw = frontMatter(text, fail);
  if (!raw || typeof raw !== 'object') fail('no claim found');
  const paths = (Array.isArray(raw.paths) ? raw.paths : typeof raw.paths === 'string' ? splitList(raw.paths) : [])
    .filter((p) => typeof p === 'string')
    .map(normalizePath)
    .filter(Boolean);
  if (!paths.length) fail('needs a non-empty paths list');
  const claimedAt = raw.claimed_at ?? raw.claimedAt;
  if (when(claimedAt) == null) fail('needs a claimed_at timestamp (ISO)');
  const status = typeof raw.status === 'string' && raw.status.trim() ? raw.status.trim().toLowerCase() : 'live';
  return { id: String(raw.id ?? basename(file, extname(file))), lane: raw.lane == null ? null : String(raw.lane), claimedAt, status, paths };
}

const splitList = (s) => s.replace(/^\[|\]$/g, '').split(',').map((x) => x.trim().replace(/^["']|["']$/g, '')).filter(Boolean);

/** The `key: value` lines (and one `paths:` list) between the first two `---` lines. */
function frontMatter(text, fail) {
  const lines = String(text).split(/\r?\n/);
  if (lines[0]?.trim() !== '---') fail('needs front matter between two --- lines');
  const end = lines.indexOf('---', 1);
  if (end < 0) fail('front matter is not closed with ---');
  const out = {};
  let list = null;
  for (const line of lines.slice(1, end)) {
    const item = /^\s*-\s+(.*\S)\s*$/.exec(line);
    if (item && list) {
      list.push(item[1].replace(/^["']|["']$/g, ''));
      continue;
    }
    const kv = /^([A-Za-z_]+):\s*(.*?)\s*$/.exec(line);
    if (!kv) continue;
    list = null;
    if (kv[1] === 'paths' && !kv[2]) out.paths = list = [];
    else out[kv[1]] = kv[2].replace(/^["']|["']$/g, '');
  }
  return out;
}

/** Every claim file in `dir`: `{ claims, problems }`. */
export function readClaims(dir) {
  const claims = [];
  const problems = [];
  for (const entry of readdirSync(dir, { withFileTypes: true }).sort((x, y) => x.name.localeCompare(y.name))) {
    if (!entry.isFile() || !['.json', '.md'].includes(extname(entry.name))) continue;
    try {
      claims.push(parseClaim(entry.name, readFileSync(join(dir, entry.name), 'utf8')));
    } catch (e) {
      problems.push(e.message);
    }
  }
  return { claims, problems };
}

/** The verdict for a new claim `{ id, lane, paths }` against `claims`, at `now` (ISO or ms). Pure. */
export function preflight({ id = null, lane = null, paths, claims, now, staleHours = STALE_HOURS }) {
  const at = typeof now === 'number' ? now : when(now);
  if (at == null) throw new Error('preflight needs a valid now (ISO timestamp)');
  const mine = [...new Set(paths.map(normalizePath).filter(Boolean))];
  const blocks = [];
  const warnings = [];
  const stale = [];
  for (const c of claims) {
    if (ENDED.has(c.status)) continue;
    if ((id != null && c.id === id) || (lane != null && c.lane != null && c.lane === lane)) continue;
    const age = at - when(c.claimedAt);
    if (age > staleHours * HOUR_MS) {
      stale.push({ claim: c.id, lane: c.lane, hours: Math.floor(age / HOUR_MS) });
      continue;
    }
    for (const p of mine) {
      for (const theirs of c.paths) {
        const how = relate(p, theirs);
        if (how === 'same') blocks.push({ path: p, claim: c.id, lane: c.lane, theirs });
        else if (how) warnings.push({ path: p, claim: c.id, lane: c.lane, theirs, why: how });
      }
    }
  }
  const verdict = blocks.length ? 'block' : warnings.length ? 'warn' : 'clear';
  return { verdict, exitCode: blocks.length ? 1 : 0, blocks, warnings, stale };
}

/** Plain text lines for a result: one per finding, `clear` when there are none. */
export function render(result) {
  const who = (f) => `${f.claim}${f.lane ? ` (lane ${f.lane})` : ''}`;
  const out = [
    ...result.blocks.map((b) => `BLOCK ${b.path} is already claimed by ${who(b)}`),
    ...result.warnings.map((w) => `WARN ${w.path} is a neighbour of ${w.theirs} claimed by ${who(w)}: ${w.why}`),
    ...result.stale.map((s) => `STALE ${s.claim}${s.lane ? ` (lane ${s.lane})` : ''} claimed ${s.hours} h ago; ignored`),
  ];
  return out.length ? out : ['clear: no other live claim touches these paths'];
}

function parseArgs(argv) {
  const o = { paths: [], json: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const v = () => argv[++i];
    if (a === '--claims') o.claims = v();
    else if (a === '--paths') o.paths.push(...splitList(v() ?? ''));
    else if (a === '--paths-file') o.paths.push(...readFileSync(v(), 'utf8').split(/\r?\n/).map((l) => l.trim()).filter((l) => l && !l.startsWith('#')));
    else if (a === '--lane') o.lane = v();
    else if (a === '--id') o.id = v();
    else if (a === '--now') o.now = v();
    else if (a === '--stale-hours') o.staleHours = Number(v());
    else if (a === '--json') o.json = true;
    else throw new Error(`unknown argument ${a}`);
  }
  return o;
}

function main(argv) {
  let o;
  try {
    o = parseArgs(argv);
  } catch (e) {
    console.error(e.message);
    return 2;
  }
  if (!o.claims || !o.paths.length || (o.staleHours != null && !(o.staleHours > 0))) {
    console.error('usage: node scripts/lane-preflight.mjs --claims <dir> --paths a,b [--paths-file f] [--lane D] [--id lane-d-1] [--now <iso>] [--stale-hours 72] [--json]');
    return 2;
  }
  const now = o.now ?? new Date().toISOString();
  if (when(now) == null) {
    console.error(`--now is not a timestamp: ${now}`);
    return 2;
  }
  const { claims, problems } = readClaims(o.claims);
  const result = preflight({ id: o.id ?? null, lane: o.lane ?? null, paths: o.paths, claims, now, staleHours: o.staleHours ?? STALE_HOURS });
  if (o.json) console.log(JSON.stringify({ ...result, problems }, null, 2));
  else {
    for (const line of render(result)) console.log(line);
    for (const p of problems) console.log(`PROBLEM ${p}`);
  }
  return result.exitCode;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) process.exit(main(process.argv.slice(2)));
