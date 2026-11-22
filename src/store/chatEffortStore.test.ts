import { beforeEach, expect, it, vi } from 'vitest';

// 2026-11-12: a chat's own effort lives in Rust. The window caches it, moves
// the old localStorage levels over once (never overwriting Rust's), and a
// refresh never undoes a write made after it began.
const h = vi.hoisted(() => {
  const values = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => values.get(k) ?? null,
    setItem: (k: string, v: string) => values.set(k, v),
    removeItem: (k: string) => values.delete(k),
    key: (i: number) => [...values.keys()][i] ?? null,
    get length() { return values.size; },
  });
  return {
    values,
    chatEffortSet: vi.fn(async (_k: string, _l: string | null) => undefined),
    chatEffortsImport: vi.fn(async (_e: [string, string][]) => 0),
    chatEffortsList: vi.fn(async (): Promise<[string, string][]> => []),
  };
});
vi.mock('../api', () => ({ api: { chatEffortSet: h.chatEffortSet, chatEffortsImport: h.chatEffortsImport, chatEffortsList: h.chatEffortsList } }));
import { cachedChatEffort, chatEffortWriteInFlight, loadChatEfforts, migrateLegacyEfforts, putChatEffort, refreshChatEfforts, resetChatEffortCache } from './chatEffortStore';

const A = 's-20261112-store001';
const B = 's-20261112-store002';

beforeEach(() => {
  h.values.clear();
  resetChatEffortCache();
  h.chatEffortSet.mockReset().mockResolvedValue(undefined);
  h.chatEffortsImport.mockReset().mockResolvedValue(0);
  h.chatEffortsList.mockReset().mockResolvedValue([]);
});

it('moves the old localStorage levels to Rust once, then drops them', async () => {
  h.values.set(`hollow.effort.${A}`, 'high');
  h.values.set(`hollow.effort.${B}`, 'ultracode'); // not a level: never sent
  h.values.set('hollow.ultracode.x', 'true');
  h.values.set('chatAgent', 'claude');
  await migrateLegacyEfforts();
  expect(h.chatEffortsImport).toHaveBeenCalledTimes(1);
  expect(h.chatEffortsImport).toHaveBeenCalledWith([[A, 'high']]);
  expect([...h.values.keys()].filter(k => k.startsWith('hollow.effort.'))).toEqual([]);
  expect(h.values.get('chatAgent')).toBe('claude');
  // A second start has nothing left to move.
  await migrateLegacyEfforts();
  expect(h.chatEffortsImport).toHaveBeenCalledTimes(1);
});

it('keeps the old values when Rust refuses the import, so the next start retries', async () => {
  h.values.set(`hollow.effort.${A}`, 'low');
  h.chatEffortsImport.mockRejectedValueOnce(new Error('store busy'));
  await migrateLegacyEfforts();
  expect(h.values.get(`hollow.effort.${A}`)).toBe('low');
  await migrateLegacyEfforts();
  expect(h.chatEffortsImport).toHaveBeenCalledTimes(2);
  expect(h.values.has(`hollow.effort.${A}`)).toBe(false);
});

it('loading reads Rust after the move: Rust wins over an old value', async () => {
  h.values.set(`hollow.effort.${A}`, 'low');
  // Rust already had a level for A (the import never overwrites it).
  h.chatEffortsList.mockResolvedValue([[A, 'max'], [B, 'medium']]);
  expect(await loadChatEfforts()).toBe(true);
  expect(cachedChatEffort(A)).toBe('max');
  expect(cachedChatEffort(B)).toBe('medium');
  expect(cachedChatEffort(null)).toBeNull();
});

