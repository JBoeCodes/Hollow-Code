/**
 * Queued chat turns (Paperclip PR 7b, D3): the pure decisions behind the
 * composer's queued-turn state.
 *
 * `chat_send` enqueues a turn and resolves at once with `SendAccepted`; the run
 * pump admits the turn once the vault lease is free and emits `runStarted`.
 * Every event of that turn echoes the send's `clientTurnId` nonce (and its
 * `queueId`), so the composer can tell which local turn an event belongs to,
 * including when `runStarted` arrives before the `SendAccepted` reply.
 *
 * A turn that never started (cancelled while queued, interrupted, or recovered
 * at claim because its working folder vanished) was never sent: its user
 * bubble is removed and its text goes back to the composer, unless the user
 * chose Discard. A recovered turn never adopts a session key.
 *
 * Type-only imports: the node vitest suite loads this module directly.
 */
import type { CannedRequest, RunQueueItem, SendAccepted } from './types';

/** A request Hollow drafted into the composer (a curated Refresh, Explain
 *  visually) and its tag (overnight 9 A1). */
export type CannedDraft = { text: string; canned: CannedRequest };

/** The tag a composer send of `sent` goes with: the draft's, only when the
 *  person sent its text unchanged. Any edit makes it their own words. */
export function cannedSend(draft: CannedDraft | null, sent: string): CannedRequest | undefined {
  return draft && draft.text.trim() === sent ? draft.canned : undefined;
}

/** The prefix of the error a turn gets when its working folder vanished:
 *  an invoke rejection before anything is enqueued, or (Paperclip PR 7b) an
 *  `error` event followed by a keyless recovered `done` when the pump finds
 *  the folder gone at claim. */
export const CHAT_CONTEXT_RECOVERED_PREFIX = 'CHAT_CONTEXT_RECOVERED:';

/** Why the composer withdrew a queued turn. Stop (and a backend-side end)
 *  carry no intent: the text is restored, as for Edit. */
export type QueuedTurnIntent = 'edit' | 'discard' | null;

/** What an event means for the local queued turn:
 *  - `started`: the pump admitted it; drop the queued badge.
 *  - `restore`: it ended without starting; remove the bubble, refill the composer.
 *  - `drop`: it ended without starting and the user discarded it.
 *  - `none`: not this turn's event, or nothing to do. */
export type QueuedTurnDecision = 'started' | 'restore' | 'drop' | 'none';

/** The `chat-event` fields the reconciliation reads. */
export interface ChatQueueEvent {
  type: string;
  clientTurnId?: string | null;
  queueId?: number | null;
  /** `cancelled` only: the turn was removed from the queue before it started. */
  queued?: boolean;
  /** `done` only: the turn's working folder vanished at claim; nothing ran. */
  recovered?: boolean;
}

/** What the composer knows about the turn it sent (or adopted at boot). */
export interface LocalQueuedTurn {
  clientTurnId: string;
  /** Null until `SendAccepted` resolves. */
  queueId: number | null;
  started: boolean;
}

/** Whether an event belongs to the local turn. The nonce is authoritative;
 *  the queue id covers a turn adopted after a reload, whose nonce the frontend
 *  never saw. An event carrying neither (a pre-7b backend) matches nothing. */
export function eventMatchesTurn(event: ChatQueueEvent, local: Pick<LocalQueuedTurn, 'clientTurnId' | 'queueId'> | null): boolean {
  if (!local) return false;
  if (event.clientTurnId && event.clientTurnId === local.clientTurnId) return true;
  return event.queueId != null && local.queueId != null && event.queueId === local.queueId;
}

/** Whether an event belongs to a turn that already ended, so it must not
 *  touch the current one. Edit and Discard end a turn as soon as
 *  `cancel_queued_run` answers, which re-enables the composer before that
 *  turn's own `cancelled` + `done` arrive; a new send can be in flight by
 *  then. An event of a turn in `ended` is stale; so is any other tagged event
 *  while a different local turn exists (only one chat turn can be live). An
 *  untagged event (a pre-7b backend) is never stale. */
