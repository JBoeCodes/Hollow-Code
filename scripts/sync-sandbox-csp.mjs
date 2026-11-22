#!/usr/bin/env node
/**
 * Keep the app CSP's sandbox script hashes in lockstep with the scripts.
 *
 * Host-owned scripts are embedded VERBATIM as inline <script>s in sandboxed
 * artifact documents (htmlSandbox.ts `sandboxHead`):
 *   - src/components/sandbox-doc.js, the host runtime (imported `?raw`);
 *   - src/canvas/kit/generated/kit.min.js, the Hollow Canvas Kit, written by
 *     scripts/build-canvas-kit.mjs (which `npm run build` runs first);
 *   - src/canvas/kit/generated/kit-ask.min.js, the ask kit (Canvas C4 A1),
 *     in ask-canvases only, written by the same build.
 * srcdoc documents inherit the app's CSP, whose script-src deliberately has
 * no 'unsafe-inline', so both are allowed there by sha256 hash-sources
 * (hashes match content, independent of the sandbox's opaque origin).
 *
 * Only the app CSP lists them. Artifact policies (served by artifacts.rs and
 * the matching <meta>) allow the two scripts by a per-document nonce instead:
 * under CSP3 a hash-source also admits an external script whose `integrity`
 * carries it (SRI repair, 2026-10-05). A srcdoc fallback stays safe because
 * its own nonce <meta> applies on top of the inherited app CSP.
 *
 * This script recomputes the hashes, rewrites src-tauri/tauri.conf.json's
 * script-src to list exactly those three (runtime, kit, ask kit), and updates
 * the SANDBOX_RUNTIME_HASH / KIT_SCRIPT_HASH / KIT_ASK_SCRIPT_HASH constants
 * in htmlSandbox.ts. It runs
 * as part of `npm run build` (which tauri build invokes via
 * beforeBuildCommand), so editing either script can never silently break
 * installed builds. A Rust test (artifacts.rs) fails on a stale copy.
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const scriptPath = join(root, 'src', 'components', 'sandbox-doc.js');
const kitPath = join(root, 'src', 'canvas', 'kit', 'generated', 'kit.min.js');
const kitAskPath = join(root, 'src', 'canvas', 'kit', 'generated', 'kit-ask.min.js');
const confPath = join(root, 'src-tauri', 'tauri.conf.json');
const sandboxPath = join(root, 'src', 'components', 'htmlSandbox.ts');

const hashOf = (path) => `'sha256-${createHash('sha256').update(readFileSync(path)).digest('base64')}'`;
const token = hashOf(scriptPath);
const kitToken = hashOf(kitPath);
const kitAskToken = hashOf(kitAskPath);

const conf = readFileSync(confPath, 'utf8');
const cspRe = /("csp":\s*")([^"]*)(")/;
const m = conf.match(cspRe);
if (!m) {
  console.error('sync-sandbox-csp: no "csp" key found in tauri.conf.json');
  process.exit(1);
}

let csp = m[2];
const scriptSrcRe = /(script-src [^;]*)/;
if (!scriptSrcRe.test(csp)) {
  console.error('sync-sandbox-csp: no script-src directive found in csp');
  process.exit(1);
}
// script-src lists exactly the runtime, kit and ask kit hashes, after its other sources.
const withHash = csp.replace(scriptSrcRe, (directive) => {
  const kept = directive.split(/\s+/).filter(t => !/^'sha256-[A-Za-z0-9+/=]+'$/.test(t));
  return [...kept, token, kitToken, kitAskToken].join(' ');
});

if (withHash === csp) {
  console.log(`sync-sandbox-csp: hashes up to date (${token} ${kitToken} ${kitAskToken})`);
} else {
  writeFileSync(confPath, conf.replace(cspRe, `$1${withHash.replace(/\$/g, '$$$$')}$3`));
  console.log(`sync-sandbox-csp: updated tauri.conf.json script-src → ${token} ${kitToken} ${kitAskToken}`);
}

const sandbox = readFileSync(sandboxPath, 'utf8');
const sandboxHashRe = /SANDBOX_RUNTIME_HASH = "'sha256-[A-Za-z0-9+/=]+'"/;
const kitHashRe = /KIT_SCRIPT_HASH = "'sha256-[A-Za-z0-9+/=]+'"/;
const kitAskHashRe = /KIT_ASK_SCRIPT_HASH = "'sha256-[A-Za-z0-9+/=]+'"/;
if (!sandboxHashRe.test(sandbox) || !kitHashRe.test(sandbox) || !kitAskHashRe.test(sandbox)) {
  console.error('sync-sandbox-csp: htmlSandbox.ts lacks SANDBOX_RUNTIME_HASH, KIT_SCRIPT_HASH or KIT_ASK_SCRIPT_HASH');
  process.exit(1);
}
const sandboxNext = sandbox
  .replace(sandboxHashRe, `SANDBOX_RUNTIME_HASH = "${token}"`)
  .replace(kitHashRe, `KIT_SCRIPT_HASH = "${kitToken}"`)
  .replace(kitAskHashRe, `KIT_ASK_SCRIPT_HASH = "${kitAskToken}"`);
if (sandboxNext !== sandbox) {
  writeFileSync(sandboxPath, sandboxNext);
  console.log(`sync-sandbox-csp: updated htmlSandbox.ts hashes → ${token} ${kitToken} ${kitAskToken}`);
}
