import { describe, expect, it } from 'vitest';
import { TABLE_CELLS_SHOWN, TABLE_COLUMNS_SHOWN, drawnRow, parseDelimited, summarizeTable, tableWindow, toNumber } from './tablePreview';

// Run 15 P1 second pass: a very wide or very long file fits in the Preview
// pane's 1.5 MB read (a 100,000-column header is about 600 KB) and was drawn
// whole, padded to the header on every row (20 million cells froze the
// window), "Show more" had no end, and the totals visited every column of
// every row (width x rows steps).
describe('the table window (run 15 P1 second pass)', () => {
  const header = (n: number) => Array.from({ length: n }, (_, i) => `c${i}`).join(',');
  it('draws at most TABLE_COLUMNS_SHOWN columns and says how many it leaves out', () => {
    const wide = summarizeTable([header(100_000), ...Array.from({ length: 200 }, (_, i) => String(i))].join('\n'), 'wide.csv');
    const view = tableWindow(wide, 200);
    expect(view.columns).toBe(TABLE_COLUMNS_SHOWN);
    expect(view.hiddenColumns).toBe(100_000 - TABLE_COLUMNS_SHOWN);
    expect(drawnRow(wide.rows[0], view.columns)).toHaveLength(TABLE_COLUMNS_SHOWN);
  });
  it('rows stop at the cell limit, never below one page', () => {
    expect(tableWindow({ header: Array(100).fill('h') }, 200).maxRows * 100).toBeLessThanOrEqual(TABLE_CELLS_SHOWN);
    expect(tableWindow({ header: Array(100_000).fill('h') }, 200).maxRows * TABLE_COLUMNS_SHOWN).toBeLessThanOrEqual(TABLE_CELLS_SHOWN);
    expect(tableWindow({ header: ['only'] }, 200).maxRows).toBe(TABLE_CELLS_SHOWN);
    expect(tableWindow({ header: [] }, 200).maxRows).toBe(TABLE_CELLS_SHOWN);
    expect(tableWindow({ header: ['a', 'b', 'c'] }, 200)).toEqual({ columns: 3, hiddenColumns: 0, maxRows: 66_666 });
  });
  it('a drawn row is padded with empty cells, or cut, to the drawn columns', () => {
    expect(drawnRow(['1'], 3)).toEqual(['1', '', '']);
    expect(drawnRow(['1', '2', '3', '4'], 3)).toEqual(['1', '2', '3']);
  });
  it('totals and number columns are the same for ragged rows, blanks and text', () => {
    const t = summarizeTable('a,b,c,d\n1,x,,5\n2\n3,y,"1,000",\n,z,2,6,9', 'r.csv');
    expect(t.numeric).toEqual([true, false, true, true, true]);
    expect(t.totals).toEqual([6, null, 1002, 11, 9]);
  });
  it('a very wide and very long file is summarised (each cell once) without overflowing the stack', () => {
    const t = summarizeTable([header(100_000), ...Array.from({ length: 400_000 }, () => '1')].join('\n'), 'big.csv');
    expect(t.rows).toHaveLength(400_000);
    expect(t.totals[0]).toBe(400_000);
    expect(t.numeric[1]).toBe(false);
    expect(t.totals).toHaveLength(100_000);
  });
});

describe('table previews (next-level P08)', () => {
  it('parses quotes, doubled quotes and newlines in fields', () => {
    expect(parseDelimited('a,b\n"x, y","say ""hi"""\n"multi\nline",2\n')).toEqual([['a', 'b'], ['x, y', 'say "hi"'], ['multi\nline', '2']]);
  });
  it('totals cover every row, not the rendered window', () => {
    const lines = ['region,revenue,note'];
    for (let i = 0; i < 5000; i++) lines.push(`R${i},${i % 2 ? '"1,000"' : '2000'},n`);
    const t = summarizeTable(lines.join('\n'), 'big.csv');
    expect(t.rows.length).toBe(5000);
    expect(t.numeric).toEqual([false, true, false]);
    expect(t.totals[1]).toBe(2500 * 1000 + 2500 * 2000);
  });
  it('reads money, percents and accounting negatives', () => {
    expect(toNumber('$1,234.50')).toBe(1234.5);
    expect(toNumber('(200)')).toBe(-200);
    expect(toNumber('-6.2%')).toBe(-6.2);
    expect(toNumber('Q3')).toBeNull();
  });
  it('uses tabs for .tsv', () => {
    expect(summarizeTable('a\tb\n1\t2', 'x.tsv').header).toEqual(['a', 'b']);
  });
});