export function isStaleTurnEvent(
  event: ChatQueueEvent,
  local: Pick<LocalQueuedTurn, 'clientTurnId' | 'queueId'> | null,
  ended: ReadonlyArray<Pick<LocalQueuedTurn, 'clientTurnId' | 'queueId'>>,
): boolean {
  if (local && eventMatchesTurn(event, local)) return false;
  if (ended.some(turn => eventMatchesTurn(event, turn))) return true;
  const tagged = !!event.clientTurnId || event.queueId != null;
  return tagged && !!local;
}

/** Decide what an event means for the local queued turn. */
export function queuedTurnEnd(
  event: ChatQueueEvent,
  local: LocalQueuedTurn | null,
  intent: QueuedTurnIntent,
): QueuedTurnDecision {
  if (!local || !eventMatchesTurn(event, local)) return 'none';
  const ended: QueuedTurnDecision = intent === 'discard' ? 'drop' : 'restore';
  switch (event.type) {
    case 'runStarted':
      return 'started';
    case 'cancelled':
      // A running turn's stop is an ordinary stop: its bubble was sent.
      return event.queued && !local.started ? ended : 'none';
    case 'done':
      if (event.recovered) return ended;
      return local.started ? 'none' : ended;
    default:
      return 'none';
  }
}

/** Whether `SendAccepted` should show the queued badge: not when `runStarted`
 *  already arrived for this send, or the turn has already ended. */
export function acceptedShowsBadge(local: LocalQueuedTurn | null, clientTurnId: string): boolean {
  return !!local && local.clientTurnId === clientTurnId && !local.started;
}

/** The badge text on a queued user bubble. It only shows for a turn that
 *  truly can't start (`isVisiblyQueued`): the reason is the backend's wait
 *  text for a chat-count limit, memory or another run holding its folder.
 *  A place in line alone is not a reason (2026-11-16, Working, not Queued). */
export function queuedBadgeText(q: Pick<SendAccepted, 'waitingFor'>): string {
  return q.waitingFor ? `Queued · waiting for ${q.waitingFor}` : 'Queued';
}

/** Where a sent turn stands before its run starts (2026-11-16, Working, not
 *  Queued): `queued` only with a real wait reason (`waitingFor`: a chat-count
 *  limit, memory, another run's folder); once the row is accepted with no
 *  reason it is `preparing` (claimed, or about to be: admission, recall,
 *  retrieval, prompt assembly and the agent's boot all count as working);
 *  `started` after `runStarted`. `sending` is before the queue has the row. */
export type TurnStanding = 'sending' | 'queued' | 'preparing' | 'started';
export function turnStanding(q: Pick<LocalQueuedTurn, 'queueId' | 'started'> & { waitingFor: string | null; heldReason?: string | null }): TurnStanding {
  if (q.started) return 'started';
  if (q.queueId == null) return 'sending';
  // A permission hold (`heldReason`) truly holds it too: the dock says
  // paused, so the composer never says Preparing (2026-11-17).
  return q.waitingFor || q.heldReason ? 'queued' : 'preparing';
}

/** The transcript notice a withdrawn queued turn leaves, if any. Edit needs
 *  none: the text reappearing in the composer says it. */
export function queuedCancelNotice(intent: QueuedTurnIntent): string | null {
  if (intent === 'edit') return null;
  if (intent === 'discard') return 'Queued message discarded. It was never sent.';
  return 'Stopped before it started. Your message is back in the composer.';
}

/** The transcript notice when a queued turn fails before it starts (for
 *  example Hollow could not admit it). Its `done` puts the text back. */
export function queuedFailureNotice(message: string, restored = true): string {
  const reason = message.trim().replace(/[.\s]+$/, '');
  // `restored` false (2026-10-15): the message box already held other text,
  // so the unsent message was not put back there; don't claim it was.
  const back = restored ? ' It is back in the composer.' : '';
  return reason
    ? `Your queued message was not sent: ${reason}.${back}`
    : `Your queued message was not sent.${back}`;
}

