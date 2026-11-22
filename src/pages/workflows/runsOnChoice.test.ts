import { describe, expect, it } from 'vitest';
import type { PeerMachinesView } from '../../generated/ipc';
import type { PhoneMachine } from '../../types';
import { stampedChanges, type StampedFields } from '../../workflowPermissions';
import {
  NO_OWNER, THIS_MAC, knownMacs, ownerLabel, placement, runsOnForSave, runsOnHint, runsOnOptions, runsOnRowNote,
  runsOnStamp, runsOnValue, vaultShared, type KnownMacs,
} from './runsOnChoice';

// Owner Mac (M4 follow-up): the editor's Runs on choice and the list note,
// by the backend's rule (Rust `runs_on::decide` and `Gate::new`).
const HERE_ID = 'm-1111111111111111';
const PEER_ID = 'm-2222222222222222';
const STRANGER_ID = 'm-3333333333333333';
const here = { machineId: HERE_ID, slug: 'laptop', name: 'Laptop' };
const peer = {
  machineId: PEER_ID, slug: 'desk', name: 'Desk Mac', sshHosts: ['desk'], remotePort: 1, pairedAt: '', lastSeenAt: null,
  lastVersion: null, state: 'online' as const, message: null, checkedAt: null, alias: null, moves: false,
};
const view = (paired: boolean): PeerMachinesView => ({ thisMac: here, machines: paired ? [peer] : [], suggestedAlias: null, moveTo: null, sleepTo: null });
const alone: KnownMacs = knownMacs(view(false), []);
const paired: KnownMacs = knownMacs(view(true), []);

describe('which Macs the owner check knows', () => {
  it('joins both sides of the pairing, one entry per Mac', () => {
    const server: PhoneMachine[] = [
      { deviceId: 'mc-00000001', machineId: PEER_ID, name: 'Desk Mac', pairedAt: '', lastSeenAt: null },
      { deviceId: 'mc-00000002', machineId: STRANGER_ID, name: 'Studio', pairedAt: '', lastSeenAt: null },
    ];
    const macs = knownMacs(view(true), server);
    expect(macs.thisMac?.machineId).toBe(HERE_ID);
    expect(macs.peers.map(p => p.name)).toEqual(['Desk Mac', 'Studio']);
    expect(knownMacs(null, server).thisMac).toBeNull();
  });

  it('counts the vault shared only on evidence of another Mac', () => {
    expect(vaultShared(alone, [null, '', HERE_ID, 'laptop'])).toBe(false);
    // A typo or an unknown slug stops only its own automation.
    expect(vaultShared(alone, ['typo', 'Not A Mac'])).toBe(false);
    expect(vaultShared(alone, [STRANGER_ID])).toBe(true);
    expect(vaultShared(paired, [])).toBe(true);
    expect(vaultShared(knownMacs({ ...view(false), thisMac: null }, []), [STRANGER_ID])).toBe(false);
  });
});

describe('where an automation runs', () => {
  it('follows the rule: this Mac, a paired Mac, another Mac’s id, or nowhere', () => {
    expect(placement(null, alone, false)).toEqual({ kind: 'here' });
    expect(placement('laptop', paired, true)).toEqual({ kind: 'here' });
    expect(placement(HERE_ID, paired, true)).toEqual({ kind: 'here' });
    expect(placement(PEER_ID, paired, true)).toEqual({ kind: 'elsewhere', mac: 'Desk Mac' });
    expect(placement('desk', paired, true)).toEqual({ kind: 'elsewhere', mac: 'Desk Mac' });
    expect(placement(STRANGER_ID, paired, true)).toEqual({ kind: 'elsewhere', mac: `another Mac (${STRANGER_ID})` });
    expect(placement('typo', paired, true)).toEqual({ kind: 'nowhere', why: 'unknown', owner: 'typo' });
    expect(placement(' ', paired, true)).toEqual({ kind: 'nowhere', why: 'no-owner', owner: '' });
  });

  it('notes a row that runs on another Mac or on none, and nothing before the Macs are read', () => {
    expect(runsOnRowNote(PEER_ID, paired, true)).toEqual({ text: 'Runs on Desk Mac', warn: false });
    expect(runsOnRowNote(HERE_ID, paired, true)).toBeNull();
    expect(runsOnRowNote(null, alone, false)).toBeNull();
    expect(runsOnRowNote(null, paired, true)).toEqual({ text: 'Runs on no Mac: it has no owner and another Mac also has this automation. Open Edit and set Runs on.', warn: true });
    expect(runsOnRowNote('typo', paired, true)?.text).toBe('Runs on no Mac: “typo” isn’t this Mac or a paired one. Open Edit and set Runs on.');
    expect(runsOnRowNote(PEER_ID, null, true)).toBeNull();
  });
});

