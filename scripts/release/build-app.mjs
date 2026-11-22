#!/usr/bin/env node
// Release build of Hollow.app: signed with an identity from the environment,
// notarized when App Store Connect API credentials are present, stamped
// `0.6.<n>`, then verified (updates design 2026-11-14, sections 2 and 3, row U1).
//
//   npm run release:build                          build HEAD, then verify
//   node scripts/release/build-app.mjs --ref <r>   <r> must be the checked-out commit
//   node scripts/release/build-app.mjs --dmg       also make the .dmg
//   node scripts/release/build-app.mjs --verify-only   verify the last build, build nothing
//   node scripts/release/build-app.mjs --no-verify     build only (the nightly builder runs
//                                                      the checks as separate named steps)
//   node scripts/release/build-app.mjs --no-selftest   verify without the embed self-test
//   node scripts/release/build-app.mjs --selftest-only  only the embed self-test of the last build
//   node scripts/release/build-app.mjs --allow-dirty   build a tree with changes (local branch
//                                                      builds; the nightly builder never passes it)
//
// The stamp counts HEAD's first-parent history. Release builds are made from
// main's tip, where every n is unique; a branch build can share an n with
// another lane's branch build.
//
// Environment (names only; values are never printed):
//   HOLLOW_SIGNING_IDENTITY, else APPLE_SIGNING_IDENTITY, else the local
//     self-signed "Hollow Dev Signing" (until the Developer ID cert exists).
//     "-" signs ad hoc (tests, CI runners without a key).
//   HOLLOW_TEAM_ID: the team the signature must carry. Default TEAM000000 for
//     any named identity other than the local self-signed one; an ad hoc
//     signature has no team.
//   APPLE_API_KEY, APPLE_API_ISSUER, APPLE_API_KEY_PATH: all three set means
//     Tauri's bundler notarizes and staples; none means no notarization; some
//     is refused. Apple ID notarization (APPLE_ID, APPLE_PASSWORD,
//     APPLE_TEAM_ID) is never used: those are removed from the build's env.
//
// The stamp is applied at build time only, so the tree stays clean: Tauri gets
// `--config {"version":"0.6.<n>"}` (Info.plist, package_info) and build.rs
// gets HOLLOW_BUILD_VERSION, which scripts/source-provenance.mjs writes into
// build-provenance.json (the running app's version, app_menu::build_identity).
// `npm run tauri:bundle` is unchanged and keeps package.json's 0.5.x version.
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, realpathSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { cargoTargetProblem } from '../check-cargo-target.mjs';
import { buildVersion } from '../version.mjs';

export const LOCAL_IDENTITY = 'Hollow Dev Signing';
export const DEFAULT_TEAM_ID = 'TEAM000000';
export const NOTARY_VARS = ['APPLE_API_KEY', 'APPLE_API_ISSUER', 'APPLE_API_KEY_PATH'];
const APPLE_ID_VARS = ['APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID'];
const TEAM_ID = /^[A-Z0-9]{10}$/;
const CERT_HASH = /^[0-9A-Fa-f]{40}$/;
const APP_NAME = 'Hollow';

const set = (env, key) => typeof env[key] === 'string' && env[key].trim() !== '';

/**
 * Which identity signs, which team the signature must carry (null: no team
 * check) and whether the bundler notarizes. Throws on a contradiction.
 */
export function resolveSigning(env = process.env) {
  const identity = (set(env, 'HOLLOW_SIGNING_IDENTITY') ? env.HOLLOW_SIGNING_IDENTITY
    : set(env, 'APPLE_SIGNING_IDENTITY') ? env.APPLE_SIGNING_IDENTITY : LOCAL_IDENTITY).trim();
  const adHoc = identity === '-';
  const explicitTeam = set(env, 'HOLLOW_TEAM_ID') ? env.HOLLOW_TEAM_ID.trim() : null;
  if (explicitTeam && !TEAM_ID.test(explicitTeam)) {
    throw new Error('HOLLOW_TEAM_ID must be a 10-character Apple team id (capital letters and digits)');
  }
  if (adHoc && explicitTeam) {
    throw new Error('an ad hoc signature ("-") has no team: unset HOLLOW_TEAM_ID or name a signing identity');
  }
  const teamId = adHoc ? null : explicitTeam ?? (identity === LOCAL_IDENTITY ? null : DEFAULT_TEAM_ID);
  const present = NOTARY_VARS.filter(k => set(env, k));
  if (present.length && present.length < NOTARY_VARS.length) {
    const missing = NOTARY_VARS.filter(k => !present.includes(k));
    throw new Error(`notarization needs ${NOTARY_VARS.join(', ')} together; ${missing.join(', ')} not set (set all three, or none to build without notarizing)`);
  }
  const notarize = present.length === NOTARY_VARS.length;
  if (notarize && (adHoc || teamId === null)) {
    throw new Error(`notarization needs a Developer ID identity with a team; "${identity}" is ${adHoc ? 'ad hoc' : 'the local self-signed identity'}. Unset the APPLE_API_* variables or set HOLLOW_SIGNING_IDENTITY`);
  }
  return { identity, adHoc, teamId, notarize };
}

