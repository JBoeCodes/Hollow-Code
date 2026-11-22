import { describe, expect, it } from 'vitest';
import { distinguishing, duplicateTitleIds, workspaceOrigin, workspaceTooltip } from './workspaceOrigin';

const ws = (id: string, title: string, extra: Record<string, unknown> = {}) => ({ id, title, path: `workspaces/${id}`, build: null, ...extra }) as never;

describe('workspace origin and duplicate names (consolidation E)', () => {
  const a = ws('local-q3', 'Q3 regional operations review');
  const b = ws('notion-0123456789abcdef0123456789abcdef', 'Q3 regional operations review');
  const c = ws('q3-review', 'Q3 Regional  Operations Review');
  const d = ws('other', 'Something else');
  it('names each origin in words', () => {
    expect(workspaceOrigin(a, 'Acme')).toBe('Project folder · Acme');
    expect(workspaceOrigin(b)).toBe('Notion project');
    expect(workspaceOrigin(c)).toBe('Hollow workspace');
    expect(workspaceOrigin(ws('b1', 'x', { build: { branch: 'feat/x' } }))).toBe('Build · feat/x');
  });
  it('finds titles shared across origins, ignoring case and spacing', () => {
    expect([...duplicateTitleIds([a, b, c, d])].sort()).toEqual(['local-q3', 'notion-0123456789abcdef0123456789abcdef', 'q3-review']);
  });
  it('tells twins apart by origin, then by folder when the origin is shared', () => {
    expect(distinguishing(b, [a, b, c, d])).toBe('Notion project');
    expect(distinguishing(d, [a, b, c, d])).toBeNull();
    const twin = ws('q3-review-2', 'Q3 regional operations review');
    expect(distinguishing(c, [c, twin])).toBe('Hollow workspace · q3-review');
    expect(workspaceTooltip(a, 'Acme')).toBe('Q3 regional operations review (Project folder · Acme)');
    // The top bar's tooltip names what tells twins of one origin apart (11-06).
    expect(workspaceTooltip(c, null, distinguishing(c, [c, twin]))).toBe('Q3 Regional  Operations Review (Hollow workspace · q3-review)');
    expect(workspaceTooltip(c, null, null)).toBe('Q3 Regional  Operations Review (Hollow workspace)');
  });
  it('answers from a title index built once per list, fresh for a new list (ST2)', () => {
    const many = Array.from({ length: 2000 }, (_, i) => ws(`w${i}`, `Workspace ${i % 1000}`));
    expect(distinguishing(many[5], many)).toBe('Hollow workspace · w5');
    expect(many.filter(w => distinguishing(w, many) !== null)).toHaveLength(2000);
    // A replaced list (the stores never mutate one) is indexed again.
    const renamed = many.map((w, i) => (i === 1005 ? ws('w1005', 'A new name') : w));
    expect(distinguishing(many[5], renamed)).toBeNull();
    expect(distinguishing(renamed[1005], renamed)).toBeNull();
    expect(distinguishing(many[5], many)).toBe('Hollow workspace · w5');
  });
});
