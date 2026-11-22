/** Geometry for the shared Today/Agenda dial. A turn is one real local day. */
import { calendarCoverage, dayBounds, instantAt, type AgendaEvent, type FocusBlockLike, type DayLayout, type Placed } from './agenda';
import type { AgendaEventsView } from './agendaTypes';
export function ringPoint(min: number, minutes: number, radius: number) {
  const a = min / minutes * Math.PI * 2 - Math.PI / 2;
  return { x: 180 + radius * Math.cos(a), y: 180 + radius * Math.sin(a) };
}
export function ringArc(start: number, end: number, minutes: number, r: number): string {
  const a = ringPoint(start, minutes, r), b = ringPoint(end, minutes, r);
  // Two half arcs also cover full-day timed items (a single SVG arc cannot).
  const mid = ringPoint((start + end) / 2, minutes, r);
  return `M ${a.x} ${a.y} A ${r} ${r} 0 0 1 ${mid.x} ${mid.y} A ${r} ${r} 0 0 1 ${b.x} ${b.y}`;
}
export function ringNow(layout: DayLayout, now: Date): number | null {
  const { start, end } = dayBounds(layout.dateKey);
  return now >= start && now < end ? (now.getTime() - start.getTime()) / 60_000 : null;
}
export interface RingItem { key: string; item: Placed; path: string; detail: string }
export function ringItems(layout: DayLayout): RingItem[] {
  const format = (min: number) => instantAt(layout.dateKey, min).toLocaleTimeString(undefined, {
    hour: 'numeric', minute: '2-digit', ...(layout.minutes !== 1440 ? { timeZoneName: 'short' as const } : {}),
  });
  const midnight = dayBounds(layout.dateKey).start.getTime();
  // layoutDay reserves 15 minutes of hit height. The dial encodes actual duration.
  const timed = layout.timed.map(p => {
    if (p.kind === 'task') return p;
    const ref = p.ref as AgendaEvent | FocusBlockLike;
    return { ...p, startMin: Math.max(0, (Date.parse(ref.start) - midnight) / 60_000),
      endMin: Math.min(layout.minutes, (Date.parse(ref.end) - midnight) / 60_000) };
  });
  return timed.map(p => {
    const r = p.kind === 'event' ? 126 - Math.min(p.col, 2) * 12 : p.kind === 'focus' ? 78 - Math.min(p.col, 2) * 10 : 144;
    const valid = Number.isFinite(p.startMin) && Number.isFinite(p.endMin) && p.endMin >= p.startMin;
    const a = ringPoint(p.startMin, layout.minutes, p.kind === 'task' ? 139 : r - 4);
    const b = ringPoint(p.startMin, layout.minutes, p.kind === 'task' ? 151 : r + 4);
    return { key: `${p.kind}:${p.id}`, item: p,
      path: !valid ? '' : p.kind === 'task' || p.endMin === p.startMin ? `M ${a.x} ${a.y} L ${b.x} ${b.y}` : ringArc(p.startMin, p.endMin, layout.minutes, r),
      detail: !valid ? 'Time unavailable' : `${p.kind === 'event' ? 'Calendar' : p.kind === 'focus' ? 'Focus · local' : 'Task due'} · ${format(p.startMin)}${p.kind === 'task' ? '' : ` – ${format(p.endMin)}`}${p.cancelled ? ' · cancelled' : ''}` };
  });
}
/** What the calendar says when Microsoft refused the stored sign-in
 *  (overnight H6): the person can fix it, in Settings, Connectors. */
export const CALENDAR_SIGN_IN_LABEL = 'Microsoft 365 needs you to sign in again';

/** Whether a calendar read failed because the Microsoft 365 sign-in is over
 *  (refused or expired), not for a reason worth retrying. The backend has no
 *  error code for it: its messages begin with `m365_auth.rs`' NEEDS_SIGN_IN
 *  or EXPIRED (as `agenda::events_with` returns the stored problem), and may
 *  arrive wrapped, so this looks anywhere in the text. */
export function calendarNeedsSignIn(error: string | null | undefined): boolean {
  return !!error && /Microsoft 365 (needs signing in again|sign-in expired)/i.test(error);
}

export function ringCalendarLabel(calendar: AgendaEventsView | null | undefined): string {
  if (!calendar) return 'Calendar unavailable';
  if (!calendar.connected) return calendar.status ?? 'Calendar not connected';
  if (calendarNeedsSignIn(calendar.error)) return CALENDAR_SIGN_IN_LABEL;
  if (calendar.error) return 'Calendar unavailable';
  return calendarCoverage(calendar).label;
}
