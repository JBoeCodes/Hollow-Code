import { describe, expect, it } from 'vitest';
import { MIN_GAP_MS, badgeCount, bannerFor, chatEvents, shouldAnnounce, type QuietContext } from './threadNotifications';
import type { SessionState } from './sessionIndicator';

// En and em dash, built from code points so this file holds neither.
const DASHES = new RegExp(`[${String.fromCharCode(0x2013)}${String.fromCharCode(0x2014)}]`);
const m = (o: Record<string, SessionState>) => new Map(Object.entries(o)) as ReadonlyMap<string, SessionState>;
const ctx = (over: Partial<QuietContext> = {}): QuietContext => ({
  enabled: true, focused: false, onScreenKey: null, lastShown: new Map(), now: 1_000_000, isSubagent: () => false, ...over,
});

describe('chatEvents', () => {
  it('announces nothing on the first snapshot', () => {
    expect(chatEvents(null, m({ a: 'running', b: 'waiting' }))).toEqual([]);
  });
  it('finds finishes, needs-you and failures, and nothing else', () => {
    const prev = m({ a: 'running', b: 'running', c: 'queued', d: 'idle', e: 'running', f: 'paused' });
    const next = m({ a: 'idle', b: 'waiting', c: 'failed', d: 'running', e: 'interrupted', f: 'idle' });
    expect(chatEvents(prev, next)).toEqual([
      { key: 'a', kind: 'finished' }, { key: 'b', kind: 'needs-you' }, { key: 'c', kind: 'failed' },
    ]);
  });
  it('a chat leaving the map counts as idle, and an unchanged state is silent', () => {
    expect(chatEvents(m({ a: 'running', b: 'waiting' }), m({ b: 'waiting' }))).toEqual([{ key: 'a', kind: 'finished' }]);
  });
  it('a waiting chat answered back to running is not an event; a failure with no turn is not either', () => {
    expect(chatEvents(m({ a: 'waiting', b: 'idle' }), m({ a: 'running', b: 'failed' }))).toEqual([]);
  });
});

describe('shouldAnnounce quiet rules', () => {
  const e = { key: 'a', kind: 'finished' as const };
  it('is off with the switch off', () => expect(shouldAnnounce(e, ctx({ enabled: false }))).toBe(false));
  it('never announces a subagent', () => expect(shouldAnnounce(e, ctx({ isSubagent: () => true }))).toBe(false));
  it('skips the chat on screen while Hollow is focused, not when it is in the background', () => {
    expect(shouldAnnounce(e, ctx({ focused: true, onScreenKey: 'a' }))).toBe(false);
    expect(shouldAnnounce(e, ctx({ focused: false, onScreenKey: 'a' }))).toBe(true);
    expect(shouldAnnounce(e, ctx({ focused: true, onScreenKey: 'b' }))).toBe(true);
  });
  it('holds one chat to one banner per gap', () => {
    const lastShown = new Map([['a', 1_000_000 - (MIN_GAP_MS - 1)]]);
    expect(shouldAnnounce(e, ctx({ lastShown }))).toBe(false);
    expect(shouldAnnounce(e, ctx({ lastShown: new Map([['a', 1_000_000 - MIN_GAP_MS]]) }))).toBe(true);
    expect(shouldAnnounce({ key: 'z', kind: 'finished' }, ctx({ lastShown }))).toBe(true);
  });
});

describe('bannerFor', () => {
  const titles: Record<string, string> = { a: 'Quarterly numbers', b: 'Fix the login', c: 'Plan', d: '' };
  const t = (k: string) => titles[k] ?? '';
  it('names the chat for one event, in plain words', () => {
    expect(bannerFor([{ key: 'a', kind: 'finished' }], t)).toEqual({ title: 'Quarterly numbers', body: 'Finished.' });
    expect(bannerFor([{ key: 'b', kind: 'needs-you' }], t)?.body).toBe('Waiting on you.');
    expect(bannerFor([{ key: 'c', kind: 'failed' }], t)?.body).toBe('Stopped with an error.');
    expect(bannerFor([{ key: 'd', kind: 'finished' }], t)?.title).toBe('A chat');
  });
  it('turns a burst into one banner with counts', () => {
    const b = bannerFor([{ key: 'a', kind: 'finished' }, { key: 'b', kind: 'needs-you' }, { key: 'c', kind: 'finished' }], t)!;
    expect(b.title).toBe('3 chats: 1 needs you, 2 finished');
    expect(b.body).toBe('Quarterly numbers, Fix the login and 1 more.');
    expect(bannerFor([{ key: 'a', kind: 'finished' }, { key: 'c', kind: 'finished' }], t)).toEqual({ title: '2 chats: 2 finished', body: 'Quarterly numbers, Plan.' });
  });
  it('clips a long title and has no dashes', () => {
    const long = bannerFor([{ key: 'x', kind: 'finished' }], () => 'w'.repeat(200))!;
    expect(long.title.length).toBeLessThanOrEqual(60);
    for (const s of [long.title, long.body]) expect(s).not.toMatch(DASHES);
    expect(bannerFor([], t)).toBeNull();
  });
});

describe('badgeCount', () => {
  it('counts chats waiting on you or failed, plus finished ones not opened, once each', () => {
    const states = m({ a: 'waiting', b: 'failed', c: 'running', d: 'idle', e: 'idle' });
    expect(badgeCount(states, new Set(['d', 'a']))).toBe(3);
    expect(badgeCount(states, new Set())).toBe(2);
  });
  it('leaves subagents out and caps at 99', () => {
    expect(badgeCount(m({ a: 'waiting', b: 'waiting' }), new Set(), k => k === 'b')).toBe(1);
    const many = Object.fromEntries(Array.from({ length: 150 }, (_, i) => [`k${i}`, 'waiting' as SessionState]));
    expect(badgeCount(m(many), new Set())).toBe(99);
  });
});
