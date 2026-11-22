import { describe, expect, it } from 'vitest';
import { canManageNote, movedNotePath, renamedNotePath } from './libraryNoteManage';
import LIBRARY from './pages/Library.tsx?raw';

describe('Library note management (P1-18)', () => {
  it('offers rename, move and trash for vault notes only', () => {
    expect(canManageNote('library/Launch plan.md')).toBe(true);
    expect(canManageNote('AGENTS.md')).toBe(true);
    // A snapshot is read only; a workspace's own files stay where it keeps them.
    expect(canManageNote('workspaces/q3/sources/policy.md')).toBe(false);
    expect(canManageNote('workspaces/q3/outputs/report.md')).toBe(false);
    expect(canManageNote('workspaces/q3/WORKSPACE.md')).toBe(false);
    expect(canManageNote('.trash/1-2-old.md')).toBe(false);
    expect(canManageNote('library/data.csv')).toBe(false);
  });

  it('renames in place and keeps the extension', () => {
    expect(renamedNotePath('library/Plan draft.md', 'Launch plan')).toEqual({ path: 'library/Launch plan.md' });
    expect(renamedNotePath('library/Plan draft.md', ' Launch plan.md ')).toEqual({ path: 'library/Launch plan.md' });
    expect(renamedNotePath('Root.md', 'Top')).toEqual({ path: 'Top.md' });
    expect(renamedNotePath('library/a.md', 'x/y')).toEqual({ error: 'Use a name without slashes or line breaks.' });
    expect(renamedNotePath('library/a.md', '  ')).toEqual({ error: 'Type a name.' });
    expect(renamedNotePath('library/a.md', '.hidden')).toEqual({ error: 'A name can’t start with a dot.' });
  });

  it('moves to a folder and keeps the name', () => {
    expect(movedNotePath('library/Plan.md', 'library/projects')).toEqual({ path: 'library/projects/Plan.md' });
    expect(movedNotePath('library/Plan.md', '/library/projects/ ')).toEqual({ path: 'library/projects/Plan.md' });
    expect(movedNotePath('library/Plan.md', '')).toEqual({ path: 'Plan.md' });
    expect(movedNotePath('library/Plan.md', '../outside')).toEqual({ error: 'Choose a folder inside the vault, not a hidden one.' });
    expect(movedNotePath('library/Plan.md', 'library/.trash')).toEqual({ error: 'Choose a folder inside the vault, not a hidden one.' });
    expect(movedNotePath('library/Plan.md', 'workspaces/q3/sources')).toEqual({ error: 'Workspace folders hold their own files; choose another folder.' });
    expect(movedNotePath('library/Plan.md', 'Workspaces/q3')).toEqual({ error: 'Workspace folders hold their own files; choose another folder.' });
    expect(movedNotePath('library/Plan.md', '/')).toEqual({ path: 'Plan.md' });
  });

  it('a folder typed in another letter case keeps the vault’s spelling', () => {
    const known = ['library/Projects/Roadmap.md', 'inbox/a.md'];
    expect(movedNotePath('inbox/a.md', 'LIBRARY/projects', known)).toEqual({ path: 'library/Projects/a.md' });
    expect(movedNotePath('inbox/a.md', 'library/projects/Q4', known)).toEqual({ path: 'library/Projects/Q4/a.md' });
    expect(movedNotePath('inbox/a.md', 'Archive', known)).toEqual({ path: 'Archive/a.md' });
  });

  it('Library offers the three actions in File actions and on each row, and marks the open row', () => {
    expect(LIBRARY).toMatch(/label:'Rename…',action:/);
    expect(LIBRARY).toMatch(/label:'Move to folder…',action:/);
    expect(LIBRARY).toMatch(/label:'Move to trash',action:/);
    expect(LIBRARY).toMatch(/<WorkFileEditor [^>]*actions=\{/);
    expect(LIBRARY).toMatch(/aria-label=\{`More actions for \$\{n\.title\}`\}|label=\{`More actions for \$\{n\.title\}`\}/);
    expect(LIBRARY).toMatch(/aria-current=\{path\(\)===n\.path\?'true':undefined\}/);
    // Backlinks read as the note's title, not its file name.
    expect(LIBRARY).toMatch(/libraryTitle\(\{basename:b\.basename/);
  });
});
