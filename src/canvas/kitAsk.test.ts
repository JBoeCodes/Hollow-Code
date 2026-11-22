import { describe, it, expect, vi } from 'vitest';
vi.mock('../api', () => ({ api: {} }));
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (id: string) => `artifact://localhost/${id}` }));
import KIT_ASK_JS from './kit/generated/kit-ask.min.js?raw';
import KIT_ASK_CSS from './kit/generated/kit-ask.min.css?raw';
import KIT_CSS from './kit/generated/kit.min.css?raw';
import ASK_INDEX_TS from './kit/ask/index.ts?raw';
import ASK_GALLERY from './kit/ask/gallery.html?raw';
import { KIT_COMPONENTS, KIT_VERSION } from './kit/manifest';
import { KIT_ASK_COMPONENTS, KIT_ASK_FIELDS } from './kit/ask/manifest';
import {
  DRAFT_ANSWERS_BYTES, DRAFT_SUMMARY_CHARS, TEXT_MAX, buildDraft, clip, display, fieldName, isIsoDate, jsonBytes,
  move, onStep, parseOptions, pickRows, rangeSpec, rankTarget, splitList, summaryText, validate,
  type FieldAnswer, type FieldRule,
} from './kit/ask/model';
import { CANVAS_KIT } from './kitAssets';
import { CANVAS_KIT_ASK } from './kitAskAssets';
import {
  KIT_ASK_SCRIPT_HASH, KIT_SCRIPT_HASH, NO_KIT, SANDBOX_RUNTIME_HASH, THEME_TOKENS,
  loadCanvasKitAsk, loadedCanvasKitAsk, sandboxHead,
} from '../components/htmlSandbox';

const NONCE = 'AAAAAAAAAAAAAAAAAAAAAAAA';
const rule = (kind: FieldRule['kind'], extra: Partial<FieldRule> = {}): FieldRule => ({ kind, required: false, ...extra });

describe('ask kit model: rows, names and bounds (Canvas C4 A1)', () => {
  it('reads one option per line, `* ` marks the recommended one, blanks and repeats are dropped', () => {
    expect(parseOptions(['', '  * Friday, after the bed ', 'Monday', '   ', 'Monday', '*not a mark'])).toEqual([
      { label: 'Friday, after the bed', recommended: true },
      { label: 'Monday', recommended: false },
      { label: '*not a mark', recommended: false },
    ]);
  });

  it('splits a short list on pipes, else commas', () => {
    expect(splitList('Low | Medium | High')).toEqual(['Low', 'Medium', 'High']);
    expect(splitList('Yes, No, Yes')).toEqual(['Yes', 'No']);
    expect(splitList('  ')).toEqual([]);
  });

  it('files an answer under the author\'s name, else one made from the label, never twice', () => {
    const taken = new Set<string>();
    expect(fieldName('ship-day', 'When?', 0, taken)).toBe('ship-day');
    expect(fieldName('ship-day', 'Again', 1, taken)).toBe('ship-day-2');
    expect(fieldName('', 'Merge cadence (weeks)', 2, taken)).toBe('merge-cadence-weeks');
    expect(fieldName('__proto__', 'Café', 3, taken)).toBe('cafe');
    expect(fieldName('bad name', '', 4, taken)).toBe('field-5');
    expect(fieldName('', '日本語', 5, taken)).toBe('field-6');
  });

  it('range bounds default to 0–100 step 1, swap a reversed pair and refuse a step ≤ 0', () => {
    expect(rangeSpec(null, null, null)).toEqual({ min: 0, max: 100, step: 1 });
    expect(rangeSpec('12', '1', '-2')).toEqual({ min: 1, max: 12, step: 1 });
    expect(rangeSpec('0', '1', '0.1')).toEqual({ min: 0, max: 1, step: 0.1 });
    expect(rangeSpec('x', '', 'abc')).toEqual({ min: 0, max: 100, step: 1 });
    expect(onStep(0.3, { min: 0, max: 1, step: 0.1 })).toBe(true);
    expect(onStep(24, { min: 16, max: 32, step: 2 })).toBe(true);
    expect(onStep(25, { min: 16, max: 32, step: 2 })).toBe(false);
  });

  it('accepts only real ISO dates', () => {
    expect(isIsoDate('2026-10-05')).toBe(true);
    expect(isIsoDate('2028-02-29')).toBe(true);
    for (const bad of ['2026-02-30', '2026-13-01', '05/10/2026', '2026-1-5', '']) expect(isIsoDate(bad), bad).toBe(false);
  });

  it('reads pick rows like hc-bars: a label and a value, or just a label', () => {
    expect(pickRows(['Week 1: 12', '', 'Week 2 | 1,200 h', 'Plain option', 'Time: 10:30am'])).toEqual([
      { label: 'Week 1', cell: '12', n: 12 },
      { label: 'Week 2', cell: '1,200 h', n: 1200 },
      { label: 'Plain option', cell: '', n: null },
      { label: 'Time: 10', cell: '30am', n: 30 },
    ]);
  });

  it('moves a ranked item by key: Up, Down, Home, End, clamped at the ends', () => {
    expect(rankTarget('ArrowUp', 2, 4)).toBe(1);
    expect(rankTarget('ArrowUp', 0, 4)).toBe(0);
    expect(rankTarget('ArrowDown', 3, 4)).toBe(3);
    expect(rankTarget('Home', 2, 4)).toBe(0);
    expect(rankTarget('End', 0, 4)).toBe(3);
    expect(rankTarget('ArrowLeft', 2, 4)).toBeNull();
    expect(move(['a', 'b', 'c', 'd'], 0, 2)).toEqual(['b', 'c', 'a', 'd']);
    expect(move(['a', 'b', 'c'], 2, 0)).toEqual(['c', 'a', 'b']);
    expect(move(['a', 'b'], 5, 0)).toEqual(['a', 'b']);
  });
});