/** The composer text after restoring a queued turn. Anything typed while the
 *  turn waited is kept below it; a draft that already holds it is unchanged. */
export function restoredDraft(queuedText: string, currentDraft: string): string {
  if (!currentDraft.trim()) return queuedText;
  if (!queuedText.trim() || currentDraft.includes(queuedText)) return currentDraft;
  return `${queuedText}\n\n${currentDraft}`;
}

/** `text` added to the composer after what is already there (a blank line
 *  between), never replacing it; a draft that already holds it is unchanged
 *  (morning M5: "Work on this" and "Update brief" keep a person's draft). */
export function addToDraft(draft: string, text: string): string {
  if (!draft.trim()) return text;
  if (!text.trim() || draft.includes(text.trim())) return draft;
  return `${draft.trimEnd()}\n\n${text}`;
}

/** A nonce for one send. Not a secret; only unique within this window. */
export function newClientTurnId(): string {
  const uuid = (globalThis as { crypto?: { randomUUID?: () => string } }).crypto?.randomUUID?.();
  if (uuid) return `ct-${uuid}`;
  return `ct-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`;
}

// ── Stop while a send gets ready (overnight 11-13 C10) ───────────────────────
// From its first await (the agent check, the working-folder preflight, a
// card's answers) until `chat_send` answers, a send is busy, so Stop shows,
// but there is no row for `chat_cancel` to find: a fresh chat has no key yet
// (the backend refused) and an existing one answered "nothing to stop", and
// the turn ran anyway. Stop marks that one send here instead; the send reads
// the mark before it queues, and withdraws its accepted row if `chat_send`
// was already in flight. The only mutable state in this module.

/** One send getting ready, by its own `clientTurnId`. `queued` once its
 *  bubble and queued turn exist (`chat_send` in flight). */
export interface PreparingSend {
  readonly clientTurnId: string;
  readonly key: string | null;
  queued: boolean;
  stopped: boolean;
}

const preparingSends = new Map<string, PreparingSend>();

/** Register a send getting ready in the conversation `key` (null for a
 *  fresh chat). Call `endPreparingSend` on every way out. */
export function beginPreparingSend(clientTurnId: string, key: string | null): PreparingSend {
  const send: PreparingSend = { clientTurnId, key, queued: false, stopped: false };
  preparingSends.set(clientTurnId, send);
  return send;
}

/** That send has queued (or given up): a Stop from here on is for its row. */
export function endPreparingSend(send: PreparingSend): void {
  if (preparingSends.get(send.clientTurnId) === send) preparingSends.delete(send.clientTurnId);
}

/** A send still getting ready (not queued yet) in the conversation `key`
 *  (ENG-73): a second Enter in its preflight is the same words again, never
 *  a steer or a queued second copy. */
export function preparingSendIn(key: string | null): boolean {
  for (const send of preparingSends.values()) if (!send.queued && send.key === key) return true;
  return false;
}

/** Stop pressed: mark the send getting ready in the conversation on screen
 *  (`key`), or the one whose queued turn with no row yet is on screen
 *  (`queuedClientTurnId`). True when one was marked, and then nothing else
 *  should be stopped. Never a send in another conversation, nor a later one. */
export function stopPreparingSend(key: string | null, queuedClientTurnId: string | null): boolean {
  for (const send of [...preparingSends.values()].reverse()) {
    if (send.queued ? send.clientTurnId !== queuedClientTurnId : send.key !== key) continue;
    send.stopped = true;
    return true;
  }
  return false;
}

/** The placeholder nonce of a turn adopted from the queue after a reload. */
export function adoptedClientTurnId(queueId: number): string {
  return `queue-${queueId}`;
}

// ── Boot adoption and restore ────────────────────────────────────────────────

export const RESTORED_QUEUED_TURNS_CAP = 20;
export const RESTORED_QUEUED_TURNS_KEY = 'restoredQueuedTurns';
/** `run_pump::CARD_HOLD_WAIT`. A queued row with this reason is a wake held
 *  for an open card, not a message the person sent. */
