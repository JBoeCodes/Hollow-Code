#!/usr/bin/env node
// Overnight 10-11/12 P1: `npm run perf`, one repeatable baseline before and
// after performance work. Not in `npm run gate`: it builds twice, starts one
// `vite preview` and (unless --skip-rust) runs the ignored Rust timing report,
// so it holds a toolchain slot for several minutes. Run it in the foreground,
// with no other dev or preview server up:
//   npm run -s perf                    # everything, JSON on stdout (-s: no npm banner)
//   npm run perf -- --skip-rust        # frontend only
//   npm run perf -- --skip-build       # reuse dist/ from a build:mock (bundle half skipped)
//   npm run perf -- --out perf.json    # also write the JSON to a file
// Halves (each runnable alone, see their headers):
//   bundle    scripts/perf-report.mjs         chunks, launch set (after `npm run build`)
//   browser   scripts/perf-mock-budgets.mjs   shell, warm nav, long thread, typing, search, tasks, heap
//             scripts/perf-launch-chat.mjs    first paint, interactive, launch IPC, sidebar, 60-turn canvas chat
//             scripts/perf-switch.mjs         cold start to Home; Home/Tasks/Library/Automations/long-chat switches (p50, p90, n)
//             scripts/perf-transcript.mjs     the 1,000-turn thread: DOM, heap, scroll frames, typing, against TRANSCRIPT_BUDGET
//   rust      perf_hot_paths_report           IPC hot paths over a synthetic vault (test profile): time and payload bytes
// The browser halves run on the mock's scale fixture (`?mock=1&demo=large`:
// 200 workspaces, 5,000 tasks, 10,000 notes); the Rust half on its own
// synthetic vault (215 workspaces, 150 conversations, receipts, tasks).
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, realpathSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';
import { cargoTargetProblem } from './check-cargo-target.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

export function parseArgs(argv) {
  const opts = { build: true, rust: true, browser: true, out: null, port: Number(process.env.PERF_PORT ?? 1459) };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--skip-build') opts.build = false;
    else if (a === '--skip-rust') opts.rust = false;
    else if (a === '--skip-browser') opts.browser = false;
    else if (a === '--out') opts.out = argv[++i] ?? null;
    else if (a === '--port') opts.port = Number(argv[++i]);
    else throw new Error(`perf: unknown argument ${a}`);
  }
  if (!Number.isInteger(opts.port) || opts.port < 1024 || opts.port === 1420) throw new Error('perf: --port must be a free port other than 1420');
  return opts;
}

/** `perf_hot_paths <name> <ms> ms wall <ms> ms cpu` lines from the Rust
 *  report (OF10; older reports end at the first `ms`), as a map of wall
 *  times, the figure earlier baselines hold. Names may contain spaces. */
export function rustTimings(text) {
  const out = {};
  for (const m of text.matchAll(/^perf_hot_paths (.+?)\s+([\d.]+) ms(?: wall\s+[\d.]+ ms cpu)?$/gm)) out[m[1]] = Number(m[2]);
  return out;
}

const log = (...a) => console.error('[perf]', ...a);
const run = (cmd, args, opts = {}) => {
  log(cmd, args.join(' '));
  const r = spawnSync(cmd, args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 64 * 1024 * 1024, ...opts });
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(' ')} exited ${r.status}`);
  return r.stdout;
};
const json = text => {
  const at = text.indexOf('{');
  if (at < 0) throw new Error(`perf: expected JSON, got: ${text.slice(0, 200)}`);
  return JSON.parse(text.slice(at));
};
/** Builds print to stderr, so stdout carries only the JSON. */
const toStderr = { stdio: ['ignore', 2, 'inherit'] };

/** Whether nothing listens on `port` (a stale server there would be measured
 *  instead of this build). */
export function portFree(port) {
  return new Promise(resolve => {
    const s = createServer();
    s.once('error', () => resolve(false));
    s.listen(port, '127.0.0.1', () => s.close(() => resolve(true)));
  });
}

async function waitFor(url, ms) {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    try { if ((await fetch(url)).ok) return; } catch { /* not up yet */ }
    await new Promise(r => setTimeout(r, 300));
  }
  throw new Error(`perf: ${url} never answered`);
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const report = { at: new Date().toISOString(), commit: run('git', ['rev-parse', '--short', 'HEAD']).trim() };
  if (opts.build) {
    run('npm', ['run', 'build'], toStderr);
    report.bundle = json(run('node', ['scripts/perf-report.mjs']));
  }
  if (opts.browser) {
    if (opts.build) run('npm', ['run', 'build:mock'], toStderr);
    else if (!existsSync(`${root}/dist/index.html`)) throw new Error('perf: --skip-build needs dist/ from `npm run build:mock`');
    if (!(await portFree(opts.port))) throw new Error(`perf: port ${opts.port} is in use; stop that server or pass --port`);
    const base = `http://127.0.0.1:${opts.port}`;
    // Vite itself, not through npx, so stopping it stops the server; its
    // output goes to stderr, so a failed start is visible.
    const preview = spawn(process.execPath, ['node_modules/vite/bin/vite.js', 'preview', '--host', '127.0.0.1', '--port', String(opts.port), '--strictPort'], { cwd: root, stdio: ['ignore', 2, 'inherit'] });
    const exited = new Promise(resolve => preview.once('exit', resolve));
    try {
      await Promise.race([
        waitFor(`${base}/?mock=1`, 30_000),
        exited.then(code => { throw new Error(`perf: vite preview exited (${code}) before it answered`); }),
      ]);
      report.browser = json(run('node', ['scripts/perf-mock-budgets.mjs'], { env: { ...process.env, BASE: base } }));
      report.launchAndChat = json(run('node', ['scripts/perf-launch-chat.mjs'], { env: { ...process.env, BASE: base } }));
      report.switches = json(run('node', ['scripts/perf-switch.mjs'], { env: { ...process.env, BASE: base } }));
      report.transcript = json(run('node', ['scripts/perf-transcript.mjs'], { env: { ...process.env, BASE: base } }));
    } finally {
      if (preview.exitCode === null) preview.kill('SIGTERM');
      await exited;
    }
  }
  if (opts.rust) {
    const problem = cargoTargetProblem(process.env.CARGO_TARGET_DIR);
    if (problem) throw new Error(`perf: ${problem}`);
    const r = spawnSync('cargo', ['test', '--lib', 'perf_hot_paths_report', '--', '--ignored', '--nocapture'], { cwd: `${root}/src-tauri`, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
    if (r.status !== 0) throw new Error(`perf: the Rust report failed:\n${r.stderr.slice(-2000)}`);
    report.rustIpcMs = rustTimings(`${r.stdout}\n${r.stderr}`);
  }
  const text = JSON.stringify(report, null, 2);
  if (opts.out) writeFileSync(opts.out, `${text}\n`);
  console.log(text);
}

const real = p => { try { return realpathSync(p); } catch { return p; } };
if (process.argv[1] && real(fileURLToPath(import.meta.url)) === real(process.argv[1])) {
  main().catch(e => { console.error(e instanceof Error ? e.message : e); process.exit(1); });
}
