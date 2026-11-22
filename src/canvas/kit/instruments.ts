/** Pure, bounded geometry/data for the additive kit instruments. */
import { parseNum, type Rows } from './data';

export function gaugeValue(raw: string, low: string, high: string) {
  const value = parseNum(raw);
  const min = low.trim() ? parseNum(low) : 0;
  const max = high.trim() ? parseNum(high) : 100;
  return { value, min, max, fraction: value === null || min === null || max === null || max <= min
    ? null : Math.min(1, Math.max(0, (value - min) / (max - min))) };
}

/** Separate subpaths keep unavailable samples unavailable. */
export function sparkPaths(values: Array<number | null>, w: number, h: number): string[] {
  const known = values.filter((v): v is number => v !== null && Number.isFinite(v));
  if (!known.length) return [];
  const lo = Math.min(...known), hi = Math.max(...known);
  const paths: string[] = [];
  let run = '';
  values.forEach((v, i) => {
    if (v === null || !Number.isFinite(v)) { if (run) paths.push(run); run = ''; return; }
    const x = 4 + (values.length === 1 ? (w - 8) / 2 : i * (w - 8) / (values.length - 1));
    const y = hi === lo ? h / 2 : h - 4 - (v - lo) * (h - 8) / (hi - lo);
    run += `${run ? ' L' : 'M'}${x.toFixed(2)},${y.toFixed(2)}`;
  });
  if (run) paths.push(run);
  return paths;
}

const DAY = 86400000;
export function calendarDay(s: string): number | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(s)) return null;
  const ms = Date.parse(`${s}T00:00:00Z`);
  return Number.isFinite(ms) && new Date(ms).toISOString().slice(0, 10) === s ? ms / DAY : null;
}
export const dayText = (day: number): string => new Date(day * DAY).toISOString().slice(0, 10);
export const weekday = (day: number): number => ((day + 3) % 7 + 7) % 7; // Monday = 0

export interface TimelineItem { date: string; label: string; until: string; start: number; end: number }
export function datedRows(rows: Rows, header?: boolean): Rows {
  return rows.length && (header ?? /^(date|start)$/i.test(rows[0][0])) ? rows.slice(1) : rows;
}
export function timelineRows(source: Rows, header?: boolean) {
  const rows = datedRows(source, header);
  const items: TimelineItem[] = [];
  let invalid = 0, omitted = 0;
  rows.forEach(r => {
    const start = calendarDay(r[0]), until = r[2]?.trim() || r[0], end = calendarDay(until);
    if (start === null || end === null || end < start || !r[1]?.trim()) { invalid++; return; }
    if (items.length >= 60) { omitted++; return; }
    items.push({ date: r[0], label: r[1], until, start, end });
  });
  items.sort((a, b) => a.start - b.start || a.end - b.end);
  return { items, invalid, omitted, rows };
}

export function heatmapDays(source: Rows, start: string, end: string, header?: boolean) {
  const rows = datedRows(source, header);
  const values = new Map<number, number | null>();
  let invalid = 0, duplicates = 0;
  rows.forEach(r => {
    const day = calendarDay(r[0]);
    if (day === null) { invalid++; return; }
    if (values.has(day)) duplicates++;
    values.set(day, parseNum(r[1]));
  });
  const keys = [...values.keys()];
  const first = start ? calendarDay(start) : keys.length ? Math.min(...keys) : null;
  const last = end ? calendarDay(end) : keys.length ? Math.max(...keys) : null;
  const invalidRange = (start !== '' && first === null) || (end !== '' && last === null)
    || (first !== null && last !== null && last < first);
  const days: Array<{ date: string; day: number; value: number | null }> = [];
  if (!invalidRange && first !== null && last !== null) {
    for (let day = first; day <= Math.min(last, first + 365); day++) days.push({ date: dayText(day), day, value: values.get(day) ?? null });
  }
  return { rows, days, invalid, duplicates, invalidRange, truncated: first !== null && last !== null && last - first >= 366,
    outside: keys.filter(k => !days.length || k < days[0].day || k > days[days.length - 1].day).length };
}
