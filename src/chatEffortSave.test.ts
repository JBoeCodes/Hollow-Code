import { beforeEach, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  chatEffortSet: vi.fn(async (_key: string, _level: string | null) => undefined),
}));
vi.mock('./api', () => ({ api: { chatEffortSet: h.chatEffortSet } }));
import { cachedChatEffort, chatEffortWriteInFlight, putChatEffort, putChatEffortTracked, resetChatEffortCache } from './store/chatEffortStore';

const key = 's-effort-save';

beforeEach(() => {
  resetChatEffortCache();
  h.chatEffortSet.mockReset().mockResolvedValue(undefined);
});

it('returns false when the effort save rejects', async () => {
  h.chatEffortSet.mockRejectedValueOnce(new Error('store busy'));
  await expect(putChatEffort(key, 'high')).resolves.toBe(false);
  expect(chatEffortWriteInFlight(key)).toBe(false);
});

it('returns true when the effort save resolves', async () => {
  await expect(putChatEffort(key, 'high')).resolves.toBe(true);
  expect(h.chatEffortSet).toHaveBeenCalledWith(key, 'high');
  expect(chatEffortWriteInFlight(key)).toBe(false);
});

it('returns false when the effort command throws synchronously', async () => {
  h.chatEffortSet.mockImplementationOnce(() => { throw new Error('command unavailable'); });
  await expect(putChatEffort(key, null)).resolves.toBe(false);
  expect(chatEffortWriteInFlight(key)).toBe(false);
});

// R11b review (2026-11-15): a failed save is put back only while it is still
// the newest pick for its chat.
it('a rejected save is the latest pick, and the prior level can go back for its key', async () => {
  await putChatEffort(key, 'low');
  h.chatEffortSet.mockRejectedValueOnce(new Error('store busy'));
  await expect(putChatEffortTracked(key, 'high')).resolves.toBe('failed-latest');
  expect(cachedChatEffort(key)).toBe('high');
  await putChatEffort(key, 'low');
  expect(cachedChatEffort(key)).toBe('low');
  expect(h.chatEffortSet).toHaveBeenLastCalledWith(key, 'low');
});

it('two quick picks where the first fails: the newest pick wins, nothing is put back', async () => {
  let fail!: (e: Error) => void;
  h.chatEffortSet.mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
  const first = putChatEffortTracked(key, 'high');
  const second = putChatEffortTracked(key, 'max');
  fail(new Error('store busy'));
  await expect(first).resolves.toBe('failed-superseded');
  await expect(second).resolves.toBe('saved');
  expect(cachedChatEffort(key)).toBe('max');
  expect(h.chatEffortSet).toHaveBeenLastCalledWith(key, 'max');
});

it('a failure on another chat is not superseded by a pick here', async () => {
  h.chatEffortSet.mockRejectedValueOnce(new Error('store busy'));
  const other = putChatEffortTracked('s-other', 'high');
  void putChatEffortTracked(key, 'max');
  await expect(other).resolves.toBe('failed-latest');
});
