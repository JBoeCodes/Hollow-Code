import { describe, expect, it } from 'vitest';
import { changeRecoverable, coverageLabel, isSharedRestoreConflict, overlapLine, restorePreview, runFilesLine, runOutcome, runRecoverable, runTitle, runWorkspaceId, sharedChanges, sideCoverage, uncapturedReason } from './historyRun';

const change = (vaultPath: string) => ({ vaultPath }) as any;

describe('recovery presentation (audit H12)', () => {
  it('names the workspace and source instead of raw ids', () => {
    const titles = (id: string) => ({ 'local-abc': 'Quarter review' } as Record<string, string>)[id];
    expect(runTitle({ source: 'chat', sourceId: 's-2026-uuid', changes: [change('workspaces/local-abc/outputs/r.md')] }, titles)).toBe('Conversation in Quarter review');
    expect(runTitle({ source: 'workflow', sourceId: 'daily-brief', changes: [change('briefs/a.md')] }, titles)).toBe('Automation “daily-brief”');
    expect(runTitle({ source: 'chat', sourceId: null, scope: 'workspaces/local-xyz', changes: [] }, titles)).toBe('Conversation in local-xyz');
  });
  it('finds one shared workspace, else none', () => {
    expect(runWorkspaceId({ changes: [change('workspaces/a/x.md'), change('workspaces/a/y.md')] })).toBe('a');
    expect(runWorkspaceId({ changes: [change('workspaces/a/x.md'), change('library/y.md')] })).toBeNull();
  });
  it('summarizes files and outcome plainly', () => {
    expect(runFilesLine({ changes: [change('a/one.md'), change('b/two.md'), change('c/three.md')] })).toBe('one.md, two.md +1 more');
    expect(runFilesLine({ changes: [] })).toBe('No file changes');
    expect(runOutcome('success')).toEqual({ label: 'Completed', tone: 'ok' });
    expect(runOutcome('error').label).toBe('Failed');
    expect(runOutcome('interrupted')).toEqual({ label: 'Interrupted', tone: 'warn' });
  });
});

describe('uncaptured changes (merged plan slice 1, review B·F01)', () => {
  it('explains why a change with an uncaptured side has no restore', () => {
    expect(uncapturedReason({ beforeUncaptured: true })).toMatch(/before the run/);
    expect(uncapturedReason({ afterUncaptured: true })).toMatch(/won’t replace it/);
    expect(uncapturedReason({})).toBeNull();
    expect(uncapturedReason({ beforeUncaptured: false, afterUncaptured: false })).toBeNull();
  });
});

describe('coverage model (merged plan slice 4, review B·F01, decision 5)', () => {
  const hash = 'a'.repeat(64);
  it('classifies each side as absent, captured, uncaptured or unknown', () => {
    expect(sideCoverage({}, 'before')).toEqual({ state: 'absent' });
    expect(sideCoverage({ afterHash: hash }, 'after')).toEqual({ state: 'captured' });
    expect(sideCoverage({ afterCoverage: { state: 'uncaptured', reason: 'tooLarge' }, afterUncaptured: true }, 'after'))
      .toEqual({ state: 'uncaptured', reason: 'tooLarge' });
    expect(sideCoverage({ beforeCoverage: { state: 'unknown' }, beforeUncaptured: true }, 'before')).toEqual({ state: 'unknown' });
    // A slice 1 record: the flag alone, no reason kept.
    expect(sideCoverage({ beforeUncaptured: true }, 'before')).toEqual({ state: 'uncaptured', reason: null });
    // A record that contradicts itself never reads as restorable.
    expect(sideCoverage({ afterCoverage: { state: 'captured' } }, 'after')).toEqual({ state: 'unknown' });
    expect(sideCoverage({ beforeCoverage: { state: 'absent' }, beforeHash: hash }, 'before')).toEqual({ state: 'unknown' });
    expect(coverageLabel({ state: 'uncaptured', reason: 'excluded' })).toBe('link or special file');
    expect(coverageLabel({ state: 'captured' })).toBe('');
  });
  it('lists every file a restore can’t return in the restore preview', () => {
    const changes = [
      { vaultPath: 'notes/plain.md', beforeHash: hash, afterHash: hash },
      { vaultPath: 'notes/export.bin', afterCoverage: { state: 'uncaptured', reason: 'tooLarge' }, afterUncaptured: true },
      { vaultPath: 'locked/x.md', beforeHash: hash, afterCoverage: { state: 'unknown' }, afterUncaptured: true },
    ] as const;
    const preview = restorePreview(changes as any);
    expect(preview.map(item => item.path)).toEqual(['locked/x.md', 'notes/export.bin']);
    expect(preview[1].reason).toMatch(/over 2 MiB/);
    expect(preview[0].reason).toMatch(/can’t tell whether/);
    expect(restorePreview([changes[0]] as any)).toEqual([]);
  });
  it('never calls a run with an uncaptured file fully recoverable', () => {
    const clean = { vaultPath: 'notes/a.md', beforeHash: hash, afterHash: hash } as any;
    const big = { vaultPath: 'notes/big.bin', beforeCoverage: { state: 'absent' }, afterCoverage: { state: 'uncaptured', reason: 'tooLarge' }, afterUncaptured: true } as any;
    expect(changeRecoverable(clean)).toBe(true);
    expect(changeRecoverable(big)).toBe(false);
    expect(runRecoverable({ changes: [clean] })).toBe(true);
    expect(runRecoverable({ changes: [clean, big] })).toBe(false);
  });
});

describe('parallel chats in one workspace (2026-10-07)', () => {
  it('lists the files another conversation also changed', () => {
    const run = { changes: [
      { vaultPath: 'workspaces/w/WORKSPACE.md', alsoChangedBy: ['run-b'] },
      { vaultPath: 'workspaces/w/a.md' },
      { vaultPath: 'workspaces/w/b.md', alsoChangedBy: [] },
    ] };
    expect(sharedChanges(run).map(c => c.vaultPath)).toEqual(['workspaces/w/WORKSPACE.md']);
  });
  it('names the peer, or counts several', () => {
    const title = (id: string) => (id === 'run-b' ? 'Conversation in Hollow' : undefined);
    expect(overlapLine({}, title)).toBe('');
    expect(overlapLine({ overlaps: ['run-b'] }, title)).toBe('Ran at the same time as Conversation in Hollow in this workspace.');
    expect(overlapLine({ overlaps: ['run-gone'] }, title)).toBe('Ran at the same time as another run in this workspace.');
    expect(overlapLine({ overlaps: ['run-b', 'run-c'] }, title)).toBe('Ran at the same time as 2 other runs in this workspace.');
  });
  it('recognizes the shared-file refusal only', () => {
    expect(isSharedRestoreConflict('history restore conflict (shared): workspaces/w/WORKSPACE.md was also changed by another conversation')).toBe(true);
    expect(isSharedRestoreConflict('history restore conflict: x no longer matches the recorded after-version')).toBe(false);
  });
});
