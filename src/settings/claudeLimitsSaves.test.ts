// Overnight 11-13 S3 (#1309 lows): Usage, Limits' saves and the agent tabs' figures.
import { describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({ claudeAccounts: vi.fn(), listConnectors: vi.fn() }));
vi.mock('../api', () => ({ api }));
vi.mock('../store', () => ({ flashFooter: vi.fn() }));
vi.mock('../connectorsLibrary', () => ({ loginOverrideLine: () => null, loginPinFrom: () => null }));

import { SAVE_WAIT_MS, createClaudeLimitsState, saveCheckbox } from './claudeLimitsState';
import { figuresFor } from './usageDetail';
import USAGE_PANEL from './UsagePanel.tsx?raw';
import LIMITS from './ClaudeLimits.tsx?raw';

describe('a failed checkbox save', () => {
  it('puts the box back to the saved value', async () => {
    const box = { checked: true };
    expect(await saveCheckbox(box, () => false, async () => false)).toBe(false);
    expect(box.checked).toBe(false);
  });

  it('leaves a saved change as it is', async () => {
    const box = { checked: true };
    expect(await saveCheckbox(box, () => false, async () => true)).toBe(true);
    expect(box.checked).toBe(true);
  });
});

describe('busy across a remount of Limits', () => {
  it('stays on in a new mount while an earlier mount is still saving', async () => {
    let finish!: () => void;
    const first = createClaudeLimitsState();
    const saving = first.run(() => new Promise(resolve => { finish = () => resolve({ accounts: [] } as never); }));
    const second = createClaudeLimitsState();
    expect(second.busy()).toBe(true);
    finish();
    await saving;
    expect(second.busy()).toBe(false);
    expect(first.busy()).toBe(false);
  });

  it('turns off after a failed save too', async () => {
    const s = createClaudeLimitsState();
    expect(await s.run(async () => { throw new Error('nope'); }, 'Could not')).toBe(false);
    expect(s.busy()).toBe(false);
  });
});

describe('the accounts view across a remount (review of #1369)', () => {
  it('shows a save from an earlier mount in the new one, and reads only after it settles', async () => {
    api.listConnectors.mockResolvedValue({ connectors: [] });
    const before = { accounts: [{ id: 'a', enabled: true }] } as never;
    const saved = { accounts: [{ id: 'a', enabled: false }] } as never;
    api.claudeAccounts.mockResolvedValue(before);
    let finish!: () => void;
    const first = createClaudeLimitsState();
    const saving = first.run(() => new Promise(resolve => { finish = () => resolve(saved); }));
    const second = createClaudeLimitsState();
    second.load();
    await Promise.resolve();
    expect(api.claudeAccounts).not.toHaveBeenCalled();
    api.claudeAccounts.mockResolvedValue(saved);
    finish();
    await saving;
    expect(second.view()).toBe(saved);
    await new Promise(r => setTimeout(r, 0));
    expect(api.claudeAccounts).toHaveBeenCalledOnce();
    expect(second.view()).toBe(saved);
    expect(first.view()).toBe(second.view());
  });

  it('reads anyway after a bounded wait when a save never settles, and clears the old error', async () => {
    vi.useFakeTimers();
    try {
      api.claudeAccounts.mockClear();
      const stuck = createClaudeLimitsState();
      void stuck.run(() => new Promise(() => {}));
      await stuck.run(async () => { throw new Error('old'); });
      expect(stuck.error()).toBe('old');
      const fresh = createClaudeLimitsState();
      fresh.load();
      expect(fresh.error()).toBe('');
      await vi.advanceTimersByTimeAsync(SAVE_WAIT_MS - 1);
      expect(api.claudeAccounts).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(api.claudeAccounts).toHaveBeenCalledOnce();
    } finally { vi.useRealTimers(); }
  });

  it('restores a failed In rotation box from the module view, not the row', () => {
    expect(LIMITS).toContain('saveCheckbox(box, () => s.view()?.accounts.find(x => x.id === a.id)?.enabled ?? a.enabled,');
  });
});

describe('an agent tab never shows another agent’s figures while it loads', () => {
  it('shows a summary only under the agent it was read for', () => {
    const claude = { agent: 'claude' };
    const all = { agent: null };
    expect(figuresFor(claude, 'codex')).toBeUndefined();
    expect(figuresFor(claude, 'claude')).toBe(claude);
    expect(figuresFor(all, null)).toBe(all);
    expect(figuresFor({}, null)).toEqual({});
    expect(figuresFor(all, 'claude')).toBeUndefined();
    expect(figuresFor(undefined, null)).toBeUndefined();
  });

  it('is what the Usage panel shows', () => {
    expect(USAGE_PANEL).toContain('const figures = () => figuresFor(latest(), agent());');
    expect(USAGE_PANEL).toContain('<Show when={figures()} fallback=');
  });
});
