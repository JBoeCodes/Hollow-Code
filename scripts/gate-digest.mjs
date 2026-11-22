// Digest of gate, build and test output (GD1, 2026-11-16).
//
// A lane that reads a whole gate log pays for thousands of passing lines to
// learn one thing: did it pass, and if not, what broke. This reads the same
// output line by line (live from `gate-run.mjs --digest`, or a saved log from
// the command line) and keeps what answers that: the verdict, the failing
// step, counts per tool, the names of every failing test, the first real
// error with a few lines of context, and the path of the full log. Under 60
// lines for a failing gate and a passing one, plain text on stdout, so any
// agent can read it. Secrets in what it quotes are masked.
//
//   node scripts/gate-run.mjs '<chain>' --digest     print only the digest
//   node scripts/gate-digest.mjs <log> [--exit N] [--host name]
//                                                     digest a saved log ('-' is stdin)
//
// The gate's full output is unchanged: it is still written to the run's log
// (`<git common dir>/hollow-gate-runs/logs/`, newest 20 kept) and printed
// whole without `--digest`. Ideas only from sdl-mcp's and OpenViking's digests
// and externalised output; no code taken.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Digest lines at most (the failing-gate and passing-gate budget). */
export const MAX_LINES = 60;
/** Failing names listed before "and N more" (the other lines stay near 25, so 60 holds). */
export const MAX_NAMES = 30;
/** Lines kept after the first error. */
const CONTEXT_AFTER = 8;
/** Longest quoted line. */
const MAX_LINE = 200;

