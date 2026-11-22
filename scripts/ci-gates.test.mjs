// Pins the CI gates that used to be "required local/release gates" only:
// bundle budget after the build, the Playwright smoke on its own port, the
// exact Rust toolchain, the rustfmt and clippy checks, and the secret-free pre-release
// `tauri build` lane. Since 2026-10-03 (owner: fewer CI runs) it also pins when
// each workflow runs: pull requests only, path-filtered, never on the push to main.
// Plain text checks (no YAML parser in the dependency tree), scoped per job.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const read = path => readFileSync(join(root, path), 'utf8');

// The text of one job under `jobs:` (two-space-indented key to the next one).
function job(workflow, name) {
  const lines = workflow.split('\n');
  const start = lines.findIndex(l => l === `  ${name}:`);
  assert.ok(start >= 0, `job ${name} not found`);
  const rest = lines.slice(start + 1);
  const end = rest.findIndex(l => /^ {2}\S/.test(l) || /^\S/.test(l));
  return rest.slice(0, end < 0 ? undefined : end).join('\n');
}

const runs = text => [...text.matchAll(/^\s*- run: (.+)$/gm)].map(m => m[1]);

test('rust-toolchain.toml pins one exact release with rustfmt and clippy', () => {
  const toml = read('rust-toolchain.toml');
  assert.match(toml, /^channel = "\d+\.\d+\.\d+"$/m, 'channel must be an exact x.y.z, not "stable"');
  assert.match(toml, /^components = \[.*"rustfmt".*\]$/m);
  assert.match(toml, /^components = \[.*"clippy".*\]$/m);
});

test('the frontend job checks the bundle budget after the build', () => {
  const steps = runs(job(read('.github/workflows/ci.yml'), 'frontend'));
  const build = steps.indexOf('npm run build');
  const budget = steps.indexOf('npm run bundle:budget');
  assert.ok(build >= 0, 'frontend job builds');
  assert.ok(budget > build, 'bundle:budget runs after npm run build');
});

test('the smoke job runs Playwright on its own port with two workers', () => {
  const smoke = job(read('.github/workflows/smoke.yml'), 'smoke');
  const port = smoke.match(/^\s+PW_PORT: (\d+)$/m);
  assert.ok(port, 'PW_PORT is set');
  assert.notEqual(port[1], '1420', 'not the shared dev port, so reuseExistingServer is off');
  const steps = runs(smoke);
  assert.ok(steps.some(s => /^npx playwright install chromium\b/.test(s)), 'installs chromium');
  assert.ok(steps.includes('npm run test:smoke -- --workers=2'), 'runs the smoke suite with --workers=2');
});

test('both Rust lanes install the toolchain named in rust-toolchain.toml', () => {
  for (const [file, name] of [['rust.yml', 'backend'], ['prerelease-build.yml', 'tauri-build']]) {
    const steps = runs(job(read(`.github/workflows/${file}`), name));
    assert.ok(
      steps.some(s => s.startsWith('rustup toolchain install ') && s.includes('rust-toolchain.toml')),
      `${file} ${name} installs the pinned toolchain from the file`,
    );
  }
});

test('the backend job checks rustfmt in src-tauri before the Rust tests', () => {
  const backend = job(read('.github/workflows/rust.yml'), 'backend');
  const steps = runs(backend);
  const fmt = steps.indexOf('cargo fmt --check');
  const tests = steps.indexOf('cargo test --lib');
  assert.ok(fmt >= 0, 'backend job runs cargo fmt --check');
  assert.ok(tests > fmt, 'cargo fmt --check runs before cargo test --lib');
  assert.match(
    backend,
    /^\s*- run: cargo fmt --check\n\s+working-directory: src-tauri$/m,
    'cargo fmt --check runs in src-tauri',
  );
});

test('the backend job runs clippy in src-tauri with warnings denied, before the Rust tests', () => {
  const backend = job(read('.github/workflows/rust.yml'), 'backend');
  const steps = runs(backend);
  const clippy = steps.indexOf('cargo clippy --lib --tests -- -D warnings');
  const tests = steps.indexOf('cargo test --lib');
  assert.ok(clippy >= 0, 'backend job runs cargo clippy --lib --tests -- -D warnings');
  assert.ok(tests > clippy, 'clippy runs before cargo test --lib');
  assert.match(
    backend,
    /^\s*- run: cargo clippy --lib --tests -- -D warnings\n\s+working-directory: src-tauri$/m,
    'cargo clippy runs in src-tauri',
  );
});