describe('ask kit model: each input\'s value shape and validation', () => {
  it('hc-choice and hc-pick: one string, required means chosen', () => {
    expect(validate(rule('choice'), null)).toBeNull();
    expect(validate(rule('choice', { required: true }), null)).toBe('Choose one option.');
    expect(validate(rule('choice', { required: true }), 'Friday')).toBeNull();
    expect(validate(rule('pick', { required: true }), null)).toBe('Pick one item.');
    expect(validate(rule('pick', { required: true }), 'Week 3')).toBeNull();
  });

  it('hc-multi: an array; required is at least one, min and max bound the count', () => {
    expect(validate(rule('multi'), [])).toBeNull();
    expect(validate(rule('multi', { required: true }), [])).toBe('Choose at least one option.');
    expect(validate(rule('multi', { minCount: 2 }), ['a'])).toBe('Choose at least 2 options.');
    expect(validate(rule('multi', { maxCount: 2 }), ['a', 'b', 'c'])).toBe('Choose at most 2 options.');
    expect(validate(rule('multi', { maxCount: 1 }), ['a', 'b'])).toBe('Choose only one option.');
    expect(validate(rule('multi', { minCount: 1, maxCount: 3 }), ['a', 'b'])).toBeNull();
  });

  it('hc-range: a number within the bounds, on a step', () => {
    const range = { min: 16, max: 32, step: 2 };
    expect(validate(rule('range', { range }), null)).toBeNull();
    expect(validate(rule('range', { range, required: true }), null)).toBe('Set a value.');
    expect(validate(rule('range', { range }), 24)).toBeNull();
    expect(validate(rule('range', { range }), 40)).toBe('Enter a number from 16 to 32.');
    expect(validate(rule('range', { range }), Number.NaN)).toBe('Enter a number from 16 to 32.');
    expect(validate(rule('range', { range }), 25)).toBe('Use steps of 2 from 16.');
  });

  it('hc-text: trimmed text within maxlength; the defaults and cap', () => {
    expect(TEXT_MAX).toEqual({ short: 200, long: 2000, cap: 8000 });
    expect(validate(rule('text', { required: true }), '   ')).toBe('Write an answer.');
    expect(validate(rule('text'), null)).toBeNull();
    expect(validate(rule('text', { maxLength: 5 }), ' hello ')).toBeNull();
    expect(validate(rule('text', { maxLength: 5 }), 'hello!')).toBe('Keep it to 5 characters.');
    expect(validate(rule('text'), 'x'.repeat(201))).toBe('Keep it to 200 characters.');
  });

  it('hc-date: an ISO date within min and max', () => {
    const r = rule('date', { minDate: '2026-10-01', maxDate: '2026-12-31' });
    expect(validate({ ...r, required: true }, null)).toBe('Pick a date.');
    expect(validate(r, '2026-10-12')).toBeNull();
    expect(validate(r, '12/10/2026')).toBe('Enter a date as YYYY-MM-DD.');
    expect(validate(r, '2026-09-30')).toBe('Pick a date on or after 2026-10-01.');
    expect(validate(r, '2027-01-01')).toBe('Pick a date on or before 2026-12-31.');
  });

  it('hc-matrix: a row → option object; required means every row', () => {
    const r = rule('matrix', { required: true, rows: ['Speed', 'Cost', 'Risk'] });
    expect(validate(r, null)).toBe('Answer every row (3 left).');
    expect(validate(r, { Speed: 'High', Cost: 'Low' })).toBe('Answer every row (1 left).');
    expect(validate(r, { Speed: 'High', Cost: 'Low', Risk: 'Low' })).toBeNull();
    expect(validate({ ...r, required: false }, null)).toBeNull();
  });

  it('hc-rank: always valid, its value is the order', () => {
    expect(validate(rule('rank'), ['b', 'a'])).toBeNull();
    expect(display('rank', ['Inputs', 'Draft'])).toBe('1. Inputs; 2. Draft');
  });

  it('shows each answered value as a person reads it, and nothing for an unanswered one', () => {
    expect(display('choice', 'Friday')).toBe('Friday');
    expect(display('multi', ['Bed rows', 'Smoke halves'])).toBe('Bed rows, Smoke halves');
    expect(display('range', 24, 'KB')).toBe('24 KB');
    expect(display('range', 0)).toBe('0');
    expect(display('text', '  two\n lines  ')).toBe('two lines');
    expect(display('matrix', { Speed: 'High', Cost: 'Low' })).toBe('Speed: High; Cost: Low');
    for (const empty of [null, '', [], {}] as const) expect(display('multi', empty as never)).toBeNull();
  });
});

