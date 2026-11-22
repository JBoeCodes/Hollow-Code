/**
 * One indicator family for a conversation's live state (asks 10-11 A1/A2).
 * Pure, unit-tested in sessionIndicator.test.ts. `SessionIndicator.tsx`
 * draws it; every surface (rail rows, a collapsed workspace, Your work,
 * Mission Control, the workspace map, the Main agent's orb and page
 * header) reads these words and this order, so they read the same everywhere.
 */

/** What a conversation is doing now. `idle` draws nothing (or its provider). */
// `paused` (P1a PR 2): the person paused its turn at a safe point; nothing
// runs and nothing failed, and Resume picks it up. Least urgent of the live
// states: it waits on no one.
export type SessionState = 'waiting' | 'failed' | 'interrupted' | 'running' | 'queued' | 'paused' | 'idle';
export type LiveSessionState = Exclude<SessionState, 'idle'>;

/** Most urgent first: what a rollup shows when its sessions disagree. */
export const SESSION_STATE_ORDER: readonly LiveSessionState[] = ['waiting', 'failed', 'interrupted', 'running', 'queued', 'paused'];

/** One word per state (the same words as workState.ts's RUN_STAGE). */
export const SESSION_STATE_WORDS: Record<LiveSessionState, string> = {
  waiting: 'Waiting on you',
  failed: 'Failed',
  interrupted: 'Interrupted',
  running: 'Running',
  queued: 'Queued',
  paused: 'Paused',
};

/** What an interrupted conversation's mark says (P1-40): Hollow's quit cut
 *  its turn short; nothing failed, and Continue picks it up. */
export const INTERRUPTED_HINT = 'Hollow quit before this turn finished. Open it and press Continue.';

/** A conversation's state from the live evidence. Waiting beats failed
 *  beats interrupted beats running beats queued: a turn paused on a card is
 *  still "running" underneath, but what matters to the person is that it
 *  needs them. */
export function sessionStateOf(input: { waiting?: boolean; failed?: boolean; interrupted?: boolean; running?: boolean; queued?: boolean; paused?: boolean }): SessionState {
  if (input.waiting) return 'waiting';
  if (input.failed) return 'failed';
  if (input.interrupted) return 'interrupted';
  if (input.running) return 'running';
  if (input.queued) return 'queued';
  if (input.paused) return 'paused';
  return 'idle';
}

/** The label a screen reader hears and a tooltip shows: "Running · Claude";
 *  an interrupted one adds what to do. */
export function sessionStateLabel(state: SessionState, provider?: string): string {
  if (state === 'idle') return provider || '';
  const words = provider ? `${SESSION_STATE_WORDS[state]} · ${provider}` : SESSION_STATE_WORDS[state];
  return state === 'interrupted' ? `${words}. ${INTERRUPTED_HINT}` : words;
}

export interface SessionRollup {
  /** The most urgent state among the sessions. */
  state: LiveSessionState;
  /** How many sessions are in that state (the badge's number when > 1). */
  count: number;
  /** "1 waiting on you: Parse receipts · 2 running: Memo, Pricing". */
  label: string;
}

/** A collapsed workspace's one badge (A1): the most urgent state wins, its
 *  count shows, and the label names every non-idle chat by state. Null when
 *  every session is idle. Titles past `maxNames` per state become "+N more". */
export function rollupSessions(entries: readonly { title: string; state: SessionState }[], maxNames = 3): SessionRollup | null {
  const by = new Map<LiveSessionState, string[]>();
  for (const e of entries) {
    if (e.state === 'idle') continue;
    const list = by.get(e.state) ?? [];
    list.push(e.title.trim() || 'Conversation');
    by.set(e.state, list);
  }
  const top = SESSION_STATE_ORDER.find(s => by.has(s));
  if (!top) return null;
  const parts = SESSION_STATE_ORDER.filter(s => by.has(s)).map(s => {
    const names = by.get(s)!;
    const shown = names.slice(0, maxNames).join(', ');
    const more = names.length > maxNames ? ` +${names.length - maxNames} more` : '';
    return `${names.length} ${SESSION_STATE_WORDS[s].toLowerCase()}: ${shown}${more}`;
  });
  return { state: top, count: by.get(top)!.length, label: parts.join(' · ') };
}

/** Who a conversation is in its project: the lead, a subagent it (or the COO)
 *  started, or any other chat in the project's folder. */
