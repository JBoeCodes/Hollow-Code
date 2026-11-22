import { describe, expect, it } from 'vitest';
import type { ChatMoveResult, HomeRow, PeerMachine, PeerMachinesView } from '../generated/ipc';
import { HINT_MAX, forgetBody, homeNotice, macName, moveConfirm, moveNotice, moveRunnerName, movedLabel, releaseConfirm, sleepHint } from './chatMoveWords';

const MINI = 'm-8d21c4be90f17a3e';
const machine = (id: string, name: string): PeerMachine => ({
  machineId: id, slug: null, name, sshHosts: ['runner'], remotePort: 47680, pairedAt: '2026-11-15T00:00:00Z',
  lastSeenAt: null, lastVersion: null, state: 'online', message: null, checkedAt: null, alias: null, moves: true,
});
const view = (moveTo: string | null): PeerMachinesView => ({ thisMac: null, machines: [machine(MINI, 'Mac mini')], suggestedAlias: null, moveTo, sleepTo: null });
const home = (key: string, state: string, waiting = false, peer = MINI): HomeRow => ({ key, state, peer, at: '2026-11-15T10:00:00Z', waiting });
const answer = (outcome: ChatMoveResult['outcome'], message: string, more: Partial<ChatMoveResult> = {}): ChatMoveResult =>
  ({ key: 's-1', outcome, to: 'Mac mini', message, continued: false, ...more });
const DASHES = new RegExp('[\\u2013\\u2014]');

describe('chat move words', () => {
  it('offers Move to runner only for the Mac the backend picks', () => {
    expect(moveRunnerName(view(MINI))).toBe('Mac mini');
    expect(moveRunnerName(view(null))).toBeNull();
    expect(moveRunnerName(view('m-gone'))).toBeNull();
    expect(moveRunnerName(null)).toBeNull();
    expect(macName(view(null), 'm-gone')).toBeNull();
  });

  it('says what a move did, plainly, and offers Open only after a move', () => {
    expect(moveNotice(answer('moved', 'Moved to Mac mini; it keeps going there.'))).toEqual({ text: 'Moved to Mac mini; it keeps going there.', failed: false, open: true });
    expect(moveNotice(answer('moved', 'Moved.', { continued: true })).text).toBe('Moved. Its turn continues there.');
    expect(moveNotice(answer('waiting', 'Mac mini hasn’t answered.'))).toMatchObject({ failed: false, open: false });
    expect(moveNotice(answer('refused', 'This chat is the COO’s.'))).toMatchObject({ failed: true, open: false });
    expect(moveNotice(answer('busy', 'Try again in a moment.'))).toMatchObject({ failed: false, open: false });
    const ask = moveConfirm('Trip plan', 'Mac mini');
    expect(ask.title).toBe('Move “Trip plan” to Mac mini?');
    expect(ask.body).toContain('logins pay');
  });

  it('gives a moving or waiting chat Keep it here and Release here, a moved one Open', () => {
    expect(homeNotice(undefined, 'Mac mini')).toBeNull();
    expect(homeNotice(home('s-1', 'leaving'), 'Mac mini')).toMatchObject({ kind: 'moving', keep: true, open: false });
    expect(homeNotice(home('s-1', 'leaving', true), 'Mac mini')).toMatchObject({ kind: 'waiting', keep: true, open: false });
    expect(homeNotice(home('s-1', 'away'), 'Mac mini')).toMatchObject({ kind: 'moved', keep: false, open: true });
    const forgot = homeNotice(home('s-1', 'away'), null)!;
    expect(forgot).toMatchObject({ kind: 'forgotten', keep: false, open: false });
    expect(forgot.text).toContain('a Mac you forgot');
    expect(homeNotice(home('s-1', 'arrived'), null)!.text).toBe('Moved here from another Mac.');
    expect(homeNotice(home('s-1', 'later-state'), 'Mac mini')).toBeNull();
    const release = releaseConfirm('Mac mini', 'Plan the trip');
    expect(release.title).toBe('Release “Plan the trip” here?');
    expect(release.body).toContain('Mac mini keeps its copy');
    expect(movedLabel(null)).toBe('Moved to a Mac you forgot');
    expect(movedLabel('Mac mini')).toBe('Moved to Mac mini');
  });

  it('says which chats Forget releases and which stay there', () => {
    const titles: Record<string, string> = { a: 'Alpha', b: 'Beta', c: 'Gamma', d: 'Delta', e: 'Echo' };
    const title = (k: string) => titles[k];
    const plain = forgetBody('Mac mini', MINI, [], title);
    expect(plain).toBe('This Mac stops opening Mac mini’s chats and deletes its key. Hollow unpairs there too when it answers. Nothing on Mac mini is deleted.');
    const one = forgetBody('Mac mini', MINI, [home('a', 'leaving'), home('x', 'leaving', false, 'm-other')], title);
    expect(one).toContain('One chat is still moving there: “Alpha”. It runs on this Mac again; Mac mini may keep a copy if it took it.');
    const many = forgetBody('Mac mini', MINI, ['a', 'b', 'c', 'd'].map(k => home(k, 'leaving', k === 'b')).concat(home('e', 'away'), home('f', 'arrived')), title);
    expect(many).toContain('4 chats are still moving there: “Alpha”, “Beta”, “Gamma” and 1 more.');
    expect(many).toContain('One chat that moved there stays there, shown here as moved to a Mac you forgot.');
    expect(forgetBody('Mac mini', MINI, [home('a', 'leaving'), home('b', 'leaving')], title)).toContain('“Alpha” and “Beta”');
    expect(forgetBody('Mac mini', MINI, [home('d', 'away'), home('e', 'away')], title)).toContain('2 chats that moved there stay there');
  });

  it('keeps Settings’ hint short and every sentence free of long dashes', () => {
    // The hint follows the switch, names the Mac, and cuts a long name to fit.
    expect(sleepHint('Mac mini', false)).toBe('Off. When on, running chats go to Mac mini, and its logins pay for them.');
    expect(sleepHint('Mac mini', true)).toBe('On: running chats go to Mac mini when this Mac sleeps; its logins pay.');
    expect(sleepHint(null, true)).toBe('On, but no paired Mac is the runner now, so nothing moves.');
    const long = 'The very long named build runner Mac in the hall closet';
    for (const on of [false, true]) {
      for (const runner of ['Mac mini', long, null]) expect(sleepHint(runner, on).length).toBeLessThanOrEqual(HINT_MAX);
      expect(sleepHint(long, on)).toContain('The very long');
      expect(sleepHint(long, on)).toContain('…');
    }
    const all = [
      sleepHint('Mac mini', false), sleepHint('Mac mini', true), sleepHint(null, false), moveConfirm('T', 'Mac mini').body, releaseConfirm(null, 'T').body,
      ...['leaving', 'away', 'arrived'].flatMap(s => [homeNotice(home('s', s), 'Mac mini')!.text, homeNotice(home('s', s, true), null)!.text]),
    ];
    for (const text of all) expect(text).not.toMatch(DASHES);
  });
});
