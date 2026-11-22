import { describe, expect, it } from 'vitest';
import { archiveBody, archiveDoneMessage, archiveSummary, hasMoves, type SharedDocArchive } from './archiveOlderEntries';

const row = (over: Partial<SharedDocArchive> = {}): SharedDocArchive => ({
  workspace: 'w', doc: 'WORKSPACE.md', mode: 'dry_run', today: '2026-11-15', hash: 'h', newHash: null,
  bytesBefore: 4096, bytesAfter: 1024, archiveBytes: 3200, archivePath: 'workspaces/w/outputs/workspace-archive/2026-11-15-current-context.md',
  moved: ['- 09-20 10:00 ET: one', '- 09-21 10:00 ET: two'], movedCount: 2, lossless: true, applied: false, busy: false, runId: null, warnings: [], ...over,
});

describe('Archive older entries text', () => {
  it('offers only docs that move something losslessly', () => {
    expect(hasMoves(row())).toBe(true);
    expect(hasMoves(row({ movedCount: 0 }))).toBe(false);
    expect(hasMoves(row({ lossless: false }))).toBe(false);
  });
  it('summarises sizes before and after and lists the first lines', () => {
    const dry = [row(), row({ doc: 'PLAN.md', movedCount: 1, moved: ['Done (10-01)'], bytesBefore: 500, bytesAfter: 300 })];
    expect(archiveSummary(dry)).toBe('Moves older text out of WORKSPACE.md 4.0 KB to 1.0 KB and PLAN.md 500 B to 300 B into archive notes, word for word.');
    const body = archiveBody(dry);
    expect(body).toContain('WORKSPACE.md: 4.0 KB to 1.0 KB, 2 older entries');
    expect(body).toContain('PLAN.md: 500 B to 300 B, 1 finished section');
    expect(body).toContain('• Done (10-01)');
  });
  it('says a doc that would not reproduce is left alone', () => {
    expect(archiveBody([row(), row({ doc: 'PLAN.md', lossless: false })])).toContain('PLAN.md is left alone');
  });
  it('words the result', () => {
    expect(archiveDoneMessage([row({ applied: true })])).toContain('Archived 2 older entries of WORKSPACE.md');
    expect(archiveDoneMessage([row()])).toContain('nothing moved');
  });
});
