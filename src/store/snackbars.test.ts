import { afterEach, describe, expect, it, vi } from 'vitest';
import { dismissSnackbar, flashFooter, flashWithUndo, holdSnackbar, releaseSnackbar, setSnackbars, snackbars } from './snackbars';
import { appMessages, setAppMessages } from './appMessages';
import { setWindowAway } from './noticeTimers';

// P0-26 (XDIALOG-01): a failure toast stays long enough to read, whatever
// hold its caller asked for; a confirmation keeps the caller's hold.
describe('toast dwell', () => {
  afterEach(() => {
    vi.stubGlobal('window', { matchMedia: () => ({ matches: true }) });
    snackbars().forEach(bar => dismissSnackbar(bar.id));
    setSnackbars([]); setAppMessages([]); setWindowAway(false);
    vi.unstubAllGlobals(); vi.useRealTimers();
  });

  it('app-only reduction dismisses immediately without an exit timer', () => {
    vi.useFakeTimers();
    vi.stubGlobal('document', { documentElement: { dataset: { motion: 'reduced' } } });
    vi.stubGlobal('window', { matchMedia: () => ({ matches: false }) });
    const id = flashWithUndo({ message: 'Saved', ttlMs: 0 });
    dismissSnackbar(id);
    expect(snackbars().some(b => b.id === id)).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('a failure stays at least 7 s even when the caller asks for 1.5 s', () => {
    vi.useFakeTimers();
    flashFooter('× x', 1500);
    expect(snackbars()[0].ttlMs).toBeGreaterThanOrEqual(7000);
    expect(snackbars()[0].kind).toBe('error');
  });

  it('a failure with a longer hold keeps it', () => {
    vi.useFakeTimers();
    flashFooter('× Import failed', 8000);
    expect(snackbars()[0].ttlMs).toBe(8000);
  });

  it('a confirmation keeps the caller’s hold', () => {
    vi.useFakeTimers();
    flashFooter('Saved', 1600);
    expect(snackbars()[0].ttlMs).toBe(1600);
  });

  it('returning from focus leaves at least three seconds to act', () => {
    vi.useFakeTimers();
    vi.stubGlobal('window', { matchMedia: () => ({ matches: true }) });
    const id = flashWithUndo({ message: 'Saved', ttlMs: 1000 });
    vi.advanceTimersByTime(900);
    holdSnackbar(id);
    vi.advanceTimersByTime(20000);
    releaseSnackbar(id);
    vi.advanceTimersByTime(2999);
    expect(snackbars().some(b => b.id === id)).toBe(true);
    vi.advanceTimersByTime(1);
    expect(snackbars().some(b => b.id === id)).toBe(false);
    vi.unstubAllGlobals();
  });

  it('evicting a sixth transient bar cancels the evicted timer', () => {
    vi.useFakeTimers();
    for (let i = 0; i < 6; i++) flashWithUndo({ message: `Saved ${i}` });
    expect(snackbars()).toHaveLength(5);
    expect(vi.getTimerCount()).toBe(5);
    expect(appMessages()).toHaveLength(6);
    expect(appMessages()[5].message).toBe('Saved 0');
  });

  it('a sticky install result survives cap eviction without acknowledging it', () => {
    vi.useFakeTimers(); const ack = vi.fn();
    const id = flashWithUndo({ message: 'Install failed', kind: 'error', ttlMs: 0, onDismiss: ack });
    for (let i = 0; i < 6; i++) flashWithUndo({ message: `Saved ${i}` });
    expect(snackbars().some(b => b.id === id)).toBe(true);
    expect(ack).not.toHaveBeenCalled();
    vi.stubGlobal('window', { matchMedia: () => ({ matches: true }) });
    dismissSnackbar(id); expect(ack).toHaveBeenCalledOnce();
  });

  it('a failure created away keeps seven seconds after returning', () => {
    vi.useFakeTimers(); vi.stubGlobal('window', { matchMedia: () => ({ matches: true }) });
    setWindowAway(true); flashFooter('× Fixture failure', 1500);
    vi.advanceTimersByTime(20000); expect(snackbars()).toHaveLength(1);
    setWindowAway(false); vi.advanceTimersByTime(6999); expect(snackbars()).toHaveLength(1);
    vi.advanceTimersByTime(1); expect(snackbars()).toHaveLength(0);
  });

  it('a sixth notice cannot evict the keyboard-focused Undo', () => {
    vi.useFakeTimers();
    const first = flashWithUndo({ message: 'Keep my Undo', undo: vi.fn() });
    for (let i = 0; i < 4; i++) flashWithUndo({ message: `Other ${i}` });
    holdSnackbar(first, 'focus');
    flashWithUndo({ message: 'Newest' });
    expect(snackbars().some(b => b.id === first)).toBe(true);
    expect(snackbars()).toHaveLength(5);
    expect(snackbars().some(b => b.message === 'Other 0')).toBe(false);
  });

  it('all-held overflow trims only after the last interaction releases', () => {
    vi.useFakeTimers();
    const ids = Array.from({ length: 5 }, (_, i) => flashWithUndo({ message: `Held ${i}` }));
    for (const id of ids) { holdSnackbar(id, 'focus'); holdSnackbar(id, 'hover'); }
    flashWithUndo({ message: 'Newest' });
    expect(snackbars()).toHaveLength(6);
    releaseSnackbar(ids[0], 'focus');
    expect(snackbars().some(b => b.id === ids[0])).toBe(true);
    releaseSnackbar(ids[0], 'hover');
    expect(snackbars()).toHaveLength(5);
    expect(snackbars().some(b => b.id === ids[0])).toBe(false);
    expect(vi.getTimerCount()).toBe(1);
  });

  it('leaving rows do not evict another live notice', () => {
    vi.useFakeTimers(); vi.stubGlobal('window', { matchMedia: () => ({ matches: false }) });
    const ids = Array.from({ length: 5 }, (_, i) => flashWithUndo({ message: `Notice ${i}` }));
    dismissSnackbar(ids[4]);
    flashWithUndo({ message: 'Newest' });
    expect(snackbars().some(b => b.id === ids[0])).toBe(true);
    vi.advanceTimersByTime(170);
    expect(snackbars()).toHaveLength(5);
  });
});
