// The COO page header's pure parts (five asks A3), testable without the
// DOM. `MainAgentSpace.tsx` draws them above the COO's conversation (the side
// band went in COO rebuild L3); nothing here reads or writes.
import type { MainAgentChild, MainAgentStatus } from './mainAgent';
import { mainAgentStateOf, type MainAgentState, type SessionState } from './sessionIndicator';

/** The standing orders file, beside the Main agent's brief. */
export const STANDING_ORDERS_FILE = 'STANDING_ORDERS.md';

/** Unknown or mixed observations never borrow one provider's identity. */
export function observedProvider(agents: readonly (string | null | undefined)[]): string | null {
  if (!agents.length || agents.some(a => !a)) return null;
  return new Set(agents).size === 1 ? agents[0]! : null;
}

/** The acts log's count (it keeps the last 200), keeping a failed or pending
 *  refresh apart from a fresh zero. */
export function recentActionsFact(count: number, read: 'ready' | 'loading' | 'error'): string {
  if (read === 'ready') return `${count} recent`;
  if (count) return `${count} last read · ${read === 'error' ? 'refresh failed' : 'refreshing'}`;
  return read === 'error' ? 'Unavailable' : 'Reading…';
}

export type SpendTone = 'ok' | 'warn' | 'stop';

/** Today's spend against the hard stop: the share (0–1, capped) and its
 *  tone (warn from 75%, stop at the cap). A zero cap means nothing may run:
 *  stop. */
export function spendGauge(s: Pick<MainAgentStatus, 'spentTodayUsd' | 'dailyCapUsd'>): { share: number; tone: SpendTone; words: string } {
  const cap = Math.max(0, s.dailyCapUsd);
  const spent = Math.max(0, s.spentTodayUsd);
  const share = cap > 0 ? Math.min(1, spent / cap) : 1;
  const tone: SpendTone = cap <= 0 || spent >= cap ? 'stop' : share >= 0.75 ? 'warn' : 'ok';
  const words = `$${spent.toFixed(2)} of $${cap.toFixed(cap % 1 ? 2 : 0)} on its own today${cap === 0 ? ': runs only when you ask' : tone === 'stop' ? ': stopped until tomorrow' : ''}`;
  return { share, tone, words };
}

/** Its check-ins in words (a phrase: the header joins it with " · "): how
 *  many ran today, then when the next can run. A check-in runs only when
 *  something new came up, or as the day's first (`decide_pulse`), and once
 *  the interval has passed the backend reports the next one as due now, so
 *  a time already past means "when something new comes up", never "No
 *  upcoming check-in" (11-06). Check-ins off names no path: the header's
 *  More holds the way to the setting (A07, 2026-10-16). */
export function checkInWords(s: Pick<MainAgentStatus, 'checkIns' | 'nextCheckIn' | 'checkInsToday'> & Partial<Pick<MainAgentStatus, 'lastCheckIn' | 'spentTodayUsd' | 'dailyCapUsd'>> | null, now = new Date()): string {
  if (!s) return 'Reading its day…';
  if (!s.checkIns) return 'Check-ins are off';
  // A status read yesterday evening still says yesterday's count after
  // midnight: the count is today's only while the last check-in is (both are
  // written together, `pulse_queued`).
  const last = s.lastCheckIn ? new Date(s.lastCheckIn) : null;
  const n = !last || Number.isNaN(last.getTime()) || last.toDateString() === now.toDateString() ? s.checkInsToday : 0;
  const ran = (none: string) => n ? `${n} check-in${n === 1 ? '' : 's'} today` : none;
  const at = s.nextCheckIn ? new Date(s.nextCheckIn) : null;
  const known = !!at && !Number.isNaN(at.getTime());
  // Its own spend at the hard stop runs no check-in today (`decide_pulse`).
  // The stop resets at midnight, so a later day's first still stands, except
  // under a $0 stop, which never lets one run.
  if (typeof s.spentTodayUsd === 'number' && typeof s.dailyCapUsd === 'number'
    && spendGauge({ spentTodayUsd: s.spentTodayUsd, dailyCapUsd: s.dailyCapUsd }).tone === 'stop'
    && (s.dailyCapUsd <= 0 || !known || at!.getTime() <= now.getTime() || at!.toDateString() === now.toDateString())) {
    if (s.dailyCapUsd <= 0) return n ? `${ran('')} · none while its hard stop is $0` : 'No check-ins while its hard stop is $0';
    return n ? `${ran('')} · no more today` : 'No check-ins today';
  }
  if (!at || !known) return `${ran('No check-ins yet today')} · none scheduled`;
  if (at.getTime() <= now.getTime()) return n ? `${ran('')} · next when something new comes up` : 'No check-ins yet today · the first is due now';
  const time = at.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  // The first of a later day always runs (the morning brief).
  if (at.toDateString() !== now.toDateString()) return `${ran('No check-ins today')} · next ${at.toLocaleDateString([], { weekday: 'short' })} ${time}`;
  return n ? `${ran('')} · next ${time} if something new comes up` : `No check-ins yet today · first at ${time}`;
}

