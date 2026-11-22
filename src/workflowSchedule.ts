// Pure helpers for workflow schedules — the structured picker's model over
// the stored 7-field cron format (sec min hr day-of-month month day-of-week
// year; hours are LOCAL wall-clock, evaluated backend-side against Local).
// The stored format never changes: `manual`, or a cron string. The picker
// round-trips the shapes Hollow itself writes (daily / weekday / weekly at
// a time); anything else is preserved verbatim as `custom`.

import { savedWindowLabel } from './windowLabels';
import type { AutomationWindows } from './generated/ipc';

export const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'] as const;
export type DayName = typeof DAYS[number];

export type ScheduleKind = 'manual' | 'daily' | 'weekday' | 'weekly' | 'window' | 'custom';

export interface ScheduleModel {
  kind: ScheduleKind;
  hour: number;    // 0..23, local
  minute: number;  // 0..59
  day: DayName;    // weekly only
  raw: string;     // the original string — authoritative for `custom`
  window?: string;
  catchUp?: 'skip' | 'run-once-on-wake' | 'run-all';
}

/** Default misfire grace (minutes) — mirrors vault_workflows::DEFAULT_MISFIRE_GRACE_MIN. */
export const DEFAULT_MISFIRE_GRACE_MIN = 120;

/** Parse a stored schedule string into the picker model. Unrecognized
 *  expressions come back as `custom` with `raw` preserved byte-for-byte. */
export function parseScheduleModel(s: string): ScheduleModel {
  const raw = (s ?? '').trim();
  const base: ScheduleModel = { kind: 'custom', hour: 9, minute: 0, day: 'Mon', raw };
  if (!raw || raw === 'manual') return { ...base, kind: 'manual' };
  const window = scheduleWindow(raw);
  if (window) return { ...base, kind: 'window', window, catchUp: scheduleCatchUp(raw)! };
  const parts = raw.split(/\s+/);
  if (parts.length !== 7) return base;
  const [sec, min, hr, dom, mon, dow, yr] = parts;
  const m = Number(min);
  const h = Number(hr);
  const fixedTime =
    sec === '0' &&
    /^\d+$/.test(min) && Number.isInteger(m) && m >= 0 && m < 60 &&
    /^\d+$/.test(hr) && Number.isInteger(h) && h >= 0 && h < 24;
  if (!fixedTime || dom !== '*' || mon !== '*' || yr !== '*') return base;
  if (dow === '*') return { ...base, kind: 'daily', hour: h, minute: m };
  if (/^mon-fri$/i.test(dow)) return { ...base, kind: 'weekday', hour: h, minute: m };
  const day = DAYS.find(d => d.toLowerCase() === dow.toLowerCase());
  if (day) return { ...base, kind: 'weekly', hour: h, minute: m, day };
  return base;
}

/** Render the model back to the stored format. Structured kinds emit the
 *  exact cron shapes Hollow writes; `custom` returns `raw` untouched. */
export function renderScheduleModel(m: ScheduleModel): string {
  switch (m.kind) {
    case 'manual':  return 'manual';
    case 'window': return `window:${m.window ?? ''}/${m.catchUp ?? 'run-once-on-wake'}`;
    case 'daily':   return `0 ${m.minute} ${m.hour} * * * *`;
    case 'weekday': return `0 ${m.minute} ${m.hour} * * Mon-Fri *`;
    case 'weekly':  return `0 ${m.minute} ${m.hour} * * ${m.day} *`;
    case 'custom':  return m.raw;
  }
}

/** "8:00 AM", "6:30 PM" (P1-04: a schedule reads as a sentence, "Mondays 8:00 AM"). */
function fmtHM(hour: number, minute: number): string {
  const h12 = hour % 12 === 0 ? 12 : hour % 12;
  return `${h12}:${String(minute).padStart(2, '0')} ${hour < 12 ? 'AM' : 'PM'}`;
}

/** Whole day names for a weekly schedule: "Mondays", not "Mons" (P1-04). */
const DAY_PLURAL: Record<DayName, string> = {
  Mon: 'Mondays', Tue: 'Tuesdays', Wed: 'Wednesdays', Thu: 'Thursdays', Fri: 'Fridays', Sat: 'Saturdays', Sun: 'Sundays',
};

