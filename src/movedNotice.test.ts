import { describe, expect, it } from 'vitest';
import { movedNotice, recordMove } from './movedNotice';

const memory = () => {
  const map = new Map<string, string>();
  return { getItem: (k: string) => map.get(k) ?? null, setItem: (k: string, v: string) => void map.set(k, v) };
};

describe('moved notice', () => {
  it('shows until the conversation has its next turn, then forgets the move', () => {
    const store = memory();
    expect(movedNotice(store, 's-1', 2)).toBeNull();
    recordMove(store, 's-1', 'Memo', 2);
    expect(movedNotice(store, 's-1', 2)).toBe("Moved to Memo. The next turn runs in Memo's folder with its brief and sources; the conversation so far comes along as written context.");
    expect(movedNotice(store, 's-2', 2)).toBeNull();
    expect(movedNotice(store, 's-1', 3)).toBeNull();
    expect(movedNotice(store, 's-1', 2)).toBeNull();
  });
  it('a second move replaces the first', () => {
    const store = memory();
    recordMove(store, 's-1', 'Memo', 2);
    recordMove(store, 's-1', 'Launch', 2);
    expect(movedNotice(store, 's-1', 2)).toMatch(/^Moved to Launch\./);
  });
});