/** Where `tauri build` leaves the app. */
export function appPath(root, env = process.env) {
  const target = set(env, 'CARGO_TARGET_DIR') ? env.CARGO_TARGET_DIR : join(root, 'src-tauri', 'target');
  return join(target, 'release', 'bundle', 'macos', `${APP_NAME}.app`);
}

/** The `tauri build` command and its environment. Pure. */
export function buildPlan({ root, env = process.env, version, signing, dmg = false }) {
  const config = { version, bundle: { macOS: { signingIdentity: signing.identity } } };
  const args = ['--no-install', 'tauri', 'build', '--ci', '--bundles', dmg ? 'app,dmg' : 'app', '--config', JSON.stringify(config)];
  const childEnv = { ...env, APPLE_SIGNING_IDENTITY: signing.identity, HOLLOW_BUILD_VERSION: version };
  for (const k of APPLE_ID_VARS) delete childEnv[k];
  if (!signing.notarize) for (const k of NOTARY_VARS) delete childEnv[k];
  return { command: 'npx', args, env: childEnv, cwd: root };
}

/** The environment for capture runs (verify tools, git, the app's self-test): no credentials. */
export function scrubbedEnv(env = process.env) {
  const out = { ...env };
  for (const k of [...NOTARY_VARS, ...APPLE_ID_VARS]) delete out[k];
  return out;
}

/** Plain-words lines about a build, safe to print: names, never credential values. */
export function describe(version, signing, dirty = false) {
  const who = signing.adHoc ? 'ad hoc ("-"), no team'
    : signing.teamId ? `"${signing.identity}", team ${signing.teamId}`
      : `"${signing.identity}" (self-signed, no team check)`;
  return [
    `version ${version}${dirty ? ' (dirty: the tree has uncommitted changes)' : ''}`,
    `signing: ${who}`,
    signing.notarize
      ? `notarization: on (${NOTARY_VARS.join(', ')} are set)`
      : 'notarization: off (no App Store Connect API key in the environment)',
  ];
}

