import { describe, expect, it } from 'vitest';
import mainAgentSource from '../src-tauri/src/main_agent.rs?raw';
import { todaySpendLine } from './mainAgent';

describe('COO spend in one line (P1-07)', () => {
  it('totals today: its own spend against the hard stop, plus what you asked for', () => {
    expect(todaySpendLine({ spentTodayUsd: 12.4, askedTodayUsd: 30, dailyCapUsd: 100 }))
      .toBe('Today $42.40 · $12.40 on its own of $100 · $30.00 you asked for');
    expect(todaySpendLine({ spentTodayUsd: 0, askedTodayUsd: 0, dailyCapUsd: 12.5 }))
      .toBe('Today $0.00 · $0.00 on its own of $12.50 · $0.00 you asked for');
    // At the cap the stop words sit with its own spend: chats you asked for are never stopped.
    expect(todaySpendLine({ spentTodayUsd: 100, askedTodayUsd: 30, dailyCapUsd: 100 }, 'stopped until tomorrow'))
      .toBe('Today $130.00 · $100.00 on its own of $100 (stopped until tomorrow) · $30.00 you asked for');
  });
});

import { actLabel, isPulseOk, mainTurnKind, skippedWakeWords, nightLine, isWakeMessage, localDay, orderChildren, reopenableMainSessions, todaysMainSession, wakeText, type MainAgentChild } from './mainAgent';

it('labels every action the COO may perform on its own', () => {
  const safe = mainAgentSource.match(/const SAFE:.*?=\s*&\[([\s\S]*?)\];/)?.[1];
  expect(safe).toBeTruthy();
  const ids = [...safe!.matchAll(/"([^"]+)"/g)].map(match => match[1]);
  expect(ids.length).toBeGreaterThan(10);
  for (const id of ids) expect(actLabel(id), id).not.toBe(id);
});

it('classifies host wakes and quiet check-ins without relabelling quoted assistant text', () => {
  expect(mainTurnKind({ role: 'user', text: '⟲ Hollow: a child finished.' })).toBe('wake');
  expect(mainTurnKind({ role: 'assistant', text: ' PULSE_OK\n' })).toBe('check-in');
  expect(mainTurnKind({ role: 'user', text: 'PULSE_OK' })).toBe('message');
  expect(mainTurnKind({ role: 'assistant', text: '⟲ Hollow: a quote' })).toBe('message');
  expect(mainTurnKind({ role: 'system', text: 'Stopped' })).toBe('message');
});

const at = (y: number, m: number, d: number, h = 12) => new Date(y, m - 1, d, h).toISOString();

describe('the COO\'s daily thread', () => {
  it('reads local calendar days', () => {
    expect(localDay(at(2026, 10, 7, 23))).toBe('2026-10-07');
    expect(localDay('not a date')).toBe('');
    expect(localDay(null)).toBe('');
  });

  it('opens the newest conversation started today', () => {
    const sessions = [
      { key: 'yesterday', createdAt: at(2026, 10, 6, 9) },
      { key: 'morning', createdAt: at(2026, 10, 7, 8) },
      { key: 'noon', createdAt: at(2026, 10, 7, 12) },
    ];
    expect(todaysMainSession(sessions, new Date(2026, 9, 7, 15))).toBe('noon');
  });

  it('starts a fresh one on a new day', () => {
    const sessions = [{ key: 'yesterday', createdAt: at(2026, 10, 6, 9) }];
    expect(todaysMainSession(sessions, new Date(2026, 9, 7, 8))).toBeNull();
    expect(todaysMainSession([], new Date(2026, 9, 7, 8))).toBeNull();
  });

  it('sorts an unreadable start time last', () => {
    const sessions = [
      { key: 'broken', createdAt: '' },
      { key: 'today', createdAt: at(2026, 10, 7, 9) },
    ];
    expect(todaysMainSession(sessions, new Date(2026, 9, 7, 15))).toBe('today');
  });

  it('keeps a thread that is still working past midnight', () => {
    const sessions = [
      { key: 'older', createdAt: at(2026, 10, 5, 9) },
      { key: 'late', createdAt: at(2026, 10, 6, 23) },
    ];
    expect(todaysMainSession(sessions, new Date(2026, 9, 7, 0, 30), k => k === 'late')).toBe('late');
    expect(todaysMainSession(sessions, new Date(2026, 9, 7, 0, 30), () => false)).toBeNull();
  });

  it('never opens an older conversation than the newest (one Main conversation, 11-07)', () => {
    // An older thread still working is an earlier chat of the one chain: the
    // button opens the newest, or today's fresh thread, never the older one.
    const sessions = [
      { key: 'older-working', createdAt: at(2026, 10, 5, 9) },
      { key: 'newest', createdAt: at(2026, 10, 6, 9) },
    ];
    expect(todaysMainSession(sessions, new Date(2026, 9, 7, 9), k => k === 'older-working')).toBeNull();
    expect(todaysMainSession(sessions, new Date(2026, 9, 7, 9), k => k === 'newest')).toBe('newest');
  });

  it('reopens the conversation last seen only when the COO can open it', () => {
    // 2026-10-08: the orb picked a key the Main agent didn't list and wasn't
    // holding, and opening failed with "no longer available in this workspace".
    const MAIN = '/vault/workspaces/main';
    const listed = [{ key: 'today', createdAt: at(2026, 10, 7, 9) }];
    const now = new Date(2026, 9, 7, 15);
    const seen = { key: 'fresh', at: at(2026, 10, 7, 14) };
    const pick = (cwd: string | undefined) =>
      todaysMainSession(reopenableMainSessions(listed, seen, now, () => cwd, MAIN), now);
    expect(pick(MAIN)).toBe('fresh');
    expect(pick(undefined)).toBe('today');
    expect(pick('/vault/workspaces/other')).toBe('today');
    // Not new today, or already listed: the list alone decides.
    const yesterday = { key: 'old', at: at(2026, 10, 6, 14) };
    expect(reopenableMainSessions(listed, yesterday, now, () => MAIN, MAIN)).toEqual(listed);
    expect(reopenableMainSessions(listed, { key: 'today', at: at(2026, 10, 7, 14) }, now, () => MAIN, MAIN)).toEqual(listed);
    expect(reopenableMainSessions(listed, null, now, () => MAIN, MAIN)).toEqual(listed);
  });
});


