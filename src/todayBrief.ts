// Today (overnight 10-11 A3): a morning briefing that greets you. Pure
// helpers: the greeting, the deterministic sections (meetings, due and
// overdue tasks, commitments due, waiting, last night) and the facts digest
// the agent's paragraph is written from (`today_brief.rs`).
import { allDayCovers, dayBounds, type AgendaEvent } from './agenda';
import type { AgendaEventsView } from './agendaTypes';
import { CALENDAR_SIGN_IN_LABEL, calendarNeedsSignIn } from './dayRing';
import type { Commitment } from './types';
import type { Task } from './generated/ipc';
import { isOpenTask } from './taskStatus';
import { localDateKey } from './utils';
import { isAwake } from './commitmentAwake';

export interface TodayRecommendation { title: string; why: string; ask: string }
export interface TodayBrief { date: string; paragraph: string; recommendations: TodayRecommendation[]; generatedAt: string; path: string }
export interface TodayCached { brief: TodayBrief | null; firstName: string | null }

/** "Good morning, Alex". */
export function greeting(now: Date, name?: string | null): string {
  const h = now.getHours();
  const part = h < 5 ? 'Up late' : h < 12 ? 'Good morning' : h < 17 ? 'Good afternoon' : 'Good evening';
  return name ? `${part}, ${name}` : part;
}

/** "Sunday, October 11". */
export function longDate(now: Date): string {
  return now.toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' });
}

/** A YYYY-MM-DD day as "Monday, November 2" (Today's header reads the day
 *  the facts are for, which the minute tick moves at midnight: P0-03). */
export function dayLongDate(key: string): string {
  return longDate(new Date(`${key}T12:00:00`));
}

export interface TodayMeeting { id: string; title: string; start: string; end: string; time: string; soon: boolean; past: boolean; allDay: boolean; organizer: string | null; joinUrl: string | null }

const timeOf = (iso: string) => new Date(iso).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

/** Today's meetings in order: not cancelled and on today (local): an all-day
 *  event by its dates (Graph sends them as UTC midnights, so never through a
 *  local time; a multi-day one counts on each of its days), a timed one when
 *  it overlaps today, midnight to midnight (a calendar day: 25 hours on the
 *  autumn DST change, so never "midnight plus 24 hours", P0-03). All-day ones first. `soon`: starts within the hour;
 *  `past`: already over. */
export function todaysMeetings(events: readonly AgendaEvent[], now: Date): TodayMeeting[] {
  const today = localDateKey(now);
  const bounds = dayBounds(today);
  const dayStart = bounds.start.getTime();
  const dayEnd = bounds.end.getTime();
  const onToday = (e: AgendaEvent) => e.allDay
    ? allDayCovers(e, today)
    : new Date(e.start).getTime() < dayEnd && new Date(e.end).getTime() > dayStart;
  return events
    .filter(e => !e.cancelled && onToday(e))
    .sort((a, b) => Number(b.allDay) - Number(a.allDay) || a.start.localeCompare(b.start))
    .map(e => {
      const start = new Date(e.start).getTime();
      const end = new Date(e.end).getTime();
      return {
        id: e.id, title: e.title || '(no title)', start: e.start, end: e.end,
        time: e.allDay ? 'All day' : timeOf(e.start),
        soon: !e.allDay && start > now.getTime() && start - now.getTime() <= 60 * 60_000,
        past: !e.allDay && end <= now.getTime(),
        allDay: e.allDay, organizer: e.organizer ?? null, joinUrl: e.joinUrl ?? null,
      };
    });
}

/** Open tasks due today, and overdue ones (oldest first). */
export function dueBuckets(tasks: readonly Task[], today: string): { overdue: Task[]; today: Task[] } {
  const open = tasks.filter(t => t.due && isOpenTask(t.status));
  const day = (t: Task) => (t.due ?? '').slice(0, 10);
  return {
    overdue: open.filter(t => day(t) < today).sort((a, b) => day(a).localeCompare(day(b))),
    today: open.filter(t => day(t) === today),
  };
}

/** Open commitments due today or earlier, overdue first. */
export function commitmentsDue(list: readonly Commitment[], today: string): Commitment[] {
  return list
    .filter(c => isAwake(c, today) && c.due && c.due.slice(0, 10) <= today)
    .sort((a, b) => (a.due ?? '').localeCompare(b.due ?? ''));
}

