import { describe, it, expect } from 'vitest';
import GOLDEN from './fence.golden.json?raw';
import { parseSegments } from '../chatSegments';
import {
  CANVAS_ERROR_REASONS, CANVAS_FENCE, CANVAS_ID, MAX_BODY_BYTES, MAX_HEIGHT, MAX_ROOT_ATTRIBUTES,
  MAX_ROOT_TAG_BYTES, MAX_TITLE_CHARS, MIN_HEIGHT, decodeAttribute, parseCanvasFence, utf8Length,
  type CanvasParse, type CanvasRoot,
} from './fence';

type Body = string | { prefix: string; fill: string; times: number; suffix: string };
type Expect = { state: string; id?: string; title?: string; ask?: boolean; height?: number | null; inner?: string; code?: string };
type Case = { name: string; body: Body; closed: boolean; expect: Expect };
const golden = JSON.parse(GOLDEN) as {
  limits: Record<string, number>; reasons: Record<string, string>; cases: Case[];
};
const bodyOf = (b: Body) => typeof b === 'string' ? b : b.prefix + b.fill.repeat(b.times) + b.suffix;

/** The root in the golden's flat shape. */
function rootOf(root: CanvasRoot): Omit<Expect, 'state'> {
  return { ...root };
}

/** The parse result in the golden's flat shape. */
function flat(p: CanvasParse): Expect {
  switch (p.state) {
    case 'pending': return { state: 'pending' };
    case 'invalid': return { state: 'invalid', code: p.code };
    case 'drawing': return { state: 'drawing', ...rootOf(p.root) };
    case 'ready': return { state: 'ready', ...rootOf(p.root), inner: p.inner };
  }
}

describe('hollow-canvas fence parser (Canvas C3)', () => {
  it('limits and reasons match the shared golden', () => {
    expect(golden.limits).toEqual({
      bodyBytes: MAX_BODY_BYTES, rootTagBytes: MAX_ROOT_TAG_BYTES, rootAttributes: MAX_ROOT_ATTRIBUTES,
      titleChars: MAX_TITLE_CHARS, minHeight: MIN_HEIGHT, maxHeight: MAX_HEIGHT,
    });
    expect(golden.reasons).toEqual(CANVAS_ERROR_REASONS);
  });

  it('the id rule is 1–64 lowercase letters, digits or hyphens (the Rust rule too)', () => {
    expect(CANVAS_ID.source).toBe('^[a-z0-9-]{1,64}$');
    expect(CANVAS_ID.flags).toBe('');
  });

  for (const k of golden.cases) {
    it(k.name, () => {
      const got = parseCanvasFence(bodyOf(k.body), k.closed);
      const want = k.expect;
      const actual = flat(got);
      if (want.inner === undefined) delete actual.inner;
      expect(actual).toEqual(want);
      if (got.state === 'invalid') expect(got.reason).toBe(golden.reasons[got.code]);
    });
  }

  it('every case names a known reason, and every reason has a case', () => {
    const codes = new Set(golden.cases.filter(k => k.expect.state === 'invalid').map(k => k.expect.code));
    expect([...codes].sort()).toEqual(Object.keys(CANVAS_ERROR_REASONS).sort());
  });

  it('a well-formed canvas never looks broken while it streams', () => {
    // Every prefix of a ready body, with the fence still open, is pending until
    // the start tag closes and drawing (with the final root) after that.
    for (const k of golden.cases) {
      if (k.expect.state !== 'ready' || typeof k.body !== 'string') continue;
      const body = k.body;
      let drawing = false;
      for (let n = 0; n <= body.length; n++) {
        const p = parseCanvasFence(body.slice(0, n), false);
        expect(p.state === 'pending' || p.state === 'drawing', `${k.name} at ${n}: ${JSON.stringify(p)}`).toBe(true);
        if (p.state === 'drawing') {
          drawing = true;
          expect(p.root.id, `${k.name} at ${n}`).toBe(k.expect.id);
          expect(p.root.title, `${k.name} at ${n}`).toBe(k.expect.title);
        } else {
          expect(drawing, `${k.name} went back to pending at ${n}`).toBe(false);
        }
      }
      expect(drawing, k.name).toBe(true);
    }
  });

  it('counts UTF-8 bytes the way TextEncoder does', () => {
    for (const s of ['', 'abc', 'é', '日本', '🚀', 'a\ud800b', '\udc00', 'x🚀y'.repeat(3)]) {
      expect(utf8Length(s), JSON.stringify(s)).toBe(new TextEncoder().encode(s).length);
    }
  });

  it('decodes only the documented references', () => {
    expect(decodeAttribute('a &amp; b')).toBe('a & b');
    expect(decodeAttribute('&nbsp;&copy;&')).toBe('&nbsp;&copy;&');
    expect(decodeAttribute('&#x1F680;&#128640;')).toBe('🚀🚀');
    expect(decodeAttribute('&#;')).toBeNull();
    expect(decodeAttribute('&#x;')).toBeNull();
  });

  it('the chat renders a hollow-canvas fence as a canvas segment this parser reads', () => {
    // Wave 2 C1 wired the parser into the transcript (chatSegments → ChatCanvas).
    const segs = parseSegments(`Status:\n\n\`\`\`${CANVAS_FENCE}\n<hc-canvas id="a" title="A"></hc-canvas>\n\`\`\``);
    expect(segs.map(s => s.kind)).toEqual(['md', 'canvas']);
    expect(segs[1]).toMatchObject({ lang: CANVAS_FENCE, closed: true });
    const seg = segs[1];
    expect(seg.kind === 'canvas' && parseCanvasFence(seg.code, seg.closed).state).toBe('ready');
  });
});
