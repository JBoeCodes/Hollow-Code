// The conversation menu's actions that change a conversation (P1-39): rename,
// archive, restore and New workspace from this chat. Loaded on first use by
// workspaceMenu.ts (the menu itself is in the App chunk) and Conversation
// history, so they cost the App chunk nothing.
import { api } from './api';
import { graphemeCut } from './turnFooter';
import { chatAgentSessionKey, conversationHistoryShown, dropChatInteractionsFor, flashFooter, flashWithUndo, refreshChatInteractions, setActivePage, showPrompt } from './store';
import {
  lifecycleFailed, moveBlockedReason, moveSessionTo, refreshWorkspace, refreshWorkspaceSessions, refreshWorkspaces, selectWorkspace, sessionWorking, workspaceId,
} from './store/workspaces';
import { workspaceFromChat } from './sessionMove';
import { setHistoryWorkspace } from './store/sessionRequest';
import { errMsg } from './utils';
import { guardNavigation } from './store/navigationGuard';
import { goToPage } from './store/workspaces';
import { resetAutomationsView } from './store/automationsViews';
import { setNewAutomationChat } from './pages/workflows/runsInChoice';

async function changed(after?: () => unknown) {
  await refreshWorkspaceSessions();
  await after?.();
  if (workspaceId()) await refreshWorkspace().catch(() => {});
}

/** Rename a conversation; `after` re-reads the list it was renamed from
 *  (Conversation history keeps its own). */
export async function renameConversation(key: string, current: string, after?: () => unknown) {
  const title = await showPrompt({ title: 'Rename conversation', confirmLabel: 'Rename', input: { label: 'Name', value: current, maxLength: 480 } });
  if (!title || title === current) return;
  await renameConversationTo(key, title, after);
}

/** The sidebar's rename in place (overnight 11-04 S3): the row's field
 *  saves on Enter or when it loses focus. An empty or unchanged name is no
 *  change. True when it was renamed. */
export async function renameConversationTo(key: string, title: string, after?: () => unknown): Promise<boolean> {
  const name = graphemeCut(title.trim(), 120).trim();
  if (!name) return false;
  try { await api.renameAgentSession(key, name); await changed(after); return true; }
  catch (e) { lifecycleFailed(String(e)); return false; }
}

/** Regenerate title (t3code parity 4.7): a new model title from the first
 *  request, on the chat's own agent. The person asked, so it replaces a
 *  rename too; the message says what it did and how long it can take. */
export async function regenerateConversationTitle(key: string, after?: () => unknown) {
  flashFooter('Writing a new title…', 2500);
  try {
    const title = await api.regenerateChatTitle(key);
    await changed(after);
    flashFooter(`Title: ${title}`, 3500);
  } catch (e) { lifecycleFailed(String(e)); }
}

/** Archive a conversation, with Undo. Refused while a turn of it is queued
 *  or running, here and by the backend (`ARCHIVE_BUSY`). With `fresh`,
 *  archiving the open one starts a fresh conversation in that workspace (the
 *  rail); Conversation history keeps it on screen. */
export async function archiveConversation(key: string, opts: { fresh?: string | null; after?: () => unknown } = {}) {
  if (sessionWorking(key)) { lifecycleFailed('This conversation is working. Archive it when the turn finishes.'); return; }
  try {
    await api.archiveAgentSession(key, true);
    // Archiving expires its open question cards (Paperclip PR 13) with no event.
    dropChatInteractionsFor(key);
    await changed(opts.after);
    const ws = opts.fresh;
    if (ws && workspaceId() === ws && chatAgentSessionKey() === key) await selectWorkspace(ws, undefined, true);
    flashWithUndo({ message: 'Conversation archived', kind: 'archive', undo: () => restoreConversation(key, opts.after) });
  } catch (e) { lifecycleFailed(String(e)); }
}

/** Restore an archived conversation; its question cards stay expired. */
export async function restoreConversation(key: string, after?: () => unknown) {
  try {
    await api.archiveAgentSession(key, false);
    void refreshChatInteractions();
    await changed(after);
    flashFooter('Conversation restored');
  } catch (e) { lifecycleFailed(String(e)); }
}

/** New workspace from this chat (Turn into project, 4.4): a workspace titled from the conversation,
 *  its outcome the first ask; the conversation moves in and opens there, and the
 *  chat's scratch files move into its sources/chat-<key> folder.
 *  A move that fails puts the new workspace in the trash and says why, so
 *  no empty workspace is left behind. */
export async function newWorkspaceFromChat(key: string, title: string | null) {
  const why = moveBlockedReason(key);
  if (why) { flashFooter(`× ${why}`, 4000); return; }
  const failed = (e: unknown) => lifecycleFailed(`Couldn't make a workspace from this chat: ${errMsg(e)}`);
  let w;
  try {
    const conversation = await api.readAgentConversation(key);
    const ask = conversation.turns.find(t => t.role === 'user')?.text ?? null;
    const made = workspaceFromChat(title ?? conversation.summary.title, ask);
    w = await api.createWorkspace(made.title, made.goal);
    await refreshWorkspaces();
  } catch (e) { failed(e); return; }
  let moveError = '';
  const carryScratch = async (chat: string) => {
    try {
      const r = await api.adoptChatScratch(chat, w.id);
      if (r.moved) flashFooter(`${r.moved} scratch file${r.moved === 1 ? '' : 's'} moved into the project’s ${r.into} folder${r.kept ? `; ${r.kept} stayed in the scratch folder` : ''}`, 6000);
    } catch (e) { flashFooter(`The project is ready, but its scratch files didn’t move: ${errMsg(e)}`, 6000); }
  };
  // No Undo here: moving it back would leave the new workspace empty.
  if (await moveSessionTo(key, w.id, { open: true, noUndo: true, failed: e => { moveError = e; } })) { await carryScratch(key); return; }
  const removed = await api.trashWorkspace(w.id).then(() => true, () => false);
  await refreshWorkspaces();
  failed(`${moveError.replace(/\.$/, '')}. ${removed ? 'The new workspace was removed' : `The new workspace “${w.title}” is still there; delete it from its menu`}.`);
}

/** Library → Conversations filtered to workspace `id`: a sidebar project's
 *  or Chats' ⋯ → Open in Conversation history (overnight 11-04 S4b). */
export function openHistory(id: string) {
  setHistoryWorkspace(id);
  void Promise.resolve(setActivePage('chat')).then(() => { if (!conversationHistoryShown()) setHistoryWorkspace(null); });
}

/** Schedule in this chat… (overhaul F1): open a new automation bound to
 *  this conversation, so each run is a new turn here. */
export function scheduleInChat(key: string, title: string) {
  void guardNavigation(() => {
    resetAutomationsView();
    setNewAutomationChat({ key, title });
    goToPage('workflows');
  });
}

/** The quick chat's scratch folder in Finder, made first if the chat has none (4.4). */
export async function revealChatScratch(key: string) {
  try {
    const info = await api.quickChatScratch(key, true);
    await api.vaultRevealInFinder(info.relative);
  } catch (e) { lifecycleFailed(`Couldn't open the scratch folder: ${errMsg(e)}`); }
}
