// Task status vocabulary, pure (no store or api imports), so helpers that
// must stay import-light can use it (dragdrop.ts re-exports it).

/// Tasks and Projects terminal states; case-insensitive and vocabulary-tolerant
/// so a task the app marked `Delivered` and one the chat wrote as `Done` /
/// `Complete` are treated the same (mirrors `vault_tasks::is_terminal_status`).
export function isCompletionStatus(status: string | null): boolean {
  if (!status) return false;
  const s = status.trim().toLowerCase();
  return s === 'delivered' || s === 'done' || s === 'deployed' || s === 'complete' || s === 'completed';
}

/** Cancelled, won't do or closed: no longer to do, without saying it was
 *  finished. Kept apart from `isCompletionStatus`, which also picks the
 *  status a tick writes (a Notion "Closed" option must never win over Done). */
export function isCancelledStatus(status: string): boolean {
  const words = status.trim().toLowerCase().replace(/[‘’`]/g, "'").split(/\s+/).filter(Boolean).join(' ');
  return ['cancelled', 'canceled', "won't do", 'wont do', 'closed'].includes(words);
}

/** Still to do (P1-12): the one rule Agenda, bulk edits, Home's Due card and
 *  Today's facts share, so a task is open or closed in the same way
 *  everywhere. Snoozes stay a display filter on top. */
export function isOpenTask(status: string | null): boolean {
  return !isCompletionStatus(status) && !isCancelledStatus(status ?? '');
}