export const CARD_HOLD_WAIT = 'an open question card';
/** Shown once when a turn queued before Hollow quit is put back. */
export const INTERRUPTED_TURN_NOTICE =
  'A message you queued before Hollow quit was never sent. It is back in the composer; send it again when you are ready.';

const LIVE: ReadonlySet<string> = new Set(['queued', 'claimed', 'running']);

/** Rows ids are per vault store, so the restored marker also carries the
 *  request time: another vault's row 3 is not this vault's row 3. */
export function restoredKey(row: Pick<RunQueueItem, 'id' | 'requestedAt'>): string {
  return `${row.id}@${row.requestedAt}`;
}

/** Newest first, deduplicated, capped. */
export function mergeRestoredKeys(existing: string[], added: string[], cap = RESTORED_QUEUED_TURNS_CAP): string[] {
  const out: string[] = [];
  for (const key of [...added, ...existing]) {
    if (!out.includes(key)) out.push(key);
    if (out.length >= cap) break;
  }
  return out;
}

/** Where a live chat row stands in the queue, read the same way on adopt and update. */
export function rowWait(row: RunQueueItem) {
  return { queuePosition: row.position, waitingFor: row.waitingHolder ?? row.waitingReason, waitingSession: row.waitingHolder ? undefined : row.waitingSession, heldReason: row.heldReason };
}
type RowWait = ReturnType<typeof rowWait>;

export type QueuedRowAction =
  /** No local turn: a live chat row exists (sent before a reload). */
  | { kind: 'adopt'; row: RunQueueItem }
  /** The local turn's row moved: refresh its place in line and holder. */
  | ({ kind: 'update'; started: boolean } & RowWait)
  /** The adopted turn's row is no longer live, and its end event was missed. */
  | { kind: 'unadopt' }
  /** A turn the previous Hollow process queued but never started. */
  | { kind: 'restore'; row: RunQueueItem; seen: string[] };

export interface QueuedRowContext {
  /** The local queued turn, if any. */
  local: (LocalQueuedTurn & { adopted: boolean }) | null;
  /** A turn is in flight locally (including one still awaiting SendAccepted). */
  busy: boolean;
  /** Only this conversation may adopt or restore a queue row. Null is an unsent draft. */
  foregroundKey: string | null;
  /** Snapshots own their events until explicitly restored by navigation. */
  backgroundKeys: readonly string[];
  composerEmpty: boolean;
  restoredKeys: string[];
}

/** Reconcile the composer with a fresh `list_run_queue` read. */
export function planQueuedChatRows(items: RunQueueItem[], ctx: QueuedRowContext): QueuedRowAction[] {
  const chat = items.filter(item => item.source === 'chat');
  const actions: QueuedRowAction[] = [];
  const local = ctx.local;
  if (local && local.queueId != null) {
    const row = chat.find(item => item.id === local.queueId);
    if (row && LIVE.has(row.status)) {
      actions.push({ kind: 'update', ...rowWait(row), started: row.status === 'running' });
    } else if (local.adopted) {
      actions.push({ kind: 'unadopt' });
    }
    return actions;
  }
  if (local || ctx.busy) return actions;
  if (!ctx.foregroundKey || ctx.backgroundKeys.includes(ctx.foregroundKey)) return actions;
  const own = chat.filter(item => item.sourceId === ctx.foregroundKey);
  // A wake held for an open card must not become the composer's turn: that
  // sets chatBusy and disables the card's Send for the whole hold.
  const live = own.find(item => LIVE.has(item.status) && !(item.status === 'queued' && item.waitingReason === CARD_HOLD_WAIT));
  if (live) {
    actions.push({ kind: 'adopt', row: live });
    return actions;
  }
  const interrupted = own
    .filter(item => item.status === 'interrupted' && !!item.detail?.trim() && !ctx.restoredKeys.includes(restoredKey(item)))
    .sort((a, b) => b.id - a.id);
  if (interrupted.length > 0 && ctx.composerEmpty) {
    actions.push({ kind: 'restore', row: interrupted[0], seen: interrupted.map(restoredKey) });
  }
  return actions;
}
