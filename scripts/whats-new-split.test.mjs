// What's new split at build (ST1): the two halves join back to the file byte
// for byte, the cut falls only on a top-level `# ` outside a fence, and the
// real docs/WHATS_NEW.md keeps its newest releases under the cap.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { RECENT_BYTES, releaseTitle, splitWhatsNew, whatsNewPlugin } from './whats-new-split.mjs';

const doc = (...parts) => parts.join('\n');
const release = (name, bytes) => doc(`# What's new: ${name}`, '', `### ${name} feature`, `- **What:** ${'x'.repeat(bytes)}`, '');

test('production history is a separate text asset with a build-resolved URL', () => {
  const plugin = whatsNewPlugin();
  plugin.configResolved?.({ command: 'build' });
  const emitted = [];
  const context = { addWatchFile() {}, emitFile(file) { emitted.push(file); return 'history'; } };
  const source = plugin.load.call(context, '\0virtual:whats-new-recent');
  assert.match(source, /export const EARLIER_URL = import\.meta\.ROLLUP_FILE_URL_history/);
  assert.equal(emitted.length, 1);
  assert.equal(emitted[0].type, 'asset');
  const md = readFileSync(fileURLToPath(new URL('../docs/WHATS_NEW.md', import.meta.url)), 'utf8');
  assert.equal(emitted[0].source, splitWhatsNew(md).earlier);
});

test('development history is served as plain text and can be requested again', () => {
  const plugin = whatsNewPlugin();
  const routes = [];
  plugin.configureServer?.({ middlewares: { use(path, fn) { routes.push([path, fn]); } } });
  assert.equal(routes.length, 1);
  assert.match(routes[0][0], /whats-new-earlier/);
  const responses = [];
  for (let i = 0; i < 2; i++) routes[0][1]({}, { setHeader() {}, end(body) { responses.push(body); } });
  assert.ok(responses[0].startsWith("# What's new:"));
  assert.equal(responses[0], responses[1]);
});

test('the halves join back to the file exactly', () => {
  const md = doc(release('c', 30), release('b', 30), release('a', 30));
  const { recent, earlier } = splitWhatsNew(md, 120);
  assert.equal(recent + earlier, md);
  assert.ok(recent.startsWith("# What's new: c"));
  assert.ok(earlier.startsWith("# What's new: b") || earlier.startsWith("# What's new: a"));
});

test('whole releases stay while they fit, and the first always stays', () => {
  const md = doc(release('c', 200), release('b', 30), release('a', 30));
  const one = splitWhatsNew(md, 50);
  assert.ok(one.recent.includes('c feature') && !one.recent.includes('b feature'));
  assert.deepEqual(one.earlierTitles, ['b', 'a']);
  const all = splitWhatsNew(md, 10_000);
  assert.equal(all.recent, md);
  assert.equal(all.earlier, '');
  assert.deepEqual(all.earlierTitles, []);
});

test('a `# ` line inside a code fence is not a release', () => {
  const fenced = doc("# What's new: b", '```', '# not a heading', 'y'.repeat(200), '```', '');
  const md = doc(release('c', 10), fenced, release('a', 10));
  const { recent, earlier, earlierTitles } = splitWhatsNew(md, 120);
  assert.equal(recent + earlier, md);
  assert.deepEqual(earlierTitles, ['b', 'a']);
  assert.ok(!recent.includes('# not a heading'));
});

test('release titles match parseWhatsNew (prefix and links dropped)', () => {
  assert.equal(releaseTitle("# What's new: overnight 2026-10-12/13"), 'overnight 2026-10-12/13');
  assert.equal(releaseTitle('# What’s new: [#453](https://example.test) fixes'), '#453 fixes');
});

test('the real What’s new keeps its newest releases under the cap', () => {
  const md = readFileSync(fileURLToPath(new URL('../docs/WHATS_NEW.md', import.meta.url)), 'utf8');
  const { recent, earlier, earlierTitles } = splitWhatsNew(md);
  assert.equal(recent + earlier, md);
  assert.ok(Buffer.byteLength(recent) <= RECENT_BYTES || !earlierTitles.length, 'recent half over the cap');
  assert.ok(earlierTitles.length > 0, 'expected earlier releases in the real file');
});