/** Scheduler v2: a schedule that names a window (`window:friday-pm`, see
 *  `schedule_windows.rs`) instead of a cron. Its name, or null. */
export function scheduleWindow(s: string): string | null {
  const m = WINDOW_RE.exec((s ?? '').trim());
  return m ? m[1] : null;
}

/** `window:<name>[/<catch-up>]`: the catch-up is part of the stamped schedule. */
const WINDOW_RE = /^window:\s*([a-z0-9-]+)\s*(?:\/\s*(skip|run-once-on-wake|run-all)\s*)?$/;

/** A window schedule's catch-up for runs missed while Hollow was closed. */
export function scheduleCatchUp(s: string): 'skip' | 'run-once-on-wake' | 'run-all' | null {
  const m = WINDOW_RE.exec((s ?? '').trim());
  if (!m) return null;
  return (m[2] as 'skip' | 'run-all' | undefined) ?? 'run-once-on-wake';
}

/** The built-in windows' plain-English names (Settings may add others). */
const WINDOW_LABELS: Record<string, string> = {
  'tonight': 'Tonight (weekdays 18:00–07:00)',
  'weekday-evening': 'Weekday evenings (18:00–22:00)',
  'friday-pm': 'Friday afternoon (15:00–18:00)',
  'nightly': 'Nightly (02:00–05:00)',
};

/** Compact human label for a stored schedule ("Weekdays 9:00 AM", "Mondays
 *  8:00 AM", "Daily 7:00 PM"); customs fall back to a truncated raw cron. */
export function describeSchedule(s: string): string {
  const window = scheduleWindow(s);
  if (window) {
    const label = savedWindowLabel(window) ?? WINDOW_LABELS[window] ?? `Window: ${window}`;
    const catchUp = scheduleCatchUp(s);
    return catchUp === 'run-all' ? `${label} · catches up every missed run`
      : catchUp === 'skip' ? `${label} · skips missed runs` : label;
  }
  const m = parseScheduleModel(s);
  switch (m.kind) {
    case 'manual':  return 'manual';
    case 'window': return m.raw;
    case 'daily':   return `Daily ${fmtHM(m.hour, m.minute)}`;
    case 'weekday': return `Weekdays ${fmtHM(m.hour, m.minute)}`;
    case 'weekly':  return `${DAY_PLURAL[m.day]} ${fmtHM(m.hour, m.minute)}`;
    case 'custom': {
      // Legacy preset labels for the interval crons the old chip row offered.
      if (m.raw === '0 */15 * * * * *') return 'Every 15 min';
      if (m.raw === '0 0 * * * * *') return 'Hourly';
      if (m.raw === '0 0 */4 * * * *') return 'Every 4 hours';
      return m.raw.length > 26 ? m.raw.slice(0, 25) + '…' : m.raw;
    }
  }
}

export type NextRunState = 'future' | 'catchup' | 'missed';

/** Classify a stored nextRunAt against now: still ahead, past-due within the
 *  misfire grace (the scheduler will fire it as a late catch-up within a
 *  minute of Hollow being open), or past grace (it will skip + reschedule). */
export function nextRunState(
  nextRunAt: string,
  misfireGraceMin: number | null,
  now: number = Date.now(),
): NextRunState {
  const t = new Date(nextRunAt).getTime();
  if (!Number.isFinite(t) || t > now) return 'future';
  const graceMs = (misfireGraceMin ?? DEFAULT_MISFIRE_GRACE_MIN) * 60_000;
  return now - t <= graceMs ? 'catchup' : 'missed';
}

