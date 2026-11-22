/**
 * "This conversation moved" (next-level P13, 2026-10-01). Moving a
 * conversation to another workspace changes its folder, brief and sources
 * for its next turn (`session_store::move_agent_session`), but a conversation
 * with no native session (every imported one, which continues through the
 * written handoff) gets no `sessionReset` notice from the backend, so nothing
 * on screen said the scope had changed. The move is remembered here (per
 * conversation, with its turn count) and shown as a notice under the
 * transcript until the conversation's next turn has run. Pure apart from the
 * storage it is handed; vitest loads it directly.
 */

export const MOVED_KEY = 'hollow.movedNotices';
const MAX = 50;

type Store = Pick<Storage, 'getItem' | 'setItem'>;
interface Moved { title: string; turns: number; at: string }

function read(store: Store): Record<string, Moved> {
  try { return JSON.parse(store.getItem(MOVED_KEY) ?? '{}') as Record<string, Moved>; } catch { return {}; }
}
function write(store: Store, all: Record<string, Moved>) {
  const keep = Object.entries(all).sort((a, b) => b[1].at.localeCompare(a[1].at)).slice(0, MAX);
  try { store.setItem(MOVED_KEY, JSON.stringify(Object.fromEntries(keep))); } catch { /* the notice is a courtesy */ }
}

/** Remember that `key` moved to `title` when it had `turns` turns. */
export function recordMove(store: Store, key: string, title: string, turns: number, now = new Date()) {
  write(store, { ...read(store), [key]: { title, turns, at: now.toISOString() } });
}

/** The notice for `key` now that it has `turns` turns, or null. A turn
 *  since the move ends it (and forgets it). */
export function movedNotice(store: Store, key: string, turns: number): string | null {
  const all = read(store);
  const m = all[key];
  if (!m) return null;
  if (turns > m.turns) { delete all[key]; write(store, all); return null; }
  return movedNoticeText(m.title);
}

export function movedNoticeText(title: string): string {
  return `Moved to ${title}. The next turn runs in ${title}'s folder with its brief and sources; the conversation so far comes along as written context.`;
}
