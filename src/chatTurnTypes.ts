// A conversation's turns and tool calls (moved out of store/runtime.ts in
// overnight 10-11 B3, so a conversation engine can live outside the global
// store; runtime.ts re-exports these names, so importers are unchanged).
import type { ApprovalBadge } from './chatApproval';
import type { ExecutionReceipt } from './executionReceipt';

export type ChatToolStatus = 'running' | 'done' | 'error';
export interface ChatToolUse {
  name: string;
  input?: unknown;
  /** Backend tool_use id — used to match the tool_result back to this call. */
  id?: string;
  /** Flattened tool_result text once the call returns. */
  result?: string;
  /** WD1: a stored result the cold open sent as a preview: its full length
   *  in characters (the row fetches the rest when it expands). */
  resultChars?: number;
  /** WD1: a trimmed Read result's numbered lines ("Read N lines"). */
  resultLines?: number;
  /** WD1: the stored turn a trimmed result is read back from. */
  resultTurn?: number;
  /** True when the tool returned an error result. */
  isError?: boolean;
  /** Lifecycle: running → done/error. Drives the per-call status indicator. */
  status?: ChatToolStatus;
  /** When its result arrived (live only; a reloaded transcript has none). The
   *  "Worked for" line runs to the last one (overnight 11-12, C1). */
  endedAt?: string;
  expanded?: boolean;
  /** Data-URL images produced by this call (e.g. a teammate-generated image),
   *  rendered inline under the tool call so the picture shows in the chat. */
  images?: string[];
  /** Paperclip PR 21: the approval bound to this exact call (the broker
   *  matched its tool and arguments). Live only; a reloaded transcript has
   *  none. Absent when no approval was ever bound to it. */
  approval?: ApprovalBadge;
  /** AG7: the spawning call's id when a Claude subagent made this call
   *  (`parent_tool_use_id`), live and reloaded. Absent for a main-thread
   *  call and for agents that don't report it. */
  parentToolUseId?: string;
}

export interface ChatTurn {
  /** Completed provider-reported reasoning, never part of the answer. */
  reasoning?: string;
  role: 'user' | 'assistant' | 'system';
  agent?: string;
  text: string;
  tools: ChatToolUse[];
  /** Live explicit provider plan proposal, not inferred from reply prose. */
  proposedPlan?: string;
  /** Live context compiler progress, never an agent acceptance claim. */
  handoff?: { state: 'pending' | 'complete' | 'unavailable'; entries: number };
  /** True while the assistant is still streaming this turn. */
  partial?: boolean;
  /** Data-URL previews of images the user attached to this (user) turn. */
  images?: string[];
  /** Files that aren't images the user attached to this (user) turn (C5):
   *  each opens in the Preview pane from its chip. Live turns only. */
  files?: { name: string; path: string; size?: number }[];
  /** A user turn that went with a page shared from the Preview pane (shared
   *  pages aren't in `images`). Live only: Retry and Edit, which send words
   *  only, aren't offered for it (`messageActions.ts` `repeatable`). */
  sharedPage?: boolean;
  /** When a user turn was sent (ISO): set by the send, or from a saved
   *  conversation's turn start (`chatHydrate.ts` `savedUserAt`). On a reply,
   *  when it finished: its saved turn's end (`savedTurnAt`) or the moment
   *  the live result came (`markReply`). Absent when unknown, so the
   *  message shows no time rather than a wrong one. */
  at?: string;
  /** Durable backend evidence for this exact live run, when available. */
  receipt?: ExecutionReceipt;
  /** User turns sent from this window: the send's nonce, echoed by every
   *  event of that turn (Paperclip PR 7b). */
  clientTurnId?: string;
  /** The run-queue row of that send, once `SendAccepted` names it. */
  queueId?: number;
  /** The exact history run id of the turn this bubble belongs to (Paperclip
   *  PR 17), the identity a saved feedback vote uses. Set ONLY from exact
   *  sources: the live `runStarted` id, the live receipt, a Hollow-store
   *  reload, an artifact summary. Legacy Claude-history turns never carry it
   *  (their receipts are joined by timing, which is not identity). */
  runId?: string;
  /** A restored turn's stored status (`success`, `error`, `cancelled`,
   *  `interrupted`, …) and result kind (Paperclip PR 14), or a live reply's
   *  `success`/`error` and kind (P1-27, `markReply`); Continue and Retry read them. */
  status?: string | null;
  resultKind?: string | null;
  /** A restored turn stopped before its agent was spawned (Paperclip PR 14b). */
  stoppedBeforeSpawn?: boolean;
  /** A live stop notice: the `cancelled` event of a turn whose agent could
   *  run (not one stopped before spawn). */
  stopNotice?: boolean;
  /** A notice appended after a restored transcript; it describes an earlier
   *  turn, so Continue looks past it. */
  hydratedNotice?: boolean;
  /** See `ContinueTurn.aboutEarlierTurn` (chatContinue.ts). */
  aboutEarlierTurn?: boolean;
  /** A system row shown quietly (no kicker, no box): `accountSwitch` is R1's
   *  "Switched to Tech (Admin hit its limit)"; `steer` is the person's own
   *  mid-turn message (`steer` below, chatSteer.ts); `remoteRun` says the
   *  turn ran on the Mac mini runner and how its work came back
   *  (remoteRunNotice.ts). */
  notice?: 'accountSwitch' | 'steer' | 'remoteRun';
  /** A `remoteRun` row whose text waits for remoteRunNotice.ts to load (it
   *  loads on first use; chatRuntime.ts fills the row, here or backgrounded). */
  remoteRunPending?: import('./remoteRunNotice').RemoteRunEvent;
  /** A steer row's id, when it was sent, where among the reply's tool calls,
   *  and whether the agent took it. */
  steer?: { id: string; at: string; state: import('./chatSteer').SteerState; afterTools: number; reason?: string | null };
}
