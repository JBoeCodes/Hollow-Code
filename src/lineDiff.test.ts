import { describe, expect, it } from 'vitest';
import { changedOnly, lineDiff } from './lineDiff';

describe('line diff (master plan #17)', () => {
  it('marks what was only then and what is only now', () => {
    const d = lineDiff('# Plan\n- [ ] a\n- [ ] b', '# Plan\n- [x] a\n- [ ] b\n- [ ] c');
    expect(d.map(l => `${l.kind[0]}${l.text}`)).toEqual(['s# Plan', 'r- [ ] a', 'a- [x] a', 's- [ ] b', 'a- [ ] c']);
  });
  it('keeps one line of context around changes', () => {
    const d = changedOnly(lineDiff('1\n2\n3\n4\n5', '1\n2\nX\n4\n5'));
    expect(d.map(l => l.text)).toEqual(['2', '3', 'X', '4']);
  });
  it('stays bounded on huge inputs', () => {
    const big = Array.from({ length: 3000 }, (_, i) => String(i)).join('\n');
    const d = lineDiff(big, 'x');
    expect(d.filter(l => l.kind === 'added')).toHaveLength(1);
  });
});
