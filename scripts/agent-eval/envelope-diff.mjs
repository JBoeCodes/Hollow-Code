// The golden envelope's per-layer deltas (overnight 2026-10-17, OB15): what a
// change to Hollow's prompt text costs each real-shaped turn, layer by layer,
// as a Markdown table for the PR body.
//
//   node scripts/agent-eval/envelope-diff.mjs [<base.json> <head.json>]
//        [--ref <git ref>] [--fixture <name>] [--all] [--json <file>]
//
// With no files it compares `git show <ref>:tests/fixtures/prompts/<name>`
// (default origin/main and envelope.json) with the working tree's copy. A
// fixture is `envelope.json` (chat_g2_budget_tests::envelope_fixture_is_current:
// `scenarios[]` of {scenario, agent, total, layers[{name, bytes}]}) or
// `turn-budget.json` (`rows[]`, whose `after` sizes are read as its layers).
// The table lists every scenario layer whose bytes changed (`--all`: every
// one), then each changed scenario's total. The ref must name a commit here
// (else exit 3) and its short SHA heads the output; a commit without the
// fixture (the PR that adds it) reads as empty: every layer is new.
// `--fixture turn-budget.json` lists systemPrompt, prompt, configured and
// workspace, which overlap (the last two are inside the first two). `--json` writes
// the counts first (number leaves, for the morning report) and the rows.
// Read-only; sizes only. A missing file exits 3, a bad argument 2.
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';

const REPO = fileURLToPath(new URL('../..', import.meta.url));
const MISSING = 3;

const USAGE = `usage: node scripts/agent-eval/envelope-diff.mjs [<base.json> <head.json>]
       [--ref <git ref>] [--fixture <name>] [--all] [--json <file>]
Without files: git show <ref>:tests/fixtures/prompts/<name> (origin/main, envelope.json)
against the working tree's copy.`;

class Missing extends Error {}

export function parseArgs(argv) {
  const o = { files: [], ref: 'origin/main', fixture: 'envelope.json', all: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      const v = argv[++i];
      if (v == null || v.startsWith('--')) throw new Error(`${a} needs a value`);
      return v;
    };
    if (a === '--ref') o.ref = val();
    else if (a === '--fixture') o.fixture = val();
    else if (a === '--json') o.json = val();
    else if (a === '--all') o.all = true;
    else if (a.startsWith('--')) throw new Error(`unknown argument ${a}`);
    else o.files.push(a);
  }
  if (o.files.length !== 0 && o.files.length !== 2) throw new Error('give two fixture files, or none');
  if (o.fixture.includes('/')) throw new Error('--fixture is a file name in tests/fixtures/prompts');
  if (o.ref.startsWith('-')) throw new Error('--ref cannot start with -');
  return o;
}

/** A fixture's scenarios, in its order: key → {total, layers: Map name → bytes}. */
export function scenariosOf(fixture) {
  const out = new Map();
  if (Array.isArray(fixture?.scenarios)) {
    for (const s of fixture.scenarios) {
      const layers = new Map((s.layers ?? []).map((l) => [l.name, Number(l.bytes) || 0]));
      out.set(`${s.scenario} · ${s.agent}`, { total: Number(s.total) || 0, layers });
    }
  } else if (Array.isArray(fixture?.rows)) {
    for (const r of fixture.rows) {
      const after = r.after ?? {};
      const layers = new Map(Object.entries(after).filter(([k]) => k !== 'total').map(([k, v]) => [k, Number(v) || 0]));
      out.set(`${r.surface}/${r.turn} · ${r.agent}`, { total: Number(after.total) || 0, layers });
    }
  }
  return out;
}

/** Per scenario and layer: base and head bytes (null where absent) and the
 *  delta; per scenario, its totals. Head's order, then what only base has. */
export function diffEnvelopes(base, head) {
  const a = scenariosOf(base);
  const b = scenariosOf(head);
  const keys = [...b.keys(), ...[...a.keys()].filter((k) => !b.has(k))];
  const rows = [];
  const totals = [];
  for (const scenario of keys) {
    const was = a.get(scenario);
    const now = b.get(scenario);
    const names = [...(now?.layers.keys() ?? []), ...[...(was?.layers.keys() ?? [])].filter((n) => !now?.layers.has(n))];
    for (const layer of names) {
      const before = was?.layers.get(layer) ?? null;
      const after = now?.layers.get(layer) ?? null;
      rows.push({ scenario, layer, base: before, head: after, delta: (after ?? 0) - (before ?? 0) });
    }
    const before = was ? was.total : null;
    const after = now ? now.total : null;
    totals.push({ scenario, base: before, head: after, delta: (after ?? 0) - (before ?? 0) });
  }
  return { rows, totals };
}

