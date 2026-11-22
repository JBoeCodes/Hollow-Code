import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { configProblems, requiredSpecProblems } from './check-required-specs.mjs';

test('each minimum project must independently retain the native height', () => {
  const config = readFileSync(new URL('../playwright.config.ts', import.meta.url), 'utf8');
  for (const project of ['chromium-minimum', 'chromium-minimum-large-text']) {
    const at = config.indexOf(`name: '${project}'`);
    const changed = config.slice(0, at) + config.slice(at).replace('width: 720, height: 480', 'width: 720, height: 600');
    assert.ok(configProblems(changed).some(p => p.includes(project) && p.includes('720')),
      `${project} must reject a taller window even when the other project is correct`);
  }
});

test('the repository passes: required specs exist, never skip, and the minimum projects run them', () => {
  assert.deepEqual([...requiredSpecProblems(), ...configProblems()], []);
});

test('a skip, fixme or only in a required spec is refused; other specs are not checked', () => {
  const files = { 'a.minimum.spec.ts': "test.skip(true, 'x');", 'b.journey.spec.ts': "test.describe.only('x', () => {});", 'c.spec.ts': "test.skip(true, 'fine here');", 'd.journey.spec.ts': "test('ok', async () => {});" };
  const problems = requiredSpecProblems('/d', f => files[f.replace('/d/', '')], () => Object.keys(files));
  assert.deepEqual(problems, ['a.minimum.spec.ts uses test.skip: a required spec may not skip', 'b.journey.spec.ts uses test.describe.only: a required spec may not skip']);
});
