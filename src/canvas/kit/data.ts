/** Pure data helpers for the kit's data components (`hc-bars`, `hc-chart`,
 *  `hc-table`) and `hc-file`. No DOM, so Vitest covers them directly.
 *
 *  **Data rows (kit@1).** A data component reads either a `<table>` child or
 *  plain text, one row per line, which stays readable when the kit script
 *  doesn't run and costs fewer tokens than markup:
 *    - `Label: 12` or `Label: 12, 8` (values after the last colon, separated
 *      by a comma and a space, `;` or `|`); or
 *    - `Label | 12 | 8` (markdown-style; outer pipes and `---` rule lines are
 *      ignored).
 *  `1,200` (a comma between digits) is one number. */

export type Rows = string[][];

/** Lines that are only a markdown table rule (`|---|:--:|`). */
const RULE = /^\|?[\s:|-]*-[\s:|-]*$/;

export function splitRow(line: string): string[] {
  const t = line.trim();
  if (t.includes('|')) return t.replace(/^\|/, '').replace(/\|$/, '').split('|').map(c => c.trim());
  const colon = t.lastIndexOf(':');
  if (colon > 0) {
    const label = t.slice(0, colon).trim();
    const rest = t.slice(colon + 1).trim();
    if (rest) return [label, ...rest.split(/\s*;\s*|,\s+/).map(c => c.trim())];
  }
  return [t];
}

export function parseRows(text: string): Rows {
  const rows: Rows = [];
  for (const line of text.split(/\r?\n/)) {
    if (!line.trim() || RULE.test(line.trim())) continue;
    rows.push(splitRow(line));
  }
  return rows;
}

/** A cell that is a number, perhaps with a currency sign, grouping commas,
 *  a percent sign or a short unit (`$1,200`, `-3.5%`, `12 h`, `−4`). A
 *  leading word is not a prefix: `Q3` and `Mon 12` are text. */
const NUMERIC = /^[^\p{L}\d\s+\-−.]{0,2}\s*[+\-−]?\s*(?:\d{1,3}(?:,\d{3})+|\d+)?(?:\.\d+)?\s*(?:%|[a-zA-Zµ°/]{1,6})?$/u;

export function isNumeric(cell: string): boolean {
  const t = cell.trim();
  return /\d/.test(t) && NUMERIC.test(t);
}

/** The number in a numeric cell, or null (empty, a dash, or text). */
export function parseNum(cell: string | null | undefined): number | null {
  if (cell == null || !isNumeric(cell)) return null;
  const t = cell.trim().replace(/−/g, '-').replace(/(\d),(?=\d{3}\b)/g, '$1');
  const m = /([+-]?)\s*(\d*\.?\d+)/.exec(t);
  if (!m) return null;
  const n = Number(m[1] + m[2]);
  return Number.isFinite(n) ? n : null;
}

/** A leading currency or other symbol every value shares (`$`, `€`), so axis
 *  ticks can carry it too. */
export function sharedPrefix(cells: string[]): string {
  let prefix: string | null = null;
  for (const c of cells) {
    if (!c.trim()) continue;
    const p = (/^[^\p{L}\d\s+\-−.]{0,2}/u.exec(c.trim()) || [''])[0];
    if (prefix === null) prefix = p;
    else if (prefix !== p) return '';
  }
  return prefix || '';
}

export interface Series { name: string; values: Array<number | null>; cells: string[] }
export interface ChartData {
  /** Header cell over the category column ('' when there was no header). */
  head: string;
  categories: string[];
  series: Series[];
}

/** Turn rows into categories × series. The first row is a header when it has
 *  more than one cell and any cell after the first is not a number (a
 *  `<table>` source says so with `<th>` cells: `hasHeader`); the `names`
 *  (from a `series` attribute) win over header cells. */
