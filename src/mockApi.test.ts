import { describe, expect, it, vi } from 'vitest';
import { CHAT_LIMIT_MAX } from './chatLimits';

describe('mockApi set_chat_limits ceilings', () => {
  it('accepts values up to CHAT_LIMIT_MAX (60 / 25 / 20) and rejects values above each ceiling', async () => {
    const store = new Map<string, string>();
    vi.stubGlobal('window', {});
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => { store.set(k, v); },
      removeItem: (k: string) => { store.delete(k); },
    });
    try {
      const { mockInvoke } = await import('./mockApi');
      const ok = await mockInvoke<typeof CHAT_LIMIT_MAX>('set_chat_limits', { limits: { ...CHAT_LIMIT_MAX } });
      expect(ok).toEqual({ chatsAtOnce: 60, chatsPerWorkspace: 25, subagentsPerChat: 20 });
      await expect(mockInvoke('set_chat_limits', { limits: { ...CHAT_LIMIT_MAX, chatsAtOnce: 61 } }))
        .rejects.toBe('Run 1 to 60 chats at once.');
      await expect(mockInvoke('set_chat_limits', { limits: { ...CHAT_LIMIT_MAX, chatsPerWorkspace: 26 } }))
        .rejects.toBe('Run 1 to 25 chats at once in one workspace.');
      await expect(mockInvoke('set_chat_limits', { limits: { ...CHAT_LIMIT_MAX, subagentsPerChat: 21 } }))
        .rejects.toBe('Run 1 to 20 subagents at once from one chat.');
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
