import type { AgentSessionSummary } from './types';

// The Main agent (main agent M1, 2026-10-07): the always-on agent behind the
// top bar's orb. Its conversations live in the built-in `main` workspace
// (Rust `workspaces::MAIN_AGENT_ID`). See docs/COO.md.

/** The local calendar day of an ISO time, `YYYY-MM-DD`; '' when unparseable. */
export function localDay(iso: string | null | undefined): string {
  const t = iso ? Date.parse(iso) : NaN;
  if (Number.isNaN(t)) return '';
  const d = new Date(t);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

/** Which Main agent conversation the Main agent button opens: the chain's
 *  current one, its newest (Rust `chat_chain::main_current`, which wakes,
 *  check-ins and sends use too; one Main conversation, 2026-11-07), when it
 *  started today or still works. One from an earlier day and idle is the
 *  daily soft reset: none, so today's thread starts and continues from it
 *  (the older ones stay under "Earlier in this conversation"). Never an
 *  older conversation than the newest: that was the second Main chat. */
export function todaysMainSession(
  sessions: readonly Pick<AgentSessionSummary, 'key' | 'createdAt'>[],
  now: Date,
  working: (key: string) => boolean = () => false,
): string | null {
  const today = localDay(now.toISOString());
  const time = (iso: string) => { const t = Date.parse(iso); return Number.isNaN(t) ? 0 : t; };
  const newest = [...sessions].sort((a, b) => time(b.createdAt) - time(a.createdAt))[0];
  if (!newest) return null;
  return localDay(newest.createdAt) === today || working(newest.key) ? newest.key : null;
}

/** The conversations the orb may reopen: the Main agent's listed ones, plus
 *  the one last on screen there when it is new today and can actually open
 *  (listed, or held in the background in the Main agent's folder). Anything
 *  else, such as a key from another workspace or a new conversation that
 *  never sent a message, is left out: opening it fails with "This
 *  conversation is no longer available in this workspace". */
export function reopenableMainSessions(
  sessions: readonly Pick<AgentSessionSummary, 'key' | 'createdAt'>[],
  lastSeen: { key: string; at: string } | null,
  now: Date,
  backgroundCwd: (key: string) => string | null | undefined,
  mainCwd: string,
): Pick<AgentSessionSummary, 'key' | 'createdAt'>[] {
  if (!lastSeen || sessions.some(s => s.key === lastSeen.key)) return [...sessions];
  if (localDay(lastSeen.at) !== localDay(now.toISOString())) return [...sessions];
  if (backgroundCwd(lastSeen.key) !== mainCwd) return [...sessions];
  return [...sessions, { key: lastSeen.key, createdAt: lastSeen.at }];
}

/** Where the orb returns to when pressed again from the Main agent. */
export interface MainAgentReturn { page: string; workspaceId: string | null; key: string | null }

/** How a wake turn's message starts (Rust `main_agent::WAKE_PREFIX`, M2):
 *  Hollow queued it because a chat the Main agent started finished. */
export const WAKE_PREFIX = '⟲ Hollow:';
export const isWakeMessage = (text: string | null | undefined) => !!text && text.startsWith(WAKE_PREFIX);
/** A wake message without its prefix, for the "from Hollow" note. */
export const wakeText = (text: string) => text.slice(WAKE_PREFIX.length).trim();

/** One chat the Main agent started (Rust `main_agent::MainAgentChild`, M2). */
export interface MainAgentChild {
  parentKey: string;
  childSession: string;
  title: string;
  workspaceId: string | null;
  workspaceTitle: string | null;
  agent: string;
  /** The turn's run-queue state, or `unknown` once its row is gone. */
  status: string;
  live: boolean;
  queueId: number;
  requestedAt: string | null;
  finishedAt: string | null;
  error: string | null;
  /** The Main agent has been told it ended. */
  reported: boolean;
  costUsd: number | null;
  /** Hollow's own wake or check-in started it, not the person (overnight
   *  10-11 A1): only these count toward the daily cap. */
  byHollow: boolean;
  /** Successors it handed over to, oldest first; `status`, `live`,
   *  `finishedAt` and `error` are the last one's. */
  successors: string[];
  /** The queue row whose end is news (the last successor's, else its own):
   *  the one Stop stops. */
  endQueueId: number;
  /** Repo ideas 2026-10-27 #7: what it said of its own work in its answer's
   *  last line (Rust `child_status`), once it ended; null while it runs or if
   *  it never answered. */
  outcome?: ChildOutcome | null;
}

/** A child's own status line (Rust `child_status::Outcome`). */
export type ChildOutcome =
  | { status: 'done' }
  | { status: 'partial' | 'blocked'; detail: string }
  | { status: 'unknown' };

/** Live first, then newest (the Main store's order). */
export function orderChildren(children: readonly MainAgentChild[]): MainAgentChild[] {
  const t = (iso: string | null) => { const v = iso ? Date.parse(iso) : NaN; return Number.isNaN(v) ? 0 : v; };
  return [...children].sort((a, b) => Number(b.live) - Number(a.live) || t(b.requestedAt) - t(a.requestedAt));
}

/** A check-in's "nothing needs you" answer (Rust `main_agent::PULSE_GUIDANCE`, M3). */
export const isPulseOk = (text: string | null | undefined) => !!text && text.trim() === 'PULSE_OK';

/** Presentation only: role matters, so quotes and a person's PULSE_OK stay messages. */
export function mainTurnKind(turn: { role: string; text: string }): 'wake' | 'check-in' | 'message' {
  if (turn.role === 'user' && isWakeMessage(turn.text)) return 'wake';
  return turn.role === 'assistant' && isPulseOk(turn.text) ? 'check-in' : 'message';
}

/** The Main agent's limits (Rust `main_agent::MainAgentSettings`). */
export interface MainAgentSettings {
  checkIns: boolean;
  intervalMin: number;
  startHour: number;
  endHour: number;
  weekdaysOnly: boolean;
  dailyCapUsd: number;
  actOnOwn: boolean;
  swarmMax: number;
  /** Swarm parts' model ("" = the agent's default); claude-haiku-5-5 by default. */
  swarmModel: string;
  /** Swarm parts that run at once, within Settings' chats-at-once limit. */
  swarmConcurrency: number;
  /** Overnight 9 A9: a 350K+ chat starts fresh after more than an hour away
   *  (absent from an older backend: on). */
  freshAfterAway?: boolean;
}

/** Its day (Rust `main_agent::MainAgentStatus`). */
/** A wake a guard held back (Rust `main_agent::WakeSkip`, overnight 10-11 A1). */
export interface WakeSkip { at: string; reason: string; sessions: string[] }

export interface MainAgentStatus {
  /** What it ran on its own today: what the daily cap stops. */
  spentTodayUsd: number;
  /** Its turns and chats the person asked for today: shown, never capped. */
  askedTodayUsd: number;
  /** The last wake a guard held back today, and why. */
  wakeSkipped: WakeSkip | null;
  dailyCapUsd: number;
  checkIns: boolean;
  lastCheckIn: string | null;
  nextCheckIn: string | null;
  checkInsToday: number;
  suggestions: { answered: number; taken: number; declinedForGood: number; takenShare: number | null };
  actOnOwn: boolean;
  /** M3b: the COO's role here (`single` in the app; absent from an older backend). */
  role?: 'single' | 'runner' | 'off';
  /** M3b: the runner COO's chat, only in a `hollow serve` that is the runner. */
  runnerChatKey?: string | null;
  /** M3a PR2: whether it is the dispatcher (absent from an older backend). */
  dispatcher?: DispatcherStatus;
}

/** Why the COO isn't dispatching (Rust `dispatcher_coo::OffReason`). */
export type DispatcherOff = 'not-the-app' | 'setting' | 'no-runner' | 'pairing-needed' | 'runner-quiet' | 'runner-not-started';

/** The COO as dispatcher (Rust `dispatcher_coo::Dispatcher`, M3a PR2). */
export interface DispatcherStatus {
  dispatching: boolean;
  /** `agents.cooDispatcher`: `auto` (the default) or `off`. */
  setting: 'auto' | 'off';
  /** The Mac with a runner COO it sends long work to (or would, with auto),
   *  or with `runner-not-started` the runner whose COO has no conversation. */
  machineId: string | null;
  machine: string | null;
  /** Since when that Mac runs its runner COO, when known. */
  since: string | null;
  off: DispatcherOff | null;
  whyOff: string | null;
}

/** Something it did on its own (Rust `main_agent::unattended::Act`, M4). */
export interface MainAgentAct { at: string; action: string; summary: string; runId: string }

const capUsd = (n: number) => `$${n.toFixed(n % 1 ? 2 : 0)}`;

/** Today's spend in one line (P1-07): the total, its own spend against the
 *  hard stop (with `stop`, what the cap means now, beside its own spend),
 *  and what you asked for (never capped). */
export function todaySpendLine(s: Pick<MainAgentStatus, 'spentTodayUsd' | 'askedTodayUsd' | 'dailyCapUsd'>, stop?: string): string {
  const own = Math.max(0, s.spentTodayUsd), asked = Math.max(0, s.askedTodayUsd ?? 0);
  return `Today $${(own + asked).toFixed(2)} · $${own.toFixed(2)} on its own of ${capUsd(s.dailyCapUsd)}${stop ? ` (${stop})` : ''} · $${asked.toFixed(2)} you asked for`;
}

/** A held-back wake in words: "Didn't report 2 ended chats at 5:51 PM: why". */
export function skippedWakeWords(skip: WakeSkip): string {
  const n = skip.sessions.length;
  const at = new Date(skip.at);
  const time = Number.isNaN(at.getTime()) ? '' : ` at ${at.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}`;
  return `Didn't report ${n === 1 ? 'an ended chat' : `${n} ended chats`}${time}: ${skip.reason}`;
}

/** An action id as a few words ("sessions.start" → "Started a chat"). */
export function actLabel(action: string): string {
  const known: Record<string, string> = {
    'sessions.start': 'Started a chat', 'sessions.send': 'Followed up', 'sessions.stop': 'Stopped a chat',
    'tasks.create_task': 'Created a task', 'tasks.update_task': 'Updated a task', 'tasks.set_task_status': 'Set task status',
    'tasks.set_task_priority': 'Set task priority', 'vault.vault_create_note': 'Wrote a note', 'vault.vault_write_note': 'Wrote a note', 'vault.vault_give_up_write': 'Kept a note edit for review',
    'vault.vault_save_note_cas': 'Saved a note', 'sessions.suggestion_outcome': 'Logged a suggestion',
    'learning.submit_learning_proposal': 'Proposed a preference',
    'agenda.prepare_meeting': 'Prepped a meeting', 'sessions.submit_result': 'Judged work',
  };
  return known[action] ?? action;
}

/** The orb's night-queue line (master plan #1): "3 queued for tonight · $40
 *  cap", counting queued and running items; empty with none. */
export function nightLine(items: readonly { status: string }[], capUsd: number | null): string {
  const n = items.filter(i => i.status === 'queued' || i.status === 'running').length;
  if (n === 0) return '';
  const cap = capUsd == null ? '' : ` · $${Number.isInteger(capUsd) ? capUsd : capUsd.toFixed(2)} cap`;
  return `${n} queued for tonight${cap}`;
}