const FIELDS: FieldAnswer[] = [
  { name: 'ship-day', label: 'When should the release ship?', kind: 'choice', value: 'Friday, after the bed passes' },
  { name: 'checks', label: 'Which checks?', kind: 'multi', value: ['Bed rows', 'Smoke halves'] },
  { name: 'budget', label: 'Kit budget', kind: 'range', value: 24, unit: 'KB' },
  { name: 'order', label: 'Build order', kind: 'rank', value: ['Inputs', 'Draft'] },
  { name: 'notes', label: 'Anything else?', kind: 'text', value: null },
  { name: 'fit', label: 'Rate each option', kind: 'matrix', value: { Speed: 'High' } },
];

describe('ask kit model: the bridge draft payload', () => {
  it('collects {canvasId, answers, summary}: answers by name (null when unanswered), a label: value line each', () => {
    const r = buildDraft('release-choices', FIELDS);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(Object.keys(r.draft)).toEqual(['canvasId', 'answers', 'summary']);
    expect(r.draft.canvasId).toBe('release-choices');
    expect(r.draft.answers).toEqual({
      'ship-day': 'Friday, after the bed passes', checks: ['Bed rows', 'Smoke halves'], budget: 24,
      order: ['Inputs', 'Draft'], notes: null, fit: { Speed: 'High' },
    });
    expect(r.draft.summary).toBe([
      'When should the release ship?: Friday, after the bed passes',
      'Which checks?: Bed rows, Smoke halves',
      'Kit budget: 24 KB',
      'Build order: 1. Inputs; 2. Draft',
      'Rate each option: Speed: High',
    ].join('\n'));
    expect(summaryText([])).toBe('No answers given.');
  });

  it('a name never reaches a prototype', () => {
    const r = buildDraft('a', [{ name: '__proto__', label: 'P', kind: 'text', value: 'x' }]);
    expect(r.ok && Object.getPrototypeOf(r.draft.answers)).toBe(Object.prototype);
    expect(r.ok && JSON.stringify(r.draft.answers)).toBe('{"__proto__":"x"}');
  });

  it('measures answers in UTF-8 bytes: 16 KB of JSON passes, one byte more is refused', () => {
    // {"t":"…"} is 8 bytes of JSON around the text.
    const ascii = 'a'.repeat(DRAFT_ANSWERS_BYTES - 8);
    const fits = buildDraft('a', [{ name: 't', label: 'T', kind: 'text', value: ascii }]);
    expect(fits.ok).toBe(true);
    expect(jsonBytes({ t: ascii })).toBe(DRAFT_ANSWERS_BYTES);
    expect(buildDraft('a', [{ name: 't', label: 'T', kind: 'text', value: `${ascii}a` }])).toEqual({ ok: false, error: 'too-large', bytes: DRAFT_ANSWERS_BYTES + 1 });
    // The same character count in a 3-byte script is three times the bytes.
    const kana = 'あ'.repeat(6000);
    expect(kana.length).toBeLessThan(DRAFT_ANSWERS_BYTES);
    const wide = buildDraft('a', [{ name: 't', label: 'T', kind: 'text', value: kana }]);
    expect(wide).toEqual({ ok: false, error: 'too-large', bytes: 18008 });
  });

  it('clips the summary to 500 UTF-16 units with an ellipsis, never splitting a pair', () => {
    expect(DRAFT_SUMMARY_CHARS).toBe(500);
    const long = buildDraft('a', [{ name: 't', label: 'T', kind: 'text', value: 'x'.repeat(2000) }]);
    expect(long.ok && long.draft.summary.length).toBe(500);
    expect(long.ok && long.draft.summary.endsWith('x…')).toBe(true);
    const astral = clip(`${'a'.repeat(498)}🚀🚀`, 500);
    expect(astral.length).toBeLessThanOrEqual(500);
    expect(astral).toBe(`${'a'.repeat(498)}…`);
    expect(clip('short', 500)).toBe('short');
  });
});

