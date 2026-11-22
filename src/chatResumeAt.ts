// Resume at a chosen time (P1c): the presets the "Resume at..." control
// offers, how a resume time reads, and the custom time's input value. Pure:
// times are unix seconds, `nowMs` is the clock in milliseconds, and every
// date is read in the machine's own zone.

export type ResumePresetId = 'hour' | 'tonight' | 'morning';
export interface ResumePreset { id: ResumePresetId; label: string; at: number }

const sec = (ms: number) => Math.floor(ms / 1000);

/** In 1 hour; tonight at 9 PM (only while that is still ahead); tomorrow at
 *  8 AM. */
export function resumePresets(nowMs: number): ResumePreset[] {
  const out: ResumePreset[] = [{ id: 'hour', label: 'In 1 hour', at: sec(nowMs) + 3600 }];
  const tonight = new Date(nowMs);
  tonight.setHours(21, 0, 0, 0);
  if (tonight.getTime() > nowMs) out.push({ id: 'tonight', label: 'Tonight, 9 PM', at: sec(tonight.getTime()) });
  const morning = new Date(nowMs);
  morning.setDate(morning.getDate() + 1);
  morning.setHours(8, 0, 0, 0);
  out.push({ id: 'morning', label: 'Tomorrow, 8 AM', at: sec(morning.getTime()) });
  return out;
}

const pad = (n: number) => String(n).padStart(2, '0');
const clock = (d: Date) => `${pad(d.getHours())}:${pad(d.getMinutes())}`;
const dayStart = (ms: number) => { const d = new Date(ms); d.setHours(0, 0, 0, 0); return d.getTime(); };
const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** When a paused chat resumes, as its row says it: "at 21:00" today,
 *  "tomorrow at 08:00", "Fri at 08:00" within a week, else "Nov 20 at
 *  08:00". A time already past reads "now". */
export function resumeAtWhen(at: number, nowMs: number): string {
  const ms = at * 1000;
  if (ms <= nowMs) return 'now';
  const d = new Date(ms);
  const days = Math.round((dayStart(ms) - dayStart(nowMs)) / 86_400_000);
  if (days === 0) return `at ${clock(d)}`;
  if (days === 1) return `tomorrow at ${clock(d)}`;
  if (days < 7) return `${WEEKDAYS[d.getDay()]} at ${clock(d)}`;
  return `${MONTHS[d.getMonth()]} ${d.getDate()} at ${clock(d)}`;
}

/** The paused row's line: "Resumes tomorrow at 08:00". */
export function resumeAtLabel(at: number, nowMs: number): string {
  return `Resumes ${resumeAtWhen(at, nowMs)}`;
}

/** A `datetime-local` value ("2026-11-16T08:00") as unix seconds, else null. */
export function parseLocalDateTime(value: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/.exec(value.trim());
  if (!m) return null;
  const [y, mo, d, h, mi] = m.slice(1).map(Number);
  const date = new Date(y, mo - 1, d, h, mi, 0, 0);
  if (date.getFullYear() !== y || date.getMonth() !== mo - 1 || date.getDate() !== d) return null;
  return sec(date.getTime());
}

/** Unix seconds as a `datetime-local` value, for the custom time's start. */
export function toLocalDateTime(at: number): string {
  const d = new Date(at * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${clock(d)}`;
}
