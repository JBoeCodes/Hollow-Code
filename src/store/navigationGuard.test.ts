import { describe, expect, it, vi } from 'vitest';
import { guardNavigation, registerNavigationGuard } from './navigationGuard';

describe('navigation guards compose (Settings PR 4 review)', () => {
  it.each([['editor first', true], ['windows first', false]])('two dirty guards both hold a navigation: %s', async (_name, editorFirst) => {
    const editor = vi.fn(() => Promise.resolve(false));
    const windows = vi.fn(() => Promise.resolve(false));
    const offs = editorFirst ? [registerNavigationGuard(editor), registerNavigationGuard(windows)] : [registerNavigationGuard(windows), registerNavigationGuard(editor)];
    const go = vi.fn();
    await guardNavigation(go);
    expect(go).not.toHaveBeenCalled();
    expect(editor.mock.calls.length + windows.mock.calls.length).toBeGreaterThan(0);
    offs.forEach(off => off());
  });

  it('asks each in turn, stops at the first refusal, and goes when all allow', async () => {
    const a = vi.fn(() => Promise.resolve(true));
    const b = vi.fn(() => true);
    const offs = [registerNavigationGuard(a), registerNavigationGuard(b)];
    const go = vi.fn(() => 'went');
    expect(await guardNavigation(go)).toBe('went');
    expect(a).toHaveBeenCalledOnce();
    expect(b).toHaveBeenCalledOnce();
    offs.forEach(off => off());
  });

  it('disposing one keeps the other, and a clean navigation stays synchronous', () => {
    const keep = vi.fn(() => false);
    const offA = registerNavigationGuard(() => true);
    const offB = registerNavigationGuard(keep);
    offA();
    const go = vi.fn();
    guardNavigation(go);
    expect(keep).toHaveBeenCalled();
    expect(go).not.toHaveBeenCalled();
    offB();
    guardNavigation(go);
    expect(go).toHaveBeenCalledOnce();
  });
});