export interface TodayFacts {
  date: string;
  meetings: TodayMeeting[];
  overdue: Task[];
  dueToday: Task[];
  commitments: Commitment[];
  /** `ids` (same order as `titles`) open each item in Review; the brief
   *  and its fingerprint read titles only. */
  waiting: { total: number; titles: string[]; ids?: string[] };
  overnight: string[];
  /** How much of each source Hollow could read (P0-04): an unread source is
   *  unknown, never an empty list. */
  coverage: TodayCoverage;
}

/** One source's read: `ok`, `partial` (what was read is real, but not all
 *  of it), `unavailable` (the read failed), `not-connected`, or `loading`
 *  (no answer yet, never shown as a failure), with a reason. */
export interface SourceCoverage { state: 'ok' | 'partial' | 'unavailable' | 'not-connected' | 'loading'; reason: string | null }
export interface TodayCoverage { calendar: SourceCoverage; commitments: SourceCoverage; overnight: SourceCoverage; waiting: SourceCoverage }

const OK: SourceCoverage = { state: 'ok', reason: null };
const short = (s: string) => (s.length > 160 ? `${s.slice(0, 159)}…` : s);

/** The calendar read as coverage. `readError`: the read itself rejected.
 *  `loading`: the first read hasn't answered yet. An expired Microsoft 365
 *  sign-in comes back as an empty, errored view. */
export function calendarCoverageOf(view: AgendaEventsView | null | undefined, readError: string | null | undefined, loading = false): SourceCoverage {
  if (!view && !readError && loading) return { state: 'loading', reason: 'still being read' };
  if (!view) return { state: 'unavailable', reason: readError ? short(readError) : 'the read failed' };
  if (!view.connected) return view.error || view.complete === false
    ? { state: 'unavailable', reason: view.error ?? view.status ?? 'Calendar unavailable' }
    : { state: 'not-connected', reason: view.status ?? 'no calendar connected' };
  if (view.error) return { state: 'unavailable', reason: calendarNeedsSignIn(view.error) ? CALENDAR_SIGN_IN_LABEL : short(view.error) };
  if (view.complete === false) return { state: 'partial', reason: view.partial ? short(view.partial) : 'the read stopped early' };
  return OK;
}

/** Every source's coverage, from what the store read. */
export function todayCoverage(i: {
  calendar: AgendaEventsView | null | undefined; calendarError: string | null; calendarLoading?: boolean;
  commitmentsError: string | null; overnightError: string | null;
  waitingFailed: boolean; waitingUnavailable: readonly string[];
}): TodayCoverage {
  const failed = (e: string | null): SourceCoverage => (e ? { state: 'unavailable', reason: short(e) } : OK);
  return {
    calendar: calendarCoverageOf(i.calendar, i.calendarError, i.calendarLoading),
    commitments: failed(i.commitmentsError),
    overnight: failed(i.overnightError),
    waiting: i.waitingFailed ? { state: 'unavailable', reason: 'the read failed' }
      : i.waitingUnavailable.length ? { state: 'partial', reason: `couldn't read ${i.waitingUnavailable.join(', ')}` } : OK,
  };
}

/** A section's header line for the model: a count only when the source was read. */
export function coverageHeader(title: string, count: number, c: SourceCoverage): string {
  if (c.state === 'ok') return `${title} (${count}):`;
  if (c.state === 'partial') return `${title} (at least ${count}; partial read: ${c.reason}):`;
  return `${title}: unknown (${c.reason})`;
}

/** What a Today card says about a source it couldn't read, or null. */
export function coverageNote(source: string, c: SourceCoverage): string | null {
  if (c.state === 'unavailable') return `Couldn't read ${source}: ${c.reason}.`;
  if (c.state === 'partial') return `Only part of ${source} was read: ${c.reason}.`;
  return null;
}

