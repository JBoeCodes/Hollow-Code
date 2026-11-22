// Record every `npm run gate`, failed ones too (overnight 10-17, OF11).
//
//   node scripts/gate-run.mjs '<the gate's && chain>'      (package.json `gate`)
//
// gate-record.mjs keeps only passing gates, because it is the chain's last
// step. Failed runs, step times, flakes and gates per merged PR had to be
// rebuilt from transcripts and /tmp logs. This wrapper runs the chain in
// `/bin/sh -c` with the environment unchanged (so `npm_lifecycle_event` is
// still `gate` and gate-record still records) except TMPDIR: one the gate's
// own tests can't use (`tmpdirProblem`: too long, inside the checkout, a
// Hollow run's scratch folder) is replaced by a fresh `/tmp/hg.XXXXXX` for the
// chain, removed when the wrapper exits and named in the run record's
// `tmpdir` (overnight 9 A12). It tees stdout and stderr to the
// caller and to a log, and always writes one run record:
//
//   <git common dir>/hollow-gate-runs/<start, ':' and '.' as '-'>-<fp8>.json
//
// first when the run starts (no `end`: a run killed outright stays visible),
// then again when it ends. It is a sibling of `hollow-gate-records`, so
// install readiness (src-tauri/src/install_ready.rs) never reads it. The
// record holds what was parsed from the output (steps and their seconds, tool
// durations, failing and >60 s test names), never the output itself, plus the
// machine it ran on (`host`) and, for a run `hollow-slot --remote` started
// from another machine, that caller's HEAD (`remote: true`, runOrigin);
// hollow-slot brings such a record back to the caller's checkout. The log
// is kept in `hollow-gate-runs/logs/` for the newest KEEP_LOGS runs only.
// scripts/gate-stats.mjs reads the records.
//
// The chain runs in its own process group (stdin is /dev/null: no step reads
// it). SIGINT, SIGTERM and SIGHUP to the wrapper go to the whole group, so the
// running step (vitest, cargo) stops too, not only /bin/sh; when the wrapper
// exits it sends SIGTERM to the group again, for leftovers. The exit code is
// taken when /bin/sh exits; a leftover still holding the pipes (a Rust test's
// `/bin/sleep 60` child) gets CLOSE_WAIT_MS to let go before the streams are
// dropped and the record written.
//
// `--digest` after the chain (`npm run gate -- --digest`, or through
// hollow-slot: `hollow-slot --on <host> npm run gate -- --digest`, since a
// variable set on that command never reaches the runner) prints only a digest
// of the run instead of every line (GD1, 2026-11-16, scripts/gate-digest.mjs):
// the verdict, the failed step, counts, every failing test's name, the first
// error with context, and the full log's path. The log is written the same
// (in a temp file when the run couldn't be recorded). `--raw`, or no flag,
// prints everything as before.
//
// The gate's exit code is the chain's (128 + n when a signal ended it).
// Nothing here can fail the gate: an error of its own is reported and the
// chain's code still wins. A caller that stops reading (`npm run gate | head`)
// gets no more output from that stream, but the chain runs on, the log and the
// record are finished, and the exit code is still the chain's.
import { execFileSync, spawn } from 'node:child_process';
import { createWriteStream, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { constants, hostname, loadavg, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { StringDecoder } from 'node:string_decoder';
import { fileURLToPath } from 'node:url';
import { Digest } from './gate-digest.mjs';
import { RECORDS_DIR, startPath } from './gate-record.mjs';

export const RUNS_DIR = 'hollow-gate-runs';
/** Run records kept (small JSON; the oldest go first). */
export const KEEP_RUNS = 1000;
/** Logs kept (a gate log is a few hundred KB to MBs), shared by every lane: about ten runs each for two. */
export const KEEP_LOGS = 20;
/** How long, after /bin/sh exits, the wrapper waits for its pipes to close. */
export const CLOSE_WAIT_MS = 5000;
/** Another checkout's start snapshot counts as a gate running there when this recent. */
export const OTHER_START_MAX_AGE_MS = 60 * 60_000;
export const SCHEMA = 1;

/** A TMPDIR longer than this is replaced (overnight 9 A12): prompt-budget tests
 *  measure the temp path (chat_g2_budget_tests.rs), and lane V lost 13 gates to it. */
export const MAX_TMPDIR_CHARS = 16;

const RUN_NAME = /^\d{4}-\d\d-\d\dT[\d-]+Z-[0-9a-f]{8}\./;

/** The variables `hollow-slot --remote` sets for a run on another machine. */
export const REMOTE_VARS = ['HOLLOW_REMOTE_HEAD', 'HOLLOW_REMOTE_DIRTY'];

/** What a run record says about where and on what it ran (2026-11-01, the Mac
 *  mini runner): this machine's short host name, and for a run that
 *  `hollow-slot --remote` started from another machine, the caller's own HEAD
 *  (`HOLLOW_REMOTE_HEAD`: a snapshot of uncommitted changes is a commit only
 *  this machine has) and whether its tree had uncommitted changes
 *  (`HOLLOW_REMOTE_DIRTY`), so gate-stats there joins it to the PR it was for. */
export function runOrigin(snap, env = process.env, host = hostname()) {
  const head = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(env.HOLLOW_REMOTE_HEAD ?? '') ? env.HOLLOW_REMOTE_HEAD : null;
  return {
    commit: head ?? snap.commit,
    dirty: head ? env.HOLLOW_REMOTE_DIRTY === '1' : snap.dirty,
    host: String(host ?? '').split('.')[0] || null,
    remote: head !== null,
  };
}

/** Why the gate can't use `tmpdir` as its TMPDIR, or null when it can: longer
 *  than MAX_TMPDIR_CHARS, inside the checkout `root` (the envelope-diff test
 *  needs a temp folder outside any checkout), or a Hollow run's scratch path.
 *  Unset is fine: every tool then uses /tmp. */
export function tmpdirProblem(tmpdir, root) {
  if (!tmpdir) return null;
  const [dir, base] = [resolve(tmpdir), resolve(root)];
  if (dir === base || dir.startsWith(`${base}/`)) return 'inside the checkout';
  if (dir.includes('hollow-run')) return "a Hollow run's scratch folder";
  if (tmpdir.replace(/\/+$/, '').length > MAX_TMPDIR_CHARS) return `longer than ${MAX_TMPDIR_CHARS} characters`;
  return null;
}

/** `<start, ':' and '.' as '-'>-<fp8>.json`: names sort by start time. */
export function runFileName(start, fingerprint) {
  return `${start.toISOString().replace(/[:.]/g, '-')}-${(fingerprint ?? 'unknown0').slice(0, 8)}.json`;
}

/** Keep the newest `keep` run files ending in `ext` in `dir` (by name, which is by start). */
export function prune(dir, ext, keep) {
  const names = readdirSync(dir).filter(n => n.endsWith(ext) && RUN_NAME.test(n)).sort();
  for (const name of names.slice(0, Math.max(0, names.length - keep))) rmSync(join(dir, name), { force: true });
}

const ANSI = /\x1b\[[0-9;?]*[ -/]*[@-~]/g;
const round = n => Math.round(n * 100) / 100;
const seconds = (n, unit) => (unit === 'ms' ? Number(n) / 1000 : Number(n));

/**
 * The gate's output, read line by line as it streams. Steps start at npm's
 * `> pkg@version script` banners and, in the Rust half (which has no
 * banners), at cargo's own lines: fmt until cargo starts checking, clippy
 * until its `Finished \`dev\``, the test build until `Finished \`test\``, then
 * the tests, then gate-record's closing lines. A half's first step is its
 * pre-check (check-installed-lock, check-cargo-target), silent when it
 * passes; a line from it renames that step, so a refusal is its own step.
 */
export class GateLog {
  constructor(startMs) {
    this.startMs = startMs;
    this.steps = [{ name: 'start', startMs }];
    this.rust = false;
    this.durations = { vitest: null, nodeTest: null, viteBuild: null, cargo: {}, libtest: null, libtestRuns: 0 };
    this.failing = new Set();
    this.slow = new Set();
    this.gateRecord = null;
  }

  get current() {
    return this.steps[this.steps.length - 1].name;
  }

  step(name, at) {
    if (this.current !== name) this.steps.push({ name, startMs: at });
  }

  line(raw, at) {
    const line = raw.replace(ANSI, '').replace(/\r/g, '');
    let m;
    if ((m = line.match(/^> \S+@\S+ (\S+)$/))) {
      const script = m[1];
      if (script === 'gate:rust') {
        this.rust = true;
        this.step('cargo fmt', at);
      } else this.step(script, at);
      return;
    }
    if ((m = line.match(/^(check-installed-lock|check-cargo-target): /))) {
      const last = this.steps[this.steps.length - 1];
      if ((m[1] === 'check-cargo-target' && last.name === 'cargo fmt') || (m[1] === 'check-installed-lock' && last.name === 'gate:frontend')) last.name = m[1];
      return;
    }
    if (this.rust) {
      if (this.current === 'cargo fmt' && /^\s*(Checking|Compiling|Blocking|Finished|Fresh) /.test(line)) this.step('cargo clippy', at);
      if (this.current === 'cargo clippy' && /^\s*Finished `dev` profile/.test(line)) this.step('cargo test (build)', at);
      else if (/^\s*Finished `test` profile/.test(line)) this.step('cargo test', at);
      if (line.startsWith('[gate-record]')) this.step('gate-record', at);
    }
    if ((m = line.match(/^\s*Duration\s+([\d.]+)(ms|s)\b/))) this.durations.vitest = round(seconds(m[1], m[2]));
    else if ((m = line.match(/^ℹ duration_ms ([\d.]+)/))) this.durations.nodeTest = round(seconds(m[1], 'ms'));
    else if ((m = line.match(/built in ([\d.]+)(ms|s)\b/))) this.durations.viteBuild = round(seconds(m[1], m[2]));
    else if ((m = line.match(/^\s*Finished `(\w+)` profile .* in (?:(\d+)m )?([\d.]+)s$/))) {
      this.durations.cargo[m[1]] = round(Number(m[2] ?? 0) * 60 + Number(m[3]));
    } else if ((m = line.match(/^test result: .*finished in ([\d.]+)s$/))) {
      // Every libtest run counts: a split or targeted pre-run prints several.
      this.durations.libtest = round((this.durations.libtest ?? 0) + Number(m[1]));
      this.durations.libtestRuns += 1;
    } else if ((m = line.match(/^---- (\S+) stdout ----$/))) this.failing.add(m[1]);
    else if ((m = line.match(/^ FAIL {2}(.+\S)\s*$/))) this.failing.add(m[1]);
    else if ((m = line.match(/^test (\S+) has been running for over 60 seconds$/))) this.slow.add(m[1]);
    else if ((m = line.match(/^\[gate-record\] Gate: .*gate fingerprint `([0-9a-f]{64})`/))) this.gateRecord = { recorded: true, fingerprint: m[1] };
    else if ((m = line.match(/^\[gate-record\] not recorded: (.*?)\.?$/))) this.gateRecord = { recorded: false, skip: m[1] };
  }

  /** What the run record keeps, for a run that ended at `endMs` with `exitCode`. */
  summary(endMs, exitCode) {
    const ok = exitCode === 0;
    const steps = this.steps.map((s, i) => {
      const next = this.steps[i + 1]?.startMs ?? endMs;
      return { name: s.name, at: round((s.startMs - this.startMs) / 1000), seconds: round((next - s.startMs) / 1000), ok: ok || i < this.steps.length - 1 };
    });
    return {
      ok,
      failedStep: ok ? null : this.current,
      steps,
      durations: this.durations,
      failingTests: [...this.failing],
      slowTests: [...this.slow],
      gateRecord: this.gateRecord,
    };
  }
}

/** Feed `GateLog` whole lines from one stream's chunks (a chunk may end inside a UTF-8 character). */
export function lineSplitter(log) {
  const decoder = new StringDecoder('utf8');
  let rest = '';
  return {
    push(chunk) {
      const text = rest + decoder.write(chunk);
      const lines = text.split('\n');
      rest = lines.pop();
      const at = Date.now();
      for (const line of lines) log.line(line, at);
    },
    end() {
      rest += decoder.end();
      if (rest) log.line(rest, Date.now());
      rest = '';
    },
  };
}

function writeJson(path, value) {
  const tmp = `${path}.${process.pid}.tmp`;
  writeFileSync(tmp, `${JSON.stringify(value, null, 2)}\n`);
  renameSync(tmp, path);
}

const isMain = () => {
  try {
    return resolve(process.argv[1]) === fileURLToPath(import.meta.url);
  } catch {
    return false;
  }
};

function main() {
  const chain = process.argv[2];
  const digestMode = process.argv.slice(3).includes('--digest') && !process.argv.slice(3).includes('--raw');
  if (!chain) {
    console.error("usage: node scripts/gate-run.mjs '<command chain>' [--digest | --raw]");
    process.exit(2);
  }
  // A caller stream that errored (EPIPE: `| head` stopped reading) gets no more writes.
  const callerGone = { stdout: false, stderr: false };
  const say = msg => {
    if (!callerGone.stderr) console.error(`[gate-run] ${msg}`);
  };
  const root = resolve(fileURLToPath(new URL('..', import.meta.url)));
  const startedAt = new Date();
  // A12: a TMPDIR the gate's own tests can't use gets a short neutral one,
  // for the chain only, removed when the wrapper exits.
  let tmp = null;
  const problem = tmpdirProblem(process.env.TMPDIR, root);
  if (problem) {
    try {
      tmp = { dir: mkdtempSync('/tmp/hg.'), replaced: process.env.TMPDIR, why: problem };
      // Never throws: a leftover still writing there must not change the exit code.
      process.on('exit', () => {
        try {
          rmSync(tmp.dir, { recursive: true, force: true });
        } catch {
          // left for the OS to clear
        }
      });
      say(`TMPDIR ${process.env.TMPDIR} is ${problem}: this gate uses ${tmp.dir}`);
    } catch (e) {
      say(`TMPDIR ${process.env.TMPDIR} is ${problem}, and no short one could be made: ${e?.message ?? e}`);
    }
  }
  const loadStart = loadavg().map(round);
  let record = null;
  let recordPath = null;
  let logFile = null;
  try {
    const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
    // HOLLOW_GATE_COMMON_DIR is for this script's tests only.
    const common = process.env.HOLLOW_GATE_COMMON_DIR || resolve(root, git('rev-parse', '--git-common-dir'));
    const dir = join(common, RUNS_DIR);
    mkdirSync(join(dir, 'logs'), { recursive: true });
    // The fingerprint gate-record compares, taken the same way.
    const snap = JSON.parse(execFileSync(process.execPath, [join(root, 'scripts/source-provenance.mjs'), '--print'], { cwd: root, encoding: 'utf8' }));
    let branch = null;
    try {
      branch = git('symbolic-ref', '--short', '-q', 'HEAD') || null;
    } catch {
      // detached HEAD
    }
    // Another checkout's gate-record start snapshot: a gate running there now.
    // A failed gate never removes its snapshot, so only one from the last hour
    // whose checkout still exists counts.
    const startsDir = join(common, RECORDS_DIR, 'start');
    const recent = startedAt.getTime() - OTHER_START_MAX_AGE_MS;
    const own = startPath(join(common, RECORDS_DIR), root);
    let otherStarts = [];
    try {
      otherStarts = readdirSync(startsDir)
        .filter(n => n.endsWith('.json') && join(startsDir, n) !== own)
        .map(n => {
          try {
            const s = JSON.parse(readFileSync(join(startsDir, n), 'utf8'));
            return { checkout: s.checkout ?? null, at: s.at ?? null };
          } catch {
            return null;
          }
        })
        .filter(s => s && s.checkout && Date.parse(s.at) >= recent && existsSync(s.checkout));
    } catch {
      // no snapshots yet
    }
    const name = runFileName(startedAt, snap.sourceFingerprint);
    recordPath = join(dir, name);
    logFile = join(dir, 'logs', name.replace(/\.json$/, '.log'));
    const origin = runOrigin(snap);
    record = {
      schema: SCHEMA,
      commit: origin.commit,
      fingerprint: snap.sourceFingerprint,
      dirty: origin.dirty,
      host: origin.host,
      remote: origin.remote,
      checkout: root,
      branch,
      command: chain,
      start: startedAt.toISOString(),
      end: null,
      seconds: null,
      exitCode: null,
      signal: null,
      load: { start: loadStart, end: null },
      otherStartSnapshot: otherStarts.length > 0,
      otherStarts,
      tmpdir: tmp ? { used: tmp.dir, replaced: tmp.replaced, why: tmp.why } : null,
    };
    writeJson(recordPath, record);
    prune(dir, '.json', KEEP_RUNS);
  } catch (e) {
    say(`not recording this run: ${e?.message ?? e}`);
    record = null;
  }

  const log = new GateLog(startedAt.getTime());
  const digest = digestMode ? new Digest(log) : null;
  // A digest names the full log, so the output is kept even when the run
  // couldn't be recorded.
  if (digestMode && !logFile) logFile = join(tmpdir(), `hollow-gate-${process.pid}.log`);
  let out = null;
  if (record || digestMode) {
    try {
      out = createWriteStream(logFile);
      out.on('error', () => {
        out = null;
      });
    } catch {
      out = null;
    }
  }
  // HOLLOW_GATE_CLOSE_WAIT_MS is for this script's tests only.
  const closeWaitMs = Number(process.env.HOLLOW_GATE_CLOSE_WAIT_MS) || CLOSE_WAIT_MS;
  // Its own process group (detached), so a signal reaches the running step,
  // not only /bin/sh. stdin is /dev/null: a background group reading the
  // terminal would be stopped, and no step reads stdin.
  // The chain doesn't get hollow-slot's remote variables: they describe this
  // run, and a gate-run.mjs a test starts inside it would take them as its own.
  const env = { ...process.env, ...(tmp ? { TMPDIR: tmp.dir } : {}) };
  for (const name of REMOTE_VARS) delete env[name];
  const child = spawn('/bin/sh', ['-c', chain], { cwd: process.cwd(), env, stdio: ['ignore', 'pipe', 'pipe'], detached: true });
  const signalGroup = sig => {
    if (!child.pid) return;
    try {
      process.kill(-child.pid, sig);
    } catch {
      // the group is gone
    }
  };
  // However the wrapper ends, leftovers in the chain's group go with it.
  process.on('exit', () => signalGroup('SIGTERM'));
  const sink = digest ? { line: (raw, at) => { log.line(raw, at); digest.line(raw); } } : log;
  const lines = { stdout: lineSplitter(sink), stderr: lineSplitter(sink) };
  for (const stream of ['stdout', 'stderr']) {
    process[stream].on('error', () => {
      callerGone[stream] = true;
    });
    child[stream].on('data', chunk => {
      if (!digestMode && !callerGone[stream]) process[stream].write(chunk);
      out?.write(chunk);
      lines[stream].push(chunk);
    });
  }
  // A signal to the wrapper goes to the chain's whole group; the record is written when /bin/sh exits.
  for (const sig of ['SIGINT', 'SIGTERM', 'SIGHUP']) process.on(sig, () => signalGroup(sig));
  child.on('error', e => {
    say(`could not start the chain: ${e.message}`);
    process.exitCode ??= 1;
  });
  let exit = null;
  let finished = false;
  let closeTimer = null;
  // The exit code is /bin/sh's, taken at `exit`; `close` waits for every
  // holder of the pipes, so a leftover gets closeWaitMs, then is signalled and
  // its streams dropped.
  child.on('exit', (code, signal) => {
    exit = { code, signal };
    closeTimer = setTimeout(() => {
      signalGroup('SIGTERM');
      child.stdout.destroy();
      child.stderr.destroy();
      finish();
    }, closeWaitMs);
  });
  child.on('close', (code, signal) => {
    exit ??= { code, signal };
    finish();
  });
  function finish() {
    if (finished) return;
    finished = true;
    clearTimeout(closeTimer);
    const { code, signal } = exit;
    const exitCode = code ?? (signal ? 128 + (constants.signals[signal] ?? 0) : 1);
    lines.stdout.end();
    lines.stderr.end();
    process.exitCode = exitCode;
    if (digest) {
      const seconds = round((Date.now() - startedAt.getTime()) / 1000);
      const host = String(hostname() ?? '').split('.')[0] || null;
      for (const l of digest.lines({ exitCode, seconds, logPath: logFile, host })) console.log(l);
    }
    if (!record) {
      out?.end();
      return;
    }
    try {
      const endedAt = Date.now();
      const final = {
        ...record,
        end: new Date(endedAt).toISOString(),
        seconds: round((endedAt - startedAt.getTime()) / 1000),
        exitCode,
        signal: signal ?? null,
        load: { start: loadStart, end: loadavg().map(round) },
        ...log.summary(endedAt, exitCode),
      };
      writeJson(recordPath, final);
      const o = out;
      out = null;
      o?.end();
      prune(join(recordPath, '..', 'logs'), '.log', KEEP_LOGS);
      say(`run recorded ${recordPath}`);
    } catch (e) {
      say(`run not recorded: ${e?.message ?? e}`);
    }
  }
}

if (process.argv[1] && isMain()) main();