it('a write goes to Rust, Default clears it, and a refresh drops what Rust no longer has', async () => {
  putChatEffort(A, 'high');
  expect(h.chatEffortSet).toHaveBeenLastCalledWith(A, 'high');
  expect(cachedChatEffort(A)).toBe('high');
  putChatEffort(A, null);
  expect(cachedChatEffort(A)).toBeNull();
  await vi.waitFor(() => expect(h.chatEffortSet).toHaveBeenLastCalledWith(A, null));
  h.chatEffortsList.mockResolvedValue([[B, 'low']]);
  await refreshChatEfforts();
  expect(cachedChatEffort(B)).toBe('low');
  h.chatEffortsList.mockResolvedValue([]);
  await refreshChatEfforts();
  expect(cachedChatEffort(B)).toBeNull();
});

it('a refresh that began before a write never undoes it', async () => {
  let release: (rows: [string, string][]) => void = () => {};
  h.chatEffortsList.mockReturnValueOnce(new Promise(resolve => { release = resolve; }));
  const refresh = refreshChatEfforts();
  putChatEffort(A, 'max');
  await Promise.resolve();
  release([[A, 'low']]);
  await refresh;
  expect(cachedChatEffort(A)).toBe('max');
});

it('a failed list read or an odd reply changes nothing', async () => {
  putChatEffort(A, 'high');
  h.chatEffortsList.mockRejectedValueOnce(new Error('no backend'));
  expect(await refreshChatEfforts()).toBe(false);
  h.chatEffortsList.mockResolvedValueOnce(null as never);
  expect(await refreshChatEfforts()).toBe(false);
  expect(cachedChatEffort(A)).toBe('high');
});

it('a window with no backend command keeps working', () => {
  h.chatEffortSet.mockImplementationOnce(() => { throw new TypeError('not a function'); });
  expect(() => putChatEffort(A, 'low')).not.toThrow();
  expect(cachedChatEffort(A)).toBe('low');
});

it('a write already on its way when a refresh began is not undone by the stale read', async () => {
  let settle: () => void = () => {};
  h.chatEffortSet.mockReturnValueOnce(new Promise<undefined>(resolve => { settle = () => resolve(undefined); }));
  let release: (rows: [string, string][]) => void = () => {};
  h.chatEffortsList.mockReturnValueOnce(new Promise(resolve => { release = resolve; }));
  putChatEffort(A, 'max');
  const refresh = refreshChatEfforts();
  // The write commits, then the read that saw the old state answers.
  settle();
  await Promise.resolve();
  await Promise.resolve();
  release([]);
  await refresh;
  expect(cachedChatEffort(A)).toBe('max');
});

it('saves for one chat reach Rust in order: the last pick wins there too (Q-R16)', async () => {
  // Rust commits each call when it answers; the first is slow.
  const committed: (string | null)[] = [];
  let slow: () => void = () => {};
  h.chatEffortSet.mockImplementationOnce((_k, level) => new Promise<undefined>(resolve => {
    slow = () => { committed.push(level); resolve(undefined); };
  }));
  h.chatEffortSet.mockImplementation(async (_k, level) => { committed.push(level); return undefined; });
  putChatEffort(A, 'high');
  putChatEffort(A, null);
  putChatEffort(B, 'low'); // another chat never waits behind A
  await Promise.resolve();
  expect(h.chatEffortSet).toHaveBeenCalledTimes(2);
  expect(h.chatEffortSet).toHaveBeenLastCalledWith(B, 'low');
  expect(cachedChatEffort(A)).toBeNull();
  expect(chatEffortWriteInFlight(A)).toBe(true);
  slow();
  await vi.waitFor(() => expect(chatEffortWriteInFlight(A)).toBe(false));
  expect(committed).toEqual(['low', 'high', null]);
  // A failed save does not stop the next one.
  h.chatEffortSet.mockRejectedValueOnce(new Error('store busy'));
  putChatEffort(A, 'max');
  putChatEffort(A, 'medium');
  await vi.waitFor(() => expect(chatEffortWriteInFlight(A)).toBe(false));
  expect(h.chatEffortSet).toHaveBeenLastCalledWith(A, 'medium');
});
