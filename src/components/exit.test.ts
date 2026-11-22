import { afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot, createSignal } from 'solid-js';
import { createExitPhase } from './exit';

// @ts-expect-error Solid's browser runtime has the public API but no subpath declaration.
vi.mock('solid-js', () => import('solid-js/dist/solid.js'));

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

describe('dialog exit motion', () => {
  for (const [app, system] of [[true, false], [false, true], [true, true]]) {
    it(`unmounts immediately with app=${app}, system=${system}`, () => {
      vi.useFakeTimers();
      vi.stubGlobal('document', { documentElement: { dataset: { motion: app ? 'reduced' : 'full' } } });
      vi.stubGlobal('window', { matchMedia: () => ({ matches: system }) });
      const state = createRoot(dispose => {
        const [open, setOpen] = createSignal(true);
        return { dispose, setOpen, phase: createExitPhase(open) };
      });
      try {
        state.setOpen(false);
        expect(state.phase()).toBe('closed');
        expect(vi.getTimerCount()).toBe(0);
      } finally { state.dispose(); }
    });
  }

  it('holds a full-motion exit and cancels it on reopening', () => {
    vi.useFakeTimers();
    vi.stubGlobal('document', { documentElement: { dataset: { motion: 'full' } } });
    vi.stubGlobal('window', { matchMedia: () => ({ matches: false }) });
    const state = createRoot(dispose => {
      const [open, setOpen] = createSignal(true);
      return { dispose, setOpen, phase: createExitPhase(open) };
    });
    try {
      state.setOpen(false); expect(state.phase()).toBe('closing');
      state.setOpen(true); vi.advanceTimersByTime(150);
      expect(state.phase()).toBe('open');
      state.setOpen(false); vi.advanceTimersByTime(150);
      expect(state.phase()).toBe('closed');
    } finally { state.dispose(); }
  });
});
