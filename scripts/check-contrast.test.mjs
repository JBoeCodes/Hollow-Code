// Overnight F5: the contrast check over src/styles/tokens.css.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { EXTRA, KNOWN, SURFACES, TEXT, TOKENS, block, check, contrast, parseColor, resolve, themes } from './check-contrast.mjs';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const css = (light, dark = '') => `:root {\n${light}\n}\n:root[data-theme="dark"] {\n${dark}\n}\n`;

test('risk and warning remain distinct in both stock themes', () => {
  for (const vars of Object.values(themes(readFileSync(TOKENS, 'utf8')))) {
    assert.notDeepEqual(resolve(vars, '--risk'), resolve(vars, '--warning'));
    assert.notEqual(vars.get('--risk-soft'), vars.get('--amber-soft'));
  }
});

test('rendered wikilink hover text meets AA over its tinted reading surfaces', () => {
  const root = join(dirname(fileURLToPath(import.meta.url)), '..');
  const sheet = readFileSync(join(root, 'src/styles/vault-inbox-chat.css'), 'utf8');
  const rule = sheet.match(/\.viewer-rendered a\.wikilink:hover\s*\{([^}]+)\}/)[1];
  const foreground = rule.match(/(?:^|;)\s*color:\s*var\((--[\w-]+)\)/)[1];
  const background = rule.match(/background:\s*var\((--[\w-]+)\)/)[1];
  for (const [theme, vars] of Object.entries(themes(readFileSync(TOKENS, 'utf8')))) {
    const tint = vars.get(background).match(/[\d.]+/g).map(Number);
    assert.equal(tint.length, 4);
    for (const surface of SURFACES) {
      const base = resolve(vars, surface);
      const fill = tint.slice(0, 3).map((value, i) => value * tint[3] + base[i] * (1 - tint[3]));
      const ratio = contrast(resolve(vars, foreground), fill);
      assert.ok(ratio >= 4.5, `${theme} wikilink hover over ${surface}: ${ratio.toFixed(2)}:1`);
    }
  }
});

