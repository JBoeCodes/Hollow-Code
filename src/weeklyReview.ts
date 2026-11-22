// The weekly review (overnight 10-11 A7), pure. The screen was cut
// (overnight 11-08 S3). Facts come from Rust's `weekly_review`.
import type { ReviewLine, WeeklyReview } from './types';
import { allDayCovers, dayBounds, shiftDay, type AgendaEvent } from './agenda';
import type { SourceCoverage } from './todayBrief';
export { reviewCardDue } from './store/weeklyReview';

const overlaps = (e: AgendaEvent, d: { start: Date; end: Date }) => new Date(e.start) < d.end && new Date(e.end) > d.start;
const clock = (d: Date) => d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
// A timed event's label on one day: its start time on the day it starts;
// on a later day "until <end>" where it ends, else "continues" (RF-876: a trip
// from Sunday evening is not a Monday 6 PM meeting, for the reader or the model).
function spanTime(e: AgendaEvent, d: { start: Date; end: Date }): string {
  const start = new Date(e.start), end = new Date(e.end);
  if (start >= d.start) return clock(start);
  if (end.getTime() === d.end.getTime()) return 'until midnight';
  return end < d.end ? `until ${clock(end)}` : 'continues';
}

/** Next week's calendar by day, from its Monday: a multi-day all-day event
 *  under each day it covers (P0-03), a timed one under each day it overlaps
 *  (midnight to midnight, like Today); all-day first, then by start time.
 *  Days with nothing are left out. */
export function nextWeekByDay(events: readonly AgendaEvent[], monday: string): [string, { time: string; title: string }[]][] {
  const live = events.filter(e => !e.cancelled);
  const out: [string, { time: string; title: string }[]][] = [];
  for (let i = 0; i < 7; i++) {
    const key = shiftDay(monday, i);
    const items = [
      ...live.filter(e => e.allDay && allDayCovers(e, key)).sort((a, b) => a.start.localeCompare(b.start)).map(e => ({ time: 'all day', title: e.title })),
      ...live.filter(e => !e.allDay && overlaps(e, dayBounds(key))).sort((a, b) => new Date(a.start).getTime() - new Date(b.start).getTime())
        .map(e => ({ time: spanTime(e, dayBounds(key)), title: e.title })),
    ];
    if (items.length) out.push([new Date(`${key}T12:00:00`).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' }), items]);
  }
  return out;
}

/** ISO week `2026-W41` → the one `by` weeks away. */
export function shiftWeek(week: string, by: number): string {
  const m = /^(\d{4})-W(\d{2})$/.exec(week.trim());
  if (!m) return week;
  // Thursday of the week decides its ISO year.
  const jan4 = new Date(Date.UTC(Number(m[1]), 0, 4));
  const monday = new Date(jan4.getTime() - ((jan4.getUTCDay() + 6) % 7) * 86_400_000 + (Number(m[2]) - 1) * 7 * 86_400_000);
  const target = new Date(monday.getTime() + by * 7 * 86_400_000 + 3 * 86_400_000);
  const year = target.getUTCFullYear();
  const y4 = new Date(Date.UTC(year, 0, 4));
  const w1 = new Date(y4.getTime() - ((y4.getUTCDay() + 6) % 7) * 86_400_000);
  const n = Math.floor((target.getTime() - w1.getTime()) / (7 * 86_400_000)) + 1;
  return `${year}-W${String(n).padStart(2, '0')}`;
}

/** The ISO week a local date falls in, `2026-W41`. */
export function isoWeek(d: Date): string {
  const t = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()));
  t.setUTCDate(t.getUTCDate() + 3 - ((t.getUTCDay() + 6) % 7));
  const y4 = new Date(Date.UTC(t.getUTCFullYear(), 0, 4));
  const n = 1 + Math.round(((t.getTime() - y4.getTime()) / 86_400_000 - 3 + ((y4.getUTCDay() + 6) % 7)) / 7);
  return `${t.getUTCFullYear()}-W${String(n).padStart(2, '0')}`;
}

/** "Week 41 · 5–11 Oct" (or across months, "28 Sep – 4 Oct"). */
export function weekLabel(r: Pick<WeeklyReview, 'week' | 'monday' | 'sunday'>): string {
  const n = Number(r.week.split('-W')[1] ?? '');
  const a = new Date(`${r.monday}T12:00:00`);
  const b = new Date(`${r.sunday}T12:00:00`);
  const month = (d: Date) => d.toLocaleDateString(undefined, { month: 'short' });
  const range = a.getMonth() === b.getMonth()
    ? `${a.getDate()}–${b.getDate()} ${month(b)}`
    : `${a.getDate()} ${month(a)} – ${b.getDate()} ${month(b)}`;
  return `Week ${n} · ${range}`;
}

/** The week Home's card is about: on Monday morning, last week. */
export function cardWeekOffset(now: Date): number {
  return now.getDay() === 1 ? -1 : 0;
}

export function money(usd: number | null | undefined): string {
  if (usd == null) return '—';
  return usd >= 100 ? `$${Math.round(usd)}` : `$${usd.toFixed(2)}`;
}

/** Bar heights (0–1) for the spend per day. */
export function barScale(values: readonly number[]): number[] {
  const max = Math.max(0, ...values);
  return values.map(v => (max > 0 ? v / max : 0));
}

/** "Mon" for a `YYYY-MM-DD`. */
export function weekday(day: string): string {
  const d = new Date(`${day}T12:00:00`);
  return Number.isNaN(d.getTime()) ? day : d.toLocaleDateString(undefined, { weekday: 'short' });
}

/** The week as plain lines for the agent's take: facts only, cut short. */
export function factsDigest(r: WeeklyReview, nextWeekCalendar: readonly string[] = [], calendar: SourceCoverage = { state: 'ok', reason: null }): string {
  const block = (title: string, lines: readonly ReviewLine[] | readonly string[], max = 15) => {
    const items = (lines as readonly (ReviewLine | string)[]).slice(0, max).map(l => `- ${typeof l === 'string' ? l : l.text}`);
    return items.length ? `${title}:\n${items.join('\n')}` : `${title}: none`;
  };
  const s = r.stats;
  return [
    `Week ${r.week} (${r.monday} to ${r.sunday}).`,
    `Results accepted ${s.accepted}, tasks done ${s.tasksDone}, PRs merged ${s.prsMerged}, decisions ${s.decisions}, spend ${money(s.spendUsd)}.`,
    block('Shipped', r.shipped),
    block('Still in progress', r.inProgress, 8),
    block('Slipped (due this week and past: not done, or finished after the due day)', r.slipped),
    block('Decisions recorded', r.decisions),
    block('Commitments kept', r.kept, 8),
    block('Commitments overdue', r.overdue, 8),
    block('Due next week', r.nextWeek),
    // A calendar Hollow couldn't read is unknown, never "none" (P0-04).
    calendar.state === 'unavailable' || calendar.state === 'not-connected' || calendar.state === 'loading' ? `Next week on the calendar: unknown (${calendar.reason})`
      : block(calendar.state === 'partial' ? `Next week on the calendar (partial read: ${calendar.reason})` : 'Next week on the calendar', nextWeekCalendar, 20),
  ].join('\n\n');
}

/** Nothing at all happened: the quiet-week state. */
export function quietWeek(r: WeeklyReview): boolean {
  return !r.shipped.length && !r.inProgress.length && !r.slipped.length && !r.decisions.length && !r.kept.length && !r.made.length && !(r.stats.spendUsd ?? 0);
}