const ANSI = /\x1b\[[0-9;?]*[ -/]*[@-~]/g;

/** What counts as a real error line. Passing tests print `Error:` inside
 *  brackets (`[chat limits] Error: ...`), so a bare "Error" is not enough. */
const STRONG = [
  /^\s*error(\[E\d+\])?: /,
  /\berror TS\d+:/,
  /^\s*FAIL\b/,
  /panicked at /,
  /^\s*✖ /,
  /^not ok \d+ /,
  /^\s*(?:Assertion|Type|Reference|Syntax|Range)?Error: /,
  /^Diff in /,
];

const SECRETS = [
  [/-----BEGIN [A-Z ]*PRIVATE KEY-----.*/g, '[masked private key]'],
  [/\b(?:sk|pk|rk)-[A-Za-z0-9_-]{16,}/g, '[masked key]'],
  [/\bgh[pousr]_[A-Za-z0-9]{20,}/g, '[masked token]'],
  [/\bgithub_pat_[A-Za-z0-9_]{20,}/g, '[masked token]'],
  [/\bxox[abprs]-[A-Za-z0-9-]{10,}/g, '[masked token]'],
  [/\bAKIA[0-9A-Z]{16}\b/g, '[masked key]'],
  [/\bAIza[0-9A-Za-z_-]{30,}/g, '[masked key]'],
  [/\bBearer\s+[A-Za-z0-9._~+/=-]{20,}/gi, 'Bearer [masked]'],
  [/\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}/g, '[masked jwt]'],
  [/((?:api[_-]?key|token|secret|password|passwd|credential)s?["']?\s*[=:]\s*["']?)[^\s"',;]{8,}/gi, '$1[masked]'],
];

/** `line` with secret-shaped values replaced. */
export function mask(line) {
  let out = line;
  for (const [pattern, replacement] of SECRETS) out = out.replace(pattern, replacement);
  return out;
}

const clip = line => (line.length > MAX_LINE ? `${line.slice(0, MAX_LINE - 1)}…` : line);
const tidy = line => clip(mask(line.replace(ANSI, '').replace(/\r/g, '').replace(/\s+$/, '')));

/** Reads output line by line; `lines()` renders the digest. */
export class Digest {
  /** `gate` is the run's `GateLog` (steps, failing names), fed the same lines by the caller. */
  constructor(gate) {
    this.gate = gate;
    this.vitest = { files: null, tests: null };
    this.nodeTest = { tests: null, pass: null, fail: null };
    this.cargo = { passed: 0, failed: 0, ignored: 0, results: 0 };
    this.tscErrors = 0;
    this.rustErrors = 0;
    this.warnings = 0;
    this.fmtFiles = new Set();
    this.names = new Set();
    this.prev = '';
    this.first = null;
    this.after = 0;
    this.tail = [];
  }

  line(raw) {
    const line = raw.replace(ANSI, '').replace(/\r/g, '');
    let m;
    if ((m = line.match(/^\s*Test Files\s+(.+)$/))) this.vitest.files = m[1].trim();
    else if ((m = line.match(/^\s+Tests\s+(\d.*)$/))) this.vitest.tests = m[1].trim();
    else if ((m = line.match(/^ℹ tests (\d+)/))) this.nodeTest.tests = Number(m[1]);
    else if ((m = line.match(/^ℹ pass (\d+)/))) this.nodeTest.pass = Number(m[1]);
    else if ((m = line.match(/^ℹ fail (\d+)/))) this.nodeTest.fail = Number(m[1]);
    else if ((m = line.match(/^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored/))) {
      this.cargo.passed += Number(m[1]);
      this.cargo.failed += Number(m[2]);
      this.cargo.ignored += Number(m[3]);
      this.cargo.results += 1;
    } else if ((m = line.match(/^Diff in (\S+?):\d+/))) this.fmtFiles.add(m[1].replace(/^.*\/src-tauri\//, 'src-tauri/'));
    if (/\berror TS\d+:/.test(line)) this.tscErrors += 1;
    else if (/^\s*error(\[E\d+\])?: /.test(line) && !/^error: aborting|^error: could not compile|^error: test failed/.test(line)) this.rustErrors += 1;
    if (/^warning: /.test(line)) this.warnings += 1;
    if ((m = line.match(/^\s*✖ (.+?)(?: \([\d.]+ms\))?$/)) && !/^(failing tests|\d+)/.test(m[1])) this.names.add(m[1]);
    else if ((m = line.match(/^not ok \d+ - (.+)$/))) this.names.add(m[1]);

    // The first real error, with a little before it and more after.
    if (this.first) {
      if (this.after > 0) {
        this.first.push(line);
        this.after -= 1;
      }
    } else if (STRONG.some(re => re.test(line))) {
      // Only a libtest `---- name stdout ----` header belongs before the line
      // that names the panic; anything else before it is passing output.
      this.first = /^---- \S+ stdout ----$/.test(this.prev) ? [this.prev, line] : [line];
      this.after = CONTEXT_AFTER;
    }
    this.prev = line;
    if (line.trim()) {
      this.tail.push(line);
      if (this.tail.length > 6) this.tail.shift();
    }
  }

  /**
   * The digest: `{ exitCode, seconds, logPath, host }` in, at most
   * [`MAX_LINES`] lines out (a verdict, the failed step, the steps' times,
   * counts, every failing name, the first error and the log's path).
   */
  lines({ exitCode, seconds, logPath = null, host = null }) {
    const ok = exitCode === 0;
    const out = [];
    const summary = this.gate.summary(Date.now(), exitCode);
    const secs = seconds == null ? '' : ` in ${Math.round(seconds)} s`;
    out.push(`gate digest: ${ok ? 'PASSED' : `FAILED (exit ${exitCode})`}${secs}${host ? ` on ${host}` : ''}`);
    if (!ok && summary.failedStep) out.push(`failed step: ${summary.failedStep}`);
    const timed = summary.steps.filter(s => s.name !== 'start' && s.seconds >= 1);
    if (timed.length) out.push(clip(`steps: ${timed.map(s => `${s.name} ${Math.round(s.seconds)}s`).join(', ')}`));

    const counts = [];
    if (this.vitest.tests) counts.push(`vitest ${this.vitest.tests}${this.vitest.files ? ` (files: ${this.vitest.files})` : ''}`);
    if (this.nodeTest.tests != null) counts.push(`node:test ${this.nodeTest.pass ?? '?'} passed, ${this.nodeTest.fail ?? 0} failed of ${this.nodeTest.tests}`);
    if (this.cargo.results) counts.push(`cargo ${this.cargo.passed} passed, ${this.cargo.failed} failed, ${this.cargo.ignored} ignored`);
    if (this.tscErrors) counts.push(`tsc ${this.tscErrors} error${this.tscErrors === 1 ? '' : 's'}`);
    if (this.rustErrors) counts.push(`rustc/clippy ${this.rustErrors} error${this.rustErrors === 1 ? '' : 's'}`);
    if (this.warnings) counts.push(`${this.warnings} warning${this.warnings === 1 ? '' : 's'}`);
    if (this.fmtFiles.size) counts.push(`cargo fmt differs in ${this.fmtFiles.size} file${this.fmtFiles.size === 1 ? '' : 's'}: ${[...this.fmtFiles].slice(0, 5).join(', ')}${this.fmtFiles.size > 5 ? ', …' : ''}`);
    for (const c of counts) out.push(clip(`counts: ${c}`));

    const failing = [...new Set([...summary.failingTests, ...this.names])];
    if (failing.length) {
      out.push(`failing tests (${failing.length}):`);
      for (const n of failing.slice(0, MAX_NAMES)) out.push(clip(`  ${mask(n)}`));
      if (failing.length > MAX_NAMES) out.push(`  … and ${failing.length - MAX_NAMES} more (see the log)`);
    }
    if (!ok) {
      const context = this.first ?? this.tail;
      if (context.length) {
        out.push(this.first ? 'first error:' : 'last output:');
        for (const l of context) out.push(`  ${tidy(l)}`);
      }
    }
    if (summary.slowTests.length) out.push(clip(`slow tests (over 60 s): ${summary.slowTests.slice(0, 5).join(', ')}`));
    out.push(logPath ? `full log: ${host ? `${host}:` : ''}${logPath}` : 'full log: not kept');
    return out.length > MAX_LINES ? [...out.slice(0, MAX_LINES - 2), '  …', out[out.length - 1]] : out;
  }
}

/** Digest saved output `text`: `{ exitCode, seconds, logPath, host }` as for `Digest.lines`.
 *  (`gate-run.mjs` imports this file, so the `GateLog` it needs is loaded here, not at the top.) */
export async function digestText(text, info) {
  const { GateLog } = await import('./gate-run.mjs');
  const gate = new GateLog(Date.now());
  const digest = new Digest(gate);
  for (const line of text.split('\n')) {
    gate.line(line, Date.now());
    digest.line(line);
  }
  return digest.lines(info);
}

async function main(argv) {
  const args = argv.filter(a => !a.startsWith('--'));
  const flag = name => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : null;
  };
  const file = args[0];
  if (!file) {
    console.error('usage: node scripts/gate-digest.mjs <log | -> [--exit N] [--host name]');
    return 2;
  }
  const text = readFileSync(file === '-' ? 0 : file, 'utf8');
  // No exit code given: a log that shows a failure reads as failed.
  const given = flag('--exit');
  const failed = /^(?:\s*FAIL\b|test result: FAILED|\s*error(?:\[E\d+\])?: |Diff in |\s*✖ )/m.test(text);
  const exitCode = given != null ? Number(given) : failed ? 1 : 0;
  console.log((await digestText(text, { exitCode, seconds: null, logPath: file === '-' ? null : resolve(file), host: flag('--host') })).join('\n'));
  return exitCode === 0 ? 0 : 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) process.exit(await main(process.argv.slice(2)));
