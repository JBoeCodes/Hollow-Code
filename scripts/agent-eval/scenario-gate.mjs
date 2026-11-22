// ND3 + ND4 + BG4: the eval gate by scenario fingerprint, with a negative
// control on every scenario. (Ideas from the not-built list; method only.)
//
//   node scripts/agent-eval/scenario-gate.mjs status [--json <file>]
//   node scripts/agent-eval/scenario-gate.mjs admit <kind/id> --attempts <file.json>
//
// A scenario is one eval case: a pilot task (`pilot-tasks.json`), a skill-lift
// case (`fixtures/skill-lift/<skill>.json`) or a skill-routing case
// (`fixtures/skill-routing/cases.json`). Two rules keep a green gate honest:
//
// 1. Negative control. Every scenario must FAIL when the agent does nothing.
//    Rule-graded scenarios (pilot, lift) are graded against an empty answer
//    and a no-op answer ("Done."): at least one positive rule (not `absent`,
//    which an empty answer passes) must fail on each. A routing case that
//    expects a skill must score `missed` when no skill is chosen; one that
//    expects none must score `falseFire` when a skill is chosen. A scenario that cannot fail proves nothing.
// 2. Fingerprint admission. A scenario joins the gate only after 5 of 5
//    passes at its CURRENT content: `fixtures/scenario-gate.json` records the
//    sha256 of the scenario's content beside its 5 of 5. Edit the scenario
//    and its fingerprint no longer matches (stale): the gate is red until it
//    is re-admitted or its entry is removed, so a green gate never means an
//    edited test. A scenario with no entry is pending (reported, not gating);
//    an entry for a scenario that is gone is an orphan (red).
//
// The BG4 form of rule 1 for ops checks: `controlledHealthCheck` requires a
// check to say yes to something alive AND no to something dead before its
// "it answers" verdict counts.
//
// `admit` records attempts someone already ran (a JSON array of 5
// `{pass: boolean}` or booleans, the paid first runs are the person's); it
// never makes a model call and refuses anything but 5 of 5 or a scenario
// whose negative control fails. Never paid, never in the gate beyond the
// free checks in `scripts/agent-eval-scenario-gate.test.mjs`.
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { checkRule, loadSuite as loadPilot } from './score.mjs';
import { gradeAnswer, loadSuite as loadLift } from './skill-lift.mjs';
import { loadCases as loadRouting, scoreAgent } from './skill-routing.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
export const LEDGER_FILE = join(HERE, 'fixtures', 'scenario-gate.json');
export const LEDGER_SCHEMA = 1;
/** Passes in a row needed, of the same number of attempts. */
export const REQUIRED_PASSES = 5;
/** What "the agent does nothing" looks like to a rule-graded scenario. */
export const NO_OP_ANSWERS = ['', 'Done.'];

// ── Fingerprints ─────────────────────────────────────────────────────────

/** JSON with object keys sorted at every depth, so key order never matters. */
export function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value && typeof value === 'object') {
    return `{${Object.keys(value).sort().map((k) => `${JSON.stringify(k)}:${canonical(value[k])}`).join(',')}}`;
  }
  return JSON.stringify(value) ?? 'null';
}

export const fingerprintOf = (content) => createHash('sha256').update(canonical(content)).digest('hex');

// ── Scenarios ────────────────────────────────────────────────────────────

/** Every scenario in the repo: `{key, kind, id, content}`; `content` is all
 *  that defines it (its fingerprint covers exactly this). */
export function loadScenarios({ root = HERE } = {}) {
  const out = [];
  for (const t of loadPilot(join(root, 'pilot-tasks.json')).tasks) {
    out.push({ key: `pilot/${t.id}`, kind: 'pilot', id: t.id, content: t });
  }
  const liftDir = join(root, 'fixtures', 'skill-lift');
  for (const name of readdirSync(liftDir).filter((n) => n.endsWith('.json') && !n.endsWith('-answers.json')).sort()) {
    const suite = loadLift(join(liftDir, name));
    for (const c of suite.cases) out.push({ key: `lift/${suite.skill}/${c.id}`, kind: 'lift', id: c.id, content: { skill: suite.skill, case: c } });
  }
  for (const c of loadRouting(join(root, 'fixtures', 'skill-routing', 'cases.json'))) {
    out.push({ key: `routing/${c.id}`, kind: 'routing', id: c.id, content: c });
  }
  return out;
}

