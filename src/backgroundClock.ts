import type { AutomationWindows } from './generated/ipc';

export type ClockBand = {
  name: string; label: string; target: 'tonight' | 'automations';
  spans: { start: number; end: number; fromYesterday: boolean }[];
};
export type BackgroundClockModel = {
  date: string; timeZone: string; minute: number; bands: ClockBand[];
  tonightKnown: boolean;
};
const DAYS = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat'];
function minutes(value: string): number | null {
  const match = /^([01]?\d|2[0-3]):([0-5]\d)$/.exec(value.trim());
  return match ? Number(match[1]) * 60 + Number(match[2]) : null;
}
export function clockTime(minute: number): string {
  return `${String(Math.floor(minute / 60)).padStart(2, '0')}:${String(minute % 60).padStart(2, '0')}`;
}

/** A civil-day picture of configured hours, not elapsed durations or run status.
 * Weekdays belong to the start date; end <= start crosses midnight (equal = 24h).
 * Never substitute the viewer's zone or turn missing coverage into no work. */
export function backgroundClock(value: AutomationWindows | null | undefined, now: Date): BackgroundClockModel | null {
  if (!value?.timeZone || !value.windows.length || !Number.isFinite(now.getTime())) return null;
  try {
    const parts = new Intl.DateTimeFormat('en-CA', {
      timeZone: value.timeZone, year: 'numeric', month: '2-digit', day: '2-digit',
      hour: '2-digit', minute: '2-digit', hourCycle: 'h23',
    }).formatToParts(now);
    const part = (type: Intl.DateTimeFormatPartTypes) => parts.find(p => p.type === type)!.value;
    const date = `${part('year')}-${part('month')}-${part('day')}`;
    const day = new Date(`${date}T12:00:00Z`).getUTCDay();
    const names = new Set<string>();
    const bands: ClockBand[] = [];
    for (const w of value.windows) {
      const start = minutes(w.start), end = minutes(w.end);
      const days = w.days.map(d => d.trim().toLowerCase().slice(0, 3));
      if (start == null || end == null || !days.length || days.some(d => !DAYS.includes(d)) || !w.name || names.has(w.name)) return null;
      names.add(w.name);
      const spans: ClockBand['spans'] = [];
      if (end <= start && days.includes(DAYS[(day + 6) % 7]) && end > 0) {
        spans.push({ start: 0, end, fromYesterday: true });
      }
      if (days.includes(DAYS[day])) spans.push({ start, end: end > start ? end : 1440, fromYesterday: false });
      bands.push({ name: w.name, label: w.label || w.name, target: w.name === 'tonight' ? 'tonight' : 'automations', spans });
    }
    return { date, timeZone: value.timeZone, minute: Number(part('hour')) * 60 + Number(part('minute')), bands, tonightKnown: names.has('tonight') };
  } catch { return null; }
}

/** SVG arcs split at noon so an all-day band is a circle, never a zero arc. */
export function clockArc(start: number, end: number, radius: number): string {
  const point = (m: number) => {
    const a = m / 1440 * Math.PI * 2 - Math.PI / 2;
    return `${80 + radius * Math.cos(a)} ${80 + radius * Math.sin(a)}`;
  };
  const mid = (start + end) / 2;
  return `M ${point(start)} A ${radius} ${radius} 0 0 1 ${point(mid)} A ${radius} ${radius} 0 0 1 ${point(end)}`;
}
