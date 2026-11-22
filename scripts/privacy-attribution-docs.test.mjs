// OC11 doc contract: the macOS privacy attribution note must stay in the two
// docs a person reads before updating the app or the headless daemon. The note
// is prose, so this only pins the facts a reader acts on, not the wording.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const read = (path) => readFileSync(join(root, path), 'utf8');
// assert.match prints the whole doc on a miss; say only what is missing.
const has = (text, pattern, what) => assert.ok(pattern.test(text), `missing: ${what} (${pattern})`);
const lacks = (text, pattern, what) => assert.ok(!pattern.test(text), `unexpected: ${what} (${pattern})`);

// Each entry is one thing the note must say; the pattern is deliberately loose.
const FACTS = [
  ['names the original path a child is charged to', /original (app )?path/i],
  ['says an update can strand a stale denial', /stale (permission )?denial/i],
  ['says Accessibility and Screen Recording need a new grant', /Accessibility[\s\S]{0,40}Screen Recording[\s\S]{0,200}(re-grant|grant (it )?again)/i],
  ['tells the person to run the helper from a signed copy started by launchd', /signed copy[\s\S]{0,120}launchd/i],
  ['gives the lsof check for what still holds the old path', /lsof \+D/],
];

const DOCS = ['docs/HEADLESS_SERVE.md', 'docs/reference/build-and-ship.md'];

for (const path of DOCS) {
  test(`${path} carries the macOS privacy attribution note`, () => {
    const text = read(path);
    has(text, /^#{2,3} macOS privacy grants after an update/m, `${path} has no "macOS privacy grants after an update" heading`);
    for (const [what, pattern] of FACTS) has(text, pattern, `${path} ${what}`);
  });
}

test('the headless daemon doc and the installer doc point at each other', () => {
  has(read('docs/HEADLESS_SERVE.md'), /build-and-ship\.md#macos-privacy-grants-after-an-update/, 'link to the installer doc');
  has(read('docs/reference/build-and-ship.md'), /HEADLESS_SERVE\.md#macos-privacy-grants-after-an-update/, 'link to the daemon doc');
});

test('the note does not claim Hollow handles grants: the installer has no privacy code', () => {
  // If the installer ever learns about grants, this fails and the note is
  // rewritten with it. Until then the note is guidance, not behaviour.
  const installer = read('scripts/install-macos-app.mjs');
  lacks(installer, /tccutil|Accessibility|ScreenCapture|kTCC/, 'privacy code in the installer');
  has(read('docs/reference/build-and-ship.md'), /(does not|doesn't) touch[\s\S]{0,40}privacy grants/i, 'the statement that the installer does not touch privacy grants');
});

// OC16 doc contract: the phone "Access and trust" note keeps the facts a person
// decides on. Each is checked against the code it was derived from, so the doc
// can't drift from `store.rs` or the route table.
test('the phone Access and trust note states what pairing allows and how revoking behaves', () => {
  const text = read('docs/reference/surfaces.md');
  const at = text.indexOf('### Access and trust for a paired phone');
  assert.ok(at >= 0, 'surfaces.md has the Access and trust section');
  const note = text.slice(at, text.indexOf('\n### ', at + 5));
  has(note, /Allow\/Deny card[\s\S]{0,120}only\s+from a phone[\s\S]{0,400}Face ID/i, 'says a phone approves only when enrolled and behind Face ID');
  has(note, /Revoking while the phone is offline: only part of this is verified/, 'the offline caveat');
  has(note, /no expiry/i, 'says the token has no expiry');
  has(note, /10 minutes or one use/, 'says the pairing window expires');
});

test('the Access and trust claims match the code', () => {
  const store = read('src-tauri/src/phone_access/store.rs');
  has(store, /PAIRING_MINUTES: i64 = 10;/, 'the window is 10 minutes');
  has(store, /MAX_WRONG_CODES: u32 = 5;/, 'five wrong codes close it');
  // No expiry field on a device: the token lives until the record is revoked.
  const device = store.slice(store.indexOf('pub(crate) struct Device {'), store.indexOf('/// v1.1: a phone\'s APNs registration.'));
  lacks(device, /expires|expiry|valid_until/i, 'a device record has no expiry');
  // No phone route approves a card or edits settings.
  const routes = read('src-tauri/src/phone_access/routes.rs');
  const table = routes.slice(routes.indexOf('("GET", "/v1/health", Scope::Open)'), routes.indexOf('("GET", "/v1/workspaces/{id}/canvases/{canvas}"'));
  // B4: the approval routes are phone routes; settings, secrets and devices still aren't, and
  // the decision route verifies the answer against the phone's key.
  const withoutApprovals = table.split('\n').filter(line => !/"\/v1\/approvals(\/key(\/challenge)?|\/\{id\}\/(challenge|decision))?"/.test(line)).join('\n');
  lacks(withoutApprovals, /approv|settings|secrets|"\/v1\/devices|revoke/i, 'no settings, secrets or device route, and no unsigned approval route');
  has(read('src-tauri/src/phone_access/approvals_api.rs'), /ECDSA_P256_SHA256_ASN1[\s\S]{0,200}\.verify\(message/, 'a decision is verified against the phone key');
});
