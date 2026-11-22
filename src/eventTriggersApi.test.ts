import { beforeEach, describe, expect, it, vi } from 'vitest';
import { eventToken, tokenFor } from './eventTriggersApi';
import { mockEventTriggersCommand } from './mockEventTriggers';

describe('event trigger tokens (F2)', () => {
  it('reads the event token among an automation\'s triggers', () => {
    expect(eventToken(['task-overdue'])).toBeNull();
    expect(eventToken(['task-overdue', 'tracker-issue:beta-2'])).toEqual(['tracker-issue', 'beta-2']);
    expect(eventToken(['pr-merged'])).toEqual(['pr-merged', null]);
    expect(eventToken(['Recording-Ended'])).toEqual(['recording-ended', null]);
  });

  it('writes a per-project token only for the per-project kinds', () => {
    expect(tokenFor('tracker-issue', 'beta-2')).toBe('tracker-issue:beta-2');
    expect(tokenFor('pr-merged', null)).toBe('pr-merged');
    expect(tokenFor('recording-ended', 'beta-2')).toBe('recording-ended');
  });
});

describe('the browser mock', () => {
  beforeEach(() => {
    const store = new Map<string, string>();
    vi.stubGlobal('localStorage', { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), removeItem: (k: string) => void store.delete(k) });
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'beta-2', title: 'Beta' }]));
  });

  it('is off by default and a lead wake turns the kind on, apart from the project trigger', () => {
    const before = mockEventTriggersCommand('event_triggers_status', {});
    expect(before.kinds.every(k => !k.on)).toBe(true);
    const after = mockEventTriggersCommand('set_event_trigger', { input: { kind: 'tracker-issue', workspaceId: 'beta-2', leadWake: true } });
    expect(after.leadWakes).toEqual(['beta-2']);
    expect(after.kinds[0].on).toBe(true);
    expect(after.kinds[0].workspaces).toEqual([]);
    // The project's trigger is its own switch: turning it off keeps the lead wake.
    const projectOff = mockEventTriggersCommand('set_event_trigger', { input: { kind: 'tracker-issue', workspaceId: 'beta-2', on: false } });
    expect(projectOff.leadWakes).toEqual(['beta-2']);
    const off = mockEventTriggersCommand('set_event_trigger', { input: { kind: 'tracker-issue', workspaceId: 'beta-2', leadWake: false } });
    expect(off.leadWakes).toEqual([]);
    expect(() => mockEventTriggersCommand('set_event_trigger', { input: { kind: 'nope' } })).toThrow(/Unknown trigger kind/);
  });
});
