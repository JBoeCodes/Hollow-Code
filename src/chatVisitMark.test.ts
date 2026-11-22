import { beforeEach, describe, expect, it, vi } from 'vitest';
import { VISITS_KEPT, recordVisit, visitedSeen, watermarkAt, watermarkText } from './chatVisitMark';

const storage = new Map<string, string>();
beforeEach(() => {
  storage.clear();
  vi.stubGlobal('localStorage', { getItem: (k: string) => storage.get(k) ?? null, setItem: (k: string, v: string) => { storage.set(k, v); }, removeItem: (k: string) => { storage.delete(k); } });
});

describe('watermarkAt', () => {
  it('marks the first turn after what was seen', () => {
    expect(watermarkAt(10, 14)).toBe(10);
    expect(watermarkAt(13, 14)).toBe(13);
  });
  it('marks nothing on a first visit, when all was seen, or when the thread got shorter', () => {
    expect(watermarkAt(null, 14)).toBe(null);
    expect(watermarkAt(14, 14)).toBe(null);
    expect(watermarkAt(20, 14)).toBe(null);
    expect(watermarkAt(0, 14)).toBe(null);
  });
});

describe('visits', () => {
  it('remembers how far a conversation was read, never less', () => {
    expect(visitedSeen('a')).toBe(null);
    recordVisit('a', 10, 1);
    recordVisit('a', 6, 2);
    expect(visitedSeen('a')).toBe(10);
    recordVisit('a', 12, 3);
    expect(visitedSeen('a')).toBe(12);
  });
  it('ignores a record that is not a turn count', () => {
    recordVisit('a', 0);
    recordVisit('a', -3);
    recordVisit('a', 2.5);
    recordVisit('', 4);
    expect(visitedSeen('a')).toBe(null);
  });
  it('keeps the latest 200 conversations', () => {
    for (let i = 0; i < VISITS_KEPT + 5; i++) recordVisit(`c${i}`, 3, i + 1);
    expect(visitedSeen('c0')).toBe(null);
    expect(visitedSeen('c4')).toBe(null);
    expect(visitedSeen('c5')).toBe(3);
    expect(visitedSeen(`c${VISITS_KEPT + 4}`)).toBe(3);
  });
  it('survives a damaged store', () => {
    localStorage.setItem('hollow.chatVisits.v1', '{not json');
    expect(visitedSeen('a')).toBe(null);
    recordVisit('a', 5);
    expect(visitedSeen('a')).toBe(5);
    localStorage.setItem('hollow.chatVisits.v1', JSON.stringify({ a: { seen: 'x', at: 1 }, b: { seen: 4, at: 2 } }));
    expect(visitedSeen('a')).toBe(null);
    expect(visitedSeen('b')).toBe(4);
  });
});

describe('watermarkText', () => {
  it('counts in the singular and the plural', () => {
    expect(watermarkText(1)).toBe('1 new message since you were here');
    expect(watermarkText(6)).toBe('6 new messages since you were here');
  });
});

describe('the parsed visits are reused', () => {
  it('re-reads when the stored text changes and never lets a failed save change what is read', () => {
    recordVisit('a', 3);
    expect(visitedSeen('a')).toBe(3);
    storage.set('hollow.chatVisits.v1', JSON.stringify({ a: { seen: 9, at: 1 } }));
    expect(visitedSeen('a')).toBe(9);
    vi.stubGlobal('localStorage', { getItem: (k: string) => storage.get(k) ?? null, setItem: () => { throw new Error('quota'); }, removeItem: () => {} });
    recordVisit('a', 20);
    expect(visitedSeen('a')).toBe(9);
  });
});
