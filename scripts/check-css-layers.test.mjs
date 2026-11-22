import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const checker = resolve('scripts/check-css-tokens.mjs');
function check(css, filename = 'fixture.css') {
  const dir = mkdtempSync(join(tmpdir(), 'hollow-css-layers-'));
  try {
    mkdirSync(join(dir, 'src/styles'), { recursive: true });
    writeFileSync(join(dir, 'src/styles', filename), css);
    writeFileSync(join(dir, 'src/styles.css'), '@layer tokens, base, components, shell, glass, a11y;');
    return spawnSync(process.execPath, [checker], { cwd: dir, encoding: 'utf8' });
  } finally { rmSync(dir, { recursive: true, force: true }); }
}

for (const css of [
  'button { outline: none; }',
  '/* @layer components {} */ button { outline: none; }',
  '@media (min-width: 1px) { button { outline: none; } }',
  '@layer components { .kept { color: inherit; } } button { outline: none; }',
]) test(`unlayered rules fail css:check: ${css}`, () => {
  const result = check(css);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /unlayered/i);
});

test('layers survive nested conditions, comments and braces in strings', () => {
  const result = check('@layer components { @media (min-width: 1px) { button::after { content: "} ; {"; } } }');
  assert.equal(result.status, 0, result.stderr);
});

test('an undeclared layer cannot outrank the shell', () => {
  const result = check('@layer focusfix { button:focus-visible { outline: none; } }');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /layer/i);
});

test('CodeMirror exception keeps library descendants but rejects arbitrary buttons', () => {
  assert.equal(check('.cm-editor { outline: none; } .cm-tooltip.cm-tooltip-autocomplete > ul > li { color: inherit; }', 'codemirror.css').status, 0);
  for (const css of [
    '.vault-editor-host button:focus-visible { outline: none; }',
    'body:has(.cm-editor) button { outline: none; }',
    '.vault-editor-host button:not(.cm-x):focus-visible { outline: none; }',
    'button:is(.cm-a, .cm-b) { outline: none; }',
    'button:not(.x .cm-y) { outline: none; }',
  ]) assert.equal(check(css, 'codemirror.css').status, 1);
});

test('component sheets cannot use the glass or accessibility layer', () => {
  for (const name of ['glass', 'a11y', 'components.extra']) {
    assert.equal(check(`@layer ${name} { button { outline: none; } }`).status, 1);
  }
  assert.equal(check('@layer a11y { .hidden { display: none; } }', 'foundations.css').status, 0);
});
