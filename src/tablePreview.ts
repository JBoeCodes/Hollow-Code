/**
 * Bounded table previews (next-level P08). Parses CSV/TSV completely, so the
 * row count and numeric column totals cover every row, then renders only a
 * window of them. Totals and exports never depend on what is on screen.
 * Pure, unit-tested.
 */

export interface ParsedTable { header: string[]; rows: string[][]; totals: (number | null)[]; numeric: boolean[]; truncatedCells: boolean }

/** RFC 4180-style parse: quoted fields, doubled quotes, newlines in quotes. */
export function parseDelimited(text: string, delimiter = ','): string[][] {
  const out: string[][] = [];
  let row: string[] = [], field = '', quoted = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"') { if (text[i + 1] === '"') { field += '"'; i++; } else quoted = false; }
      else field += c;
    } else if (c === '"' && field === '') quoted = true;
    else if (c === delimiter) { row.push(field); field = ''; }
    else if (c === '\n' || c === '\r') {
      if (c === '\r' && text[i + 1] === '\n') i++;
      row.push(field); field = '';
      if (row.length > 1 || row[0] !== '') out.push(row);
      row = [];
    } else field += c;
  }
  if (field !== '' || row.length) { row.push(field); if (row.length > 1 || row[0] !== '') out.push(row); }
  return out;
}

const NUMBER = /^-?\(?[$€£]?\s*-?\d[\d,]*(\.\d+)?\)?%?$/;
export function toNumber(cell: string): number | null {
  const t = cell.trim();
  if (!t || !NUMBER.test(t)) return null;
  const neg = /^\(.*\)$/.test(t) || t.startsWith('-');
  const n = Number(t.replace(/[()$€£,%\s-]/g, ''));
  return Number.isFinite(n) ? (neg ? -n : n) : null;
}

export function summarizeTable(text: string, name: string): ParsedTable {
  const delimiter = /\.tsv$/i.test(name) ? '\t' : ',';
  const all = parseDelimited(text, delimiter);
  const header = all[0] ?? [];
  const rows = all.slice(1);
  // Not Math.max(...rows): hundreds of thousands of rows overflow the stack.
  const width = rows.reduce((w, r) => Math.max(w, r.length), header.length);
  // Each cell once, in row order (run 15 P1 second pass): every column for
  // every row was width x rows steps, billions for a very wide, long file.
  const sums = new Array<number>(width).fill(0);
  const nums = new Array<number>(width).fill(0);
  const filled = new Array<number>(width).fill(0);
  for (const r of rows) {
    for (let c = 0; c < r.length; c++) {
      const v = r[c];
      if (v.trim() === '') continue;
      filled[c]++;
      const n = toNumber(v);
      if (n != null) { sums[c] += n; nums[c]++; }
    }
  }
  const numeric = filled.map((f, c) => f > 0 && nums[c] === f);
  const totals = numeric.map((isNum, c) => (isNum ? Math.round(sums[c] * 1e6) / 1e6 : null));
  return { header, rows, totals, numeric, truncatedCells: rows.some(r => r.some(c => c.length > 2000)) };
}

/** A table preview draws at most this many columns, and rows only while the
 *  drawn cells stay within TABLE_CELLS_SHOWN (run 15 P1 second pass): a very
 *  wide or very long file fits in a 1.5 MB read, and drawn whole it built
 *  millions of cells and froze the window. Counts and totals still cover
 *  the whole file. */
export const TABLE_COLUMNS_SHOWN = 100;
export const TABLE_CELLS_SHOWN = 200_000;

export interface TableWindow { columns: number; hiddenColumns: number; maxRows: number }

/** How much of `table` a preview may draw: its first TABLE_COLUMNS_SHOWN
 *  columns, and at most `maxRows` rows (never fewer than one `page`). */
export function tableWindow(table: Pick<ParsedTable, 'header'>, page: number): TableWindow {
  const columns = Math.min(table.header.length, TABLE_COLUMNS_SHOWN);
  return { columns, hiddenColumns: table.header.length - columns, maxRows: Math.max(page, Math.floor(TABLE_CELLS_SHOWN / Math.max(1, columns))) };
}

/** One row as drawn: padded with empty cells, or cut, to `columns`. */
export function drawnRow(row: readonly string[], columns: number): string[] {
  return Array.from({ length: columns }, (_, i) => row[i] ?? '');
}
