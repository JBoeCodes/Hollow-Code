import { afterEach, describe, expect, it, vi } from 'vitest';
import { MODAL_SEL, modalOpen } from './modalGuard';

// Overnight 10-11/12 P4: `modalOpen` runs three times a keystroke (leader
// keys, list keys, the page's own keys). Matching MODAL_SEL against every
// element cost ~4 ms a call with 5,000 tasks drawn; it reads live class
// collections instead, with the same answer.
type El = { classes: Set<string> };
function fakeDocument(elements: El[]) {
  const querySelector = vi.fn(() => null);
  const getElementsByClassName = vi.fn((cls: string) => {
    // Live, like the DOM's: read the elements each time it is indexed.
    const list = () => elements.filter(e => e.classes.has(cls)).map(e => ({ classList: { contains: (c: string) => e.classes.has(c) } }));
    return new Proxy({}, { get: (_, k) => (k === 'length' ? list().length : typeof k === 'string' && /^\d+$/.test(k) ? list()[+k] : undefined) });
  });
  return { querySelector, getElementsByClassName };
}
const el = (...c: string[]): El => ({ classes: new Set(c) });

afterEach(() => { vi.unstubAllGlobals(); });

describe('modalOpen', () => {
  it('finds every MODAL_SEL surface, and none that is closing', () => {
    const parts = MODAL_SEL.split(',').map(p => p.trim());
    for (const part of parts) {
      const cls = /^\.([\w-]+)/.exec(part)![1];
      const closing = part.includes(':not(.is-closing)');
      const open = el(cls);
      const els = [el('page'), open];
      vi.stubGlobal('document', fakeDocument(els));
      expect(modalOpen(), part).toBe(true);
      open.classes.add('is-closing');
      expect(modalOpen(), `${part} closing`).toBe(!closing);
      vi.unstubAllGlobals();
    }
  });

  it('is false with no surface, and never walks the whole document', () => {
    const doc = fakeDocument([el('page'), el('task-line'), el('modal-content'), el('is-closing')]);
    vi.stubGlobal('document', doc);
    expect(modalOpen()).toBe(false);
    expect(doc.querySelector).not.toHaveBeenCalled();
  });

  it('follows the document as it changes (live collections)', () => {
    const els = [el('page')];
    vi.stubGlobal('document', fakeDocument(els));
    expect(modalOpen()).toBe(false);
    els.push(el('palette-backdrop'));
    expect(modalOpen()).toBe(true);
    els.pop();
    expect(modalOpen()).toBe(false);
  });

  it('is false without a document (unit tests, workers)', () => {
    expect(modalOpen()).toBe(false);
  });
});
