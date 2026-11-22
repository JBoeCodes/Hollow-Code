import { describe, expect, it } from 'vitest';
import { capturedWhen, trackerMirrorOf, isSourceChanged, isWorkspaceSnapshot, readOnlyWord, snapshotParts, sourceStateLine, vaultFileKind, vaultRelativePath, opensAsDocument } from './sourceAuthority';

/** Plan L3 X3 (2026-10-14): one vocabulary for snapshot, live, tracked,
 *  output and note, summary first, never "current" without evidence. */
const NOW = new Date(2026, 9, 14, 12, 0);
const NOT_CURRENT = /\bcurrent\b|up to date|\bfresh\b|latest/i;

describe('sourceStateLine', () => {
  it('names each kind in plain words', () => {
    expect(sourceStateLine({ kind: 'live', project: 'Acme ops' }).summary).toBe('Live file from Acme ops: read each time, never copied');
    expect(sourceStateLine({ kind: 'live' }).summary).toBe('Live file: read each time, never copied');
    expect(sourceStateLine({ kind: 'output' }).summary).toBe('Made in this workspace');
    expect(sourceStateLine({ kind: 'output', workspace: 'Q3 review' }).summary).toBe('Made in Q3 review');
    expect(sourceStateLine({ kind: 'note' }).summary).toBe('Library note: edits save here');
  });

  it('says what a snapshot is by its freshness, captured date first', () => {
    const snap = (freshness: string | null, captured: string | null = '2026-10-02T09:00:00Z') =>
      sourceStateLine({ kind: 'snapshot', freshness, captured, original: '/Users/x/plan.md' }, NOW);
    expect(snap('unchanged_metadata', '2026-10-13T09:00:00Z').summary).toBe('Snapshot captured yesterday; doesn’t update on its own');
    expect(snap('changed').summary).toBe('Snapshot captured 2 Oct; the original has changed since');
    expect(snap('unavailable', null).summary).toBe('Snapshot; its original can’t be found');
    expect(snap('unknown').summary).toBe('Snapshot captured 2 Oct; nothing records where it came from');
    expect(snap('changed').tone).toBe('warn');
    // The exact original stays in the secondary line, not the summary.
    expect(snap('changed').detail).toContain('/Users/x/plan.md');
    expect(snap('changed').summary).not.toContain('/Users/x');
  });

  it('never claims a snapshot is current; only a content check says "same content"', () => {
    for (const freshness of ['unchanged_metadata', 'changed', 'unavailable', 'unknown', null]) {
      const s = sourceStateLine({ kind: 'snapshot', freshness, captured: '2026-10-14T08:00:00Z' }, NOW);
      expect(`${s.summary} ${s.detail}`).not.toMatch(NOT_CURRENT);
      expect(s.summary).not.toMatch(/same content/);
      expect(s.tone).not.toBe('ok');
    }
    const same = sourceStateLine({ kind: 'snapshot', freshness: 'changed', captured: '2026-10-14T08:00:00Z', checked: 'same' }, NOW);
    expect(same.summary).toBe('Snapshot captured today; same content as its original when checked');
    expect(same.tone).toBe('ok');
  });

  it('dates a capture as today, yesterday, a day and month, or with the year', () => {
    expect(capturedWhen('2026-10-14T08:00:00Z', NOW)).toBe('today');
    expect(capturedWhen('2025-12-24T12:00:00Z', NOW)).toBe('24 Dec 2025');
    expect(capturedWhen('not a date', NOW)).toBeNull();
    expect(capturedWhen(null, NOW)).toBeNull();
  });
});

describe('paths', () => {
  it('knows a workspace snapshot from an output, a note and a hidden sidecar', () => {
    expect(isWorkspaceSnapshot('workspaces/w1/sources/plan.md')).toBe(true);
    expect(isWorkspaceSnapshot('workspaces/w1/sources/.provenance/plan.md.json')).toBe(false);
    expect(isWorkspaceSnapshot('workspaces/w1/outputs/plan.md')).toBe(false);
    expect(isWorkspaceSnapshot('library/plan.md')).toBe(false);
    expect(vaultFileKind('workspaces/w1/sources/plan.md')).toBe('snapshot');
    expect(vaultFileKind('workspaces/w1/outputs/report.md')).toBe('output');
    expect(vaultFileKind('library/plan.md')).toBe('note');
    expect(snapshotParts('workspaces/w1/sources/sub/plan.md')).toEqual({ workspaceId: 'w1', path: 'sources/sub/plan.md' });
    expect(snapshotParts('library/plan.md')).toBeNull();
    expect(readOnlyWord('workspaces/w1/sources/plan.md')).toBe('Snapshot · read only');
    expect(readOnlyWord('library/plan.md')).toBe('Read only');
  });

  it('finds an original inside the vault, ignoring /private and refusing a sibling folder', () => {
    expect(vaultRelativePath('/Users/x/vault/library/a.md', '/Users/x/vault')).toBe('library/a.md');
    expect(vaultRelativePath('/private/tmp/hollow/vault/library/a.md', '/tmp/hollow/vault/')).toBe('library/a.md');
    expect(vaultRelativePath('/Users/x/vault-old/a.md', '/Users/x/vault')).toBeNull();
    expect(vaultRelativePath('/Users/x/Downloads/a.md', '/Users/x/vault')).toBeNull();
    expect(vaultRelativePath('/Users/x/vault/../a.md', '/Users/x/vault')).toBeNull();
    expect(vaultRelativePath(null, '/Users/x/vault')).toBeNull();
  });

  it('a tracker mirror says Tracked in its tracker (B3)', () => {
    expect(trackerMirrorOf('---\ntitle: "ENG-37"\ntracker: "linear"\ntracker_id: "i-37"\n---\n')).toBe('linear');
    expect(trackerMirrorOf('---\ntracker: linear\ntracker_project: p-1\n---\n')).toBe('linear');
    expect(trackerMirrorOf('---\ntracker: linear\n---\n')).toBeNull();
    expect(trackerMirrorOf('no frontmatter\ntracker: linear\ntracker_id: x1\n')).toBeNull();
    expect(sourceStateLine({ kind: 'tracked', service: 'Linear' }).summary).toBe('Tracked in Linear: its fields come from there');
  });

  it('recognises the refresh conflict error', () => {
    expect(isSourceChanged('SOURCE_CHANGED: The original or the snapshot changed after you checked.')).toBe(true);
    expect(isSourceChanged(new Error('SOURCE_CHANGED: x'))).toBe(true);
    expect(isSourceChanged('Source unavailable')).toBe(false);
  });
});

// L3 review (2026-10-14): the sidecar naming an original is agent-editable,
// so Open original never launches an app or a script.
describe('opensAsDocument', () => {
  it('opens documents and reveals apps, scripts and extensionless files', () => {
    expect(opensAsDocument('/Users/x/Report.pdf')).toBe(true);
    expect(opensAsDocument('/Users/x/notes.md')).toBe(true);
    for (const p of ['/tmp/run.command', '/Applications/Evil.app', '/Applications/Evil.app/', '/tmp/x.sh', '/tmp/install.pkg', '/tmp/README', '/tmp/a.scpt', '/tmp/page.html', '/tmp/page.HTM']) expect(opensAsDocument(p), p).toBe(false);
  });
});
