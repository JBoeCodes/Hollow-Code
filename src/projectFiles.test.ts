import { describe, expect, it } from 'vitest';
import { PINNED_NOTES, filesPickerOptions, inVault, pinnedNotes, planFileOf, resultFileOf, vaultRelative } from './projectFiles';
import { planItemLine, planRowsInOrder } from './planModel';

// COO rebuild L4: a project's files in the Preview pane's Files tab.
const Q3 = { id: 'local-q3', path: 'workspaces/local-q3' };
const BUILD = { id: 'build-1', path: 'workspaces/build-1' };

describe('project files in the Files tab (COO rebuild L4)', () => {
  it('joins and splits vault paths, and refuses anything outside the vault', () => {
    expect(inVault('/v/', '/workspaces/a/PLAN.md')).toBe('/v/workspaces/a/PLAN.md');
    expect(vaultRelative('/v/workspaces/a/PLAN.md', '/v')).toBe('workspaces/a/PLAN.md');
    expect(vaultRelative('/v/workspaces/a/PLAN.md', '/v/')).toBe('workspaces/a/PLAN.md');
    expect(vaultRelative('/vault2/x.md', '/v')).toBeNull();
    expect(vaultRelative('/v', '/v')).toBeNull();
    expect(vaultRelative('workspaces/a/PLAN.md', '/v')).toBeNull();
    expect(vaultRelative('/v/x.md', null)).toBeNull();
  });

  it('pins the project notes that exist, in WORKSPACE, PLAN, SOURCES, AGENTS order', () => {
    expect(PINNED_NOTES).toEqual(['WORKSPACE.md', 'PLAN.md', 'SOURCES.md', 'AGENTS.md']);
    const files = ['AGENTS.md', 'outputs/r.md', 'PLAN.md', 'WORKSPACE.md', 'notes/SOURCES.md'].map(n => ({ path: `${Q3.path}/${n}` }));
    expect(pinnedNotes(Q3, files)).toEqual(['workspaces/local-q3/WORKSPACE.md', 'workspaces/local-q3/PLAN.md', 'workspaces/local-q3/AGENTS.md']);
    expect(pinnedNotes(Q3, [])).toEqual([]);
  });

  it('a file in a workspace’s outputs/ is a Result of that workspace; nothing else is', () => {
    const ws = [Q3, BUILD];
    expect(resultFileOf('workspaces/local-q3/outputs/review.md', ws)).toEqual({ workspaceId: 'local-q3', path: 'workspaces/local-q3/outputs/review.md' });
    expect(resultFileOf('workspaces/local-q3/outputs/canvases/plan.canvas.html', ws)?.workspaceId).toBe('local-q3');
    expect(resultFileOf('workspaces/local-q3/PLAN.md', ws)).toBeNull();
    expect(resultFileOf('workspaces/local-q3/sources/outputs/x.md', ws)).toBeNull();
    expect(resultFileOf('workspaces/other/outputs/x.md', ws)).toBeNull();
    expect(resultFileOf(null, ws)).toBeNull();
    // A workspace whose path is a prefix of another's never claims its files.
    expect(resultFileOf('workspaces/local-q3-b/outputs/x.md', [Q3, { id: 'b', path: 'workspaces/local-q3-b' }])?.workspaceId).toBe('b');
  });

  it('only a workspace’s own PLAN.md ticks', () => {
    expect(planFileOf('workspaces/local-q3/PLAN.md', [Q3])).toBe('local-q3');
    expect(planFileOf('workspaces/local-q3/notes/PLAN.md', [Q3])).toBeNull();
    expect(planFileOf('library/PLAN.md', [Q3])).toBeNull();
  });

  it('the picker lists notes, then changed files, then the conversation’s, each path once', () => {
    const opts = filesPickerOptions({
      notes: ['/v/workspaces/a/WORKSPACE.md', '/v/workspaces/a/PLAN.md'],
      changed: ['/wt/src/app.ts', '/v/workspaces/a/PLAN.md'],
      conversation: [{ value: '/wt/src/app.ts', label: 'app.ts' }, { value: '/v/out.md', label: 'out.md', detail: '/v' }],
    });
    expect(opts.map(o => [o.label, o.group])).toEqual([
      ['WORKSPACE.md', 'Project notes'], ['PLAN.md', 'Project notes'], ['app.ts', 'Changed files'], ['out.md', 'This conversation'],
    ]);
    expect(opts[2].detail).toBe('/wt/src');
    // With no project, the conversation's files stand alone, ungrouped.
    expect(filesPickerOptions({ notes: [], changed: [], conversation: [{ value: '/x.md', label: 'x.md' }] })).toEqual([{ value: '/x.md', label: 'x.md', group: undefined }]);
  });
});

describe('PLAN.md checkboxes and the From: plan link (COO rebuild L4)', () => {
  const PLAN = '---\nstatus: active\n---\n# Plan\n\n- [x] Draft\n\n## Next\n- [ ] Review\n  - [ ] Nested check\n```\n- [ ] not a row\n```\n- [ ] **Ship** it\n';
  it('lists the checklist rows in file order, outside fences and frontmatter', () => {
    const rows = planRowsInOrder(PLAN);
    expect(rows.map(r => [r.id.line, r.done])).toEqual([[6, true], [9, false], [10, false], [14, false]]);
  });
  it('finds a task’s row by its text, Markdown and case aside, or says none', () => {
    expect(planItemLine(PLAN, 'Ship it')).toBe(14);
    expect(planItemLine(PLAN, 'review')).toBe(9);
    expect(planItemLine(PLAN, 'Not in the plan')).toBeNull();
  });
});