// ── Negative control ─────────────────────────────────────────────────────

const POSITIVE = (rule) => rule.kind !== 'absent';

/** The positive rules a do-nothing answer fails (an empty list means the
 *  scenario would pass, or has nothing that can fail). */
function failedPositive(rules, text) {
  return rules.filter((r) => POSITIVE(r) && !checkRule(r, text).pass).map((r) => r.id);
}

/** `{ok, detail}`: does this scenario fail when the agent does nothing? */
export function negativeControl(scenario) {
  const { kind, content } = scenario;
  if (kind === 'pilot' || kind === 'lift') {
    const rules = kind === 'pilot' ? content.score.rules : content.case.rules;
    if (!rules.some(POSITIVE)) return { ok: false, detail: 'no positive rule (every rule is `absent`, which an empty answer passes)' };
    for (const text of NO_OP_ANSWERS) {
      if (!failedPositive(rules, text).length) return { ok: false, detail: `a positive rule must fail on ${JSON.stringify(text)}, none did` };
    }
    if (kind === 'lift') {
      for (const text of NO_OP_ANSWERS) {
        if (gradeAnswer(content.case, text).pass) return { ok: false, detail: `gradeAnswer passes ${JSON.stringify(text)}` };
      }
    }
    return { ok: true, detail: 'fails on an empty and a no-op answer' };
  }
  if (kind === 'routing') {
    const c = content;
    const wantNone = c.expect == null;
    const chose = wantNone ? (c.confusable[0] ?? 'any-skill') : null;
    const row = scoreAgent('control', [c], new Map([[c.id, { skill: chose }]])).rows[0];
    const want = wantNone ? 'falseFire' : 'missed';
    return row.outcome === want
      ? { ok: true, detail: wantNone ? 'firing a near miss scores falseFire' : 'choosing no skill scores missed' }
      : { ok: false, detail: `expected ${want} for the do-nothing choice, got ${row.outcome}` };
  }
  return { ok: false, detail: `unknown scenario kind ${kind}` };
}

// ── Ledger and verdict ───────────────────────────────────────────────────

export function loadLedger(file = LEDGER_FILE) {
  if (!existsSync(file)) return { schema: LEDGER_SCHEMA, required: REQUIRED_PASSES, scenarios: {} };
  const l = JSON.parse(readFileSync(file, 'utf8'));
  if (l?.schema !== LEDGER_SCHEMA || !l.scenarios || typeof l.scenarios !== 'object') throw new Error(`${file}: not a schema ${LEDGER_SCHEMA} scenario ledger`);
  return l;
}

/** Each scenario's state and the gate's verdict.
 *  admitted: fingerprint matches and 5 of 5; pending: no entry;
 *  stale: content changed since admission; weak: entry short of 5 of 5;
 *  orphan: entry whose scenario is gone. Red on control, stale, weak, orphan. */
export function gateReport(scenarios, ledger) {
  const rows = scenarios.map((s) => {
    const control = negativeControl(s);
    const fingerprint = fingerprintOf(s.content);
    const e = ledger.scenarios[s.key];
    let state = 'pending';
    if (e) {
      if (e.fingerprint !== fingerprint) state = 'stale';
      else if (e.passes !== REQUIRED_PASSES || e.of !== REQUIRED_PASSES) state = 'weak';
      else state = 'admitted';
    }
    return { key: s.key, kind: s.kind, fingerprint, state, control };
  });
  const known = new Set(scenarios.map((s) => s.key));
  const orphans = Object.keys(ledger.scenarios).filter((k) => !known.has(k));
  const problems = [
    ...rows.filter((r) => !r.control.ok).map((r) => `${r.key}: negative control failed: ${r.control.detail}`),
    ...rows.filter((r) => r.state === 'stale').map((r) => `${r.key}: content changed since admission (re-admit at 5 of 5 or remove its ledger entry)`),
    ...rows.filter((r) => r.state === 'weak').map((r) => `${r.key}: ledger entry is not ${REQUIRED_PASSES} of ${REQUIRED_PASSES}`),
    ...orphans.map((k) => `${k}: ledger entry for a scenario that no longer exists`),
  ];
  const count = (s) => rows.filter((r) => r.state === s).length;
  return { ok: problems.length === 0, problems, rows, orphans, admitted: count('admitted'), pending: count('pending'), total: rows.length };
}

