import { describe, expect, it } from 'vitest';
import { cooDispatcherShown, dispatcherLine, dispatcherRowShown, startableRunner } from './cooDispatcher';
import { agentSettingsPayload, draftFromSettings, savePayload, withPiece } from '../agentSettingsPayload';
import type { DispatcherStatus } from '../mainAgent';
import type { AgentSettings } from '../types';

const base: AgentSettings = { defaultAgent: null, models: {} };

const on: DispatcherStatus = {
  dispatching: true, setting: 'auto', machineId: 'm-8d21c4be90f17a3e', machine: 'Studio', since: '2026-11-15T09:00:00Z', off: null, whyOff: null,
};

describe('COO on this Mac (M3a PR2)', () => {
  it('shows off only when saved off; anything else is the default', () => {
    expect(cooDispatcherShown('off')).toBe('off');
    expect(cooDispatcherShown(' off ')).toBe('off');
    for (const v of [undefined, null, '', 'auto', 'moon']) expect(cooDispatcherShown(v)).toBe('auto');
  });

  it('sends off and never auto, and every other save keeps it', () => {
    expect(agentSettingsPayload({ ...base, cooDispatcher: 'off' }).cooDispatcher).toBe('off');
    expect(agentSettingsPayload({ ...base, cooDispatcher: 'auto' })).not.toHaveProperty('cooDispatcher');
    expect(agentSettingsPayload(base)).not.toHaveProperty('cooDispatcher');
    const live: AgentSettings = { ...base, cooDispatcher: 'off' };
    const draft = draftFromSettings(live);
    expect(draft.cooDispatcher).toBe('off');
    // The runner piece's Save sends the whole payload: the choice stays.
    expect(savePayload(withPiece(live, { ...draft, runsOn: 'runner' }, 'runner'), live).cooDispatcher).toBe('off');
  });

  it('says where long work goes now, or why it stays here', () => {
    expect(dispatcherLine(null)).toBe('');
    expect(dispatcherLine(on, at => `[${at}]`)).toBe('Now: the dispatcher, sending long work to Studio (its runner COO since [2026-11-15T09:00:00Z]).');
    expect(dispatcherLine({ ...on, since: null })).toBe('Now: the dispatcher, sending long work to Studio.');
    expect(dispatcherLine({ ...on, dispatching: false, setting: 'off', off: 'setting', whyOff: 'Off in Settings.' })).toBe('Now: the full COO. Off in Settings.');
    expect(dispatcherLine({ ...on, dispatching: false, off: 'no-runner', whyOff: null })).toBe('Now: the full COO.');
  });

  it('shows the row once a runner COO is paired, or while saved off', () => {
    expect(dispatcherRowShown(null, undefined)).toBe(false);
    expect(dispatcherRowShown({ ...on, dispatching: false, machineId: null, machine: null, since: null, off: 'no-runner', whyOff: 'x' }, undefined)).toBe(false);
    expect(dispatcherRowShown({ ...on, dispatching: false, machineId: null, machine: null, since: null, off: 'not-the-app', whyOff: 'x' }, 'auto')).toBe(false);
    expect(dispatcherRowShown(null, 'off')).toBe(true);
    expect(dispatcherRowShown(on, undefined)).toBe(true);
    expect(dispatcherRowShown({ ...on, dispatching: false, machineId: null, machine: null, since: null, off: 'runner-quiet', whyOff: 'x' }, undefined)).toBe(true);
    expect(dispatcherRowShown({ ...on, dispatching: false, machineId: null, machine: null, since: null, off: 'pairing-needed', whyOff: 'x' }, undefined)).toBe(true);
  });

  it('offers Start its COO only for a runner whose COO has no conversation yet (M3a follow-up)', () => {
    const waiting: DispatcherStatus = { ...on, dispatching: false, since: null, off: 'runner-not-started', whyOff: 'Studio runs a runner COO with no conversation yet.' };
    expect(startableRunner(waiting)).toEqual({ machineId: 'm-8d21c4be90f17a3e', machine: 'Studio' });
    expect(dispatcherRowShown({ ...waiting, machine: null }, undefined)).toBe(true);
    expect(startableRunner({ ...waiting, machineId: null })).toBeNull();
    expect(startableRunner(on)).toBeNull();
    expect(startableRunner({ ...waiting, off: 'runner-quiet' })).toBeNull();
    expect(startableRunner(null)).toBeNull();
  });
});
