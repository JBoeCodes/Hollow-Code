import { describe, expect, it } from 'vitest';
import {
  FOLD_MIN,
  foldTools,
  groupLabel,
  groupStatus,
  toolFamily,
  windowRange,
  type FoldableTool,
  type LedgerItem,
} from './toolFolding';

const t = (name: string, id: string, extra: Partial<FoldableTool> = {}): FoldableTool => ({ name, id, status: 'done', ...extra });

describe('toolFamily', () => {
  it('groups tools by what they do, and MCP calls by server', () => {
    expect(toolFamily('Read')).toBe('read');
    expect(toolFamily('LS')).toBe('read');
    expect(toolFamily('Grep')).toBe('search');
    expect(toolFamily('Glob')).toBe('search');
    expect(toolFamily('MultiEdit')).toBe('edit');
    expect(toolFamily('Write')).toBe('edit');
    expect(toolFamily('Bash')).toBe('shell');
    expect(toolFamily('web_search')).toBe('web');
    expect(toolFamily('TodoWrite')).toBe('todo');
    expect(toolFamily('mcp__hollow_vault__search_vault')).toBe('mcp:hollow_vault');
    expect(toolFamily('file_change')).toBe('file_change');
  });
});

describe('foldTools', () => {
  it('folds three or more consecutive same-family calls and nothing shorter', () => {
    expect(FOLD_MIN).toBe(3);
    expect(foldTools([t('Read', 'a'), t('Read', 'b')]).map(i => i.kind)).toEqual(['single', 'single']);
    // Read and Grep are different families: no run of three.
    expect(foldTools([t('Read', 'a'), t('Read', 'b'), t('Grep', 'c')]).map(i => i.kind)).toEqual(['single', 'single', 'single']);

    const four = foldTools([t('Read', 'a'), t('Read', 'b'), t('Read', 'c'), t('Read', 'd')]);
    expect(four).toHaveLength(1);
    const group = four[0] as Extract<LedgerItem<FoldableTool>, { kind: 'group' }>;
    expect(group.kind).toBe('group');
    expect(group.start).toBe(0);
    expect(groupLabel(group.family, group.tools.length)).toBe('Read 4 files');
  });

  it('reports a running member, and failure only once nothing runs', () => {
    expect(groupStatus([t('Read', 'a'), t('Read', 'b', { status: 'running' }), t('Read', 'c', { status: 'error' })])).toBe('running');
    expect(groupStatus([t('Read', 'a'), t('Read', 'b', { status: 'error' })])).toBe('error');
    expect(groupStatus([t('Read', 'a'), t('Read', 'b')])).toBe('done');
  });

  it('never folds a call carrying an approval, and splits the run around it', () => {
    const tools = [
      t('mcp__hollow_computer__quit_app', 'q1'),
      t('mcp__hollow_computer__quit_app', 'q2', { approval: 'pending' }),
      t('mcp__hollow_computer__quit_app', 'q3'),
      t('mcp__hollow_computer__quit_app', 'q4'),
    ];
    const items = foldTools(tools);
    expect(items.map(i => i.kind)).toEqual(['single', 'single', 'single', 'single']);
    expect(items[1]).toMatchObject({ kind: 'single', index: 1 });

    const longer = foldTools([...tools, t('mcp__hollow_computer__quit_app', 'q5')]);
    // q3..q5 is a run of three after the approval split.
    expect(longer.map(i => i.kind)).toEqual(['single', 'single', 'group']);
    expect(longer[2]).toMatchObject({ kind: 'group', start: 2 });
  });

  it('folds three calls to one MCP server', () => {
    const items = foldTools([
      t('mcp__hollow_vault__search_vault', 'v1'),
      t('mcp__hollow_vault__search_vault', 'v2'),
      t('mcp__hollow_vault__search_vault', 'v3'),
    ]);
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe('group');
    expect(groupLabel('mcp:hollow_vault', 3)).toBe('3 vault calls');
  });

  it('keys id-less calls by position, never colliding', () => {
    const items = foldTools([{ name: 'Read' }, { name: 'Bash' }]);
    expect(new Set(items.map(i => i.key)).size).toBe(2);
  });
});

describe('row keys', () => {
  it('keeps a growing group under the key of its first call', () => {
    const a = t('Read', 'a');
    const b = t('Read', 'b');
    const c = t('Read', 'c');
    const d = t('Read', 'd');
    const three = foldTools([a, b, c]);
    const four = foldTools([a, b, c, d]);
    expect(three).toHaveLength(1);
    expect(four).toHaveLength(1);
    expect(four[0].key).toBe(three[0].key);
    expect(four[0].key).toBe('g:a');
    // Two singles have their own keys, distinct from the group they become.
    expect(foldTools([a, b]).map(i => i.key)).toEqual(['t:a', 't:b']);
  });

  it('never repeats a key, even when a call id repeats', () => {
    const items = foldTools([t('Read', 'x'), t('Bash', 'x'), t('Grep', 'x')]);
    expect(items.map(i => i.key)).toEqual(['t:x', 't:x~1', 't:x~2']);
  });
});

describe('windowRange', () => {
  it('renders the visible rows plus overscan, clamped to the list', () => {
    expect(windowRange(0, 420, 28, 1000, 20)).toEqual({ start: 0, end: 35 });
    expect(windowRange(28 * 500, 420, 28, 1000, 20)).toEqual({ start: 480, end: 535 });
    expect(windowRange(28 * 990, 420, 28, 1000, 20)).toEqual({ start: 970, end: 1000 });
    expect(windowRange(28 * 5000, 420, 28, 1000, 20)).toEqual({ start: 1000, end: 1000 });
    expect(windowRange(-50, 420, 28, 10, 20)).toEqual({ start: 0, end: 10 });
    expect(windowRange(0, 420, 28, 0, 20)).toEqual({ start: 0, end: 0 });
  });
});