export function toChartData(rows: Rows, names: string[] = [], fallbackName = 'Value', hasHeader?: boolean): ChartData {
  let head = '';
  let header: string[] | null = null;
  let body = rows;
  if (hasHeader ?? (rows.length > 1 && rows[0].length > 1 && rows[0].slice(1).some(c => !isNumeric(c)))) {
    header = rows[0];
    head = header[0] || '';
    body = rows.slice(1);
  }
  const width = Math.max(1, ...body.map(r => r.length - 1), header ? header.length - 1 : 0);
  const series: Series[] = [];
  for (let s = 0; s < width; s++) {
    const name = names[s] || (header && header[s + 1]) || (width === 1 ? fallbackName : `Series ${s + 1}`);
    const cells = body.map(r => (r[s + 1] ?? '').trim());
    series.push({ name, cells, values: cells.map(parseNum) });
  }
  return { head, categories: body.map(r => r[0] || ''), series };
}

/** A cell as shown: the author's text, plus `unit` when it ends in a digit. */
export function withUnit(cell: string, unit: string): string {
  if (!cell) return '—';
  return unit && /\d$/.test(cell) ? `${cell}${unit === '%' ? '' : ' '}${unit}` : cell;
}

/** Split a `series="A, B"` attribute (or `A | B`). */
export function splitNames(attr: string | null): string[] {
  const t = (attr || '').trim();
  if (!t) return [];
  return t.split(t.includes('|') ? '|' : ',').map(s => s.trim());
}

export interface Ticks { lo: number; hi: number; step: number; ticks: number[] }

/** Round axis bounds and about `count` evenly spaced ticks covering
 *  [min, max]. Always a non-empty range. */
export function niceTicks(min: number, max: number, count = 4): Ticks {
  if (!Number.isFinite(min) || !Number.isFinite(max)) { min = 0; max = 1; }
  if (min > max) [min, max] = [max, min];
  if (min === max) { if (min === 0) max = 1; else if (min > 0) min = 0; else max = 0; }
  const raw = (max - min) / Math.max(1, count);
  const mag = Math.pow(10, Math.floor(Math.log10(raw)));
  const norm = raw / mag;
  const step = (norm < 1.5 ? 1 : norm < 3.5 ? 2 : norm < 7.5 ? 5 : 10) * mag;
  const lo = Math.floor(min / step + 1e-9) * step;
  const hi = Math.ceil(max / step - 1e-9) * step;
  const ticks: number[] = [];
  for (let v = lo; v <= hi + step / 2; v += step) ticks.push(Number(v.toPrecision(12)));
  return { lo, hi, step, ticks };
}

/** Linear map of [d0, d1] onto [r0, r1]. */
export function scale(d0: number, d1: number, r0: number, r1: number): (v: number) => number {
  const k = d1 === d0 ? 0 : (r1 - r0) / (d1 - d0);
  return v => r0 + (v - d0) * k;
}

/** Compact tick label: 1,200 → 1.2K. */
export function formatTick(v: number, prefix = '', unit = ''): string {
  let s: string;
  try {
    s = new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(v);
  } catch {
    s = String(Math.round(v * 10) / 10);
  }
  return prefix + s + (unit === '%' ? '%' : '');
}

/** Donut slices: positive values only, at most `max` segments; the tail
 *  folds into "Other" (in authored order, never by rank). */
export function donutSlices(categories: string[], values: Array<number | null>, max = 6): Array<{ label: string; value: number; other: boolean }> {
  const all = categories.map((label, i) => ({ label, value: values[i] ?? 0, other: false })).filter(s => s.value > 0);
  if (all.length <= max) return all;
  const keep = all.slice(0, max - 1);
  const rest = all.slice(max - 1).reduce((a, s) => a + s.value, 0);
  return [...keep, { label: 'Other', value: rest, other: true }];
}

/** Sort comparator for a table column: numeric when every non-empty cell is a
 *  number, otherwise natural text order. Empty cells sort last either way. */
export function columnCompare(cells: string[]): (a: string, b: string) => number {
  const filled = cells.filter(c => c.trim());
  const numeric = filled.length > 0 && filled.every(isNumeric);
  return (a, b) => {
    const ea = !a.trim();
    const eb = !b.trim();
    if (ea || eb) return ea === eb ? 0 : ea ? 1 : -1;
    if (numeric) return (parseNum(a) ?? 0) - (parseNum(b) ?? 0);
    return a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' });
  };
}

