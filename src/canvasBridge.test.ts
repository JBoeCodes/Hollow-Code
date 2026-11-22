import { describe, it, expect } from 'vitest';
import {
  BRIDGE_VERSION, CHILD_KINDS, DRAFT_FIELD_NAME, LOGGED_REASONS, bridgeRefusal, refusalLimiter, DRAFT_INTERVAL_MS, LIMITS, MAX_MESSAGE_BYTES, RESERVED_CHILD_KINDS, acceptFrameMessage, answeredMessage, canvasState, draftRefusal, collectMessage, designCheckMessage, newCollectNonce, COLLECT_NONCE,
  initMessage, isSafeThemeValue, messageBytes, parseChildMessage, themeMessage,
} from './canvasBridge';
import { DRAFT_ANSWERS_BYTES, DRAFT_SUMMARY_CHARS, FIELD_NAME, buildDraft } from './canvas/kit/ask/model';
import RUNTIME_JS from './components/sandbox-doc.js?raw';

const v1 = (kind: string, fields: Record<string, unknown>) => ({ __hc: 1, kind, ...fields });

const N = '0123456789abcdef0123456789abcdef';

describe('canvas bridge v1: child messages', () => {
  it('accepts settled height, rejects extra or out-of-range fields, and settles only after init', () => {
    expect(parseChildMessage(v1('settled', { h: 320 }))).toEqual({ ok: true, message: { kind: 'settled', h: 320 } });
    for (const data of [v1('settled', { h: 100001 }), v1('settled', { h: 20, extra: true }), v1('settled', { h: NaN })]) expect(parseChildMessage(data).ok).toBe(false);
    expect(RUNTIME_JS.indexOf("post('settled'")).toBeGreaterThan(RUNTIME_JS.indexOf("d.kind !== 'init'"));
    expect(RUNTIME_JS.match(/post\('settled'/g)).toHaveLength(1);
  });
  it('accepts each supported kind with exactly its fields', () => {
    expect(parseChildMessage(v1('size', { h: 320 }))).toEqual({ ok: true, message: { kind: 'size', h: 320 } });
    expect(parseChildMessage(v1('error', { message: 'x is not defined (line 3)' }))).toEqual({ ok: true, message: { kind: 'error', message: 'x is not defined (line 3)' } });
    expect(parseChildMessage(v1('navigate', { target: ' Projects/Hollow ' }))).toEqual({ ok: true, message: { kind: 'navigate', target: 'Projects/Hollow' } });
    expect(parseChildMessage(v1('open', { href: ' https://example.com/a ' }))).toEqual({ ok: true, message: { kind: 'open', href: 'https://example.com/a', link: 'web' } });
    expect(parseChildMessage(v1('open', { href: 'mailto:a@b.c' }))).toEqual({ ok: true, message: { kind: 'open', href: 'mailto:a@b.c', link: 'mail' } });
  });

  it('rejects anything that is not a plain v1 envelope', () => {
    for (const data of [null, undefined, 'https://example.com', 42, [], new Date(), Object.assign(Object.create({ __hc: 1 }), { kind: 'size', h: 1 })]) {
      expect(parseChildMessage(data), String(data)).toEqual({ ok: false, reason: 'not-object' });
    }
    expect(parseChildMessage({ __hc: 2, kind: 'size', h: 1 })).toEqual({ ok: false, reason: 'version' });
    expect(parseChildMessage({ __hc: '1', kind: 'size', h: 1 })).toEqual({ ok: false, reason: 'version' });
    // The pre-C1 envelopes are no longer understood.
    expect(parseChildMessage({ __jbDoc: 1, kind: 'open', href: 'https://example.com' })).toEqual({ ok: false, reason: 'version' });
    expect(parseChildMessage({ __jbArtifact: 1, h: 10 })).toEqual({ ok: false, reason: 'version' });
    expect(parseChildMessage(v1('jsError', { message: 'x' }))).toEqual({ ok: false, reason: 'unknown-kind' });
    expect(parseChildMessage(v1('send', { text: 'hi' }))).toEqual({ ok: false, reason: 'unknown-kind' });
  });

  it('caps the whole message at 64 KB, before looking at its kind', () => {
    const fits = v1('error', { message: 'x' });
    expect(messageBytes(fits)).toBeLessThan(MAX_MESSAGE_BYTES);
    const big = v1('open', { href: 'https://example.com/?' + 'a'.repeat(MAX_MESSAGE_BYTES) });
    expect(parseChildMessage(big)).toEqual({ ok: false, reason: 'too-large' });
    // Bytes, not UTF-16 units: 22k three-byte characters are ~66 KB.
    expect(parseChildMessage(v1('error', { message: '€'.repeat(22_000) }))).toEqual({ ok: false, reason: 'too-large' });
    const cyclic: Record<string, unknown> = v1('size', { h: 1 }); cyclic.self = cyclic;
    expect(parseChildMessage(cyclic)).toEqual({ ok: false, reason: 'too-large' });
  });

  it('enforces each kind\'s smaller limits and exact fields', () => {
    const bad = [
      v1('size', { h: -1 }), v1('size', { h: Infinity }), v1('size', { h: NaN }), v1('size', { h: '10' }),
      v1('size', { h: LIMITS.maxHeight + 1 }), v1('size', { h: 10, extra: true }), v1('size', {}),
      v1('error', { message: 'x'.repeat(LIMITS.errorChars + 1) }), v1('error', { message: 7 }),
      v1('navigate', { target: '' }), v1('navigate', { target: '   ' }), v1('navigate', { target: 'a\nb' }),
      v1('navigate', { target: 'n'.repeat(LIMITS.navigateChars + 1) }),
      v1('open', { href: 'javascript:alert(1)' }), v1('open', { href: 'file:///etc/passwd' }),
      v1('open', { href: 'data:text/html,hi' }), v1('open', { href: 'https://example.com/a b' }),
      v1('open', { href: 'https://example.com/\u0000' }), v1('open', {}),
      v1('open', { href: 'https://example.com/?' + 'q'.repeat(LIMITS.hrefChars) }),
      v1('open', { href: 'https://example.com', target: '_top' }),
    ];
    for (const data of bad) expect(parseChildMessage(data), JSON.stringify(data).slice(0, 80)).toEqual({ ok: false, reason: 'invalid' });
  });
  it('C4 A2: the ask kit\'s draft is accepted with exactly its fields, as a fresh copy', () => {
    expect(DRAFT_ANSWERS_BYTES).toBe(LIMITS.draftBytes);
    expect(DRAFT_SUMMARY_CHARS).toBe(LIMITS.draftSummaryChars);
    expect(DRAFT_FIELD_NAME.source).toBe(FIELD_NAME.source);
    const built = buildDraft('pick-a-plan', [
      { name: 'plan', label: 'Plan', kind: 'choice', value: 'Friday' },
      { name: 'checks', label: 'Checks', kind: 'multi', value: ['Bed', 'Budget'] },
      { name: 'budget', label: 'Budget', kind: 'range', value: 24 },
      { name: 'fit', label: 'Fit', kind: 'matrix', value: { Speed: 'High' } },
      { name: 'notes', label: 'Notes', kind: 'text', value: null },
    ]);
    expect(built.ok).toBe(true);
    if (!built.ok) return;
    const nonce = newCollectNonce();
    expect(nonce).toMatch(COLLECT_NONCE);
    expect(newCollectNonce()).not.toBe(nonce);
    expect(collectMessage(nonce)).toEqual({ __hc: 1, kind: 'collect', nonce });
    const message = v1('draft', { ...built.draft, nonce });
    expect(Object.keys(message).sort()).toEqual(['__hc', 'answers', 'canvasId', 'kind', 'nonce', 'summary']);
    expect(messageBytes(message)!).toBeLessThan(MAX_MESSAGE_BYTES);
    const r = parseChildMessage(message);
    expect(r).toEqual({ ok: true, message: { kind: 'draft', canvasId: 'pick-a-plan', answers: built.draft.answers, summary: built.draft.summary, nonce } });
    expect(r.ok && r.message.kind === 'draft' && r.message.answers).not.toBe(built.draft.answers);
    expect(RESERVED_CHILD_KINDS).toEqual([]);
    // Question cards (2026-10-14): the reply to Send when answers need attention.
    expect(parseChildMessage(v1('ask-invalid', { nonce, count: 2 }))).toEqual({ ok: true, message: { kind: 'ask-invalid', nonce, count: 2 } });
    for (const bad of [{ nonce, count: 0 }, { nonce, count: 1.5 }, { nonce: 'short', count: 1 }, { nonce, count: 1, extra: 1 }, { count: 1 }]) {
      expect(parseChildMessage(v1('ask-invalid', bad))).toEqual({ ok: false, reason: 'invalid' });
    }
  });

  it('C4 A2: a draft is refused for extra or missing fields, a bad id, oversize answers or summary, and any non-A1 answer shape', () => {
    const draft = (fields: Record<string, unknown>) => v1('draft', { canvasId: 'pick', answers: { a: 'x' }, summary: 'A: x', nonce: N, ...fields });
    const fill = (n: number, ch = 'x') => ({ k: ch.repeat(n) });
    const overhead = messageBytes(fill(0))!;
    expect(parseChildMessage(draft({ answers: fill(LIMITS.draftBytes - overhead) })).ok).toBe(true);
    expect(parseChildMessage(draft({ summary: 's'.repeat(LIMITS.draftSummaryChars) })).ok).toBe(true);
    expect(parseChildMessage(draft({ answers: {} })).ok).toBe(true);
    expect(parseChildMessage(draft({ summary: 'A: x\nB: y\tz' })).ok).toBe(true);
    const bad = [
      draft({ extra: 1 }), draft({ nonce: undefined }), draft({ nonce: 'UPPER0123456789ABCD' }), draft({ nonce: 'short' }), v1('draft', { canvasId: 'pick', answers: {} }), v1('draft', { canvasId: 'pick', summary: 's' }), v1('draft', { answers: {}, summary: 's' }),
      draft({ canvasId: 'Pick' }), draft({ canvasId: '' }), draft({ canvasId: 'a'.repeat(65) }), draft({ canvasId: 7 }),
      // 16 KB of answers, counted in UTF-8 bytes.
      draft({ answers: fill(LIMITS.draftBytes - overhead + 1) }), draft({ answers: fill(6_000, '€') }),
      draft({ summary: 's'.repeat(LIMITS.draftSummaryChars + 1) }), draft({ summary: '' }), draft({ summary: 7 }), draft({ summary: 'a\u0000b' }), draft({ summary: 'a\rb' }),
      // Answers: a plain object of names to the A1 value shapes, nothing deeper.
      draft({ answers: null }), draft({ answers: [] }), draft({ answers: 'x' }), draft({ answers: new Map() }),
      draft({ answers: { 'bad name': 'x' } }), draft({ answers: { '-lead': 'x' } }), draft({ answers: { ['n'.repeat(65)]: 'x' } }),
      draft({ answers: { a: true } }), draft({ answers: { a: NaN } }), draft({ answers: { a: Infinity } }), draft({ answers: { a: undefined } }),
      draft({ answers: { a: [1, 2] } }), draft({ answers: { a: ['x', null] } }), draft({ answers: { a: [['x']] } }),
      draft({ answers: { a: { row: 1 } } }), draft({ answers: { a: { row: { deeper: 'x' } } } }), draft({ answers: { a: { row: ['x'] } } }),
      draft({ answers: { a: new Date() } }), draft({ answers: Object.assign(Object.create({ inherited: 1 }), { a: 'x' }) }),
    ];
    for (const data of bad) expect(parseChildMessage(data), JSON.stringify(data)?.slice(0, 90)).toEqual({ ok: false, reason: 'invalid' });
  });

  it('C4 A2: a "__proto__" answer or matrix row is data, never a prototype', () => {
    const posted = JSON.parse('{"__proto__": "x", "fit": {"__proto__": "High"}}');
    const r = parseChildMessage(v1('draft', { canvasId: 'pick', answers: posted, summary: 's', nonce: N }));
    // "__proto__" is not a field name the kit makes (FIELD_NAME), so the whole draft is refused.
    expect(r).toEqual({ ok: false, reason: 'invalid' });
    const row = JSON.parse('{"fit": {"__proto__": "High"}}');
    const ok = parseChildMessage(v1('draft', { canvasId: 'pick', answers: row, summary: 's', nonce: N }));
    expect(ok.ok).toBe(true);
    const fit = ok.ok && ok.message.kind === 'draft' ? ok.message.answers.fit as Record<string, unknown> : {};
    expect(Object.getPrototypeOf(fit)).toBe(Object.prototype);
    expect(Object.keys(fit)).toEqual(['__proto__']);
    expect(({} as Record<string, unknown>).High).toBeUndefined();
  });

  it('ideas 2026-10-27 #1: a design report is accepted within its limits, as a fresh copy', () => {
    const items = [
      { rule: 'contrast', text: 'Revenue by month', ratio: 3.12, need: 4.5 },
      { rule: 'small-text', text: 'footnote', size: 9.5 },
      { rule: 'texture-text', text: 'Over dots', need: 4.5, box: [12, 340, 220, 18] },
      { rule: 'texture-text', text: 'Shares its box', need: 3, box: null },
    ];
    const message = v1('design', { ticket: N, items, scanned: 12, more: 0, complete: true, width: 720, texture: true });
    const r = parseChildMessage(message);
    expect(r).toEqual({ ok: true, message: { kind: 'design', ticket: N, items, scanned: 12, more: 0, complete: true, width: 720, texture: true } });
    expect(r.ok && r.message.kind === 'design' && r.message.items[0]).not.toBe(items[0]);
    const full = v1('design', { ticket: N, items: Array.from({ length: LIMITS.designItems }, () => ({ rule: 'texture-text', text: 'x'.repeat(LIMITS.designTextChars), need: 3, box: [0, 0, LIMITS.designCoord, LIMITS.designCoord] })), scanned: LIMITS.designScan, more: LIMITS.designMore, complete: false, width: LIMITS.designWidth, texture: false });
    expect(parseChildMessage(full).ok).toBe(true);
    expect(messageBytes(full)!).toBeLessThan(MAX_MESSAGE_BYTES);
    expect(newCollectNonce()).toMatch(COLLECT_NONCE);
  });

  it('ideas 2026-10-27 #1: a design report is refused for any field out of shape or past its limits', () => {
    const box = { rule: 'texture-text', text: 'a', need: 4.5, box: [12, 340, 220, 18] };
    const design = (fields: Record<string, unknown>) => v1('design', { ticket: N, items: [], scanned: 0, more: 0, complete: true, width: 640, texture: false, ...fields });
    const item = (fields: Record<string, unknown>) => design({ items: [{ rule: 'contrast', text: 'a', ratio: 2, need: 4.5, ...fields }] });
    const bad = [
      design({ extra: 1 }), design({ ticket: 'short' }), design({ ticket: undefined }), v1('design', { ticket: N, items: [], scanned: 0, complete: true, width: 640, texture: false }),
      v1('design', { ticket: N, items: [], scanned: 0, more: 0, width: 640, texture: false }), design({ complete: 1 }),
      v1('design', { ticket: N, items: [], scanned: 0, more: 0, complete: true }), design({ width: 0 }), design({ width: 1.5 }), design({ width: LIMITS.designWidth + 1 }), design({ texture: 'yes' }), design({ complete: 'true' }), design({ complete: null }),
      v1('design', { nonce: N, items: [], scanned: 0, more: 0, complete: true, width: 640, texture: false }),
      design({ scanned: -1 }), design({ scanned: 1.5 }), design({ scanned: LIMITS.designScan + 1 }), design({ more: LIMITS.designMore + 1 }), design({ more: '3' }),
      design({ items: null }), design({ items: {} }), design({ items: Array.from({ length: LIMITS.designItems + 1 }, () => ({ rule: 'texture-text', text: 'x', need: 4.5, box: [12, 340, 220, 18] })) }),
      item({ ratio: 0.5 }), item({ ratio: 22 }), item({ ratio: NaN }), item({ need: 7 }), item({ need: '4.5' }), item({ extra: true }), item({ text: '' }), item({ text: '   ' }),
      item({ text: 'x'.repeat(LIMITS.designTextChars + 1) }), item({ text: 'a\u0000b' }), item({ text: 7 }), item({ rule: 'other' }),
      design({ items: [{ rule: 'small-text', text: 'a', size: -1 }] }), design({ items: [{ rule: 'small-text', text: 'a', size: 201 }] }),
      design({ items: [{ rule: 'small-text', text: 'a' }] }), design({ items: [{ rule: 'texture-text', text: 'a', ratio: 2 }] }),
      design({ items: [{ rule: 'texture-text', text: 'a' }] }), design({ items: [{ ...box, need: 7 }] }), design({ items: [{ ...box, box: [-1, 0, 5, 5] }] }), design({ items: [{ ...box, box: [0, 0, 0, 5] }] }),
      design({ items: [{ ...box, box: [0, 0, 5, 2.5] }] }), design({ items: [{ ...box, box: [LIMITS.designCoord, 0, 1, 1] }] }), design({ items: [{ ...box, box: [0, 0, 5] }] }),
      design({ items: [{ ...box, box: { x: 0 } }] }), design({ items: [{ ...box, box: undefined }] }), design({ items: [{ ...box, extra: 1 }] }), design({ items: ['text'] }),
    ];
    for (const data of bad) expect(parseChildMessage(data), JSON.stringify(data)?.slice(0, 120)).toEqual({ ok: false, reason: 'invalid' });
  });

  it('ideas 2026-10-27 #1: the runtime measures only when asked, once per load, after it settled', () => {
    expect(designCheckMessage(N)).toEqual({ __hc: 1, kind: 'design-check', ticket: N });
    const listener = RUNTIME_JS.indexOf("d.kind !== 'design-check'");
    expect(listener).toBeGreaterThan(0);
    // The ticket is checked like a collect nonce and taken once.
    expect(RUNTIME_JS.slice(listener, listener + 400)).toContain('/^[a-z0-9]{16,40}$/');
    expect(RUNTIME_JS.slice(listener, listener + 400)).toContain('if (designTicket) return;');
    // It runs from the settle (or at once if settled), and posts one report.
    expect(RUNTIME_JS).toContain("post('settled',{h:measure()});settledOnce=true;if(designTicket)runDesign();");
    expect(RUNTIME_JS.match(/post\('design'/g)).toHaveLength(1);
    // Kit elements and their rendered parts are never judged.
    expect(RUNTIME_JS).toContain("/^HC-/.test(el.tagName) || !!(el.closest && el.closest('[data-hc-part]'))");
  });

  it('C4 A2: a frame drafts only for its own canvas; another window\'s draft is a source rejection', () => {
    const win = {};
    const frame = { contentWindow: win, isConnected: true };
    const draft = v1('draft', { canvasId: 'pick', answers: {}, summary: 's', nonce: N });
    expect(acceptFrameMessage({ source: win, origin: 'null', data: draft }, frame, 'pick').ok).toBe(true);
    expect(acceptFrameMessage({ source: win, origin: 'null', data: draft }, frame, 'other')).toEqual({ ok: false, reason: 'canvas' });
    expect(acceptFrameMessage({ source: win, origin: 'null', data: draft }, frame)).toEqual({ ok: false, reason: 'canvas' });
    expect(acceptFrameMessage({ source: {}, origin: 'null', data: draft }, frame, 'pick')).toEqual({ ok: false, reason: 'source' });
  });
});

describe('canvas bridge v1: the refusal log record (Canvas C6 P1)', () => {
  const draft = v1('draft', { canvasId: 'release-choices', answers: { pick: 'secret answer' }, summary: 'Pick: secret answer' });

  it('keeps the kind, size, reason and ids, never the payload', () => {
    const r = bridgeRefusal(draft, 'busy', 'release-choices', 's-20261005-abc')!;
    expect(r).toEqual({ kind: 'draft', bytes: messageBytes(draft), reason: 'busy', canvasId: 'release-choices', conversation: 's-20261005-abc', suppressed: 0 });
    expect(JSON.stringify(r)).not.toContain('secret');
    expect(Object.keys(r).sort()).toEqual(['bytes', 'canvasId', 'conversation', 'kind', 'reason', 'suppressed']);
  });

  it('an unknown kind is "other", an unserialisable message has no size, and bad ids are null', () => {
    expect(bridgeRefusal(v1('<img src=x>', {}), 'unknown-kind', 'Not An Id', '../etc', 3))
      .toMatchObject({ kind: 'other', reason: 'unknown-kind', canvasId: null, conversation: null, suppressed: 3 });
    expect(bridgeRefusal('just text', 'not-object', null, null)).toMatchObject({ kind: 'other', bytes: 11 });
    const cyclic: Record<string, unknown> = { __hc: 1, kind: 'state' };
    cyclic.self = cyclic;
    expect(bridgeRefusal(cyclic, 'too-large', null, null)).toMatchObject({ kind: 'state', bytes: null });
  });

  it('logs refusals of this frame only: not another window\'s message, not the person\'s own choice', () => {
    // Nor Hollow's own send failing after it took the draft (2026-10-15).
    for (const reason of ['source', 'origin', 'confirm', 'cancelled', 'not-sent', 'send-failed'] as const) expect(bridgeRefusal(draft, reason, null, null)).toBeNull();
    for (const reason of LOGGED_REASONS) expect(bridgeRefusal(draft, reason, null, null)?.reason).toBe(reason);
    expect(CHILD_KINDS).toEqual(['size', 'settled', 'error', 'navigate', 'open', 'blocked', 'source', 'state', 'draft', 'ask-invalid', 'design']);
  });

  it('is limited: thirty a minute, and the next one taken says how many were dropped', () => {
    const l = refusalLimiter();
    for (let i = 0; i < 30; i++) expect(l.take(1000)).toBe(0);
    expect(l.take(1001)).toBeNull();
    expect(l.take(60_999)).toBeNull();
    expect(l.take(61_000)).toBe(2);
    expect(l.take(61_001)).toBe(0);
    expect(l.take(10)).toBe(0); // a clock that steps back starts a new window
  });
});

describe('canvas bridge v1: the draft gate (Canvas C4 A2; question cards 2026-10-14)', () => {
  const open = { ask: true, locked: false, host: null, requested: true, pending: false, now: 10_000, last: null };

  it('takes a draft only as the reply to Hollow\'s own Send, from an unlocked, current ask-canvas', () => {
    expect(draftRefusal(open)).toBeNull();
    expect(draftRefusal({ ...open, ask: false })).toBe('not-ask');
    expect(draftRefusal({ ...open, locked: true })).toBe('locked');
    expect(draftRefusal({ ...open, host: 'not-current' })).toBe('not-current');
    expect(draftRefusal({ ...open, host: 'unavailable' })).toBe('unavailable');
    // A draft the frame posts on its own (no Send click, a stale nonce) never sends.
    expect(draftRefusal({ ...open, requested: false })).toBe('unrequested');
    expect(draftRefusal({ ...open, pending: true })).toBe('busy');
    expect(draftRefusal({ ...open, last: open.now - DRAFT_INTERVAL_MS + 1 })).toBe('busy');
    expect(draftRefusal({ ...open, last: open.now - DRAFT_INTERVAL_MS })).toBeNull();
    expect(DRAFT_INTERVAL_MS).toBe(1000);
  });

  it('a locked canvas is refused before anything else it could try', () => {
    expect(draftRefusal({ ...open, locked: true, requested: false, pending: true, host: 'not-current' })).toBe('locked');
    expect(draftRefusal({ ...open, ask: false, locked: true })).toBe('not-ask');
  });

  it('the host tells a locked frame its answer: answered {summary}, clipped', () => {
    expect(answeredMessage('- Plan: Friday')).toEqual({ __hc: 1, kind: 'answered', summary: '- Plan: Friday' });
    expect(answeredMessage('x'.repeat(LIMITS.answeredChars + 5)).summary).toHaveLength(LIMITS.answeredChars);
  });
});

describe('canvas bridge v1: state (child → host, Canvas C3)', () => {
  const state = (s: unknown, canvasId: unknown = 'release-status') => v1('state', { canvasId, state: s });

  it('accepts a plain JSON object or null for a canvas id, as a fresh copy', () => {
    const posted = { 'hc:hc-tabs:0': 'Table', 'hc:hc-table:0': ['Hours', 'd'], 'hc:hc-details:0': true, slider: 4.5, note: null, deep: { a: [1, { b: 'x' }] } };
    const r = parseChildMessage(state(posted));
    expect(r).toEqual({ ok: true, message: { kind: 'state', canvasId: 'release-status', state: posted } });
    expect(r.ok && r.message.kind === 'state' && r.message.state).not.toBe(posted);
    expect(parseChildMessage(state(null))).toEqual({ ok: true, message: { kind: 'state', canvasId: 'release-status', state: null } });
    expect(parseChildMessage(state({}))).toEqual({ ok: true, message: { kind: 'state', canvasId: 'release-status', state: {} } });
  });

  it('caps the state at 16 KB of UTF-8 JSON, counted in bytes', () => {
    const fill = (n: number, ch = 'x') => ({ k: ch.repeat(n) });
    const overhead = messageBytes(fill(0))!;
    expect(parseChildMessage(state(fill(LIMITS.stateBytes - overhead))).ok).toBe(true);
    expect(parseChildMessage(state(fill(LIMITS.stateBytes - overhead + 1)))).toEqual({ ok: false, reason: 'invalid' });
    // 6,000 three-byte characters are 18 KB, though only 6,000 UTF-16 units.
    expect(parseChildMessage(state(fill(6_000, '€')))).toEqual({ ok: false, reason: 'invalid' });
    expect(canvasState(fill(LIMITS.stateBytes))).toBeUndefined();
  });

  it('refuses extra or missing fields, a bad canvas id, and anything that is not plain JSON', () => {
    const bad = [
      v1('state', { canvasId: 'a', state: {}, extra: 1 }), v1('state', { canvasId: 'a' }), v1('state', { state: {} }),
      state({}, 'Release Status'), state({}, ''), state({}, 'a'.repeat(65)), state({}, 7), state({}, '../x'),
      state([1, 2]), state('tab-2'), state(3), state(true), state(undefined),
      state({ when: new Date() }), state({ m: new Map() }), state({ n: NaN }), state({ n: Infinity }), state({ b: new Uint8Array(2) }),
      state(Object.assign(Object.create({ inherited: 1 }), { a: 1 })),
    ];
    for (const data of bad) expect(parseChildMessage(data), JSON.stringify(data)?.slice(0, 80)).toEqual({ ok: false, reason: 'invalid' });
    // Nesting is bounded.
    let deep: unknown = 1;
    for (let i = 0; i < LIMITS.stateDepth + 1; i++) deep = { d: deep };
    expect(parseChildMessage(state(deep))).toEqual({ ok: false, reason: 'invalid' });
  });

  it('a "__proto__" key is copied as data, never as a prototype', () => {
    const posted = JSON.parse('{"__proto__": {"polluted": true}, "a": 1}');
    const r = parseChildMessage(state(posted));
    expect(r.ok).toBe(true);
    const copy = r.ok && r.message.kind === 'state' ? r.message.state as Record<string, unknown> : {};
    expect(Object.getPrototypeOf(copy)).toBe(Object.prototype);
    expect(Object.keys(copy)).toEqual(['__proto__', 'a']);
    expect(({} as Record<string, unknown>).polluted).toBeUndefined();
  });

  it('a frame may save only its own canvas\'s state', () => {
    const win = {};
    const frame = { contentWindow: win, isConnected: true };
    const ev = (data: unknown) => ({ source: win, origin: 'null', data });
    expect(acceptFrameMessage(ev(state({ a: 1 })), frame, 'release-status').ok).toBe(true);
    expect(acceptFrameMessage(ev(state({ a: 1 }, 'other-canvas')), frame, 'release-status')).toEqual({ ok: false, reason: 'canvas' });
    // A frame that is not a saved canvas (an html fence, a brief) saves nothing.
    expect(acceptFrameMessage(ev(state({ a: 1 })), frame)).toEqual({ ok: false, reason: 'canvas' });
    expect(acceptFrameMessage(ev(state({ a: 1 })), frame, null)).toEqual({ ok: false, reason: 'canvas' });
    // Other kinds don't care about the canvas id.
    expect(acceptFrameMessage(ev(v1('size', { h: 1 })), frame, 'release-status').ok).toBe(true);
    // The source check still comes first.
    expect(acceptFrameMessage({ source: {}, origin: 'null', data: state({}) }, frame, 'release-status')).toEqual({ ok: false, reason: 'source' });
  });
});

describe('canvas bridge v1: init (host → child, Canvas C3)', () => {
  const base = { canvasId: 'release-status', kit: 'kit@1', locale: 'en-US', reducedMotion: false, state: { 'hc:hc-tabs:0': 'Table' } };

  it('carries the canvas id, kit version, locale, reduced motion and saved state', () => {
    expect(initMessage(base)).toEqual({ __hc: 1, kind: 'init', canvasId: 'release-status', kit: 'kit@1', locale: 'en-US', reducedMotion: false, state: { 'hc:hc-tabs:0': 'Table' } });
    expect(initMessage({ ...base, reducedMotion: true }).reducedMotion).toBe(true);
    expect(messageBytes(initMessage(base))).toBeLessThan(MAX_MESSAGE_BYTES);
  });

  it('drops what the runtime would refuse: a frame without a canvas gets no state', () => {
    expect(initMessage({ ...base, canvasId: null })).toMatchObject({ canvasId: null, state: null });
    expect(initMessage({ ...base, canvasId: 'Not An Id' })).toMatchObject({ canvasId: null, state: null });
    expect(initMessage({ ...base, state: [1] }).state).toBeNull();
    expect(initMessage({ ...base, state: 'x' }).state).toBeNull();
    expect(initMessage({ ...base, state: { k: 'x'.repeat(LIMITS.stateBytes) } }).state).toBeNull();
    expect(initMessage({ ...base, state: undefined }).state).toBeNull();
    for (const locale of ['', 'e', 'en_US;x', 'x'.repeat(36), 'en US']) expect(initMessage({ ...base, locale }).locale).toBe('en');
    expect(initMessage({ ...base, kit: 'k'.repeat(80) }).kit).toHaveLength(LIMITS.kitChars);
  });

  it('the frame runtime re-checks init and posts state only for its canvas', () => {
    expect(RUNTIME_JS).toContain("d.kind !== 'init'");
    expect(RUNTIME_JS).toContain('/^[a-z0-9-]{1,64}$/');
    expect(RUNTIME_JS).toContain(`> ${LIMITS.stateBytes}`);
    expect(RUNTIME_JS).toContain("'hollow-init'");
    expect(RUNTIME_JS).toContain("defineProperty(window, 'hollowCanvas'");
    // Only one init per document: a later one is ignored.
    expect(RUNTIME_JS).toMatch(/if \(info\) return;/);
  });
});

describe('canvas bridge v1: sender', () => {
  const win = {};
  const frame = { contentWindow: win, isConnected: true };
  const msg = v1('size', { h: 10 });

  it('accepts only the frame\'s own window, from an opaque origin', () => {
    expect(acceptFrameMessage({ source: win, origin: 'null', data: msg }, frame).ok).toBe(true);
    expect(acceptFrameMessage({ source: {}, origin: 'null', data: msg }, frame)).toEqual({ ok: false, reason: 'source' });
    expect(acceptFrameMessage({ source: null, origin: 'null', data: msg }, frame)).toEqual({ ok: false, reason: 'source' });
    expect(acceptFrameMessage({ source: win, origin: 'tauri://localhost', data: msg }, frame)).toEqual({ ok: false, reason: 'origin' });
    expect(acceptFrameMessage({ source: win, origin: 'null', data: msg }, undefined)).toEqual({ ok: false, reason: 'source' });
    expect(acceptFrameMessage({ source: win, origin: 'null', data: msg }, { contentWindow: win, isConnected: false })).toEqual({ ok: false, reason: 'source' });
    expect(acceptFrameMessage({ source: null, origin: 'null', data: msg }, { contentWindow: null, isConnected: true })).toEqual({ ok: false, reason: 'source' });
  });
});

describe('canvas bridge v1: theme (host → child)', () => {
  it('carries the scheme and only safe --app-* tokens', () => {
    const m = themeMessage('dark', [
      ['--app-ink', '#f4efe8'], ['--app-display', "'Inter', system-ui, sans-serif"],
      ['--ink', '#000'], ['--app-x', 'red;background:url(https://x)'], ['--app-y', '</style>'],
      ['--app-z', 'a\\62 c'], ['--app-Upper', '#fff'], ['--app-empty', ''],
    ]);
    expect(m).toEqual({ __hc: BRIDGE_VERSION, kind: 'theme', scheme: 'dark', vars: { '--app-ink': '#f4efe8', '--app-display': "'Inter', system-ui, sans-serif" } });
    expect(messageBytes(m)).toBeLessThan(MAX_MESSAGE_BYTES);
  });

  it('caps the token count and value length', () => {
    const many = Array.from({ length: 80 }, (_, i) => [`--app-t${i}`, '#fff'] as [string, string]);
    expect(Object.keys(themeMessage('light', many).vars)).toHaveLength(LIMITS.themeVars);
    expect(isSafeThemeValue('x'.repeat(LIMITS.themeValueChars))).toBe(true);
    expect(isSafeThemeValue('x'.repeat(LIMITS.themeValueChars + 1))).toBe(false);
  });

  it('the frame runtime speaks v1 and re-checks the same theme rules', () => {
    expect(RUNTIME_JS).not.toMatch(/__jbDoc|__jbArtifact/);
    for (const kind of ['size', 'navigate', 'open', 'error', 'state']) expect(RUNTIME_JS).toContain(`post('${kind}'`);
    expect(RUNTIME_JS).toContain('/^--app-[a-z0-9-]{1,40}$/');
    expect(RUNTIME_JS).toContain(`names.length > ${LIMITS.themeVars}`);
    expect(RUNTIME_JS).toContain(`value.length <= ${LIMITS.themeValueChars}`);
    expect(RUNTIME_JS).toContain('if (e.source !== host) return;');
    // The host window is captured before any artifact script can shadow `parent`.
    expect(RUNTIME_JS.indexOf('var host = window.parent;')).toBeLessThan(RUNTIME_JS.indexOf('RTCPeerConnection'));
  });
});
