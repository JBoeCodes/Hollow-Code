import { describe, expect, it } from 'vitest';
import type { Workflow } from './generated/ipc';
import { automationWords, automationsCount, chatAutomations, serversCount } from './cardAutomationsModel';

const wf = (slug: string, inChat: string | undefined, next: string | null = null, schedule = '0 0 9 * * 1-5 *'): Workflow => ({ slug, title: slug, inChat, nextRunAt: next, schedule, lastStatus: null, lastRunAt: null } as unknown as Workflow);

describe('the card Servers and Automations sections (parity 5.5)', () => {
  it('keeps only this chat\'s automations, soonest first, none without a key', () => {
    const list = [wf('a', 's-1', '2026-11-22T09:00:00Z'), wf('b', 's-2'), wf('c', 's-1', '2026-11-21T09:00:00Z'), wf('d', 's-1')];
    expect(chatAutomations(list, 's-1').map(w => w.slug)).toEqual(['c', 'a', 'd']);
    expect(chatAutomations(list, null)).toEqual([]);
    expect(chatAutomations(undefined, 's-1')).toEqual([]);
  });
  it('words the counts', () => {
    expect(automationsCount(0)).toBe('None');
    expect(automationsCount(2)).toBe('2 here');
    expect(serversCount([])).toBe('None running');
    expect(serversCount([{ status: 'running' }, { status: 'exited' }, { status: 'running' }])).toBe('2 running, 1 exited');
    expect(serversCount([{ status: 'exited' }])).toBe('1 exited');
  });
  it('words a row with the schedule and how the last run went', () => {
    const base = { schedule: 'manual', lastStatus: null, lastRunAt: null, nextRunAt: null };
    expect(automationWords(base)).toBe('manual');
    expect(automationWords({ ...base, lastStatus: 'error' })).toBe('manual, last run failed');
    expect(automationWords({ ...base, lastStatus: 'ok', nextRunAt: '2020-01-01T00:00:00Z' })).toBe('manual, last run ok');
    expect(automationWords({ ...base, nextRunAt: '2030-01-01T09:00:00Z' }, Date.parse('2029-12-31T00:00:00Z'))).toMatch(/^manual, next /);
  });
});