/** Sort row indices by one column; stable, and empties stay last when
 *  descending too. */
export function sortOrder(cells: string[], dir: 'ascending' | 'descending'): number[] {
  const cmp = columnCompare(cells);
  const sign = dir === 'ascending' ? 1 : -1;
  return cells.map((_, i) => i).sort((i, j) => {
    const ei = !cells[i].trim();
    const ej = !cells[j].trim();
    if (ei !== ej) return ei ? 1 : -1;
    return sign * cmp(cells[i], cells[j]) || i - j;
  });
}

/** A vault path `hc-file` may hand to the bridge's `navigate`, or null. Never
 *  a URL: anything with a scheme (`https:`, `file:`, `javascript:`, `C:`) or a
 *  network path is refused, so the element can't become a web link. The host
 *  still resolves and checks the path itself. */
export function vaultPath(raw: string | null | undefined): string | null {
  const p = (raw || '').trim();
  if (!p || p.length > 1024 || /[\u0000-\u001f\u007f]/.test(p)) return null;
  if (/^[a-z][a-z0-9+.-]*:/i.test(p) || /^[\\/]{2}/.test(p)) return null;
  return p;
}

export function baseName(path: string): string {
  return path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || path;
}

/** Strip blank edge lines and the common indentation from a code block. */
export function dedent(text: string): string {
  const lines = text.replace(/\t/g, '  ').split(/\r?\n/);
  while (lines.length && !lines[0].trim()) lines.shift();
  while (lines.length && !lines[lines.length - 1].trim()) lines.pop();
  const indent = Math.min(...lines.filter(l => l.trim()).map(l => /^ */.exec(l)![0].length));
  return lines.map(l => l.slice(Number.isFinite(indent) ? indent : 0)).join('\n');
}

export type CodeToken = [kind: '' | 'comment' | 'string' | 'number' | 'keyword', text: string];

const KEYWORDS = new Set(('async await break case catch class const def elif else export false fn for from function if import '
  + 'in let new none null pass pub return self struct this true try type use var while').split(' '));
const HASH_LANGS = /^(py|python|sh|bash|zsh|shell|console|rb|ruby|ya?ml|toml|r|pl|perl|make|makefile|dockerfile|ini|conf)$/i;

/** A deliberately small highlighter: comments, strings, numbers and common
 *  keywords across languages, coloured with the `--app-syn-*` tokens. Text
 *  only: the caller renders each token with textContent. `lang="text"` (or
 *  none at all) skips highlighting. */
export function highlight(code: string, lang: string): CodeToken[] {
  const l = lang.trim().toLowerCase();
  if (!l || l === 'text' || l === 'txt' || l === 'plain') return [['', code]];
  const comment = HASH_LANGS.test(l) ? '#[^\\n]*' : '\\/\\/[^\\n]*|\\/\\*[\\s\\S]*?\\*\\/|<!--[\\s\\S]*?-->';
  const re = new RegExp(`(${comment})|("(?:\\\\.|[^"\\\\\\n])*"|'(?:\\\\.|[^'\\\\\\n])*'|\`(?:\\\\.|[^\`\\\\])*\`)|(\\b\\d[\\d_]*(?:\\.\\d+)?(?:e[+-]?\\d+)?\\b)|([A-Za-z_]\\w*)`, 'g');
  const out: CodeToken[] = [];
  let at = 0;
  const push = (kind: CodeToken[0], text: string) => {
    if (!text) return;
    const last = out[out.length - 1];
    if (last && last[0] === kind) last[1] += text;
    else out.push([kind, text]);
  };
  for (let m = re.exec(code); m; m = re.exec(code)) {
    push('', code.slice(at, m.index));
    const kind = m[1] ? 'comment' : m[2] ? 'string' : m[3] ? 'number' : KEYWORDS.has(m[4].toLowerCase()) ? 'keyword' : '';
    push(kind, m[0]);
    at = m.index + m[0].length;
  }
  push('', code.slice(at));
  return out;
}