function realRun(command, args, { env = process.env, cwd, capture = false } = {}) {
  const r = spawnSync(command, args, { env, cwd, encoding: 'utf8', stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit' });
  return { status: r.error ? null : r.status, stdout: r.stdout ?? '', stderr: r.stderr ?? '', error: r.error?.message };
}

const said = (r) => (r.error || `${r.stderr ?? ''}\n${r.stdout ?? ''}`).trim().split('\n').slice(-3).join(' ');

/**
 * The checks of design section 3 on a built app. Throws with the reason on
 * the first failure; returns the names of the checks that passed.
 * `expect`: { identity, adHoc, teamId, notarized, version }. `parts` picks
 * the signature, version and provenance checks and/or the embed self-test
 * (the nightly builder reports them as separate steps).
 */
export function verifyApp(app, expect, { run = realRun, root, log = () => {}, env = process.env, parts = { checks: true, selftest: true } } = {}) {
  const passed = [];
  const step = (name) => { passed.push(name); log(`[release] ok: ${name}`); };
  const quiet = scrubbedEnv(env);
  const capture = (cmd, args) => run(cmd, args, { capture: true, cwd: root, env: quiet });
  if (!existsSync(join(app, 'Contents', 'Info.plist'))) throw new Error(`no app at ${app}`);
  const plist = (key) => {
    const p = capture('/usr/libexec/PlistBuddy', ['-c', `Print :${key}`, join(app, 'Contents', 'Info.plist')]);
    if (p.status !== 0) throw new Error(`Info.plist has no ${key}: ${said(p)}`);
    return p.stdout.trim();
  };
  let r;
  if (parts.checks !== false) {
    r = capture('/usr/bin/codesign', ['--verify', '--deep', '--strict', '--verbose=2', app]);
    if (r.status !== 0) throw new Error(`the signature does not verify (codesign --verify --deep --strict): ${said(r)}`);
    step('codesign --verify --deep --strict');

    r = capture('/usr/bin/codesign', ['-dv', '--verbose=4', app]);
    if (r.status !== 0) throw new Error(`could not read the signature (codesign -dv): ${said(r)}`);
    const details = `${r.stderr}\n${r.stdout}`;
    const field = (key) => details.match(new RegExp(`^${key}=(.*)$`, 'm'))?.[1]?.trim() ?? null;
    if (expect.adHoc) {
      if (field('Signature') !== 'adhoc') throw new Error('expected an ad hoc signature, but the app is signed with an identity');
    } else {
      if (field('Signature') === 'adhoc') throw new Error(`expected a signature by "${expect.identity}", but the app is signed ad hoc`);
      const authority = field('Authority');
      if (!CERT_HASH.test(expect.identity) && !(authority ?? '').includes(expect.identity)) {
        throw new Error(`signed by "${authority ?? 'nobody'}", expected "${expect.identity}"`);
      }
      if (expect.teamId) {
        const team = field('TeamIdentifier');
        if (team !== expect.teamId) throw new Error(`the signature's team is ${team ?? 'not set'}, expected ${expect.teamId}`);
      }
    }
    step(expect.adHoc ? 'ad hoc signature' : expect.teamId ? `team ${expect.teamId}` : `signed by "${expect.identity}"`);

    if (expect.notarized) {
      r = capture('/usr/sbin/spctl', ['--assess', '--type', 'execute', '--verbose=2', app]);
      if (r.status !== 0) throw new Error(`Gatekeeper rejects the app (spctl --assess): ${said(r)}`);
      step('spctl --assess --type execute');
      r = capture('/usr/bin/xcrun', ['stapler', 'validate', app]);
      if (r.status !== 0) throw new Error(`the notarization ticket is not stapled (stapler validate): ${said(r)}`);
      step('stapler validate');
    }

    const shortVersion = plist('CFBundleShortVersionString');
    if (shortVersion !== expect.version) throw new Error(`the app says version ${shortVersion}, expected ${expect.version}`);
    step(`version ${expect.version}`);

    // As .github/workflows/prerelease-build.yml: provenance, then the self-test.
    const provenance = join(app, 'Contents', 'Resources', 'build-provenance.json');
    let embedded;
    try { embedded = JSON.parse(readFileSync(provenance, 'utf8')); } catch (e) { throw new Error(`no readable build-provenance.json in the app: ${e.message}`); }
    if (embedded.version !== expect.version) throw new Error(`build-provenance.json says version ${embedded.version}, expected ${expect.version}`);
    r = capture(process.execPath, [join(root, 'scripts', 'source-provenance.mjs'), '--check', provenance]);
    if (r.status !== 0) throw new Error(`the app was not built from this tree (source-provenance --check): ${said(r)}`);
    step('source provenance');
  }
  if (parts.selftest === false) return passed;

  const exe = join(app, 'Contents', 'MacOS', plist('CFBundleExecutable'));
  r = capture(exe, ['embed-selftest']);
  const out = (r.stdout ?? '').trim();
  if (r.status !== 0 || !out.startsWith('EMBED_SELFTEST_OK ')) throw new Error(`embed self-test failed: ${said(r)}`);
  // default_model_dir() falls back to src-tauri/models; require the bundled one.
  if (!out.includes('/Contents/MacOS/../Resources/models/')) throw new Error(`embed self-test did not load the bundled model: ${out}`);
  step('embed-selftest');
  return passed;
}

function flagValue(argv, name) {
  const i = argv.indexOf(name);
  if (i < 0) return undefined;
  const v = argv[i + 1];
  if (!v || v.startsWith('--')) throw new Error(`${name} needs a value`);
  return v;
}

/**
 * The whole release build. Every outside command goes through `run`, so tests
 * drive it with a fake. Returns { version, signing, app, passed }.
 */
export function releaseBuild({ root, env = process.env, argv = [], run = realRun, log = console.log } = {}) {
  const known = new Set(['--ref', '--dmg', '--verify-only', '--allow-dirty', '--no-verify', '--no-selftest', '--selftest-only']);
  argv.forEach((a, i) => {
    if (a.startsWith('-')) { if (!known.has(a)) throw new Error(`unknown option ${a}`); return; }
    if (argv[i - 1] !== '--ref') throw new Error(`unexpected argument ${JSON.stringify(a)} (options: ${[...known].join(', ')})`);
  });
  const has = (flag) => argv.includes(flag);
  if (has('--no-verify') && (has('--verify-only') || has('--selftest-only') || has('--no-selftest'))) {
    throw new Error('--no-verify builds without checks; it cannot be combined with --verify-only, --selftest-only or --no-selftest');
  }
  if (has('--selftest-only') && has('--no-selftest')) throw new Error('--selftest-only and --no-selftest contradict each other');
  if (has('--selftest-only') && has('--verify-only')) throw new Error('--selftest-only already verifies nothing else; drop --verify-only');
  const verifyOnly = has('--verify-only') || has('--selftest-only');
  const targetProblem = cargoTargetProblem(env.CARGO_TARGET_DIR);
  if (targetProblem) throw new Error(targetProblem);
  const signing = resolveSigning(env);
  const ref = flagValue(argv, '--ref') ?? 'HEAD';
  const quiet = scrubbedEnv(env);
  const git = (cwd, args) => {
    const r = run('git', args, { capture: true, cwd, env: quiet });
    if (r.status !== 0) throw new Error(`git ${args.join(' ')} failed: ${said(r)}`);
    return r.stdout.trim();
  };
  if (ref !== 'HEAD') {
    if (ref.startsWith('-')) throw new Error(`bad ref ${JSON.stringify(ref)}`);
    const want = git(root, ['rev-parse', '--verify', `${ref}^{commit}`]);
    const head = git(root, ['rev-parse', '--verify', 'HEAD^{commit}']);
    if (want !== head) throw new Error(`--ref ${ref} is ${want.slice(0, 12)} but the checkout is at ${head.slice(0, 12)}: check out the commit to build first`);
  }
  const version = buildVersion(root, ref, git);
  const dirty = git(root, ['status', '--porcelain']) !== '';
  for (const line of describe(version, signing, dirty)) log(`[release] ${line}`);
  const app = appPath(root, env);

  if (!verifyOnly) {
    if (dirty && !argv.includes('--allow-dirty')) {
      throw new Error('the tree has uncommitted changes (dirty); commit them, or pass --allow-dirty for a local branch build');
    }
    const plan = buildPlan({ root, env, version, signing, dmg: argv.includes('--dmg') });
    log(`[release] running: ${plan.command} ${plan.args.map(a => (/[\s{"]/.test(a) ? `'${a}'` : a)).join(' ')}`);
    const r = run(plan.command, plan.args, { env: plan.env, cwd: plan.cwd });
    if (r.status !== 0) throw new Error(`tauri build failed (exit ${r.status ?? 'none'})${r.error ? `: ${r.error}` : ''}`);
  }
  if (has('--no-verify')) {
    log(`[release] ${APP_NAME} ${version} built, not verified (--no-verify): ${app}`);
    return { version, signing, app, passed: [] };
  }
  // --verify-only takes the notarization expectation from the environment too.
  const parts = { checks: !has('--selftest-only'), selftest: !has('--no-selftest') };
  const passed = verifyApp(app, { ...signing, notarized: signing.notarize, version }, { run, root, log, env, parts });
  log(`[release] ${APP_NAME} ${version} ${verifyOnly ? 'checked' : 'built and verified'}: ${app}`);
  return { version, signing, app, passed };
}

// realpath: /tmp is a symlink on macOS, and node runs the real file.
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('../..', import.meta.url));
  try {
    releaseBuild({ root, argv: process.argv.slice(2) });
  } catch (e) {
    console.error(`[release] failed: ${e.message}`);
    process.exit(1);
  }
}