describe('ask kit bundle (Canvas C4 A1)', () => {
  it('registers every ask component, distinct from the display kit, and the bundle defines each', () => {
    expect([...KIT_ASK_COMPONENTS]).toEqual(['hc-ask', 'hc-choice', 'hc-multi', 'hc-range', 'hc-rank', 'hc-text', 'hc-date', 'hc-matrix', 'hc-pick']);
    expect(KIT_ASK_FIELDS).toEqual(KIT_ASK_COMPONENTS.slice(1));
    const registered = [...ASK_INDEX_TS.matchAll(/^\s+'(hc-[a-z-]+)': Hc\w+,$/gm)].map(m => m[1]);
    expect(registered).toEqual([...KIT_ASK_COMPONENTS]);
    for (const name of KIT_ASK_COMPONENTS) {
      expect(KIT_ASK_JS).toContain(`"${name}"`);
      expect((KIT_COMPONENTS as readonly string[]).includes(name), name).toBe(false);
    }
    expect(CANVAS_KIT_ASK).toEqual({ version: KIT_VERSION, js: KIT_ASK_JS, css: KIT_ASK_CSS });
  });

  it('posts only a v1 draft (the bridge field survives minifying) and nothing that loads', () => {
    expect(KIT_ASK_JS).toContain('__hc:1,kind:"draft"');
    // Question cards (2026-10-14): only as the reply to Hollow's `collect`,
    // with no Send button of its own; it locks on `answered`.
    expect(KIT_ASK_JS).toContain('"collect"');
    // Overnight 10 A7: it hears a host `collect` first (capture phase) and
    // stops it there, so a canvas script's listener can't answer it.
    expect(KIT_ASK_JS).toContain('stopImmediatePropagation');
    expect(KIT_ASK_JS).toContain('kind:"ask-invalid"');
    expect(KIT_ASK_JS).not.toContain('hc-ask-send');
    expect(KIT_ASK_JS).not.toContain('"drafted"');
    expect(KIT_ASK_JS).toContain('"answered"');
    expect(KIT_ASK_JS).toContain('.inert=!0');
    expect(KIT_ASK_JS).not.toContain('isn’t available in this version');
    expect(KIT_ASK_JS).not.toMatch(/kind:"(send|open|navigate|state)"/);
    expect(KIT_ASK_JS).not.toMatch(/\bfetch\(|XMLHttpRequest|WebSocket|sendBeacon|innerHTML|\.submit\(/);
  });

  it('reaches `_` members by dot only: the build mangles them, so a quoted name would miss (P1-33)', () => {
    // A quoted `'_v'` or `['_sel']` survives minifying while `this._v` is
    // renamed, so a kept answer would be written where no field reads it.
    expect(KIT_ASK_JS).not.toMatch(/["'`]_[A-Za-z]\w*["'`]/);
    expect(KIT_ASK_JS).toContain('hollowCanvas');
  });

  it('its script hash is the sha256 of the inlined bytes, distinct from the runtime and the kit', async () => {
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(KIT_ASK_JS)));
    expect(KIT_ASK_SCRIPT_HASH).toBe(`'sha256-${btoa(String.fromCharCode(...digest))}'`);
    expect(new Set([KIT_ASK_SCRIPT_HASH, KIT_SCRIPT_HASH, SANDBOX_RUNTIME_HASH]).size).toBe(3);
  });

  it('is inlined only when asked: after the kit\'s stylesheet and the kit\'s script, with the document nonce', () => {
    const plain = sandboxHead('body{margin:0}', 'offline', CANVAS_KIT, NONCE);
    expect(plain).not.toContain(KIT_ASK_JS);
    expect(plain).not.toContain(KIT_ASK_CSS);
    const head = sandboxHead('body{margin:0}', 'offline', CANVAS_KIT, NONCE, CANVAS_KIT_ASK);
    const kitCss = head.indexOf(`<style>${CANVAS_KIT.css}</style>`);
    const askCss = head.indexOf(`<style>${KIT_ASK_CSS}</style>`);
    const runtime = head.indexOf(`<script nonce="${NONCE}">/**`);
    const kitJs = head.indexOf(`<script nonce="${NONCE}">${CANVAS_KIT.js}</script>`);
    const askJs = head.indexOf(`<script nonce="${NONCE}">${KIT_ASK_JS}</script>`);
    expect(kitCss).toBeGreaterThan(-1);
    expect(askCss).toBeGreaterThan(kitCss);
    expect(runtime).toBeGreaterThan(askCss);
    expect(kitJs).toBeGreaterThan(runtime);
    expect(askJs).toBeGreaterThan(kitJs);
    // Canvas mode: no nonce, 'unsafe-inline' runs it.
    expect(sandboxHead('', 'canvas', CANVAS_KIT, NONCE, CANVAS_KIT_ASK)).toContain(`<script>${KIT_ASK_JS}</script>`);
    expect(sandboxHead('', 'offline', CANVAS_KIT, NONCE, NO_KIT).match(/<script nonce=/g)).toHaveLength(2);
  });

  it('loads lazily, once, and is then available synchronously', async () => {
    const first = await loadCanvasKitAsk();
    expect(first).toBe(CANVAS_KIT_ASK);
    expect(await loadCanvasKitAsk()).toBe(first);
    expect(loadedCanvasKitAsk()).toBe(CANVAS_KIT_ASK);
  });

  it('reads only tokens the host sends (each with a fallback) and kit variables the kit defines', () => {
    const sent = new Set(THEME_TOKENS.map(([name]) => name));
    const reads = [...KIT_ASK_CSS.matchAll(/var\((--app-[a-z0-9-]+)(,?)/g)];
    expect(reads.length).toBeGreaterThan(3);
    for (const [, name, comma] of reads) {
      expect(sent.has(name), name).toBe(true);
      expect(comma, `${name} needs a fallback`).toBe(',');
    }
    for (const [, name] of KIT_ASK_CSS.matchAll(/var\((--hc-[a-z0-9-]+)/g)) expect(KIT_CSS, name).toContain(`${name}:`);
  });

  it('respects reduced motion, shows focus, and keeps the questions readable without the script', () => {
    expect(KIT_ASK_CSS).toMatch(/@media \(prefers-reduced-motion: ?reduce\)\{[^}]*transition:none/);
    expect(KIT_ASK_CSS).toContain('[data-hc-motion=reduce]');
    expect(KIT_ASK_CSS).toMatch(/\.hc-rank-item[^{]*:focus-visible/);
    expect(KIT_ASK_CSS).toContain(':not([data-hc-ready])');
    expect(KIT_ASK_CSS).not.toMatch(/url\(|@import/);
  });
});

describe('ask kit gallery', () => {
  it('uses every ask component and is markup only: no script, no URL', () => {
    const used = new Set([...ASK_GALLERY.matchAll(/<(hc-[a-z]+)/g)].map(m => m[1]));
    expect([...used].sort()).toEqual([...KIT_ASK_COMPONENTS, 'hc-checklist'].sort());
    expect(ASK_GALLERY).not.toMatch(/<script|\son[a-z]+=|https?:|src=|href=|url\(|@import/i);
    for (const tag of KIT_ASK_FIELDS) expect(ASK_GALLERY, tag).toMatch(new RegExp(`<${tag} name="[a-z-]+" label="`));
  });
});