test('the pre-release lane builds the app on demand and on release branches, with no secrets', () => {
  const wf = read('.github/workflows/prerelease-build.yml');
  assert.match(wf, /^ {2}workflow_dispatch:/m);
  assert.match(wf, /^ {4}branches: \['release\/\*\*'\]$/m);
  assert.doesNotMatch(wf, /^ {2}pull_request:/m, 'never runs on a pull request (fewer CI runs, 2026-10-03)');
  assert.doesNotMatch(wf, /\$\{\{\s*secrets\./, 'uses no secrets');
  assert.doesNotMatch(wf, /upload-artifact/, 'uploads nothing');
  const steps = runs(job(wf, 'tauri-build'));
  const build = steps.find(s => s.startsWith('npx tauri build '));
  assert.ok(build, 'runs tauri build');
  assert.match(build, /"signingIdentity":"-"/, 'ad-hoc signed, not the local dev identity');
  assert.match(wf, /embed-selftest/, 'runs the bundle embedding self-test');
  assert.match(wf, /"EMBED_SELFTEST_OK "\*"\/Contents\/MacOS\/\.\.\/Resources\/models\/"/, 'the self-test must load the model from the bundle, not the src-tauri fallback');
});

// The `on:` block of a workflow (from `on:` to the next top-level key).
function triggers(workflow) {
  const lines = workflow.split('\n');
  const start = lines.findIndex(l => l === 'on:');
  assert.ok(start >= 0, 'workflow has an on: block');
  const rest = lines.slice(start + 1);
  const end = rest.findIndex(l => /^\S/.test(l));
  return rest.slice(0, end < 0 ? undefined : end).join('\n');
}

test('CI runs only when it can matter: PRs only, path-filtered, and by hand', () => {
  for (const file of ['ci.yml', 'smoke.yml', 'rust.yml']) {
    const on = triggers(read(`.github/workflows/${file}`));
    assert.match(on, /^ {2}workflow_dispatch:/m, `${file} can be run by hand`);
    assert.match(on, /^ {2}pull_request:/m, `${file} runs on pull requests`);
    assert.doesNotMatch(on, /^ {2}push:/m, `${file} does not run again on the push to main`);
  }
  const ci = triggers(read('.github/workflows/ci.yml'));
  assert.match(ci, /paths:\n {6}- '\*\*'\n {6}- '!\*\*\/\*\.md'\n {6}- '!docs\/\*\*'/, 'ci.yml skips unchecked docs-only changes');
  for (const file of ['AGENTS.md', 'CLAUDE.md', 'docs/agents/**', 'docs/WHATS_NEW.md', 'docs/FRONTEND_MODULES.md', 'docs/HOLLOW_APP_COMMAND_MAP.tsv', 'rust-toolchain.toml']) {
    assert.ok(ci.includes(`      - '${file}'`), `ci.yml runs when checked file ${file} changes`);
  }
  const smoke = triggers(read('.github/workflows/smoke.yml'));
  assert.match(smoke, /^ {6}- 'src\/\*\*'$/m, 'smoke runs for frontend changes');
  assert.doesNotMatch(smoke, /src-tauri/, 'smoke does not run for Rust-only changes');
  const rust = triggers(read('.github/workflows/rust.yml'));
  assert.match(rust, /^ {6}- 'src-tauri\/\*\*'$/m, 'rust runs for Rust changes');
  assert.match(rust, /^ {6}- 'rust-toolchain\.toml'$/m, 'rust runs when the toolchain pin changes');
  assert.match(rust, /^ {6}- 'docs\/HOLLOW_APP_COMMAND_MAP\.tsv'$/m, 'rust runs when the command map changes');
});

test('the smoke runs on Linux; the Rust checks stay on macOS', () => {
  assert.match(job(read('.github/workflows/smoke.yml'), 'smoke'), /^ {4}runs-on: ubuntu-latest$/m);
  assert.match(job(read('.github/workflows/rust.yml'), 'backend'), /^ {4}runs-on: macos-14$/m);
});

// 2026-10-09: main `0d0dccfc` merged on a hand-run gate that skipped `npm run build`, so 9 CSS token
// violations reached main and the install build failed. The local merge gate is `npm run gate`; it must
// run every step the CI frontend and backend jobs run, so a green gate means a green CI.
test('npm run gate runs every frontend and backend CI step', () => {
  const scripts = JSON.parse(read('package.json')).scripts;
  const frontend = scripts['gate:frontend'];
  const rust = scripts['gate:rust'];
  assert.match(scripts.gate, /npm run gate:frontend/);
  assert.match(scripts.gate, /npm run gate:rust/);
  for (const step of runs(job(read('.github/workflows/ci.yml'), 'frontend'))) {
    if (step === 'npm ci') continue;
    const name = step === 'npm test' ? 'npm test' : step;
    assert.ok(frontend.includes(name), `gate:frontend is missing CI step: ${step}`);
  }
  assert.ok(frontend.indexOf('npm run bundle:budget') > frontend.indexOf('npm run build'), 'budget after build');
  for (const step of ['cargo fmt --check', 'cargo clippy --lib --tests -- -D warnings', 'cargo test --lib']) {
    assert.ok(rust.includes(step), `gate:rust is missing ${step}`);
  }
});

// Overnight S2 (2026-10-10): what CI does besides its steps. Both CI jobs pin
// TZ (date-sensitive tests were written in the operator's zone), so both gate
// halves set the same zone; and CI installs fresh with npm ci, so the gate
// first refuses a node_modules that differs from package-lock.json.
test('npm run gate matches CI on the time zone and on what is installed', () => {
  const scripts = JSON.parse(read('package.json')).scripts;
  const zones = ['ci.yml', 'rust.yml'].map(f => read(`.github/workflows/${f}`).match(/^\s+TZ: (\S+)$/m)?.[1]);
  assert.ok(zones[0] && zones[0] === zones[1], 'both CI jobs pin one TZ');
  for (const half of ['gate:frontend', 'gate:rust']) {
    assert.ok(scripts[half].includes(`export TZ=${zones[0]} &&`), `${half} sets TZ=${zones[0]} like CI`);
  }
  assert.ok(scripts['gate:frontend'].startsWith('node scripts/check-installed-lock.mjs && '), 'gate:frontend checks node_modules against the lock first');
});

// Overnight 10-11/12 B1: with GitHub CI refused, a fast browser check stands
// beside the gate. It stays out of `npm run gate`: two lanes gate at once and
// only one dev server may run machine-wide, so the smoke takes the dev lock.
test('smoke:critical is a desktop-only, one-worker subset of existing specs on its own port, outside the gate', () => {
  const pkg = JSON.parse(read('package.json'));
  const cmd = pkg.scripts['smoke:critical'];
  assert.ok(cmd, 'npm run smoke:critical exists');
  // Its own port: playwright.config.ts reuses a server only on the shared 1420.
  assert.match(cmd, /^PW_PORT=\$\{PW_PORT:-(\d+)\} playwright test /);
  assert.notEqual(cmd.match(/PW_PORT:-(\d+)/)[1], '1420');
  const args = cmd.split(' ');
  assert.deepEqual(args.filter(a => a.startsWith('--project')), ['--project=chromium-desktop']);
  assert.deepEqual(args.filter(a => a.startsWith('--workers')), ['--workers=1']);
  const specs = args.filter(a => a.endsWith('.spec.ts'));
  assert.ok(specs.length >= 8 && specs.length <= 16, `a short list (${specs.length})`);
  for (const spec of specs) assert.doesNotThrow(() => read(spec), `${spec} exists`);
  // The gate, with every `npm run x` it reaches, starts no browser or dev server.
  const seen = new Set();
  const walk = key => {
    if (seen.has(key)) return;
    seen.add(key);
    const script = pkg.scripts[key] ?? '';
    assert.doesNotMatch(script, /playwright|test:smoke|smoke:critical|vite(?! build)|npm run dev/, `${key} starts no dev server`);
    for (const m of script.matchAll(/npm run ([\w:-]+)/g)) walk(m[1]);
  };
  walk('gate');
  assert.ok(seen.has('gate:frontend') && seen.has('gate:rust'));
});
