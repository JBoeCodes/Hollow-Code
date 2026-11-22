// Bulk review of learning proposals in Waiting on you: which learnings are
// selectable, the Select all box's state, the optional Reject reason, and
// what the footer says after a batch or its Undo. Pure, so the selection
// rules are tested without the panel. The backend resolves each proposal
// on its own (`resolve_learning_proposals_batch`), so every message here
// names partial failures rather than reading a batch as all-or-nothing.
import type { LearningBatchOutcome } from './generated/ipc';
import type { ReviewInboxItem } from './types';
import type { NeedsYouItem } from './workspaceTypes';
import { isWorkspaceReviewItem } from './workspaceContent';

/** Longest Reject reason the backend stores (`MAX_REJECT_REASON_CHARS`). */
export const REJECT_REASON_MAX = 500;
/** Most proposals one batch takes (the backend's `MAX_BATCH`). */
export const BATCH_MAX = 500;

/** Whether a review item can be ticked for bulk review: a pending learning. */
export function isBulkLearning(item: ReviewInboxItem | undefined): boolean {
  return !!item && item.kind === 'learning' && item.status === 'pending' && isWorkspaceReviewItem(item);
}

/** Whether a Needs you row is a pending learning. The bell and Needs you
 *  list and count it, but it opens Library → Learnings, its one home, and
 *  is never decided there (overhaul J7 review). */
export function opensInLearnings(item: Pick<NeedsYouItem, 'kind' | 'category'>): boolean {
  return item.kind === 'review' && item.category === 'learning';
}

/** The count on Library's Learnings tab: pending learnings in the review
 *  queue snapshot. None while there are none, or while the count is
 *  unknown (not read yet, or the last read failed): never a false number. */
export function learningsWaiting(items: readonly ReviewInboxItem[] | undefined, failed: boolean): number | undefined {
  const count = !items || failed ? 0 : pendingLearningIds(items).length;
  return count > 0 ? count : undefined;
}

/** Every pending learning in the review queue snapshot, in its order. The
 *  snapshot holds the whole queue, so Select all reaches learnings the
 *  paged list hasn't shown yet. */
export function pendingLearningIds(items: readonly ReviewInboxItem[] | undefined): string[] {
  return (items ?? []).filter(isBulkLearning).map(item => item.id);
}

/** What Select all ticks: every pending learning, up to [`BATCH_MAX`]. */
export function selectAllIds(pending: readonly string[]): Set<string> {
  return new Set(pending.slice(0, BATCH_MAX));
}

/** How many of `ids` the Jev duplicate check flagged as possibly repeating
 *  a saved learning (`possibleDuplicate`). They stay selectable; the bar
 *  says so, so a bulk Accept doesn't surprise anyone. */
export function mayRepeat(items: readonly ReviewInboxItem[] | undefined, ids: ReadonlySet<string>): number {
  return (items ?? []).filter(item => ids.has(item.id) && isBulkLearning(item)
    && typeof item.payload.possibleDuplicate === 'string' && item.payload.possibleDuplicate.trim() !== '').length;
}

/** `selected` with `id` flipped. */
export function toggleId(selected: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(selected);
  if (next.has(id)) next.delete(id); else next.add(id);
  return next;
}

/** The selection, keeping only learnings still pending (one resolved
 *  elsewhere, or by the last batch, drops out). */
export function keepPending(selected: ReadonlySet<string>, pending: readonly string[]): Set<string> {
  const live = new Set(pending);
  return new Set([...selected].filter(id => live.has(id)));
}

/** The Select all box: ticked when every pending learning is (or the
 *  first [`BATCH_MAX`] of more), mixed when some are. */
export function selectAllState(selected: ReadonlySet<string>, pending: readonly string[]): 'none' | 'some' | 'all' {
  const count = pending.filter(id => selected.has(id)).length;
  if (count === 0) return 'none';
  return count >= Math.min(pending.length, BATCH_MAX) ? 'all' : 'some';
}

/** The reason as sent: trimmed, blank as none; `problem` when too long. */
export function rejectReason(text: string): { reason: string | undefined; problem: string | null } {
  const reason = text.trim();
  if ([...reason].length > REJECT_REASON_MAX) return { reason: undefined, problem: `Keep the reason under ${REJECT_REASON_MAX} characters.` };
  return { reason: reason || undefined, problem: null };
}

/** The words a row's Save sends with its one-item batch: the person's edit
 *  when it differs from the proposal's statement, else none (saved as
 *  proposed). Either way the Save has the batch's Undo. */
export function editedWords(edited: string | null, statement: string): string | undefined {
  return edited !== null && edited !== statement ? edited : undefined;
}

/** ME1: the version an edit is pinned to. The first change of words keeps
 *  the version the card showed then; a refresh of the item under an open
 *  edit doesn't move it, so a stale edit can't borrow a newer version. */
export function pinEditVersion(pinned: string | undefined, shown: string | undefined): string | undefined {
  return pinned ?? shown;
}

/** ME1: the version a Save sends: the one its edit began at, else the
 *  version the card shows now (a Save of the words as shown). */
export function versionForSave(editedAt: string | undefined, shown: string | undefined): string | undefined {
  return editedAt ?? shown;
}

const learnings = (n: number) => (n === 1 ? 'one learning' : `${n} learnings`);