describe('the Runs on choice', () => {
  it('offers no owner, this Mac, each paired Mac, and a saved owner it doesn’t know', () => {
    expect(runsOnOptions(null, paired)).toEqual([
      { value: NO_OWNER, label: 'Any Mac that holds it (until you pair another Mac)' },
      { value: THIS_MAC, label: 'This Mac (Laptop)' },
      { value: PEER_ID, label: 'Desk Mac' },
    ]);
    // A paired Mac saved by its slug keeps that value, so nothing changes.
    expect(runsOnOptions('desk', paired)[2]).toEqual({ value: 'desk', label: 'Desk Mac' });
    expect(runsOnOptions('typo', paired).at(-1)).toEqual({ value: 'typo', label: 'Unknown Mac “typo” (runs nowhere)' });
    expect(runsOnOptions(STRANGER_ID, alone).at(-1)?.label).toBe(`Another Mac (${STRANGER_ID})`);
    expect(runsOnOptions(null, null).map(o => o.label)).toEqual(['Any Mac that holds it (until you pair another Mac)', 'This Mac']);
  });

  it('shows the saved owner and sends only a change', () => {
    expect(runsOnValue('laptop', paired)).toBe(THIS_MAC);
    expect(runsOnValue(PEER_ID, paired)).toBe(PEER_ID);
    expect(runsOnValue('', paired)).toBe(NO_OWNER);
    expect(runsOnForSave(THIS_MAC, 'laptop', paired)).toBeUndefined();
    expect(runsOnForSave(THIS_MAC, null, paired)).toBe('this');
    expect(runsOnForSave(NO_OWNER, PEER_ID, paired)).toBe('');
    expect(runsOnForSave(PEER_ID, HERE_ID, paired)).toBe(PEER_ID);
  });

  it('compares the stamp by what the save writes', () => {
    expect(runsOnStamp(THIS_MAC, 'laptop', paired)).toBe('laptop');
    expect(runsOnStamp(THIS_MAC, null, paired)).toBe(HERE_ID);
    expect(runsOnStamp(THIS_MAC, null, null)).toBe(THIS_MAC);
    expect(runsOnStamp(NO_OWNER, PEER_ID, paired)).toBeNull();
    expect(ownerLabel(PEER_ID, paired)).toBe('Desk Mac');
    expect(ownerLabel(HERE_ID, paired)).toBe('this Mac');
    expect(ownerLabel(null, paired)).toBe('no owner');
  });

  it('says plainly when an ownerless automation won’t run, and what each choice means', () => {
    expect(runsOnHint(NO_OWNER, paired, true)).toEqual({ text: expect.stringMatching(/^Won’t run on any Mac until it has an owner/), warn: true });
    expect(runsOnHint(NO_OWNER, alone, false).warn).toBe(false);
    expect(runsOnHint(THIS_MAC, paired, true).text).toMatch(/^Runs only on this Mac/);
    expect(runsOnHint(PEER_ID, paired, true).text).toMatch(/^Runs only on Desk Mac: .*Run now still runs it here\./);
    expect(runsOnHint('typo', paired, true)).toEqual({ text: expect.stringMatching(/runs nowhere\. Pick a Mac/), warn: true });
    // Macs not read yet (or unreadable): no false "runs nowhere".
    expect(runsOnHint('typo', null, false)).toEqual({ text: 'Reading your Macs.', warn: false });
    expect(runsOnHint(PEER_ID, null, false).warn).toBe(false);
  });
});

describe('a new owner is a permission change', () => {
  const base: StampedFields = {
    permissionMode: 'default', allowedTools: [], disallowedTools: [], cwd: null, agent: null, model: null,
    triggers: [], schedule: 'manual', prompt: 'Do it.', runsOn: null,
  };
  const name = (o: string | null) => ownerLabel(o, paired);

  it('lists the move and widens on gaining or changing an owner, not on removing one', () => {
    const gained = stampedChanges(base, { ...base, runsOn: PEER_ID }, name);
    expect(gained.changes).toEqual([{ field: 'runs on', text: 'Runs on: no owner → Desk Mac' }]);
    expect(gained.widens).toBe(true);
    expect(stampedChanges({ ...base, runsOn: HERE_ID }, { ...base, runsOn: PEER_ID }, name).widens).toBe(true);
    const removed = stampedChanges({ ...base, runsOn: PEER_ID }, base, name);
    expect(removed.changes.map(c => c.field)).toEqual(['runs on']);
    expect(removed.widens).toBe(false);
    expect(stampedChanges(base, { ...base }).changes).toEqual([]);
  });
});