export type SessionRole = 'lead' | 'subagent' | 'chat';

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** A project row's one badge (running indicator, 11-11): like
 *  `rollupSessions`, but it says who is in each state, subagents included:
 *  "Lead and 2 subagents running · 1 chat queued". The count is how many
 *  sessions share the most urgent state. Null when every session is idle. */
export function projectRollup(entries: readonly { title: string; state: SessionState; role: SessionRole }[]): SessionRollup | null {
  const by = new Map<LiveSessionState, { lead: number; subagent: number; chat: number }>();
  for (const e of entries) {
    if (e.state === 'idle') continue;
    const c = by.get(e.state) ?? { lead: 0, subagent: 0, chat: 0 };
    c[e.role] += 1;
    by.set(e.state, c);
  }
  const top = SESSION_STATE_ORDER.find(s => by.has(s));
  if (!top) return null;
  const parts = SESSION_STATE_ORDER.filter(s => by.has(s)).map(s => {
    const c = by.get(s)!;
    const who = [c.lead ? 'Lead' : '', c.chat ? plural(c.chat, 'chat', 'chats') : '', c.subagent ? plural(c.subagent, 'subagent', 'subagents') : ''].filter(Boolean);
    const names = who.length > 1 ? `${who.slice(0, -1).join(', ')} and ${who[who.length - 1]}` : who[0];
    return `${names} ${SESSION_STATE_WORDS[s].toLowerCase()}`;
  });
  const t = by.get(top)!;
  return { state: top, count: t.lead + t.subagent + t.chat, label: parts.join(' · ') };
}

// ── The Main agent's own variant: the doorway inside the same ring.
export type MainAgentState = 'resting' | 'watching' | 'working' | 'waiting' | 'paused' | 'failed' | 'night';

export const MAIN_AGENT_STATE_WORDS: Record<MainAgentState, string> = {
  resting: 'Resting',
  watching: 'Watching',
  working: 'Working',
  waiting: 'Waiting on you',
  paused: 'Paused by the cost cap',
  failed: 'A conversation failed',
  night: 'Tonight',
};

/** Own waiting, own unseen failure, verified night activity, working, cost
 * cap, watching, resting. Presence words retain concurrent work even when
 * attention takes priority; neither global bell errors nor clock time qualify. */
export function mainAgentStateOf(input: { waiting?: boolean; working?: boolean; paused?: boolean; watching?: boolean; failed?: boolean; night?: boolean }): MainAgentState {
  if (input.waiting) return 'waiting';
  if (input.failed) return 'failed';
  if (input.night) return 'night';
  if (input.working) return 'working';
  if (input.paused) return 'paused';
  if (input.watching) return 'watching';
  return 'resting';
}

// ── Every conversation's state at once, from the live signals.
/** What the store hands over (all of it already event-driven: chat events,
 *  the run queue, approvals and the "waiting on you" list). */
export interface SessionEvidence {
  /** The conversation on screen and whether its turn is running. */
  onScreen: { key: string | null; busy: boolean };
  /** Conversations with a turn running off screen. */
  background: readonly string[];
  /** Run-queue rows (chat rows carry the session key as `sourceId`). */
  queue: readonly { id: number; source: string; sourceId: string | null; status: string; /** A queued chat row that reads Working (`RunQueueItem.working`). */ working?: boolean }[];
  /** Session keys with an approval, open question or unanswered canvas. */
  waitingKeys: readonly (string | null | undefined)[];
  /** Mission Control's feed: each conversation's latest turn. */
  feed: Readonly<Record<string, { running: boolean; error: string | null; finishedAt: number | null; queueId?: number | null }>>;
  /** Failures already looked at (key → failure token), which no longer show. */
  seenFailures: ReadonlyMap<string, string>;
  /** Paused conversations (P1a PR 2, `store/chatPause.ts`). Optional so
   *  older callers and fixtures read as none. */
  paused?: readonly string[];
}

export interface SessionStateIndex {
  /** Every conversation not idle. Look up with `?? 'idle'`. */
  states: Map<string, LiveSessionState>;
  /** Each failed or interrupted conversation's failure token (to mark it
   *  seen when opened). */
  failures: Map<string, string>;
}

/** The words recovery ends a cut-short turn with (`run_queue.rs`
 *  INTERRUPTED_RUNNING / INTERRUPTED_QUEUED), for a feed entry whose queue
 *  row has aged out. */