/** The facts the agent writes the paragraph from, as plain lines. */
export function factsDigest(f: TodayFacts): string {
  const lines: string[] = [];
  const cov = f.coverage;
  const task = (t: Task) => `- ${t.title}${t.due ? ` (due ${t.due.slice(0, 10)})` : ''}${t.priority ? ` [${t.priority}]` : ''}${t.projectName ? ` · ${t.projectName}` : ''}`;
  lines.push(coverageHeader('Meetings today', f.meetings.length, cov.calendar));
  for (const m of f.meetings) lines.push(`- ${m.time} ${m.title}${m.organizer ? ` (organizer ${m.organizer})` : ''}${m.past ? ' (over)' : ''}`);
  lines.push(`Overdue tasks (${f.overdue.length}):`, ...f.overdue.slice(0, 15).map(task));
  lines.push(`Tasks due today (${f.dueToday.length}):`, ...f.dueToday.slice(0, 15).map(task));
  lines.push(coverageHeader('Commitments due', f.commitments.length, cov.commitments));
  if (cov.commitments.state !== 'unavailable') for (const c of f.commitments.slice(0, 10)) lines.push(`- ${c.direction === 'owed_by_me' ? 'I owe' : 'Owed to me by'} ${c.counterparty}: ${c.title}${c.due ? ` (due ${c.due.slice(0, 10)})` : ''}`);
  lines.push(coverageHeader('Waiting on me', f.waiting.total, cov.waiting));
  if (cov.waiting.state !== 'unavailable') lines.push(...f.waiting.titles.slice(0, 8).map(t => `- ${t}`));
  lines.push(coverageHeader('Last night', f.overnight.length, cov.overnight), ...f.overnight.slice(0, 10).map(l => `- ${l}`));
  return lines.join('\n');
}

/** Whether Today writes the day's brief on its own (P1-12): open, facts in,
 *  the cache read, none for this day yet, and the window not away. Left open
 *  overnight, the minute tick crossing midnight would otherwise write from a
 *  near-empty night and cache it all day; it writes when the person is back. */
export function shouldAutoWriteBrief(s: { open: boolean; ready: boolean; cached: boolean; briefDate: string | null; day: string; writing: boolean; error: boolean; away: boolean }): boolean {
  return s.open && s.ready && s.cached && s.briefDate !== s.day && !s.writing && !s.error && !s.away;
}

/** What Today's automatic write does now (P1-12). While the window is away a
 *  write that is due is held; when the person is back, a held write reads
 *  every source again first (`refresh`), so the brief covers the night and
 *  not what was read at 00:01. Otherwise it writes when due. */
export function autoWriteStep(s: Parameters<typeof shouldAutoWriteBrief>[0], held: boolean): 'none' | 'hold' | 'write' | 'refresh' {
  if (s.away) return shouldAutoWriteBrief({ ...s, away: false }) ? 'hold' : 'none';
  if (!shouldAutoWriteBrief(s)) return 'none';
  return held ? 'refresh' : 'write';
}

/** The facts a brief was written from, as a short hash (P1-12): a meeting
 *  passing doesn't change it, new facts do. */
export function briefFingerprint(f: TodayFacts): string {
  const text = factsDigest({ ...f, meetings: f.meetings.map(m => ({ ...m, past: false })) });
  let h = 5381;
  for (let i = 0; i < text.length; i++) h = ((h << 5) + h + text.charCodeAt(i)) | 0;
  return (h >>> 0).toString(36);
}

/** One line for Home's hero when there's no paragraph yet. A source that
 *  wasn't read is named as unknown, and never makes "a clear day" (P0-04). */
export function countsLine(f: Pick<TodayFacts, 'meetings' | 'overdue' | 'dueToday' | 'commitments' | 'waiting'> & { coverage?: TodayCoverage }): string {
  const n = (count: number, one: string, many: string) => (count ? `${count} ${count === 1 ? one : many}` : '');
  // A failed read is unknown; no calendar connected just has no meetings to count.
  const unread = (c?: SourceCoverage) => c?.state === 'unavailable';
  const cov = f.coverage;
  if (cov && Object.values(cov).some(c => c.state === 'loading')) return "Reading today's facts…";
  const parts = [
    unread(cov?.calendar) ? 'calendar unknown' : n(f.meetings.filter(m => !m.past).length, 'meeting', 'meetings'),
    n(f.dueToday.length, 'task due', 'tasks due'),
    n(f.overdue.length, 'overdue', 'overdue'),
    unread(cov?.commitments) ? 'commitments unknown' : n(f.commitments.length, 'commitment due', 'commitments due'),
    unread(cov?.waiting) ? 'waiting list unknown' : n(f.waiting.total, 'thing waiting on you', 'things waiting on you'),
  ].filter(Boolean);
  return parts.length ? `${parts.join(' · ')}.` : 'A clear day: nothing scheduled or due.';
}
