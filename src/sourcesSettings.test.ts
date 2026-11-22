import { describe, expect, it } from 'vitest';
import type { SourcesBinding, SourcesMigrationPlan } from './generated/ipc';
import {
  bindingLine, itemLabel, labelTone, planLines, switchBlocked, switchVerb, taskChoices, waitingLine, withReference, withoutReference,
} from './sourcesSettings';

const binding = (over: Partial<SourcesBinding> = {}): SourcesBinding => ({
  role: 'tasks', owner: 'vault', inherited: true, chosen: false, workspace: null, provider: null, providerName: null, teamId: null, teamKey: null,
  trackerProject: null, wasTracked: null, readOnly: null, reachable: true, unreachableSince: null, problems: [], ...over,
});

const plan = (over: Partial<SourcesMigrationPlan> = {}): SourcesMigrationPlan => ({
  target: { workspace: null, to: 'vault' }, from: 'tracker', nothingToDo: false, refused: null, files: [], configChange: null,
  generationFrom: 1, generationTo: 2, storedInHollowAfter: true, queuedEdits: 0, heldEdits: 0, conflicts: 0, mirroredTasks: 0, mirroredProjects: 0,
  trackerWrites: 0, notes: [], ...over,
});

describe('Settings, Sources words (H4b, T5)', () => {
  it('offers T5’s two choices: Vault only, or Vault plus the tracker', () => {
    expect(taskChoices().map(c => c.label)).toEqual(['Vault only', 'Vault plus Linear']);
    expect(taskChoices('Jira').map(c => c.label)).toEqual(['Vault only', 'Vault plus Jira']);
  });

  it('says whether a workspace inherits or overrides', () => {
    expect(bindingLine(binding({ owner: 'tracker', providerName: 'Linear' }))).toBe('Inherits: Linear');
    expect(bindingLine(binding({ inherited: false }))).toBe('Override: vault only');
  });

  it('tones a label: unreachable warns, read-only is muted', () => {
    expect(labelTone('tracked')).toBe('ok');
    expect(labelTone('stored')).toBe('ok');
    expect(labelTone('readOnly')).toBe('off');
    expect(labelTone('unreachable')).toBe('warn');
  });

  it('blocks a switch while edits wait or one stopped part way', () => {
    const clear = { queuedEdits: 0, heldEdits: 0, conflicts: 0 };
    expect(waitingLine(clear)).toBeNull();
    expect(switchBlocked(clear, null)).toBeNull();
    expect(switchBlocked({ ...clear, queuedEdits: 2, conflicts: 1 }, null)).toBe('Settle these first: 2 edits waiting to send, 1 conflict.');
    expect(switchBlocked(clear, 'switch-1')).toMatch(/stopped part way/);
  });

  it('reads a dry run as counts first, and never as a tracker write when there is none', () => {
    const lines = planLines(plan({ mirroredTasks: 12, mirroredProjects: 1, files: [{ path: 'a', action: 'edit', why: '' }] }));
    expect(lines[0]).toBe('12 tasks and 1 project mirrored now; every file stays in your vault.');
    expect(lines).toContain('1 file in your vault change, each backed up first.');
    expect(lines).toContain('Nothing is sent to the tracker, archived or deleted there.');
    expect(planLines(plan({ refused: 'Settle the outbox first.' }))).toEqual(['Settle the outbox first.']);
    expect(planLines(plan({ nothingToDo: true }))[0]).toMatch(/Nothing to change/);
  });

  it('names the switch by where tasks go', () => {
    expect(switchVerb('tracker', null)).toBe('Switch to Vault plus Linear');
    expect(switchVerb('vault')).toBe('Switch to Vault only');
  });

  it('edits the knowledge list without duplicates', () => {
    const one = withReference([], 'notion-official');
    expect(withReference(one, 'notion-official')).toEqual(one);
    expect(withoutReference(one, 'notion-official')).toEqual([]);
  });

  it('prefers an item’s own label over its role’s', () => {
    const own = { kind: 'unreachable', text: 'Linear unreachable', providerName: 'Linear', url: null, openLabel: null, since: null, wasTracked: null, wasTrackedUrl: null } as const;
    const role = { ...own, kind: 'tracked' as const, text: 'Tracked in Linear' };
    expect(itemLabel(own, role)?.text).toBe('Linear unreachable');
    expect(itemLabel(null, role)?.text).toBe('Tracked in Linear');
    expect(itemLabel(undefined, undefined)).toBeNull();
  });
});