const QUIT_ERROR = /^Hollow (stopped|quit) before /;

/** Failed means the conversation's latest turn ended in an error, and
 *  interrupted that Hollow's quit cut it short (P1-40; same token, so
 *  opening either marks it seen): its newest run-queue row says so, or its feed entry (the
 *  latest turn) carries an error and is no longer running. A feed entry
 *  names its queue row when it has one, and then the row decides: a row that
 *  finished fine (a recoverable error mid-turn) or a newer row wins, and a
 *  row gone from the queue's short history keeps the same `q:` token, so a
 *  failure seen once stays seen. Opening it marks that failure seen; a later
 *  failure has a new token and shows again. */
export function sessionStateIndex(ev: SessionEvidence): SessionStateIndex {
  const running = new Set<string>(ev.background);
  if (ev.onScreen.key && ev.onScreen.busy) running.add(ev.onScreen.key);
  const queued = new Set<string>();
  const latest = new Map<string, { id: number; status: string }>();
  for (const row of ev.queue) {
    if (row.source !== 'chat' || !row.sourceId) continue;
    const key = row.sourceId;
    // A queued row nothing truly holds back is working too (2026-11-16).
    if (row.status === 'claimed' || row.status === 'running' || (row.status === 'queued' && row.working)) running.add(key);
    else if (row.status === 'queued') queued.add(key);
    const prev = latest.get(key);
    if (!prev || row.id > prev.id) latest.set(key, { id: row.id, status: row.status });
  }
  const failures = new Map<string, string>();
  // Of those, the ones Hollow's quit cut short (P1-40): interrupted, not failed.
  const quit = new Set<string>();
  for (const [key, row] of latest) {
    if (row.status === 'failed' || row.status === 'interrupted') failures.set(key, `q:${row.id}`);
    if (row.status === 'interrupted') quit.add(key);
  }
  const rowStatus = new Map<number, string>();
  for (const row of ev.queue) rowStatus.set(row.id, row.status);
  for (const [key, entry] of Object.entries(ev.feed)) {
    if (!entry.error || entry.running || failures.has(key)) continue;
    const row = latest.get(key);
    const qid = typeof entry.queueId === 'number' ? entry.queueId : null;
    if (qid !== null) {
      const status = rowStatus.get(qid);
      // Its own row is listed and didn't fail, or a newer turn came after it.
      if (status !== undefined && status !== 'failed' && status !== 'interrupted') continue;
      if (row && row.id > qid) continue;
      failures.set(key, `q:${qid}`);
      if (status === 'interrupted' || (status === undefined && QUIT_ERROR.test(entry.error))) quit.add(key);
      continue;
    }
    // No row to ask (a turn outside the queue): a newer finished row wins.
    if (row && row.status === 'done') continue;
    failures.set(key, `f:${entry.finishedAt ?? 0}`);
    if (QUIT_ERROR.test(entry.error)) quit.add(key);
  }
  const waiting = new Set(ev.waitingKeys.filter((k): k is string => !!k));
  const paused = new Set(ev.paused ?? []);
  const states = new Map<string, LiveSessionState>();
  const keys = new Set<string>([...running, ...queued, ...waiting, ...failures.keys(), ...paused]);
  for (const key of keys) {
    const unseen = failures.has(key) && ev.seenFailures.get(key) !== failures.get(key) && !running.has(key) && !queued.has(key);
    // A paused turn ended `cancelled`, never failed: its earlier failure
    // token (if any) is not what it is doing now.
    const isPaused = paused.has(key);
    const state = sessionStateOf({ waiting: waiting.has(key), failed: unseen && !quit.has(key) && !isPaused, interrupted: unseen && quit.has(key) && !isPaused, running: running.has(key), queued: queued.has(key) && !running.has(key), paused: isPaused });
    if (state !== 'idle') states.set(key, state);
  }
  return { states, failures };
}

/** A run's status word (run queue, Mission Control card) as a session state:
 *  running/starting/claimed run, queued waits for a slot, failed failed,
 *  interrupted its own (P1-40); finished, stopped and skipped are idle. */
export function stateOfRunStatus(status: string): SessionState {
  if (status === 'running' || status === 'starting' || status === 'claimed') return 'running';
  if (status === 'queued') return 'queued';
  if (status === 'failed') return 'failed';
  if (status === 'interrupted') return 'interrupted';
  return 'idle';
}
