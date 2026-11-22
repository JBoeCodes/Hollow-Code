/**
 * Agenda layout (next-level P10). Pure, unit-tested.
 *
 * Three kinds of thing share a day and never merge: calendar events (read
 * only, from the user's Microsoft 365 sign-in), local focus blocks (Hollow
 * planning) and due tasks (from task files). Everything is placed by elapsed
 * minutes from local midnight, so a daylight-saving day really has 23 or 25
 * hours and the repeated 1:30 AM of a fall-back night lands in the right
 * place. All-day events cover the local dates in [start, end). Cancelled
 * events stay visible, marked cancelled, rather than silently vanishing.
 */
import { isOpenTask } from './taskStatus';

export interface AgendaEvent {
  id: string; title: string; start: string; end: string; allDay: boolean; cancelled: boolean;
  location?: string | null; joinUrl?: string | null; webLink?: string | null; showAs?: string | null; organizer?: string | null;
}
export interface FocusBlockLike { id: string; title: string; start: string; end: string; taskUrl?: string | null; workspaceId?: string | null; updatedAt?: string }
export interface TaskLike { url: string; title: string; due: string | null; dueTime: string | null; durationMin: number | null; status: string; priority?: string | null }

export type AgendaKind = 'event' | 'focus' | 'task';
export interface Placed { kind: AgendaKind; id: string; title: string; startMin: number; endMin: number; col: number; cols: number; cancelled: boolean; ref: AgendaEvent | FocusBlockLike | TaskLike }
export interface DayLayout {
  dateKey: string;
  /** Minutes in this local day: 1440, or 1380 / 1500 across a DST change. */
  minutes: number;
  /** Hour lines: elapsed minute and its local label (1 AM may appear twice). */
  hours: { min: number; label: string }[];
  allDay: AgendaEvent[];
  timed: Placed[];
  /** Due on this day with no time. */
  dueNoTime: TaskLike[];
}

// Finished or cancelled tasks leave the day: the one rule (P1-12).
const isDone = (status: string) => !isOpenTask(status);

export const dateKey = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;

/** Local midnight of a YYYY-MM-DD date, and of the next date. */
export function dayBounds(key: string): { start: Date; end: Date } {
  const [y, m, d] = key.split('-').map(Number);
  return { start: new Date(y, m - 1, d, 0, 0, 0, 0), end: new Date(y, m - 1, d + 1, 0, 0, 0, 0) };
}

export function shiftDay(key: string, days: number): string {
  const [y, m, d] = key.split('-').map(Number);
  return dateKey(new Date(y, m - 1, d + days, 12));
}

/** A 7-day range from `key` in the locale's own range form ("Oct 22 – 28",
 *  "Oct 29 – Nov 4", years when it spans two): the Agenda's week header. */
export function weekRangeLabel(key: string): string {
  const [a, b] = [key, shiftDay(key, 6)].map(k => new Date(`${k}T12:00:00`));
  const year = a.getFullYear() !== b.getFullYear() ? { year: 'numeric' as const } : {};
  return new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', ...year }).formatRange(a, b);
}

/** The local dates an all-day event covers ([start, end) by local date). */
export function allDayCovers(e: AgendaEvent, key: string): boolean {
  const s = e.start.slice(0, 10), en = e.end.slice(0, 10);
  return s <= key && (en > key || (en === s && s === key));
}

function minutesInto(day: { start: Date; end: Date }, at: Date): number {
  return Math.round((at.getTime() - day.start.getTime()) / 60_000);
}

/** Assign overlap columns (greedy, by start then length). */
function columns(items: Omit<Placed, 'col' | 'cols'>[]): Placed[] {
  const sorted = [...items].sort((a, b) => a.startMin - b.startMin || (b.endMin - b.startMin) - (a.endMin - a.startMin));
  const out: Placed[] = [];
  let cluster: Placed[] = [];
  let clusterEnd = -Infinity;
  const flush = () => { const n = Math.max(1, ...cluster.map(c => c.col + 1)); for (const c of cluster) c.cols = n; out.push(...cluster); cluster = []; };
  for (const it of sorted) {
    if (it.startMin >= clusterEnd && cluster.length) flush();
    const used = new Set(cluster.filter(c => c.endMin > it.startMin).map(c => c.col));
    let col = 0; while (used.has(col)) col++;
    cluster.push({ ...it, col, cols: 1 });
    clusterEnd = Math.max(clusterEnd === -Infinity ? it.endMin : clusterEnd, it.endMin);
  }
  if (cluster.length) flush();
  return out;
}

