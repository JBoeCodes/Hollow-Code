import { describe, expect, it } from 'vitest';
import { VAULT_CHANGE_MAX_PATHS, newestModified, sourceLineText, vaultChangeTouches } from './sectionSource';
import type { WorkFile } from './workspaceTypes';

describe('vaultChangeTouches', () => {
  const brief = 'workspaces/a/WORKSPACE.md';
  it('is true only for a listed path', () => {
    expect(vaultChangeTouches({ folders: ['workspaces'], paths: [brief] }, brief)).toBe(true);
    expect(vaultChangeTouches({ folders: ['workspaces'], paths: ['workspaces/a/PLAN.md'] }, brief)).toBe(false);
    expect(vaultChangeTouches({ folders: ['workspaces'], paths: ['workspaces/ab/WORKSPACE.md'] }, brief)).toBe(false);
    expect(vaultChangeTouches(null, brief)).toBe(false);
    expect(vaultChangeTouches({ folders: ['workspaces'], paths: [brief] }, '')).toBe(false);
  });
  it('assumes a touch when the path list was cut at the cap and the folder changed', () => {
    const many = Array.from({ length: VAULT_CHANGE_MAX_PATHS }, (_, i) => `workspaces/z/outputs/${i}.md`);
    expect(vaultChangeTouches({ folders: ['workspaces'], paths: many }, brief)).toBe(true);
    expect(vaultChangeTouches({ folders: ['tasks'], paths: many }, brief)).toBe(false);
    expect(vaultChangeTouches({ folders: ['workspaces'], paths: many.slice(1) }, brief)).toBe(false);
  });
});

describe('source line', () => {
  const now = Date.parse('2026-10-05T12:00:00Z');
  const file = (name: string, modified: string | null): WorkFile => ({ path: `workspaces/a/${name}`, name, kind: 'context', size: 1, modified });
  it('names the file and how long ago it changed', () => {
    expect(sourceLineText(['WORKSPACE.md'], '2026-10-05T11:58:00Z', now)).toBe('From WORKSPACE.md · updated 2m ago');
    expect(sourceLineText(['WORKSPACE.md', 'PLAN.md'], '2026-10-05T11:59:50Z', now)).toBe('From WORKSPACE.md and PLAN.md · updated just now');
    expect(sourceLineText(['A', 'B', 'C'], null, now)).toBe('From A, B and C');
  });
  it('leaves an unknown time out instead of guessing', () => {
    expect(sourceLineText(['WORKSPACE.md'], null, now)).toBe('From WORKSPACE.md');
    expect(sourceLineText(['WORKSPACE.md'], 'not a date', now)).toBe('From WORKSPACE.md');
  });
  it('takes the newest time among the named files', () => {
    const files = [file('WORKSPACE.md', '2026-10-05T10:00:00Z'), file('PLAN.md', '2026-10-05T11:00:00Z'), file('outputs/x.md', '2026-10-05T11:30:00Z')];
    expect(newestModified(files, ['WORKSPACE.md', 'PLAN.md'])).toBe('2026-10-05T11:00:00Z');
    expect(newestModified(files, ['WORKSPACE.md'])).toBe('2026-10-05T10:00:00Z');
    expect(newestModified([file('WORKSPACE.md', null)], ['WORKSPACE.md'])).toBeNull();
    expect(newestModified(undefined, ['WORKSPACE.md'])).toBeNull();
  });
});
