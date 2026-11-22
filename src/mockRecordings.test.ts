import { beforeEach, describe, expect, it, vi } from 'vitest';
import { recordingsFixture } from './mockRecordings';

describe('browser-mock recordings', () => {
  const storage = new Map<string, string>();
  beforeEach(() => {
    storage.clear();
    vi.stubGlobal('localStorage', { getItem: (k: string) => storage.get(k) ?? null, setItem: (k: string, v: string) => storage.set(k, v) });
  });
  it('lists recordings when the links file is fine', () => {
    const out = recordingsFixture('list_recordings', { days: 14 }) as { value: { state: string; recordings: unknown[] } };
    expect(out.value.state).toBe('ok');
    expect(out.value.recordings).toHaveLength(4);
  });
  it('rejects list_recordings like the backend does for a damaged links file (B·F08)', () => {
    storage.set('mockRecordingLinksDamaged', '1');
    expect(() => recordingsFixture('list_recordings', { days: 14 })).toThrow(/Recording link store was damaged.*moved to/);
  });
});