// ── Next runs (next-level P17) ───────────────────────────────────────────
//
// A local evaluator of the backend's schedule grammar, so the editor can show
// the next three runs of an unsaved schedule without asking the backend. It
// mirrors `vault_workflows::next_run_after`, which evaluates the `cron` crate
// (0.12) against `chrono::Local`:
// - 6 or 7 fields (sec min hour day-of-month month day-of-week [year]) or a
//   `@yearly|@monthly|@weekly|@daily|@hourly` shorthand; `*`, `?` (day
//   fields), numbers, `a-b`, names (`Jan`, `Mon`, `Mon-Fri`), lists and
//   `/step`. Day of week counts 1 = Sunday … 7 = Saturday, as the crate does.
// - Day of month AND day of week must both match (the crate does not OR
//   them the way Vixie cron does).
// - A local time that does not exist (the hour clocks skip in spring) or
//   exists twice (the hour clocks repeat in autumn) is skipped for that day:
//   the crate builds each candidate with chrono's `.single()`, which is None
//   for both. The result lists such skips so the UI can say so.
// - "After" is strict: a run due exactly now is not the next one.

interface CronFields {
  sec: number[]; min: number[]; hour: number[];
  dom: Set<number>; month: Set<number>; dow: Set<number>;
  year: Set<number> | null;
}

const MONTH_NAMES: Record<string, number> = {
  jan: 1, january: 1, feb: 2, february: 2, mar: 3, march: 3, apr: 4, april: 4, may: 5, jun: 6, june: 6,
  jul: 7, july: 7, aug: 8, august: 8, sep: 9, sept: 9, september: 9, oct: 10, october: 10, nov: 11, november: 11,
  dec: 12, december: 12,
};
const DOW_NAMES: Record<string, number> = {
  sun: 1, sunday: 1, mon: 2, monday: 2, tue: 3, tues: 3, tuesday: 3, wed: 4, wednesday: 4,
  thu: 5, thurs: 5, thursday: 5, fri: 6, friday: 6, sat: 7, saturday: 7,
};

function range(lo: number, hi: number): number[] {
  return Array.from({ length: hi - lo + 1 }, (_, i) => lo + i);
}

/** One field's values, or null when the crate would reject it. */
function parseField(text: string, lo: number, hi: number, names: Record<string, number> | null, allowAny: boolean): number[] | null {
  const out = new Set<number>();
  const value = (token: string): number | null => {
    if (/^\d+$/.test(token)) return Number(token);
    const named = names?.[token.toLowerCase()];
    return named ?? null;
  };
  for (const part of text.split(',')) {
    if (!part) return null;
    const [base, stepText, extra] = part.split('/');
    if (extra !== undefined) return null;
    const step = stepText === undefined ? 1 : /^\d+$/.test(stepText) ? Number(stepText) : NaN;
    if (!Number.isFinite(step) || step < 1) return null;
    let values: number[];
    if (base === '*' || (allowAny && base === '?')) values = range(lo, hi);
    else if (base.includes('-')) {
      const [a, b, more] = base.split('-');
      if (more !== undefined) return null;
      const start = value(a); const end = value(b);
      // Names pair with names, numbers with numbers, as the crate parses them.
      if (start === null || end === null || /^\d+$/.test(a) !== /^\d+$/.test(b)) return null;
      if (start < lo || end > hi || start > end) return null;
      values = range(start, end);
    } else {
      const point = value(base);
      if (point === null || point < lo || point > hi) return null;
      // `n/step` runs from n to the field's maximum; a bare name takes no step.
      if (stepText !== undefined && !/^\d+$/.test(base)) return null;
      values = stepText === undefined ? [point] : range(point, hi);
    }
    values.forEach((v, i) => { if (i % step === 0) out.add(v); });
  }
  return [...out].sort((a, b) => a - b);
}

const SHORTHANDS: Record<string, string> = {
  '@yearly': '0 0 0 1 1 * *', '@monthly': '0 0 0 1 * * *', '@weekly': '0 0 0 * * 1 *',
  '@daily': '0 0 0 * * * *', '@hourly': '0 0 * * * * *',
};

