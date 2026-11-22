// Settings, Sources, Task tracker (overhaul A14, board S2): the pure
// words and choices behind `settings/TaskTrackerPanel.tsx`, the capability
// doctor's tracker check and Service health's tracker row. Everything here
// reads a `TrackerStatus` (`tracker_status`, which never carries the key).

import type { TrackerStatus } from './generated/ipc';

/** The shared team Hollow may read and write, per provider: Engineering,
 *  where real project work lives and your team sees it. Settings offers no
 *  other as the shared team (guardrail 2026-11-07). */
export const ALLOWED_TEAMS: Record<string, { id: string; key: string; name: string }> = {
  linear: { id: '11111111-1111-4111-8111-111111111111', key: 'ENG', name: 'Engineering' },
};

/** The personal team beside it: Personal (PER), items only the person and
 *  Hollow use. Settings' "Add Personal" button saves exactly this id and
 *  key; there is no free-text team field. */
export const PERSONAL_TEAMS: Record<string, { id: string; key: string; name: string }> = {
  linear: { id: '22222222-2222-4222-8222-222222222222', key: 'PER', name: 'Personal' },
};

/** The only team keys Hollow accepts anywhere (Rust refuses any other). */
export const ALLOWED_TEAM_KEYS = ['ENG', 'PER'] as const;
export type TeamKey = (typeof ALLOWED_TEAM_KEYS)[number];
export const isTeamKey = (key: string | null | undefined): key is TeamKey => !!key && (ALLOWED_TEAM_KEYS as readonly string[]).includes(key);

/** The interval choices, 1 to 60 minutes (Rust refuses anything else). */
export const INTERVAL_MINUTES = [1, 2, 5, 10, 15, 30, 60] as const;

export type TrackerHealth = 'ok' | 'attention' | 'off' | 'busy';

/** The row's dot: on, needs you, off, or working. */
export function trackerHealth(s: TrackerStatus | null | undefined): TrackerHealth {
  if (!s || s.state === 'off') return 'off';
  if (s.state === 'syncing') return 'busy';
  if (s.state === 'ready' && !s.lastError) return 'ok';
  return 'attention';
}

/** One line on where the tracker stands, in the order a person acts on it. */
export function trackerHeadline(s: TrackerStatus | null | undefined): string {
  if (!s) return 'Reading the task tracker settings…';
  const name = s.displayName ?? 'The task tracker';
  switch (s.state) {
    case 'off': return 'Off. Tasks stay local until you choose a tracker.';
    case 'needsSetup': return `${name} is chosen but not set up: ${s.missing ?? 'finish the steps below.'}`;
    case 'blocked': return s.blockedBy ?? `${name} is set up, but something else holds the task files.`;
    case 'syncing': return `${name} is syncing now.`;
    case 'error': return `The last ${name} pass failed: ${s.lastError ?? 'no reason was given.'}`;
    case 'ready': return s.lastError ? `${name} is on; the last pass reported: ${s.lastError}` : `${name} is on and syncing ${teamLabel(s)}.`;
  }
}

/** "Engineering (ENG)" for the allowed team, the bare key otherwise. */
export function teamLabel(s: Pick<TrackerStatus, 'provider' | 'teamId' | 'teamKey'>): string {
  const allowed = s.provider ? ALLOWED_TEAMS[s.provider] : undefined;
  if (allowed && s.teamId === allowed.id) return `${allowed.name} (${allowed.key})`;
  if (s.teamKey) return `team ${s.teamKey}`;
  return 'no team yet';
}

/** "Personal (PER)" when the personal team is the allowed one, else null (none added). */
export function personalTeamLabel(s: Pick<TrackerStatus, 'provider' | 'personalTeamId' | 'personalTeamKey'>): string | null {
  if (!s.personalTeamKey) return null;
  const allowed = s.provider ? PERSONAL_TEAMS[s.provider] : undefined;
  if (allowed && s.personalTeamId === allowed.id) return `${allowed.name} (${allowed.key})`;
  return `team ${s.personalTeamKey}`;
}

/** Whether the saved team is the one Hollow may use. */
export function teamAllowed(s: Pick<TrackerStatus, 'provider' | 'teamId'>): boolean {
  const allowed = s.provider ? ALLOWED_TEAMS[s.provider] : undefined;
  return !!allowed && s.teamId === allowed.id;
}

/** "3m ago" / "in 2m" for a sync time, or a plain word when there is none. */
export function syncWhen(iso: string | null | undefined, now = Date.now(), none = 'not yet'): string {
  if (!iso) return none;
  const at = new Date(iso).getTime();
  if (!Number.isFinite(at)) return none;
  const secs = Math.round((at - now) / 1000);
  const abs = Math.abs(secs);
  const amount = abs < 60 ? `${abs}s` : abs < 3600 ? `${Math.round(abs / 60)}m` : abs < 86400 ? `${Math.round(abs / 3600)}h` : `${Math.round(abs / 86400)}d`;
  if (abs < 5) return 'now';
  return secs < 0 ? `${amount} ago` : `in ${amount}`;
}

/** "2,431 of 2,500 requests left this hour", or null when not reported. */
export function requestsLeft(s: TrackerStatus | null | undefined): string | null {
  const r = s?.rate;
  if (!r || r.requestsRemaining == null) return null;
  const left = r.requestsRemaining.toLocaleString('en-US');
  return r.requestsLimit ? `${left} of ${r.requestsLimit.toLocaleString('en-US')} requests left this hour` : `${left} requests left this hour`;
}

/** Whether Sync now can run: set up, not blocked, the engine present. */
export function canSyncNow(s: TrackerStatus | null | undefined): boolean {
  return !!s && s.engine && (s.state === 'ready' || s.state === 'error');
}

/** The interval choice nearest the saved seconds (the Select needs one). */
export function intervalChoice(pollSeconds: number): number {
  const minutes = pollSeconds / 60;
  return INTERVAL_MINUTES.reduce((best, m) => Math.abs(m - minutes) < Math.abs(best - minutes) ? m : best, INTERVAL_MINUTES[0]);
}
