/**
 * Paperclip PR 14 (14b): the manual Continue after an interrupted or
 * stopped turn, and the notice a fresh native session is announced with.
 * Pure and dependency-free (vitest loads it in node).
 */

/** Exactly what Continue sends, on the same conversation. Never automatic. */
export const CONTINUE_PROMPT = 'Your previous run was interrupted. Continue from where you left off.';

/** The fields of a chat turn Continue reads (a subset of `ChatTurn`). */
export interface ContinueTurn {
  role: 'user' | 'assistant' | 'system';
  /** A hydrated turn's stored status (`success`, `error`, `cancelled`, `interrupted`, …),
   *  or a live reply's `success`/`error` (P1-27, store/chatRuntime.ts `markReply`). */
  status?: string | null;
  /** The turn's result kind (Paperclip PR 14), when known. */
  resultKind?: string | null;
  /** A restored turn stopped before its agent was spawned: its native
   *  session never saw the request, so Continue would skip past it. */
  stoppedBeforeSpawn?: boolean;
  /** A live stop notice (a `cancelled` event of a turn whose agent could
   *  run; false for a stop before spawn). */
  stopNotice?: boolean;
  /** A notice ConversationHistory.tsx appends after a restored transcript
   *  (incomplete history, a turn's error): it describes an EARLIER turn. */
  hydratedNotice?: boolean;
  /** A notice about an earlier turn that leaves it as it was (the "message
   *  you queued … is back in the composer" notice, plan completion L5):
   *  Continue looks past it; nothing else does. */
  aboutEarlierTurn?: boolean;
}

export interface ContinueState {
  busy: boolean;
  queued: boolean;
  sessionId: string | null;
}

/** Whether Continue is offered: the chat is idle, nothing is queued, the
 *  conversation has a native session to resume, and its last turn (skipping
 *  restored-history notices, which ConversationHistory.tsx appends after every turn) is a
 *  live stop notice, or an assistant turn that was cancelled, interrupted or
 *  stalled, with no user turn after it. A turn stopped before its agent was
 *  spawned never qualifies: its request never reached the native session. */
export function canContinueChat(turns: readonly ContinueTurn[], state: ContinueState): boolean {
  if (state.busy || state.queued || !state.sessionId) return false;
  for (let i = turns.length - 1; i >= 0; i--) {
    const turn = turns[i];
    if (turn.role === 'system' && (turn.hydratedNotice || turn.aboutEarlierTurn)) continue;
    if (turn.role === 'user') return false;
    if (turn.role === 'system') return !!turn.stopNotice;
    if (turn.stoppedBeforeSpawn) return false;
    return turn.status === 'cancelled' || turn.status === 'interrupted' || turn.resultKind === 'stalled';
  }
  return false;
}

/** The fields Retry reads (a subset of `ChatTurn`). */
export interface RetryTurn extends ContinueTurn {
  text?: string;
  /** The receipt's run status (`runStatus`, the run's own exit), never its
   *  `status`, which also says failed for a stop or for checks that failed
   *  after a successful run. */
  receipt?: { runStatus?: string } | null;
  notice?: string;
}

const FAILED_RUN = new Set(['error', 'failed', 'failure']);

/** P1-27: the index of the message Retry sends again, or -1. Offered when
 *  the chat is idle with nothing queued and its last turn (past restored
 *  notices, as Continue) is a reply whose run failed: its status (restored,
 *  or set by a live error, store/chatRuntime.ts) or its receipt's run status.
 *  A stall is Continue's; a refusal isn't a failure to retry (live it ends
 *  with its own notice). The message is the user turn the reply answered,
 *  if the person wrote it (`hostWritten`: a Main agent wake or check-in is
 *  Hollow's) and it has words (an image-only message's path isn't kept). */
export function retryIndex<T extends RetryTurn>(turns: readonly T[], state: { busy: boolean; queued: boolean }, hostWritten: (turn: T) => boolean = () => false): number {
  if (state.busy || state.queued) return -1;
  let i = turns.length - 1;
  while (i >= 0 && turns[i].role === 'system' && (turns[i].hydratedNotice || turns[i].aboutEarlierTurn)) i--;
  const last = turns[i];
  if (!last || last.role !== 'assistant' || last.resultKind === 'stalled' || last.resultKind === 'refusal') return -1;
  if (!FAILED_RUN.has(last.status ?? '') && !FAILED_RUN.has(last.receipt?.runStatus ?? '')) return -1;
  while (i >= 0 && turns[i].role !== 'user') i--;
  return i >= 0 && !hostWritten(turns[i]) && turns[i].text?.trim() ? i : -1;
}

/** Retry under a finished last reply (overnight 2026-11-04 C3, ChatGPT's
 *  regenerate): the reply's footer index and the message it sends again, or
 *  null. Offered when the chat is idle with nothing queued and its last turn
 *  (past restored notices, as Retry above) is a reply whose run succeeded.
 *  A failed run has Retry above; a stopped, interrupted or stalled one has
 *  Continue; a refusal is left alone, as above. The message is the one the
 *  reply answered, on the same terms as Retry: written by the person, with
 *  words. Sending it is a new message; the earlier reply stays. */
