import { describe, expect, it } from 'vitest';
import type { HomeRow, OfflineRow } from '../generated/ipc';
import { continueConfirm, offlineBadgeLabel, offlineNotice, offlineOf } from './chatOfflineWords';

const MINI = 'm-8d21c4be90f17a3e';
const K = 's-20261115-k0000001';
const O = 's-20261115-o0000001';
const at = '2026-11-15T10:00:00.000Z';
const home = (state: string): HomeRow => ({ key: K, state, peer: MINI, at, waiting: false });
const row = (state: string, note: string | null = null, offlineKey = O, partly = false): OfflineRow => ({ offlineKey, key: K, peer: MINI, state, note, partly });

describe('offline copies', () => {
  it('finds a chat’s own record and the newest copy of a moved chat', () => {
    const rows = [row('appended', null, 'o-1'), row('open', null, 'o-2')];
    expect(offlineOf(rows, K).copy?.offlineKey).toBe('o-2');
    expect(offlineOf(rows, 'o-1').own?.state).toBe('appended');
    expect(offlineOf(rows, 'nope')).toEqual({ own: null, copy: null });
  });

  it('offers Continue here offline on a moved chat, or its open copy', () => {
    const moved = offlineNotice(K, home('away'), [], 'Mac mini');
    expect(moved).toMatchObject({ canContinue: true, openCopy: null, text: null, badge: false });
    const open = offlineNotice(K, home('away'), [row('open')], 'Mac mini');
    expect(open).toMatchObject({ canContinue: false, openCopy: O });
    // A chat that never moved, or that lives here, offers nothing.
    expect(offlineNotice(K, undefined, [], 'Mac mini')).toBeNull();
    expect(offlineNotice(K, { ...home('leaving') }, [], 'Mac mini')).toBeNull();
  });

  it('says how the last copy synced on the moved chat', () => {
    expect(offlineNotice(K, home('away'), [row('appended')], 'Mac mini')?.text).toBe('The turns you added offline synced into it on Mac mini. It keeps going there.');
    expect(offlineNotice(K, home('away'), [row('forked')], 'Mac mini')?.text).toBe('Mac mini had gone on with it, so the turns you added offline became their own chat there.');
    expect(offlineNotice(K, home('away'), [row('kept', 'Too large.')], null)?.text).toBe('Your offline copy stayed on this Mac: Too large.');
  });

  it('badges the copy and says where it stands', () => {
    const waiting = offlineNotice(O, undefined, [row('open')], 'Mac mini');
    expect(waiting).toMatchObject({ kind: 'offline', badge: true, openOriginal: K });
    expect(waiting?.text).toContain('It syncs to Mac mini once Mac mini answers');
    // Part of a long stretch synced already.
    expect(offlineNotice(O, undefined, [row('open', null, O, true)], 'Mac mini')?.text).toBe('Part of its turns synced to Mac mini already. The rest follows when Mac mini answers next, and it runs on this Mac until then.');
    // While it is sent, the home notice speaks.
    expect(offlineNotice(O, { ...home('leaving'), key: O }, [row('open')], 'Mac mini')).toMatchObject({ text: null, badge: true });
    expect(offlineNotice(O, { ...home('away'), key: O }, [row('forked')], 'Mac mini')).toMatchObject({ kind: 'forked', badge: true, openOriginal: null });
    expect(offlineNotice(O, undefined, [row('kept', 'One of its offline turns is too large.')], null)?.text)
      .toBe('Kept on this Mac: One of its offline turns is too large. It is an ordinary chat here now.');
  });

  it('marks a fork another Mac’s offline sync made here', () => {
    const fork = offlineNotice(O, { ...home('arrived'), key: O }, [], 'MacBook', { forkKind: 'offline', parentKey: K });
    expect(fork).toMatchObject({ kind: 'fork-here', badge: true, openOriginal: K });
    expect(fork?.text).toBe('Continued offline on MacBook. The original went on here meanwhile, so these turns are their own chat.');
    expect(offlineNotice(O, { ...home('arrived'), key: O }, [], 'MacBook', { forkKind: null, parentKey: null })).toBeNull();
  });

  it('asks before continuing, saying where the turns go', () => {
    const c = continueConfirm('Plan the trip', 'Mac mini');
    expect(c.title).toBe('Continue “Plan the trip” here offline?');
    expect(c.body).toContain('into the chat if Mac mini didn’t go on with it');
    expect(offlineBadgeLabel(true)).toContain('its own chat');
  });

  it('uses no dashes as punctuation', () => {
    const all = [
      continueConfirm('t', null).body,
      ...['open', 'forked', 'kept', 'appended'].flatMap(s => [
        offlineNotice(O, undefined, [row(s)], null)?.text ?? '',
        offlineNotice(K, home('away'), [row(s)], null)?.text ?? '',
      ]),
    ].join(' ');
    expect(all).not.toMatch(/[\u2013\u2014]/);
  });
});