test('rail aurora retains AA at the maximum overlapping tint in both themes', () => {
  const source = readFileSync(TOKENS, 'utf8');
  for (const [theme, vars] of Object.entries(themes(source))) {
    const raw = theme === 'dark' ? source.match(/:root\[data-theme="dark"\]\s*\{\s*--ambient-tint:\s*([\d.]+)%/)?.[1] : vars.get('--ambient-tint')?.replace('%','');
    const tint = Number(raw) / 100;
    assert.ok(Number.isFinite(tint) && tint > 0 && tint < 1);
    const surface = resolve(vars, '--surface-soft');
    let bg = surface;
    for (const accent of ['--accent-teal', '--primary']) bg = resolve(vars, accent).map((v,i)=>v*tint+bg[i]*(1-tint));
    const opacity = Number(vars.get('--ambient-rail-opacity'));
    assert.ok(opacity > 0 && opacity <= 1);
    bg = bg.map((v,i)=>v*opacity+surface[i]*(1-opacity));
    for (const {token} of TEXT) assert.ok(contrast(resolve(vars,token),bg)>=4.5, `${theme} ${token} on overlapping rail aurora`);
  }
});

test('Glass material wells retain AA text even over opposite-polarity backdrops', () => {
  const source = readFileSync(TOKENS, 'utf8');
  for (const [theme, vars] of Object.entries(themes(source))) {
    const glass = block(source, theme === 'light' ? ':root[data-glass]' : 'html[data-glass][data-theme="dark"]');
    for (const token of ['--glass-well', '--glass-well-strong']) {
      const channels = glass.get(token).match(/[\d.]+/g).map(Number);
      assert.equal(channels.length, 4, token);
      const alpha = channels[3];
      const background = channels.slice(0, 3).map(v => v * alpha + (theme === 'light' ? 0 : 255) * (1 - alpha));
      for (const { token: foreground } of TEXT) {
        const ratio = contrast(resolve(vars, foreground), background);
        assert.ok(ratio >= 4.5, `${theme} ${foreground} on ${token}: ${ratio.toFixed(3)}:1`);
      }
    }
  }
});

test('the shipped tokens meet WCAG AA in both themes', () => {
  const rows = check(readFileSync(TOKENS, 'utf8'));
  const failed = rows.filter(r => !r.ok && !r.known).map(r => `${r.theme} ${r.token} on ${r.bg}: ${r.ratio?.toFixed(2)}`);
  assert.deepEqual(failed, []);
  // The known failures still fail (a fixed one must leave KNOWN), and the
  // list only shrinks.
  const known = rows.filter(r => r.known).map(r => `${r.theme} ${r.token} on ${r.bg}`);
  assert.deepEqual(known, [...KNOWN.keys()]);
  assert.equal(KNOWN.size, 0, 'KNOWN is empty; a new known failure needs a reason and a reviewer');
  assert.equal(rows.length, 2 * [...TEXT, ...EXTRA].reduce((n, p) => n + p.on.length, 0));
});

test('every text token and surface resolves to a colour in both themes', () => {
  const { light, dark } = themes(readFileSync(TOKENS, 'utf8'));
  for (const vars of [light, dark]) {
    for (const token of [...SURFACES, ...TEXT.map(t => t.token), ...EXTRA.map(e => e.token)]) {
      assert.ok(resolve(vars, token), `${token} resolves`);
    }
  }
});

test('contrast is the WCAG ratio', () => {
  assert.equal(contrast([0, 0, 0], [255, 255, 255]).toFixed(1), '21.0');
  assert.equal(contrast([255, 255, 255], [255, 255, 255]), 1);
  // #767676 on white is the classic 4.54:1 boundary.
  assert.equal(contrast(parseColor('#767676'), parseColor('#fff')).toFixed(2), '4.54');
});

test('a token under AA fails, in the theme where it fails', () => {
  const pairs = [{ token: '--text', on: ['--bg'], min: 4.5 }];
  const rows = check(css('--bg: #ffffff; --text: #777777;', '--bg: #111111; --text: #eeeeee;'), pairs);
  assert.deepEqual(rows.map(r => [r.theme, r.ok]), [['light', false], ['dark', true]]);
});

test('aliases resolve, dark inherits what it does not set, and unreadable colours fail', () => {
  const sheet = css('--bg: #ffffff; --base: #222222; --text: var(--base); --wash: rgba(0, 0, 0, .5);', '--bg: #000000;');
  const { light, dark } = themes(sheet);
  assert.deepEqual(resolve(light, '--text'), [34, 34, 34]);
  assert.deepEqual(resolve(dark, '--text'), [34, 34, 34]);
  assert.equal(resolve(light, '--wash'), null, 'a translucent colour is not checked as opaque');
  const rows = check(sheet, [{ token: '--wash', on: ['--bg'], min: 4.5 }]);
  assert.ok(rows.every(r => !r.ok && r.ratio === null));
});

test('block reads only the named selector, ignoring comments and later blocks', () => {
  const vars = block(':root { /* --x: #000; */ --a: #111111; }\n@media (max-width: 860px) { :root { --a: #333333; } }\n:root[data-density="compact"] { --a: #222222; }', ':root');
  assert.deepEqual([...vars], [['--a', '#111111']]);
  assert.throws(() => block(':root { }', ':root[data-theme="dark"]'), /no :root\[data-theme="dark"\] block/);
});

test('run through a symlink, it still runs the check', t => {
  const dir = mkdtempSync(join(tmpdir(), 'contrast-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const link = join(dir, 'check-contrast.mjs');
  symlinkSync(join(dirname(fileURLToPath(import.meta.url)), 'check-contrast.mjs'), link);
  const out = execFileSync(process.execPath, [link], { encoding: 'utf8' });
  assert.match(out, /^contrast: ok \(\d+ pairs/m);
});