const changed = (r) => r.delta !== 0 || r.base === null || r.head === null;

/** Counts for `--json`: number leaves first (the morning report reads them). */
export function summarize(diff) {
  const perLayer = {};
  for (const r of diff.rows.filter(changed)) perLayer[r.layer] = (perLayer[r.layer] ?? 0) + r.delta;
  return {
    scenarios: diff.totals.length,
    scenariosChanged: diff.totals.filter(changed).length,
    layersChanged: diff.rows.filter(changed).length,
    bytesDelta: diff.totals.reduce((sum, t) => sum + t.delta, 0),
    perLayer,
    rows: diff.rows,
    totals: diff.totals,
  };
}

const num = (n) => (n === null ? '—' : n.toLocaleString('en-US'));
const signed = (n) => (n > 0 ? `+${n.toLocaleString('en-US')}` : n.toLocaleString('en-US'));

export function renderMarkdown(diff, { all = false } = {}) {
  const rows = all ? diff.rows : diff.rows.filter(changed);
  const totals = diff.totals.filter(changed);
  if (!rows.length && !totals.length) return `No layer changed in ${diff.totals.length} scenarios.`;
  const lines = ['| Scenario | Layer | Base B | Head B | Δ B |', '|---|---|---:|---:|---:|'];
  for (const r of rows) lines.push(`| ${r.scenario} | ${r.layer} | ${num(r.base)} | ${num(r.head)} | ${signed(r.delta)} |`);
  if (totals.length) {
    lines.push('', '| Scenario | Total base B | Total head B | Δ B |', '|---|---:|---:|---:|');
    for (const t of totals) lines.push(`| ${t.scenario} | ${num(t.base)} | ${num(t.head)} | ${signed(t.delta)} |`);
  }
  lines.push('', `${totals.length} of ${diff.totals.length} scenarios changed.`);
  return lines.join('\n');
}

function readFixture(file) {
  if (!existsSync(file)) throw new Missing(`no fixture at ${file}`);
  try {
    return JSON.parse(readFileSync(file, 'utf8'));
  } catch (e) {
    throw new Missing(`${file} is not JSON: ${e.message}`);
  }
}

/** `git -C <repo> <args>`'s stdout; throws when git fails. */
const runGit = (repo) => (args) =>
  execFileSync('git', ['-C', repo, ...args], { encoding: 'utf8', maxBuffer: 16 << 20, stdio: ['ignore', 'pipe', 'pipe'] });

/** The commit `ref` names, or Missing: a typo'd or unfetched ref is never
 *  read as "no fixture". */
function resolveRef(git, ref) {
  try {
    const sha = git(['rev-parse', '--verify', '--quiet', '--end-of-options', `${ref}^{commit}`]).trim();
    if (/^[0-9a-f]{40,64}$/.test(sha)) return sha;
  } catch {
    // Falls through to Missing.
  }
  throw new Missing(`${ref} is not a commit here (a typo, or not fetched)`);
}

/** `git show <sha>:<path>`, or null when that commit has no such file. */
function gitShow(git, sha, path) {
  try {
    return git(['show', `${sha}:${path}`]);
  } catch {
    return null;
  }
}

export async function main(argv, { repo = REPO, git = runGit(repo) } = {}) {
  let o;
  try {
    o = parseArgs(argv);
  } catch (e) {
    console.error(`${e.message}\n${USAGE}`);
    return 2;
  }
  try {
    let base;
    let head;
    const notes = [];
    if (o.files.length) {
      base = readFixture(o.files[0]);
      head = readFixture(o.files[1]);
    } else {
      const rel = `tests/fixtures/prompts/${o.fixture}`;
      head = readFixture(`${repo.replace(/\/$/, '')}/${rel}`);
      const sha = resolveRef(git, o.ref);
      notes.push(`Base: ${o.ref} at ${sha.slice(0, 10)}; head: the working tree's ${rel}.`);
      const text = gitShow(git, sha, rel);
      if (text === null) notes.push(`${o.ref} has no ${rel}: every layer is new.`);
      try {
        base = text === null ? {} : JSON.parse(text);
      } catch (e) {
        throw new Missing(`${o.ref}:${rel} is not JSON: ${e.message}`);
      }
    }
    const diff = diffEnvelopes(base, head);
    console.log([...notes, renderMarkdown(diff, { all: o.all })].join('\n\n'));
    if (o.json) writeFileSync(o.json, `${JSON.stringify(summarize(diff), null, 2)}\n`);
    return 0;
  } catch (e) {
    if (!(e instanceof Missing)) throw e;
    console.error(`envelope-diff: ${e.message}`);
    return MISSING;
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exitCode = await main(process.argv.slice(2));
}
