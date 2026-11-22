import { describe, expect, it } from 'vitest';
import { FOCUSABLE, nextInRing } from './focusHold';
import HOLD from './focusHold.ts?raw';

describe('dialog focus ring', () => {
  it('moves forward and back around the ring, entering from outside at an end', () => {
    expect(nextInRing(3, 0, false)).toBe(1);
    expect(nextInRing(3, 2, false)).toBe(0);
    expect(nextInRing(3, 0, true)).toBe(2);
    expect(nextInRing(3, -1, false)).toBe(0);
    expect(nextInRing(3, -1, true)).toBe(2);
    expect(nextInRing(1, 0, false)).toBe(0);
    expect(nextInRing(0, -1, false)).toBe(-1);
  });

  it('never stops on a disabled control or a tabindex=-1 element', () => {
    expect(FOCUSABLE).toContain('button:not([disabled])');
    expect(FOCUSABLE).toContain('[tabindex]:not([tabindex="-1"])');
  });

  it('listens on the document and yields to a modal on top', () => {
    expect(HOLD).toContain("document.addEventListener('keydown', onKey, true)");
    expect(HOLD).toContain("closest('[aria-modal=\"true\"]')");
  });
});