function parseCron(expr: string): CronFields | null {
  const raw = (SHORTHANDS[expr.trim()] ?? expr).trim();
  const parts = raw.split(/\s+/);
  if (parts.length !== 6 && parts.length !== 7) return null;
  const sec = parseField(parts[0], 0, 59, null, false);
  const min = parseField(parts[1], 0, 59, null, false);
  const hour = parseField(parts[2], 0, 23, null, false);
  const dom = parseField(parts[3], 1, 31, null, true);
  const month = parseField(parts[4], 1, 12, MONTH_NAMES, false);
  const dow = parseField(parts[5], 1, 7, DOW_NAMES, true);
  const year = parts[6] === undefined ? [] : parseField(parts[6], 1970, 2100, null, false);
  if (!sec || !min || !hour || !dom || !month || !dow || !year) return null;
  return {
    sec, min, hour, dom: new Set(dom), month: new Set(month), dow: new Set(dow),
    year: parts[6] === undefined || parts[6] === '*' ? null : new Set(year),
  };
}

/** Whether the backend would accept this schedule (`manual` or a cron the
 *  crate parses). Mirrors `vault_workflows::validate_schedule`'s grammar. */
export function isValidSchedule(schedule: string): boolean {
  const s = (schedule ?? '').trim();
  return s === 'manual' || scheduleWindow(s) !== null || parseCron(s) !== null;
}

export interface Occurrence {
  at: Date;
  timeZone?: string;
  /** The zone's short name at that instant (`EDT`, `GMT+1`). */
  zoneName: string;
}

export interface SkippedTime {
  /** The local wall time that was skipped, `YYYY-MM-DD HH:MM`. */
  wall: string;
  /** `gap`: clocks skip it (spring forward); `repeat`: it happens twice. */
  reason: 'gap' | 'repeat';
}

export interface NextRuns {
  runs: Occurrence[];
  /** Matching wall times before the last listed run that will not run. */
  skipped: SkippedTime[];
  /** The IANA zone the times are computed in. */
  timeZone: string;
}

/** The local zone (IANA name), as the backend's `Local` reads it. */
export function localTimeZone(): string {
  try { return Intl.DateTimeFormat().resolvedOptions().timeZone || 'local time'; } catch { return 'local time'; }
}

export function zoneShortName(at: Date, locale?: string, timeZone?: string): string {
  try {
    return new Intl.DateTimeFormat(locale, { timeZoneName: 'short', timeZone }).formatToParts(at)
      .find(p => p.type === 'timeZoneName')?.value ?? '';
  } catch { return ''; }
}

const pad2 = (n: number) => String(n).padStart(2, '0');
const wallText = (y: number, mo: number, d: number, h: number, mi: number) => `${y}-${pad2(mo)}-${pad2(d)} ${pad2(h)}:${pad2(mi)}`;

/** Resolve a local wall time the way chrono's `.single()` does: the instant,
 *  or why there isn't exactly one. */
function resolveLocal(y: number, mo: number, d: number, h: number, mi: number, s: number): Date | 'gap' | 'repeat' {
  const t = new Date(y, mo - 1, d, h, mi, s);
  const same = (x: Date) => x.getFullYear() === y && x.getMonth() === mo - 1 && x.getDate() === d
    && x.getHours() === h && x.getMinutes() === mi && x.getSeconds() === s;
  if (!same(t)) return 'gap';
  // A repeated wall time: the same reading exists at the other offset of a
  // transition within a day of it.
  const before = new Date(t.getTime() - 12 * 3_600_000).getTimezoneOffset();
  const after = new Date(t.getTime() + 12 * 3_600_000).getTimezoneOffset();
  const shift = Math.abs(before - after) * 60_000;
  if (shift > 0 && (same(new Date(t.getTime() + shift)) || same(new Date(t.getTime() - shift)))) return 'repeat';
  return t;
}

/** The next `count` runs strictly after `from`, in the local zone, or null
 *  for `manual` and for a schedule the backend would reject. Searches at
 *  most `maxDays` days ahead (a schedule pinned to a far year may list
 *  fewer). */
