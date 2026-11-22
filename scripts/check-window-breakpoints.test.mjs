import assert from 'node:assert/strict';
import { test } from 'node:test';
import { windowBreakpoints } from './check-window-breakpoints.mjs';

test('repository window widths stay named and narrow hosts stay documented', () => {
  assert.deepEqual(windowBreakpoints().problems, []);
});

test('a comment covers only the immediately following narrow block', () => {
  const text = '/* Narrow window: 390px Playwright host. */\n@media(max-width:600px) { .a {} }\n@media(max-width:600px) { .b {} }';
  const result = windowBreakpoints([{ name: 'example.css', text }]);
  assert.equal(result.problems.length, 1);
  assert.match(result.problems[0], /example.css:3/);
});

test('local pane containers are independent of window widths', () => {
  const text = '@container (max-width:420px) { .a {} } @container hollow-window (max-width:53.75em) { .b {} }';
  assert.deepEqual(windowBreakpoints([{ name: 'example.css', text }]), { widths: ['(max-width:53.75em)'], problems: [] });
});

test('the rail compact token cannot drift from the shared CSS threshold', () => {
  for (const declaration of ['--window-compact-em: 54;', '']) {
    const text = `:root { ${declaration} } @container hollow-window (max-width: 53.75em) { body {} }`;
    assert.ok(windowBreakpoints([{ name: 'tokens.css', text }]).problems.some(p => p.includes('--window-compact-em')));
  }
});

test('the medium token cannot drift from pane CSS', () => {
  for (const declaration of ['--window-medium-em: 70;', '']) {
    const text = `:root { --window-compact-em: 53.75; ${declaration} }`;
    assert.ok(windowBreakpoints([{ name: 'tokens.css', text }]).problems.some(p => p.includes('--window-medium-em')));
  }
});
