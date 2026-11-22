import { describe, expect, it } from 'vitest';
import { commitmentsDue, countsLine, dueBuckets, factsDigest, greeting, todayCoverage, todaysMeetings, calendarCoverageOf } from './todayBrief';
import { GO_TARGETS } from './keyboardLeader';
import { ACTION_BY_PALETTE } from './actionDefs';
import type { AgendaEvent } from './agenda';
import type { Commitment } from './types';
import type { Task } from './generated/ipc';

const at = (h: number, m = 0) => new Date(2026, 9, 11, h, m);
const iso = (h: number, m = 0) => at(h, m).toISOString();
const event = (id: string, h: number, over: Partial<AgendaEvent> = {}): AgendaEvent => ({ id, title: id, start: iso(h), end: iso(h + 1), allDay: false, cancelled: false, ...over });
const task = (title: string, due: string | null, status = 'Todo'): Task => ({ url: `tasks/${title}.md`, title, status, due } as unknown as Task);
const commitment = (title: string, due: string | null, status: Commitment['status'] = 'open'): Commitment => ({ url: title, vaultPath: `c/${title}.md`, title, direction: 'owed_by_me', counterparty: 'Pat', personPath: null, projectUrl: null, due, status, channel: 'meeting', sourcePath: null, created: '', fulfilledAt: null, fingerprint: '', context: '' } as unknown as Commitment);

describe('Today (overnight 10-11 A3)', () => {
  it('treats denied Mac access and damaged settings as unknown coverage, while Off is deliberate', () => {
    const view = { connected: false, events: [], fetchedAt: '', error: null, source: 'Mac', complete: false, status: 'Calendar access is denied' };
    expect(calendarCoverageOf(view, null)).toEqual({ state: 'unavailable', reason: view.status });
    expect(calendarCoverageOf({ ...view, error: 'Settings damaged' }, null)).toEqual({ state: 'unavailable', reason: 'Settings damaged' });
    expect(calendarCoverageOf({ ...view, complete: true, status: 'Calendar is off' }, null)).toEqual({ state: 'not-connected', reason: 'Calendar is off' });
  });
  it('greets by the time of day, with the name when known', () => {
    expect(greeting(at(8), 'Alex')).toBe('Good morning, Alex');
    expect(greeting(at(13))).toBe('Good afternoon');
    expect(greeting(at(19), 'Ada')).toBe('Good evening, Ada');
    expect(greeting(at(2))).toBe('Up late');
  });

  it('lists today’s meetings in order, all-day first, with soon and past', () => {
    const events = [event('late', 15), event('early', 9), event('gone', 10, { cancelled: true }), event('allday', 0, { allDay: true }),
      { ...event('tomorrow', 9), start: new Date(2026, 9, 12, 9).toISOString() }];
    const list = todaysMeetings(events, at(10, 30));
    expect(list.map(m => m.id)).toEqual(['allday', 'early', 'late']);
    expect(list[1].past).toBe(true);
    expect(todaysMeetings([event('next', 11)], at(10, 30))[0].soon).toBe(true);
    expect(list[0].time).toBe('All day');
  });

  it('keeps Graph-shaped all-day events on their own dates, and multi-day ones on each day', () => {
    // Graph sends all-day events as UTC midnights; a local time would shift
    // them a day in negative offsets (review A3 #4).
    const allDay = (id: string, from: string, to: string): AgendaEvent => ({ id, title: id, start: `${from}T00:00:00Z`, end: `${to}T00:00:00Z`, allDay: true, cancelled: false });
    const list = todaysMeetings([allDay('today', '2026-10-11', '2026-10-12'), allDay('tomorrow', '2026-10-12', '2026-10-13'), allDay('offsite', '2026-10-10', '2026-10-13'),
      { ...event('overnight', 0), start: new Date(2026, 9, 10, 23).toISOString(), end: new Date(2026, 9, 11, 1).toISOString() }], at(8));
    expect(list.map(m => m.id).sort()).toEqual(['offsite', 'overnight', 'today']);
  });

  it('splits open tasks into overdue and due today, never done ones', () => {
    const b = dueBuckets([task('a', '2026-10-09'), task('b', '2026-10-11'), task('c', '2026-10-08'), task('d', '2026-10-11', 'Done'), task('e', null), task('f', '2026-10-12')], '2026-10-11');
    expect(b.overdue.map(t => t.title)).toEqual(['c', 'a']);
    expect(b.today.map(t => t.title)).toEqual(['b']);
  });

  it('keeps open commitments due by today', () => {
    const due = commitmentsDue([commitment('x', '2026-10-12'), commitment('y', '2026-10-10'), commitment('z', '2026-10-11'), commitment('w', '2026-10-01', 'fulfilled'), commitment('v', null)], '2026-10-11');
    expect(due.map(c => c.title)).toEqual(['y', 'z']);
    // Snoozed in the ledger (A5): out until its day.
    const snoozed = { ...commitment('s', '2026-10-09'), snoozedUntil: '2026-10-13' };
    expect(commitmentsDue([snoozed], '2026-10-11')).toEqual([]);
    expect(commitmentsDue([snoozed], '2026-10-13').map(c => c.title)).toEqual(['s']);
  });

  it('turns the day into facts the agent can write from, and counts when there is no brief', () => {
    const facts = {
      date: '2026-10-11', meetings: todaysMeetings([event('Atlas review', 14)], at(8)), overdue: [task('Call Pat', '2026-10-09')], dueToday: [],
      commitments: [commitment('Q3 numbers', '2026-10-11')], waiting: { total: 2, titles: ['Review a result', 'Answer a question'] }, overnight: ['merged #519'],
      coverage: todayCoverage({ calendar: { connected: true, fetchedAt: '', events: [], error: null, source: 'graph' }, calendarError: null, commitmentsError: null, overnightError: null, waitingFailed: false, waitingUnavailable: [] }),
    };
    const text = factsDigest(facts);
    expect(text).toContain('Meetings today (1):');
    expect(text).toContain('Atlas review');
    expect(text).toContain('- Call Pat (due 2026-10-09)');
    expect(text).toContain('I owe Pat: Q3 numbers (due 2026-10-11)');
    expect(text).toContain('Waiting on me (2):');
    expect(countsLine(facts)).toBe('1 meeting · 1 overdue · 1 commitment due · 2 things waiting on you.');
    expect(countsLine({ meetings: [], overdue: [], dueToday: [], commitments: [], waiting: { total: 0, titles: [] } })).toBe('A clear day: nothing scheduled or due.');
  });

  it('keeps unique go-to keys after Today was cut', () => {
    expect(GO_TARGETS.find(t => t.id === 'today')).toBeUndefined();
    expect(new Set(GO_TARGETS.map(t => t.key)).size).toBe(GO_TARGETS.length);
    expect(ACTION_BY_PALETTE['act:today']).toBeUndefined();
  });
});