export function layoutDay(key: string, events: readonly AgendaEvent[], blocks: readonly FocusBlockLike[], tasks: readonly TaskLike[]): DayLayout {
  const day = dayBounds(key);
  const minutes = minutesInto(day, day.end);
  const hours: DayLayout['hours'] = [];
  for (let t = day.start.getTime(); t < day.end.getTime(); t += 3_600_000) {
    const at = new Date(t);
    hours.push({ min: minutesInto(day, at), label: at.toLocaleTimeString(undefined, { hour: 'numeric' }) });
  }
  const clip = (s: Date, e: Date) => ({ startMin: Math.max(0, minutesInto(day, s)), endMin: Math.min(minutes, Math.max(minutesInto(day, s) + 15, minutesInto(day, e))) });
  const overlaps = (s: Date, e: Date) => s < day.end && e > day.start;
  const timed: Omit<Placed, 'col' | 'cols'>[] = [];
  const allDay: AgendaEvent[] = [];
  for (const ev of events) {
    if (ev.allDay) { if (allDayCovers(ev, key)) allDay.push(ev); continue; }
    const s = new Date(ev.start), e = new Date(ev.end);
    if (Number.isNaN(s.getTime()) || Number.isNaN(e.getTime()) || !overlaps(s, e)) continue;
    timed.push({ kind: 'event', id: ev.id, title: ev.title || '(No title)', ...clip(s, e), cancelled: !!ev.cancelled, ref: ev });
  }
  for (const b of blocks) {
    const s = new Date(b.start), e = new Date(b.end);
    if (Number.isNaN(s.getTime()) || !overlaps(s, e)) continue;
    timed.push({ kind: 'focus', id: b.id, title: b.title, ...clip(s, e), cancelled: false, ref: b });
  }
  const dueNoTime: TaskLike[] = [];
  for (const t of tasks) {
    if (!t.due || t.due.slice(0, 10) !== key || isDone(t.status)) continue;
    const time = t.dueTime ?? (t.due.length > 10 ? t.due.slice(11, 16) : null);
    if (!time) { dueNoTime.push(t); continue; }
    const [hh, mm] = time.split(':').map(Number);
    const [y, m, d] = key.split('-').map(Number);
    const s = new Date(y, m - 1, d, hh, mm || 0);
    const e = new Date(s.getTime() + Math.max(15, t.durationMin ?? 30) * 60_000);
    timed.push({ kind: 'task', id: t.url, title: t.title, ...clip(s, e), cancelled: false, ref: t });
  }
  return { dateKey: key, minutes, hours, allDay, timed: columns(timed), dueNoTime };
}

/** Free gaps of at least `need` minutes between `fromMin` and `toMin` (busy:
 *  events that aren't cancelled or free, and focus blocks). */
export function freeSlots(layout: DayLayout, need: number, fromMin = 8 * 60, toMin = 18 * 60): { startMin: number; endMin: number }[] {
  const busy = layout.timed.filter(p => p.kind !== 'task' && !p.cancelled && !(p.kind === 'event' && (p.ref as AgendaEvent).showAs === 'free'))
    .map(p => [p.startMin, p.endMin] as const).sort((a, b) => a[0] - b[0]);
  const out: { startMin: number; endMin: number }[] = [];
  let at = fromMin;
  for (const [s, e] of busy) {
    if (e <= at) continue;
    if (s - at >= need && s <= toMin) out.push({ startMin: at, endMin: Math.min(s, toMin) });
    at = Math.max(at, e);
    if (at >= toMin) break;
  }
  if (toMin - at >= need) out.push({ startMin: at, endMin: toMin });
  return out;
}

/** An elapsed minute of a local day as an instant (DST-safe). */
export function instantAt(key: string, min: number): Date {
  return new Date(dayBounds(key).start.getTime() + min * 60_000);
}

/** RFC 3339 with the local offset, for focus blocks. */
export function localIso(d: Date): string {
  const off = -d.getTimezoneOffset();
  const sign = off >= 0 ? '+' : '-';
  const hh = String(Math.floor(Math.abs(off) / 60)).padStart(2, '0'), mm = String(Math.abs(off) % 60).padStart(2, '0');
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}:00${sign}${hh}:${mm}`;
}

/** The calendar source pill for a read that worked: its freshness when every
 * page came back, "partial" with the reason when the read stopped early, so a
 * cut-off week never looks fresh (merged plan slice 5). */
export function calendarCoverage(v: { fetchedAt: string; source: string; complete?: boolean; partial?: string | null }, now = Date.now()): { label: string; tone: 'ok' | 'warn'; hint?: string } {
  if (v.complete === false) {
    return { label: `${v.source} · partial, ${freshness(v.fetchedAt, now)}`, tone: 'warn', hint: `Some events in this range may be missing: ${v.partial || 'the calendar read stopped early'}.` };
  }
  return { label: `${v.source} · ${freshness(v.fetchedAt, now)}`, tone: 'ok' };
}

/** "Fetched 3 min ago" style freshness for external data. */
export function freshness(fetchedAt: string | null | undefined, now = Date.now()): string {
  const t = Date.parse(fetchedAt ?? '');
  if (Number.isNaN(t)) return 'not fetched yet';
  const m = Math.max(0, Math.round((now - t) / 60_000));
  return m < 1 ? 'fetched just now' : m < 60 ? `fetched ${m} min ago` : `fetched ${Math.round(m / 60)} h ago`;
}