export function nextRuns(schedule: string, from: Date | number = Date.now(), count = 3, maxDays = 1500, windows?: AutomationWindows | null): NextRuns | null {
  const s = (schedule ?? '').trim();
  if (!s || s === 'manual') return null;
  const window = scheduleWindow(s);
  if (window) {
    const starts = windows?.windows.find(w => w.name === window)?.nextStarts;
    if (!starts || !windows?.timeZone) return null;
    try { new Intl.DateTimeFormat('en', { timeZone: windows.timeZone }); } catch { return null; }
    const fromMs = typeof from === 'number' ? from : from.getTime();
    const runs = starts.map(value => new Date(value)).filter(at => Number.isFinite(at.getTime()) && at.getTime() > fromMs)
      .slice(0, count).map(at => ({ at, timeZone: windows.timeZone, zoneName: zoneShortName(at, undefined, windows.timeZone) }));
    return { runs, skipped: [], timeZone: windows.timeZone };
  }
  const f = parseCron(s);
  if (!f) return null;
  const fromMs = typeof from === 'number' ? from : from.getTime();
  const start = new Date(fromMs);
  const runs: Occurrence[] = [];
  const skipped: SkippedTime[] = [];
  const startTuple = [start.getHours(), start.getMinutes(), start.getSeconds()];
  for (let i = 0; i <= maxDays && runs.length < count; i++) {
    const day = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i, 12);
    const y = day.getFullYear(), mo = day.getMonth() + 1, d = day.getDate(), w = day.getDay() + 1;
    if (f.year && !f.year.has(y)) {
      if ([...f.year].every(v => v < y)) break;
      continue;
    }
    if (!f.month.has(mo) || !f.dom.has(d) || !f.dow.has(w)) continue;
    outer: for (const h of f.hour) {
      if (i === 0 && h < startTuple[0]) continue;
      for (const mi of f.min) {
        if (i === 0 && h === startTuple[0] && mi < startTuple[1]) continue;
        for (const sec of f.sec) {
          if (i === 0 && h === startTuple[0] && mi === startTuple[1] && sec <= startTuple[2]) continue;
          const at = resolveLocal(y, mo, d, h, mi, sec);
          if (at === 'gap' || at === 'repeat') {
            const wall = wallText(y, mo, d, h, mi);
            if (!skipped.some(x => x.wall === wall)) skipped.push({ wall, reason: at });
            continue;
          }
          if (at.getTime() <= fromMs) continue;
          runs.push({ at, zoneName: zoneShortName(at) });
          if (runs.length >= count) break outer;
        }
      }
    }
  }
  return { runs, skipped, timeZone: localTimeZone() };
}

/** The first matching wall time within `days` that a daylight-saving change
 *  skips (it doesn't exist, or it repeats), or null. Only transition days are
 *  examined, so a whole year is cheap. */
export function firstDstSkip(schedule: string, from: Date | number = Date.now(), days = 400): SkippedTime | null {
  const f = parseCron((schedule ?? '').trim());
  if (!f || (schedule ?? '').trim() === 'manual') return null;
  const start = new Date(typeof from === 'number' ? from : from.getTime());
  for (let i = 0; i <= days; i++) {
    const midnight = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i);
    const next = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i + 1);
    if (midnight.getTimezoneOffset() === next.getTimezoneOffset()) continue;
    const y = midnight.getFullYear(), mo = midnight.getMonth() + 1, d = midnight.getDate(), w = midnight.getDay() + 1;
    if ((f.year && !f.year.has(y)) || !f.month.has(mo) || !f.dom.has(d) || !f.dow.has(w)) continue;
    for (const h of f.hour) for (const mi of f.min) for (const sec of f.sec) {
      const at = resolveLocal(y, mo, d, h, mi, sec);
      if (at === 'gap' || at === 'repeat') return { wall: wallText(y, mo, d, h, mi), reason: at };
    }
  }
  return null;
}

/** "Tue, Oct 6 · 7:00 AM EDT". */
export function formatRun(o: Occurrence, locale?: string): string {
  const date = o.at.toLocaleDateString(locale, { weekday: 'short', month: 'short', day: 'numeric', timeZone: o.timeZone });
  const time = o.at.toLocaleTimeString(locale, { hour: 'numeric', minute: '2-digit', timeZone: o.timeZone });
  return `${date} · ${time}${o.zoneName ? ` ${o.zoneName}` : ''}`;
}