/** The ids a batch couldn't resolve, with why. */
export function failures(outcome: LearningBatchOutcome): { id: string; error: string }[] {
  return outcome.items.filter(item => !item.ok).map(item => ({ id: item.id, error: item.error ?? 'failed' }));
}

/** The snackbar's Details: one line per failure, named by its subject
 *  (`titleOf`), never by a raw review id. */
export function failureDetail(outcome: LearningBatchOutcome, titleOf: (id: string) => string | undefined): string | undefined {
  const bad = failures(outcome);
  return bad.length ? bad.map(f => `${titleOf(f.id) || 'A learning'}: ${f.error}`).join('\n') : undefined;
}

/** The footer line after a bulk Accept or Reject. `failed` is true when any
 *  proposal didn't go through; the first reason is quoted. */
export function batchMessage(outcome: LearningBatchOutcome): { text: string; failed: boolean } {
  const done = outcome.items.filter(item => item.ok).length;
  const bad = failures(outcome);
  const verb = outcome.action === 'accept' ? 'Accepted' : 'Rejected';
  if (!bad.length) return { text: `${verb} ${learnings(done)}`, failed: false };
  const why = `: ${bad[0].error}${bad.length > 1 ? ` (and ${bad.length - 1} more)` : ''}`;
  if (!done) return { text: `× Couldn’t ${outcome.action} ${learnings(bad.length)}${why}`, failed: true };
  return { text: `× ${verb} ${done} of ${outcome.items.length} learnings; ${bad.length} couldn’t be${why}`, failed: true };
}

/** The footer line after Undo. */
export function undoMessage(outcome: LearningBatchOutcome): { text: string; failed: boolean } {
  const done = outcome.items.filter(item => item.ok).length;
  const bad = failures(outcome);
  if (!bad.length) return { text: `Undone: ${learnings(done)} back in Needs you`, failed: false };
  return { text: `× Undid ${done} of ${outcome.items.length}; ${bad.length} stayed as they were: ${bad[0].error}`, failed: true };
}

/** The last bulk Accept or Reject that can still be undone, after its
 *  snackbar has gone (overnight A06): its batch id, what it did, and how
 *  many learnings it changed (after a partial Undo, how many stayed). */
export interface LastBatch {
  id: string;
  action: 'accept' | 'reject';
  count: number;
  /** The batch's own Undo snackbar, dismissed once the button's Undo worked:
   *  its Undo would only say "already undone" (overnight A06 review). */
  snackbar?: string;
  /** The last Undo of it put nothing back (overnight A06 review). */
  stalled?: boolean;
}

/** What "Undo last batch" holds after an Undo of `batch` (overnight A06
 *  review): nothing once every learning went back; after a partial Undo, the
 *  ones that stayed, a best guess at what another press can still put back
 *  (a rolled-back trash failure can; a note that isn't the proposal's never
 *  will). A second Undo in a row that put nothing back lets it go, so a
 *  learning that can't be undone doesn't pin the button. */
export function batchAfterUndo(batch: LastBatch, outcome: LearningBatchOutcome): LastBatch | null {
  const left = failures(outcome).length;
  if (!left) return null;
  const stalled = !outcome.items.some(item => item.ok);
  if (stalled && batch.stalled) return null;
  return { ...batch, count: left, stalled };
}

/** An Undo refused because nothing in the batch is left to undo (the
 *  backend's "nothing to undo: …": undone already, or each learning resolved
 *  again since). Any other error keeps "Undo last batch" to try again. */
export function nothingLeftToUndo(error: unknown): boolean {
  return String(error instanceof Error ? error.message : error).startsWith('nothing to undo');
}

/** The batch one saved learning's own Undo reverses, or null. Only an
 *  automatic Know-how save gets that button: its batch is the backend's
 *  single-item `learning-batch-auto-<id>`. A bulk Accept's batch holds other
 *  learnings too, so it is undone with "Undo last batch", never per card. */
export function autoSaveUndoBatch(id: string, batchId: string | null | undefined): string | null {
  return batchId === `learning-batch-auto-${id}` ? batchId : null;
}

/** The words on the "Undo last batch" button, e.g. "Undo last batch
 *  (2 learnings rejected)". */
export function undoLastLabel(action: 'accept' | 'reject', count: number): string {
  return `Undo last batch (${count} ${count === 1 ? 'learning' : 'learnings'} ${action === 'accept' ? 'accepted' : 'rejected'})`;
}

/** Before a bulk Accept (P0-10): the confirm naming the selected learnings
 *  that aren't listed on screen (Select all takes every pending one, past
 *  the list's first page too), or null when every one is shown. */
export function offScreenAccept(
  ids: readonly string[],
  listed: ReadonlySet<string>,
  subjectOf: (id: string) => string | undefined,
): { title: string; body: string; confirmLabel: string } | null {
  const hidden = ids.filter(id => !listed.has(id));
  if (!hidden.length) return null;
  const names = hidden.slice(0, 5).map(id => `“${subjectOf(id) ?? id}”`).join(', ');
  const more = hidden.length > 5 ? `, and ${hidden.length - 5} more` : '';
  return {
    title: `Accept ${ids.length} ${ids.length === 1 ? 'learning' : 'learnings'}?`,
    body: `${hidden.length} of them ${hidden.length === 1 ? 'isn’t' : 'aren’t'} on screen: ${names}${more}.`,
    confirmLabel: `Accept ${ids.length}`,
  };
}