export function regenerateIndex<T extends RetryTurn>(turns: readonly T[], state: { busy: boolean; queued: boolean }, hostWritten: (turn: T) => boolean = () => false): { reply: number; ask: number } | null {
  if (state.busy || state.queued) return null;
  let i = turns.length - 1;
  while (i >= 0 && turns[i].role === 'system' && (turns[i].hydratedNotice || turns[i].aboutEarlierTurn)) i--;
  const last = turns[i];
  if (!last || last.role !== 'assistant' || last.resultKind === 'stalled' || last.resultKind === 'refusal') return null;
  if (FAILED_RUN.has(last.status ?? '') || FAILED_RUN.has(last.receipt?.runStatus ?? '')) return null;
  if (last.status !== 'success' && !(last.status == null && last.receipt?.runStatus === 'success')) return null;
  const reply = i;
  while (i >= 0 && turns[i].role !== 'user') i--;
  return i >= 0 && !hostWritten(turns[i]) && turns[i].text?.trim() ? { reply, ask: i } : null;
}

/** Whether a live `cancelled` event's notice can offer Continue: not after
 *  a stop before spawn (`beforeSpawn`), whose request never reached the
 *  native session. */
export function stopOffersContinue(event: { beforeSpawn?: boolean }): boolean {
  return !event.beforeSpawn;
}

/** Why the backend moved the conversation to a fresh native session. */
export type SessionResetReason = 'configChanged' | 'resumeFailed' | 'moved' | 'invalidId' | 'rested';

/** Overnight 9 A9: a large Main-agent chat back after more than an hour
 *  started fresh; `away` is the backend's "away 3 h at 714K" (absent when it
 *  had no figures). */
export function restedNotice(agentLabel: string, away?: string | null): string {
  const why = away && away.trim() ? `Started fresh: ${away.trim()}.` : 'Started fresh after more than an hour away.';
  return `${why} Hollow gave a new ${agentLabel} session the recent conversation and its task state instead of re-reading it all.`;
}

/** What the fingerprint parts a config-changed reset names
 *  (`chat_resume::PARTS`, OB03) are called in the notice, in plain words, in
 *  the order the notice lists them; a phrase is said once whichever of its
 *  parts changed. `contract` (Hollow's prompt contract) reads as an update,
 *  not a change, so it is said on its own. `mcp` hashes only the names of
 *  Hollow's own attached tool servers (not the person's connectors) and
 *  `caps` one bit per attached server plus contract and read-only bits, so
 *  the two nearly always change together and share one phrase. `mode` is
 *  the mode class (plan or acting); `workspace` is the prompt kind (bare,
 *  main, quick, workspace, vault), not which workspace (a move is the
 *  separate `moved` reason). `unknown` and any name not here are left out. */
const RESET_PART_NAMES: ReadonlyArray<readonly [readonly string[], string]> = [
  [['agent'], 'the agent'],
  [['model'], 'the model'],
  [['mode'], 'the permission mode'],
  [['cwd'], 'the working folder'],
  [['workspace'], 'the kind of conversation'],
  [['mcp', 'caps'], "Hollow's tools for the agent"],
  [['canvases'], 'the Canvases setting'],
  [['skills'], 'the skills'],
];
const GUIDANCE_UPDATED = "Hollow's guidance for the agent was updated";

/** "a", "a and b", "a, b and c". */
function joinPlain(items: readonly string[]): string {
  if (items.length <= 1) return items.join('');
  return `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]}`;
}

/** The plain-language clause saying what a reset's `changed` parts are,
 *  capitalised ("The model and the permission mode changed"), or null when
 *  the reason isn't `configChanged` or it names none this knows (absent,
 *  empty, `unknown`, unrecognised). An agent change always changes `caps`
 *  (its `contract:{agent}` bits), so beside `agent` a `caps` alone isn't
 *  said; an `mcp` still is. The chat's notice uses it. */
export function configResetClause(
  reason: SessionResetReason | string | null | undefined,
  changed: readonly string[] | null | undefined,
): string | null {
  if (reason !== 'configChanged' || !Array.isArray(changed) || !changed.length) return null;
  const said = changed.includes('agent') ? changed.filter(part => part !== 'caps') : changed;
  const names = RESET_PART_NAMES.filter(([parts]) => parts.some(part => said.includes(part))).map(([, name]) => name);
  const clauses: string[] = [];
  if (names.length) clauses.push(`${joinPlain(names)} changed`);
  if (changed.includes('contract')) clauses.push(GUIDANCE_UPDATED);
  if (!clauses.length) return null;
  const text = clauses.join(' and ');
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** The neutral notice a `sessionReset` event is shown with. A
 *  `configChanged` reset says what changed when the event names it
 *  (`changed`, OB03); otherwise, and for every other reason, the wording is
 *  the generic one. */
export function sessionResetNotice(
  reason: SessionResetReason | string,
  agentLabel: string,
  changed?: readonly string[] | null,
  away?: string | null,
): string {
  if (reason === 'rested') return restedNotice(agentLabel, away);
  if (reason === 'moved') {
    return `This conversation moved to another workspace, so Hollow started a fresh ${agentLabel} session there with the recent conversation.`;
  }
  if (reason === 'invalidId') {
    return `Hollow couldn’t safely pick up the saved ${agentLabel} conversation, so it started a fresh one with the recent conversation.`;
  }
  if (reason === 'resumeFailed') {
    return `The previous ${agentLabel} session could not be resumed, so Hollow started a fresh one with the recent conversation.`;
  }
  const clause = configResetClause(reason, changed);
  if (clause) return `${clause}, so Hollow started a fresh ${agentLabel} session with the recent conversation.`;
  return `This conversation's agent settings changed, so Hollow started a fresh ${agentLabel} session with the recent conversation.`;
}
