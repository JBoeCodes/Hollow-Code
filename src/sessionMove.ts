/**
 * Pure helpers for Quick chats and moving conversations (2026-09-30): the
 * workspace choices the palette offers, which workspace a conversation is
 * in, and the new workspace "New workspace from this chat" creates. The
 * store (`store/workspaces.ts`) does the work; these decide the shape.
 */
import { QUICK_CHATS_ID, QUICK_CHATS_LABEL, isBuiltinWorkspace, isQuickChats, type WorkWorkspace } from './workspaceTypes';

export interface WorkspaceChoice { id: string; title: string; hint: string; quick: boolean }

/** Where a conversation can go (or start): Quick chats first, whether or
 *  not it exists yet (it is created on first use), then the rail's
 *  workspaces in rail order, then every other one, newest first as listed.
 *  `exclude` (the conversation's own workspace) is left out. */
export function workspaceChoices(all: readonly WorkWorkspace[], railOrder: readonly WorkWorkspace[], exclude?: string | null): WorkspaceChoice[] {
  const out: WorkspaceChoice[] = [];
  if (exclude !== QUICK_CHATS_ID) out.push({ id: QUICK_CHATS_ID, title: QUICK_CHATS_LABEL, hint: 'No project', quick: true });
  const seen = new Set<string>([QUICK_CHATS_ID]);
  for (const w of [...railOrder, ...all]) {
    if (seen.has(w.id) || isBuiltinWorkspace(w) || w.id === exclude) continue;
    seen.add(w.id);
    out.push({ id: w.id, title: w.title, hint: w.status, quick: false });
  }
  return out;
}

/** The workspace a conversation is in: the rail group that lists it, else
 *  the workspace whose cwd is its cwd, else null (a vault-root chat). */
export function sessionWorkspaceId(
  key: string,
  cwd: string | null | undefined,
  groups: Record<string, { sessions: readonly { key: string }[] }>,
  all: readonly WorkWorkspace[],
): string | null {
  for (const [id, group] of Object.entries(groups)) if (group.sessions.some(s => s.key === key)) return id;
  return (cwd && all.find(w => w.cwd === cwd)?.id) || null;
}

/** The workspace "New workspace from this chat" creates: titled from the
 *  conversation (else its first ask), its outcome the first ask. Fits the
 *  backend's limits: a one-line title of at most 100 characters, an outcome
 *  of at most 12,000 bytes. */
export function workspaceFromChat(title: string | null | undefined, firstAsk: string | null | undefined): { title: string; goal: string } {
  const ask = (firstAsk ?? '').trim();
  const line = (text: string) => text.split(/\r?\n/).map(l => l.trim()).find(Boolean) ?? '';
  const name = (line(title ?? '') || line(ask) || 'Conversation').slice(0, 100).trim();
  let goal = ask || `Continue the conversation “${name}”.`;
  const bytes = new TextEncoder();
  while (bytes.encode(goal).length > 12_000) goal = goal.slice(0, Math.floor(goal.length * 0.9));
  return { title: name, goal };
}