function graceText(min: number): string {
  if (min % 60 === 0) return min === 60 ? '1 hour' : `${min / 60} hours`;
  if (min < 60) return min === 1 ? '1 minute' : `${min} minutes`;
  return `${Math.floor(min / 60)} h ${min % 60} min`;
}

function skippedWallText(skip: SkippedTime, locale?: string): string {
  const [date, time] = skip.wall.split(' ');
  const [y, mo, d] = date.split('-').map(Number);
  const [h, mi] = time.split(':').map(Number);
  // Format through UTC so a wall time that doesn't exist locally still prints.
  const at = new Date(Date.UTC(y, mo - 1, d, h, mi));
  const day = at.toLocaleDateString(locale, { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' });
  const clock = at.toLocaleTimeString(locale, { hour: 'numeric', minute: '2-digit', timeZone: 'UTC' });
  return `${day}, ${clock}`;
}

/** Plain sentences about when runs happen and what happens to a missed one,
 *  stated only as the backend implements it (`enqueue_due`, the pump's claim
 *  rules, `quiet_check`, the run queue's coalescing). */
export function missedRunSentences(input: {
  schedule: string;
  misfireGraceMin: number | null;
  skipIfQuiet?: boolean;
  triggers?: readonly string[];
  /** The first DST skip ahead, from {@link firstDstSkip}. */
  dstSkip?: SkippedTime | null;
  timeZone?: string;
  locale?: string;
}): string[] {
  const schedule = (input.schedule ?? '').trim();
  const triggers = (input.triggers ?? []).filter(t => t.trim());
  const out: string[] = [];
  const triggerLine = 'A trigger that fires again while its run is still queued joins that run instead of adding another.';
  if (!schedule || schedule === 'manual') {
    if (triggers.length) {
      out.push(`Runs when a trigger fires (${triggers.join(', ')}), or when you start it.`);
      out.push(triggerLine);
    } else {
      out.push('Runs only when you start it. Nothing is scheduled, so nothing can be missed.');
    }
    return out;
  }
  if (!isValidSchedule(schedule)) {
    out.push('Hollow can’t read this schedule, so it never runs on its own. Pick a schedule above or fix the cron expression.');
    return out;
  }
  if (scheduleWindow(schedule)) {
    out.push(`Window times are in ${input.timeZone ?? localTimeZone()}.`);
    const policy = scheduleCatchUp(schedule);
    out.push(policy === 'skip' ? 'Skips missed window runs after their normal grace period. A late start may still run during that grace period; there is no catch-up backlog.'
      : policy === 'run-all' ? 'Catches up every missed window run, at most 5 runs on wake.'
      : 'Runs once on wake when a window was missed.');
    if (input.skipIfQuiet) out.push('A scheduled run is skipped when nothing it watches changed since the last successful run started.');
    if (triggers.length) out.push(`It also runs when a trigger fires (${triggers.join(', ')}). ${triggerLine}`);
    return out;
  }
  const grace = input.misfireGraceMin ?? DEFAULT_MISFIRE_GRACE_MIN;
  out.push(`Times are in ${input.timeZone ?? localTimeZone()}, this Mac’s time zone.`);
  out.push(`If Hollow is closed, asleep or busy at a scheduled time, that run starts once Hollow is free, as long as that is within ${graceText(grace)} of the time. Later than that it is skipped, noted in the run history, and the next time is scheduled. A missed time runs at most once, never as a backlog.`);
  if (input.dstSkip) {
    const when = skippedWallText(input.dstSkip, input.locale);
    out.push(input.dstSkip.reason === 'gap'
      ? `${when} doesn’t exist that day (clocks move forward), so that run is skipped.`
      : `${when} happens twice that day (clocks move back), so that run is skipped.`);
  }
  if (input.skipIfQuiet) out.push('A scheduled run is skipped when nothing it watches changed since the last successful run started.');
  if (triggers.length) out.push(`It also runs when a trigger fires (${triggers.join(', ')}). ${triggerLine}`);
  return out;
}
