import { describe, expect, it } from 'vitest';
import { localMinute, organizationGroup, parseSnoozeTime, snoozePresets, type ConversationOrganization } from './conversationOrganization';
import type { AgentSessionSummary } from './types';

const session = { key: 's-test', lastActivityAt: '2030-01-01T12:00:00Z' } as AgentSessionSummary;
const row: ConversationOrganization = { key: session.key, session, pinned: false, settled: false, snoozedUntil: '2030-01-01T13:00:00Z', reason: null, revision: 1 };
const now = Date.parse('2030-01-01T12:30:00Z');
describe('conversation organization precedence', () => {
  it('keeps attention, working, paused and current rows active', () => {
    for (const state of ['waiting', 'failed', 'interrupted', 'running', 'queued', 'paused'] as const) expect(organizationGroup(row, session, state, false, now)).toBe('active');
    expect(organizationGroup(row, session, 'idle', true, now)).toBe('active');
  });
  it('wakes at the exact time and on new activity', () => {
    expect(organizationGroup(row, session, 'idle', false, now)).toBe('snoozed');
    expect(organizationGroup(row, session, 'idle', false, Date.parse(row.snoozedUntil!))).toBe('active');
    expect(organizationGroup(row, { ...session, lastActivityAt: '2030-01-01T12:01:00Z' }, 'idle', false, now)).toBe('active');
  });
  it('pins stay visible and settled rows remain reversible', () => {
    expect(organizationGroup({ ...row, pinned: true, settled: true }, session, 'idle', false, now)).toBe('pinned');
    expect(organizationGroup({ ...row, settled: true }, session, 'idle', false, now)).toBe('settled');
  });
  it('accepts only a real future local minute', () => {
    expect(parseSnoozeTime('2030-02-30T12:00', 0)).toBeNull();
    expect(parseSnoozeTime('tomorrow', 0)).toBeNull();
    expect(parseSnoozeTime('2000-01-01T12:00', now)).toBeNull();
    expect(parseSnoozeTime('2030-01-02T12:00', now)).toBe(new Date('2030-01-02T12:00').toISOString());
  });
});

describe('snooze presets', () => {
  // Wednesday 2030-01-02 10:30 local.
  const wed = new Date(2030, 0, 2, 10, 30).getTime();
  const at = (list: ReturnType<typeof snoozePresets>, id: string) => localMinute(Date.parse(list.find(p => p.id === id)!.at));
  it('offers hours, tomorrow, the weekend and next week, all in the future', () => {
    const list = snoozePresets(wed);
    expect(list.map(p => p.id)).toEqual(['hour', 'three-hours', 'tomorrow', 'weekend', 'next-week']);
    expect(at(list, 'hour')).toBe('2030-01-02T11:30');
    expect(at(list, 'three-hours')).toBe('2030-01-02T13:30');
    expect(at(list, 'tomorrow')).toBe('2030-01-03T09:00');
    expect(at(list, 'weekend')).toBe('2030-01-05T09:00');
    expect(at(list, 'next-week')).toBe('2030-01-07T09:00');
    expect(list.every(p => Date.parse(p.at) > wed)).toBe(true);
  });
  it('skips the weekend on a weekend and starts next week on Monday', () => {
    const sat = new Date(2030, 0, 5, 10, 0).getTime();
    expect(snoozePresets(sat).map(p => p.id)).not.toContain('weekend');
    expect(at(snoozePresets(sat), 'next-week')).toBe('2030-01-07T09:00');
    const mon = new Date(2030, 0, 7, 8, 0).getTime();
    expect(at(snoozePresets(mon), 'next-week')).toBe('2030-01-14T09:00');
  });
  it('writes a minute a datetime-local field and parseSnoozeTime both accept', () => {
    expect(localMinute(new Date(2030, 0, 2, 9, 5).getTime())).toBe('2030-01-02T09:05');
    expect(parseSnoozeTime(localMinute(wed + 3_600_000), wed)).not.toBeNull();
  });
});
