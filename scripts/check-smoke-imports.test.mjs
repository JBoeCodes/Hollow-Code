import assert from 'node:assert/strict';
import { test } from 'node:test';
import { specsBypassingFixture, specsWritingCheckoutFiles } from './check-smoke-imports.mjs';

test('the repository passes: every spec imports the page-error fixture', () => {
  assert.deepEqual(specsBypassingFixture(), []);
});

test('a spec that imports @playwright/test directly is reported; helpers are not', () => {
  const files = { 'a.spec.ts': "import { test } from '@playwright/test';", 'b.spec.ts': "import { test } from './fixtures/test';", 'choose.ts': "import type { Page } from '@playwright/test';" };
  assert.deepEqual(specsBypassingFixture('/d', f => files[f.replace('/d/', '')], () => Object.keys(files)), ['a.spec.ts']);
});

test('the repository passes: no spec saves a screenshot into the checkout', () => {
  assert.deepEqual(specsWritingCheckoutFiles(), []);
});

test('a fixed screenshot path is reported; outputPath and attach are not', () => {
  const files = {
    'a.spec.ts': "await page.screenshot({ path: '.context/x.png' });",
    'b.spec.ts': 'await page.screenshot({ path: info.outputPath(`x-${w}.png`) });',
    'c.spec.ts': "await info.attach('x', { body: await page.screenshot(), contentType: 'image/png' });",
    'd.spec.ts': 'await page.screenshot({ path: `shots/${name}.png` });',
    'e.spec.ts': 'await page.screenshot({ path: `${process.env.SHOTS}/x.png`, fullPage: false });',
    'f.spec.ts': "const ws = [{ id: 'a', path: 'workspaces/a' }];",
    'g.spec.ts': 'await page.screenshot({ path: `${__dirname}/x.png` });',
    'h.spec.ts': "await page.screenshot({ clip: { x: 0, y: 0, width: 9, height: 9 }, path: join('.context', 'x.png') });",
  };
  assert.deepEqual(specsWritingCheckoutFiles('/d', f => files[f.replace('/d/', '')], () => Object.keys(files)), ['a.spec.ts', 'd.spec.ts', 'g.spec.ts', 'h.spec.ts']);
});
