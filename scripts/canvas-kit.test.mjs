import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  KIT_ASK_BUDGET_BYTES, KIT_ASK_CSS_PATH, KIT_ASK_JS_PATH, KIT_BUDGET_BYTES, KIT_CSS_PATH, KIT_JS_PATH,
  buildKit, buildKitAsk, checkKit, kitBytes,
} from './build-canvas-kit.mjs';

const kit = buildKit();
const ask = buildKitAsk();

test('the committed kit matches a fresh build (run npm run kit:build when this fails)', () => {
  // Like sandbox-doc.js's hash: artifacts.rs and the app CSP allow the kit
  // script by the sha256 of the committed bytes, so a stale copy would ship
  // a kit that doesn't match its source.
  assert.equal(readFileSync(KIT_JS_PATH, 'utf8'), kit.js, 'generated/kit.min.js is stale');
  assert.equal(readFileSync(KIT_CSS_PATH, 'utf8'), kit.css, 'generated/kit.min.css is stale');
});

test('the build is deterministic', () => {
  assert.deepEqual(buildKit(), kit);
});

test('kit CSS + JS stay within the 99 KiB minified budget', () => {
  assert.equal(KIT_BUDGET_BYTES, 99 * 1024);
  const size = kitBytes(kit);
  assert.ok(size <= KIT_BUDGET_BYTES, `kit is ${size} bytes, budget ${KIT_BUDGET_BYTES}`);
  assert.deepEqual(checkKit(kit), []);
});

test('the budget and policy checks fail when they should', () => {
  assert.match(checkKit(kit, kitBytes(kit) - 1).join('\n'), /over the \d+-byte budget/);
  const bad = {
    js: 'eval("1");fetch("/x");new WebSocket("ws://x");el.innerHTML=s;"</script>"',
    css: '@import "x.css";a{background:url(https://x/y.png)}</style>',
  };
  const problems = checkKit(bad).join('\n');
  for (const p of ['</script', '</style', 'eval', 'fetch', 'WebSocket', 'innerHTML', 'url() or @import']) {
    assert.ok(problems.includes(p), `missing ${p} in:\n${problems}`);
  }
});

// Canvas C4 A1: the ask kit, inlined only into ask-canvases, has its own
// budget and the same rules.
test('the committed ask kit matches a fresh build (run npm run kit:build when this fails)', () => {
  assert.equal(readFileSync(KIT_ASK_JS_PATH, 'utf8'), ask.js, 'generated/kit-ask.min.js is stale');
  assert.equal(readFileSync(KIT_ASK_CSS_PATH, 'utf8'), ask.css, 'generated/kit-ask.min.css is stale');
  assert.deepEqual(buildKitAsk(), ask);
});

test('ask kit CSS + JS stay within its own budget, at most 25 KiB', () => {
  assert.equal(KIT_ASK_BUDGET_BYTES, 25 * 1024);
  const size = kitBytes(ask);
  assert.ok(size <= KIT_ASK_BUDGET_BYTES, `kit-ask is ${size} bytes, budget ${KIT_ASK_BUDGET_BYTES}`);
  assert.deepEqual(checkKit(ask, KIT_ASK_BUDGET_BYTES, 'kit-ask'), []);
  assert.match(checkKit(ask, size - 1, 'kit-ask').join('\n'), /kit-ask is \d+ bytes minified, over the \d+-byte budget/);
  // The display kit is still separate: the inputs did not move into it.
  // Checklist recognizes its parent tag; the draft transport remains ask-only.
  assert.ok(!kit.js.includes('kind:"draft"'));
  assert.ok(!kit.js.includes('Your answers go into the message box.'));
});

test('the bridge field survives minifying (private `_x` names are mangled, `__hc` never)', () => {
  assert.match(ask.js, /__hc:1,kind:"draft"/);
  assert.match(ask.js, /\.__hc!==1/);
});
