import { describe, expect, it } from 'vitest';
import { cellText, keepTableScroll, tableLabel, tableToTsv } from './replyTable';

describe('a reply table on the clipboard', () => {
  it('writes tab-separated rows, header first, with no trailing newline', () => {
    expect(tableToTsv([['Option', 'Cost'], ['Fly', '$12'], ['Render', '$19']])).toBe('Option\tCost\nFly\t$12\nRender\t$19');
  });

  it('keeps a cell on one line: tabs and line breaks become single spaces', () => {
    expect(cellText('  two\tlines\n  here  ')).toBe('two lines here');
    expect(tableToTsv([['a\tb', 'c\nd']])).toBe('a b\tc d');
  });

  it('keeps empty cells so the columns still line up', () => {
    expect(tableToTsv([['a', '', 'c'], ['', '', '']])).toBe('a\t\tc\n\t\t');
    expect(cellText(null)).toBe('');
    expect(cellText(undefined)).toBe('');
  });

  it('keeps a table scrolled where it was when the markup is rebuilt', () => {
    // Stand-ins for the scroll boxes the view makes: one offset and one listener each.
    const box = () => {
      const listeners: (() => void)[] = [];
      return { scrollLeft: 0, addEventListener: (_: string, fn: () => void) => { listeners.push(fn); }, scroll(to: number) { this.scrollLeft = to; listeners.forEach(fn => fn()); } };
    };
    const rootOf = (boxes: ReturnType<typeof box>[]) => ({ querySelectorAll: () => boxes }) as unknown as ParentNode;
    const saved: number[] = [];
    const first = [box(), box()];
    keepTableScroll(rootOf(first), saved);
    expect(first.map(b => b.scrollLeft)).toEqual([0, 0]);
    first[1].scroll(140);
    expect(saved).toEqual([undefined, 140]);
    // The next flush replaces every box: the second table is put back, the first left alone.
    const second = [box(), box(), box()];
    keepTableScroll(rootOf(second), saved);
    expect(second.map(b => b.scrollLeft)).toEqual([0, 140, 0]);
    // A box that is scrolled again keeps its own place for the flush after.
    second[0].scroll(30);
    expect(saved).toEqual([30, 140]);
    const third = [box(), box(), box()];
    keepTableScroll(rootOf(third), saved);
    expect(third.map(b => b.scrollLeft)).toEqual([30, 140, 0]);
  });

  it('numbers a table only when the reply has several', () => {
    expect(tableLabel('Copy table', 0, 1)).toBe('Copy table');
    expect(tableLabel('Copy table', 1, 3)).toBe('Copy table 2 of 3');
    expect(tableLabel('Table', 0, 2)).toBe('Table 1 of 2');
  });
});