describe('the COO\'s mission control', () => {
  const child = (over: Partial<MainAgentChild>): MainAgentChild => ({
    parentKey: 'p', childSession: 's', title: 't', workspaceId: 'w', workspaceTitle: 'W', agent: 'claude',
    status: 'done', live: false, queueId: 1, requestedAt: null, finishedAt: null, error: null, reported: true, costUsd: null, byHollow: false, successors: [], endQueueId: 1, ...over,
  });

  it('lists live chats first, then the newest', () => {
    const list = orderChildren([
      child({ childSession: 'old', requestedAt: '2026-10-07T08:00:00Z' }),
      child({ childSession: 'new', requestedAt: '2026-10-07T10:00:00Z' }),
      child({ childSession: 'live', live: true, status: 'running', requestedAt: '2026-10-07T07:00:00Z' }),
    ]);
    expect(list.map(c => c.childSession)).toEqual(['live', 'new', 'old']);
  });

  it('recognises a wake message as Hollow\'s', () => {
    const text = '⟲ Hollow: a chat you started has finished.';
    expect(isWakeMessage(text)).toBe(true);
    expect(wakeText(text)).toBe('a chat you started has finished.');
    expect(isWakeMessage('Hollow: hi')).toBe(false);
    expect(isWakeMessage(null)).toBe(false);
  });
});

describe('the COO\'s day', () => {
  it('a held-back wake with its reason (overnight 10-11 A1)', () => {
    const skip = { at: new Date(2026, 9, 10, 17, 51).toISOString(), reason: "today's cap is reached", sessions: ['s-a', 's-b'] };
    expect(skippedWakeWords(skip)).toMatch(/^Didn't report 2 ended chats at 5:51.*PM: today's cap is reached$/);
    expect(skippedWakeWords({ ...skip, sessions: ['s-a'] })).toContain('an ended chat');
  });

  it('names what it did on its own, and a quiet check-in', () => {
    expect(actLabel('sessions.start')).toBe('Started a chat');
    expect(actLabel('something.else')).toBe('something.else');
    expect(isPulseOk(' PULSE_OK\n')).toBe(true);
    expect(isPulseOk('PULSE_OK, but also')).toBe(false);
  });
});

describe('the night queue on the orb (master plan #1)', () => {
  it('counts queued and running items with the cap, and says nothing with none', () => {
    const items = [{ status: 'queued' }, { status: 'running' }, { status: 'queued' }, { status: 'done' }, { status: 'removed' }];
    expect(nightLine(items, 40)).toBe('3 queued for tonight · $40 cap');
    expect(nightLine(items, 12.5)).toBe('3 queued for tonight · $12.50 cap');
    expect(nightLine(items, null)).toBe('3 queued for tonight');
    expect(nightLine([{ status: 'done' }], 40)).toBe('');
  });
});

describe('a started chat\'s own status line (repo ideas 2026-10-27 #7)', () => {
  it('matches the Rust outcome\'s wire shape', () => {
    expect(mainAgentSource).toContain('pub outcome: Option<crate::child_status::Outcome>');
  });
});