/** Whether its day is read and check-ins are off: the header's More then
 *  offers "Turn check-ins on in Settings". Unread (null) is not off. */
export function checkInsOff(s: Pick<MainAgentStatus, 'checkIns'> | null): boolean {
  return !!s && !s.checkIns;
}

/** Its state in the session-indicator family (asks 10-11 A2). */
export type Presence = MainAgentState;

/** What it is doing now: waiting on you (a card in one of its own
 *  conversations), else working (its turn, or subagents it started), else paused
 *  (its own spend reached today's hard stop), else watching (check-ins on),
 *  else resting. */
export function presence(
  working: boolean,
  children: readonly Pick<MainAgentChild, 'live'>[],
  s: (Pick<MainAgentStatus, 'checkIns'> & Partial<Pick<MainAgentStatus, 'spentTodayUsd' | 'dailyCapUsd'>>) | null,
  waiting = false,
  extra: { failed?: boolean; nightRunning?: number; waitingCount?: number } = {},
): { state: Presence; words: string } {
  const live = children.filter(c => c.live).length;
  const chats = live ? `${live} subagent${live === 1 ? '' : 's'} running` : '';
  // `status` is re-read on run-queue changes, so a cap reached late at night
  // can read "paused" past midnight until the next queue event.
  const capped = !!s && typeof s.spentTodayUsd === 'number' && typeof s.dailyCapUsd === 'number'
    && spendGauge({ spentTodayUsd: s.spentTodayUsd, dailyCapUsd: s.dailyCapUsd }).tone === 'stop';
  const state = mainAgentStateOf({ failed: extra.failed, night: (extra.nightRunning ?? 0) > 0, waiting, working: working || live > 0, paused: capped, watching: !!s?.checkIns });
  switch (state) {
    case 'failed': return { state, words: `A conversation failed${working ? ' · Working' : ''}${chats ? ` · ${chats}` : ''}` };
    case 'night': return { state, words: `Tonight · ${extra.nightRunning} running` };
    // The count is the orb's badge: its own conversations waiting on you,
    // never the bell's every-item count (11-06).
    case 'waiting': return { state, words: `Waiting on you${extra.waitingCount ? ` in ${extra.waitingCount} conversation${extra.waitingCount === 1 ? '' : 's'}` : ''}${chats ? ` · ${chats}` : ''}` };
    case 'working': return { state, words: working ? (chats ? `Working · ${chats}` : 'Working') : chats };
    case 'paused': return { state, words: 'Paused: today’s cost cap is reached' };
    case 'watching': return { state, words: 'Watching' };
    default: return { state, words: 'Resting: answers when you ask' };
  }
}

/** Count distinct own conversations, not cards elsewhere in the bell. */
export function mainAgentAttention(entries: readonly { key: string; state: SessionState }[]) {
  const states = [...new Map(entries.map(e => [e.key, e.state])).values()];
  return { waitingCount: states.filter(s => s === 'waiting').length, failed: states.includes('failed') };
}

/** A night item can lag its turn until the minute scheduler settles it.
 * Require live queue evidence; unknown rows and runbooks without a queue id
 * do not establish night activity. No extra IPC or polling. */
export function runningNightCount(rows: readonly { status: string; queueId?: number }[] | null, queue: readonly { id: number; status: string }[] = []): number {
  const live = new Set(queue.filter(r => r.status === 'claimed' || r.status === 'running').map(r => r.id));
  return new Set(rows?.filter(r => r.status === 'running' && r.queueId !== undefined && live.has(r.queueId)).map(r => r.queueId)).size;
}