/** A ledger with `scenario` admitted, from five recorded attempts. Throws
 *  unless there are exactly 5, all passes, and the negative control holds. */
export function admit(ledger, scenario, attempts, { now = new Date() } = {}) {
  if (!Array.isArray(attempts) || attempts.length !== REQUIRED_PASSES) throw new Error(`need exactly ${REQUIRED_PASSES} recorded attempts, got ${Array.isArray(attempts) ? attempts.length : 'none'}`);
  const passes = attempts.filter((a) => (typeof a === 'boolean' ? a : a?.pass === true)).length;
  if (passes !== REQUIRED_PASSES) throw new Error(`${scenario.key}: ${passes} of ${REQUIRED_PASSES} passed; admission needs every one`);
  const control = negativeControl(scenario);
  if (!control.ok) throw new Error(`${scenario.key}: negative control failed: ${control.detail}`);
  return {
    ...ledger,
    scenarios: { ...ledger.scenarios, [scenario.key]: { fingerprint: fingerprintOf(scenario.content), passes, of: REQUIRED_PASSES, admitted: now.toISOString().slice(0, 10) } },
  };
}

// ── BG4: a health check proves nothing until it can also say no ──────────

/** Run `check(target)` on something alive and something dead. `ok` only when
 *  it says yes to the first and no to the second (a throw counts as no). A
 *  check that answers yes to everything is vacuous. */
export async function controlledHealthCheck(check, { live, dead }) {
  const ask = async (t) => { try { return (await check(t)) === true; } catch { return false; } };
  const [yes, no] = [await ask(live), !(await ask(dead))];
  const detail = !yes ? 'the check failed on a live target' : !no ? 'the check passed a dead target (vacuous)' : 'says yes to live and no to dead';
  return { ok: yes && no, detail };
}

// ── CLI ──────────────────────────────────────────────────────────────────

export function renderGate(r) {
  const lines = [`Eval gate: ${r.ok ? 'green' : 'RED'}; ${r.admitted} admitted, ${r.pending} pending, ${r.total} scenarios`];
  for (const p of r.problems) lines.push(`  - ${p}`);
  for (const row of r.rows.filter((x) => x.state === 'pending')) lines.push(`  pending ${row.key} (needs 5 of 5 recorded runs)`);
  return lines.join('\n');
}

export function main(argv, { log = console.log } = {}) {
  const [cmd, ...rest] = argv;
  const flag = (n) => { const i = rest.indexOf(n); return i >= 0 ? rest[i + 1] : null; };
  const scenarios = loadScenarios();
  if (cmd === 'status') {
    const r = gateReport(scenarios, loadLedger());
    log(renderGate(r));
    if (flag('--json')) writeFileSync(flag('--json'), `${JSON.stringify(r, null, 2)}\n`);
    return r.ok ? 0 : 1;
  }
  if (cmd === 'admit') {
    const s = scenarios.find((x) => x.key === rest[0]);
    if (!s) { log(`unknown scenario ${rest[0]}`); return 2; }
    if (!flag('--attempts')) { log('admit needs --attempts <file.json>'); return 2; }
    try {
      const next = admit(loadLedger(), s, JSON.parse(readFileSync(flag('--attempts'), 'utf8')));
      writeFileSync(LEDGER_FILE, `${JSON.stringify(next, null, 2)}\n`);
      log(`admitted ${s.key}`);
      return 0;
    } catch (e) { log(e.message); return 1; }
  }
  log('usage: scenario-gate.mjs status [--json <file>] | admit <kind/id> --attempts <file.json>');
  return 2;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) process.exitCode = main(process.argv.slice(2));
