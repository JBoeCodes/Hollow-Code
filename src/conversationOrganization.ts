import type { AgentSessionSummary } from './types';
import type { SessionState } from './sessionIndicator';

export interface ConversationOrganization {
  key: string;
  pinned: boolean;
  snoozedUntil: string | null;
  settled: boolean;
  reason: string | null;
  revision: number;
  autoSettle?: { kind: 'idle' | 'merged'; activity: string; revision: number; pr: { number: number; url: string } | null } | null;
  session: AgentSessionSummary;
}
export interface OrganizationChange {
  key: string;
  action: 'pin' | 'unpin' | 'snooze' | 'wake' | 'settle';
  until?: string;
  expectedRevision: number;
  expectedActivity: string;
}

/** Live work and attention outrank organization. New activity invalidates a
 * snooze or settlement even before the next organization read completes. */
export function organizationGroup(row: ConversationOrganization | undefined, session: AgentSessionSummary | undefined, state: SessionState, current: boolean, now: number): 'active' | 'pinned' | 'snoozed' | 'settled' {
  if (current) return 'active';
  if (row?.pinned) return 'pinned';
  if (state !== 'idle') return 'active';
  if (!row || !session || session.lastActivityAt !== row.session.lastActivityAt) return 'active';
  if (row.settled) return 'settled';
  return row.snoozedUntil && Date.parse(row.snoozedUntil) > now ? 'snoozed' : 'active';
}

export function parseSnoozeTime(value: string, now: number): string | null {
  // A local date and time is explicit in the prompt; do not accept natural
  // language or silently reinterpret an invalid calendar date.
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value)) return null;
  const at = new Date(value);
  if (!Number.isFinite(at.getTime()) || at.getTime() <= now) return null;
  const [date, time] = value.split('T');
  const [y, m, d] = date.split('-').map(Number), [h, minute] = time.split(':').map(Number);
  if (at.getFullYear() !== y || at.getMonth() + 1 !== m || at.getDate() !== d || at.getHours() !== h || at.getMinutes() !== minute) return null;
  return at.toISOString();
}

const pad = (n: number) => String(n).padStart(2, '0');
/** A local `YYYY-MM-DDTHH:mm`, the value a datetime-local field takes and
 *  `parseSnoozeTime` accepts. */
export function localMinute(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export interface SnoozePreset { id: string; label: string; at: string }

/** Quick snooze times from `now` (local time): in an hour, in three hours,
 *  tomorrow at 9, the coming Saturday at 9 (not offered on a Saturday or
 *  Sunday), next Monday at 9. Each `at` is an ISO instant in the future. */
export function snoozePresets(now: number): SnoozePreset[] {
  const base = new Date(now);
  const at = (days: number, hour: number) => new Date(base.getFullYear(), base.getMonth(), base.getDate() + days, hour, 0, 0, 0).getTime();
  const day = base.getDay();
  const out: { id: string; label: string; ms: number }[] = [
    { id: 'hour', label: 'In 1 hour', ms: now + 3_600_000 },
    { id: 'three-hours', label: 'In 3 hours', ms: now + 3 * 3_600_000 },
    { id: 'tomorrow', label: 'Tomorrow morning', ms: at(1, 9) },
  ];
  if (day >= 1 && day <= 5) out.push({ id: 'weekend', label: 'This weekend', ms: at(6 - day, 9) });
  out.push({ id: 'next-week', label: 'Next week', ms: at(((8 - day) % 7) || 7, 9) });
  return out.filter(p => p.ms > now).map(p => ({ id: p.id, label: p.label, at: new Date(Math.round(p.ms / 60_000) * 60_000).toISOString() }));
}
