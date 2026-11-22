// Right-click menus for workspaces and their conversations. One builder so
// the rail tree and the Your work cards offer the same actions.
import { api } from './api';
import { setCanvasLookTarget } from './store/canvasLookTarget';
import { customThemes } from './store/theme';
import { backgroundChatKeys, canLeaveConversation, chatBusy, chatCwd, flashFooter, type ContextMenuItem } from './store';
import { runQueue } from './store/runQueue';
import {
  archiveFromRail, archiveOlderEntries, chooseMergeTarget, chooseMoveTarget, deleteWorkspace, isInRail, sessionWorking,
  confirmUndoMerge, lifecycleFailed, newWorkspaceSession, openProjectAgent, pickConversation, removeWorktree, renameWorkspace, selectWorkspace, setWorkspaceStatus,
  proposeArchive, railHiddenIds, startQuickChat, unarchiveWorkspace, workspaceEditLock, workspaceOfSession, workspaceSessions,
} from './store/workspaces';
import { isMainAgent, isQuickChats, unarchiveStatus, workspaceDeleteLock, type WorkWorkspace } from './workspaceTypes';
import { canMergeFrom } from './workspaceMerge';
import { openMainAgent } from './store/mainAgent';
import { cooMenuItems } from './cooMenu';
import { setIconEditor } from './store/customIcons';
import { moveWorkspacePin, pinWorkspace, pinnedWorkspaces, unpinWorkspace } from './store/workspacePins';
import { canPinWorkspace, pinStep } from './workspacePins';
import type { AgentSessionSummary } from './types';
import { chatHomes, moveRunner, movingKeys } from './store/moveFlags';
import { conversationOrganization } from './store/conversationOrganization';
import { pickProjectColor, projectColorPicks } from './store/projectColors';
import { nextProjectColor, projectColor } from './projectIconColors';

/** Library → Conversations filtered to workspace `id` (the sidebar's old
 *  "N older" link; its older chats load in place since overnight 11-04 S4b). */
const historyItem = (id: string): ContextMenuItem => ({ label: 'Open in Conversation history', action: () => act(m => m.openHistory(id)) });

/** Status choices offered from the menu (the brief accepts any one-line status). */
const STATUSES = ['active', 'planning', 'blocked', 'done'] as const;

async function copy(text: string, what: string) {
  try { await navigator.clipboard.writeText(text); flashFooter(`${what} copied`); }
  catch (e) { lifecycleFailed(`Could not copy: ${e}`); }
}
/** A menu failure shows wherever the menu was opened (P1-41). */
const failed = (e: unknown) => { lifecycleFailed(String(e)); };
/** Whether an app opens obsidian:// links (P1-41), from Launch Services.
 *  Asked once at load, so the first menu already knows, and again as each
 *  menu is built, so the next one follows an install or removal. */
let obsidianOk = true;
function askObsidian() {
  try { void api.obsidianAvailable().then(ok => { obsidianOk = ok; }, () => {}); } catch { /* no bridge (tests) */ }
}
askObsidian();
function obsidianInstalled(): boolean {
  askObsidian();
  return obsidianOk;
}
/** "Wake the lead for new issues" (F2): which leads wake and which projects
 *  are tracked, asked at load and as each menu is built (like Obsidian's),
 *  so the next menu follows a change. Its IPC loads on first use. */
let leadWake = { on: new Set<string>(), tracked: new Set<string>() };
const triggersApi = () => import('./eventTriggersApi');
function askLeadWakes() {
  triggersApi().then(m => m.eventTriggersStatus()).then(s => { leadWake = { on: new Set(s.leadWakes), tracked: new Set(s.tracked) }; }).catch(() => {});
}
askLeadWakes();
function leadWakeItems(w: WorkWorkspace): ContextMenuItem[] {
  askLeadWakes();
  if (!leadWake.tracked.has(w.id) || w.build) return [];
  const on = leadWake.on.has(w.id);
  return [{
    label: `Wake the lead for new issues (${on ? 'on' : 'off'})`,
    title: 'A new issue in team AI for this project starts a short turn in its lead. Off until you turn it on.',
    action: () => triggersApi().then(m => m.setEventTrigger({ kind: 'tracker-issue', workspaceId: w.id, leadWake: !on })).then(s => {
      leadWake = { on: new Set(s.leadWakes), tracked: new Set(s.tracked) };
      flashFooter(on ? 'The lead no longer wakes for new issues.' : 'The lead wakes for new issues in team AI from now on.');
    }, failed),
  }];
}
const locked = (label: string, reason: string) => reason ? `${label} (${reason[0].toLowerCase()}${reason.slice(1)})` : label;
/** Run-queue states of a chat turn that holds, or is about to hold, its folder. */
const LIVE_TURN = new Set(['queued', 'claimed', 'running']);
/** A conversation in workspace `id` has a turn running or queued (P0-13):
 *  the backend refuses to trash a folder a live turn holds, and a queued one
 *  is about to. Read from the turns themselves, not the shown state, which
 *  ranks "waiting on you" first (a turn paused on an approval card still
 *  runs). Includes the conversation on screen before it is listed. */
function hasLiveConversation(w: WorkWorkspace): boolean {
  const id = w.id;
  // The chat on screen counts only when it works in this workspace's folder.
  if (chatBusy() && chatCwd() === w.cwd) return true;
  const live = new Set(backgroundChatKeys());
  for (const item of runQueue()) if (item.source === 'chat' && item.sourceId && LIVE_TURN.has(item.status)) live.add(item.sourceId);
  return (workspaceSessions()[id]?.sessions ?? []).some(s => live.has(s.key));
}

/** Change color cycles the project's monogram tint (A8); Reset color shows once one is picked. */
function colorItems(w: WorkWorkspace): ContextMenuItem[] {
  const picked = projectColorPicks()[w.id];
  const now = projectColor(w.id, picked);
  const items: ContextMenuItem[] = [{ label: `Change color (${now.name}, next: ${nextProjectColor(now).name})`, title: 'The tint behind the project’s letters in the sidebar, kept on this Mac.', action: () => pickProjectColor(w.id, nextProjectColor(now).id) }];
  if (picked) items.push({ label: 'Reset color', action: () => pickProjectColor(w.id, null) });
  return items;
}


/** `rename` renames in place (the sidebar row turns into a field); else a dialog. */
export function workspaceMenuItems(w: WorkWorkspace, rename?: () => void): ContextMenuItem[] {
  const busy = !canLeaveConversation();
  // Quick chats is built in: no rename, status, hide or delete.
  if (isQuickChats(w)) {
    const items: ContextMenuItem[] = [{ label: 'New chat', action: () => startQuickChat(), disabled: busy }];
    if (w.cwd) items.push({ label: 'Open latest chat', action: () => selectWorkspace(w.id), disabled: busy }, historyItem(w.id), { separator: true },
      { label: 'Reveal in Finder', action: () => api.vaultRevealInFinder(w.path).catch(failed) });
    return items;
  }
  // The Main agent is built in too: open it, change its icon (it was a link
  // in its page's header until 11-06; Settings, General has it too), or
  // show its folder.
  if (isMainAgent(w)) {
    return [{ label: 'Open the COO', action: () => void openMainAgent(), disabled: busy },
      { label: 'Change icon…', action: () => { setIconEditor({ target: 'mainAgent', mode: 'upload' }); } }, { separator: true },
      ...cooMenuItems(), { separator: true },
      { label: 'Reveal in Finder', action: () => api.vaultRevealInFinder(w.path).catch(failed) }];
  }
  const editLock = workspaceEditLock(w.id);
  const deleteLock = workspaceDeleteLock(w, hasLiveConversation(w));
  const status = w.status.trim().toLowerCase();
  const archived = status === 'archived';
  // Rename, Pin and Archive lead, as in ChatGPT's and Claude's project
  // menus (overnight 11-04 S3); the rest follow unchanged.
  const items: ContextMenuItem[] = [
    { label: locked('Rename…', editLock), action: () => rename ? rename() : renameWorkspace(w), disabled: !!editLock },
    ...pinItems(w),
    archived
      ? { label: locked(`Unarchive (back to ${unarchiveStatus(w)})`, editLock), action: () => unarchiveWorkspace(w), disabled: !!editLock }
      : { label: locked('Archive… (off the active list)', editLock), action: () => void proposeArchive(w.id, w.title, w.status), disabled: !!editLock },
    { separator: true },
    { label: 'New conversation', action: () => newWorkspaceSession(w.id), disabled: busy },
    { label: 'Open lead', action: () => void openProjectAgent(w.id), disabled: busy },
    ...leadWakeItems(w),
    { label: 'Open project', action: () => selectWorkspace(w.id), disabled: busy },
    historyItem(w.id),
    { separator: true },
    { label: w.canvasTheme ? `Canvas look… (${w.canvasTheme}${customThemes.latest && !customThemes.latest.some(t => t.name === w.canvasTheme) ? " isn't on this Mac; using app theme" : ''})` : 'Canvas look…', action: () => { setCanvasLookTarget(w); } },
    ...colorItems(w),
    // An archived workspace's only status item is Unarchive (P1-38).
    ...STATUSES.filter(s => s !== status && !archived).map(s => ({
      label: locked(`Mark as ${s}`, editLock),
      action: () => setWorkspaceStatus(w, s),
      disabled: !!editLock,
    })),
    { separator: true },
    { label: 'Reveal in Finder', action: () => api.vaultRevealInFinder(w.path).catch(failed) },
    obsidianInstalled()
      ? { label: 'Open brief in Obsidian', action: () => api.vaultOpenInObsidian(`${w.path}/WORKSPACE.md`).catch(failed) }
      : { label: 'Open brief in Obsidian (Obsidian isn’t installed)', disabled: true },
    { label: 'Copy folder path', action: () => copy(w.cwd, 'Folder path') },
  ];
  if (w.build?.exists) {
    const tree = w.build.worktree;
    items.push(
      { separator: true },
      { label: 'Reveal worktree in Finder', action: async () => { await api.chatPreviewOpen(tree, true).catch(failed); } },
      { label: 'Copy worktree path', action: () => copy(tree, 'Worktree path') },
      { label: 'Copy branch name', action: () => copy(w.build!.branch, 'Branch name') },
      { label: 'Remove worktree…', action: () => removeWorktree(w), disabled: busy },
    );
  }
  items.push({ separator: true });
  // Two different things, named apart (review U shell; plan completion L4,
  // X14): Hide from sidebar only takes the workspace (and its conversations)
  // out of the rail, keeping its status; Archive… takes it off the active
  // list (it asks first, nothing moves or is deleted, Undo or Unarchive puts
  // it back).
  if (isInRail(w.id)) items.push({ label: 'Hide from sidebar (keeps its status)', action: () => archiveFromRail(w) });
  if (!archived) items.push({ label: locked('Archive older entries…', editLock), action: () => void archiveOlderEntries(w), disabled: !!editLock || busy });
  if (canMergeFrom(w)) items.push({ label: 'Merge into…', action: () => chooseMergeTarget(w), disabled: busy });
  if (w.mergedInto) items.push({ label: 'Undo merge…', action: () => confirmUndoMerge(w), disabled: busy });
  items.push({ label: locked('Delete…', deleteLock), action: () => deleteWorkspace(w), disabled: !!deleteLock, danger: true });
  return items;
}

/** Pin, or Unpin and (with two pins or more) Move up / Move down: the
 *  keyboard way to reorder the sidebar's Pinned group (drag is the other).
 *  None for an archived workspace, which can't be pinned. */
export function pinItems(w: WorkWorkspace): ContextMenuItem[] {
  if (!canPinWorkspace(w)) return [];
  const pins = pinnedWorkspaces();
  const at = pins.indexOf(w.id);
  if (at < 0) return [{ label: 'Pin', action: () => void pinWorkspace(w) }];
  const items: ContextMenuItem[] = [{ label: 'Unpin', action: () => void unpinWorkspace(w) }];
  // Steps over pins hidden from the sidebar (ENG-116).
  const up = pinStep(pins, railHiddenIds(), w.id, -1);
  const down = pinStep(pins, railHiddenIds(), w.id, 1);
  if (pins.length > 1) items.push(
    { label: 'Move up', action: () => { if (up != null) void moveWorkspacePin(w, up); }, disabled: up == null },
    { label: 'Move down', action: () => { if (down != null) void moveWorkspacePin(w, down); }, disabled: down == null },
  );
  return items;
}

/** Why the person's own pick can't happen yet: a message is still being sent
 *  (also the top bar's disabled New workspace, P1-42). */
export const SENDING ='wait: the current message is still being sent';
/** Why a conversation can't move or be archived right now, in the words its
 *  menu items carry (P1-39), or null. */
export function conversationBusy(key: string): string | null {
  if (!canLeaveConversation()) return SENDING;
  return sessionWorking(key) ? 'this conversation is working' : null;
}
/** A label that says why its item is off; never a bare disabled one. */
const off = (label: string, why: string | null | undefined) => why ? `${label} (${why})` : label;

interface ConversationTarget {
  key: string;
  /** Its stored title (New workspace from this chat names the workspace from it). */
  title: string | null;
  /** The title shown, which Rename… starts from. */
  shown: string;
  /** Its workspace (Quick chats included), or null for a Hollow folder chat. */
  workspaceId: string | null;
  archived?: boolean;
  /** How this home opens it, and why it can't right now. */
  open: () => unknown;
  openBlocked: string | null;
  /** Archiving the open one starts a fresh one in its workspace (the rail). */
  fresh?: boolean;
  /** Re-reads this home's own list after a rename, archive or restore. */
  after?: () => unknown;
  /** Rename in place (the sidebar's row turns into a field); else a dialog. */
  rename?: () => void;
  /** Run just before Pin or Unpin (`unpin`) and before Archive: the sidebar
   *  keeps its project in view and the focus in its list (overnight 11-04). */
  beforeArchive?: () => void;
}

/** One conversation menu (P1-39) for its three homes: a rail row, a
 *  Conversation history row and the ⋯ beside the open conversation's title.
 *  Same items in the same order everywhere; a disabled one says why. The
 *  everyday ones lead, as in ChatGPT's and Claude's row menus (overnight
 *  11-04 S3): Rename, Move to project (Pin went with the sidebar's chat
 *  lists in L8, 11-10). */
function conversationMenuItems(c: ConversationTarget): ContextMenuItem[] {
  const why = conversationBusy(c.key);
  const organization = conversationOrganization()[c.key];
  const hideWhy = why || (organization?.pinned ? 'unpin this conversation first' : null);
  const organize = (action: import('./conversationOrganization').OrganizationChange['action'] | 'automatic') => {
    void import('./conversationOrganizationActions').then(m => m.organizeConversation(c.key, action, c.after, { fresh: c.fresh ? c.workspaceId : null, beforeSettled: c.beforeArchive }), () => lifecycleFailed('That action did not load. Try again.'));
  };
  const sending = canLeaveConversation() ? null : SENDING;
  const ws = c.workspaceId;
  const items: ContextMenuItem[] = [
    { label: 'Rename…', action: () => c.rename ? c.rename() : act(m => m.renameConversation(c.key, c.shown, c.after)) },
    { label: off('Regenerate title', why), title: 'Ask the chat’s agent for a new title from its first message. Replaces the current one.', action: () => act(m => m.regenerateConversationTitle(c.key, c.after)), disabled: !!why },
    { label: off('Move to project…', why), action: () => chooseMoveTarget(c.key), disabled: !!why },
    { label: off('New project from this chat', why), title: 'Make a project from this chat and move it in, with the files in its scratch folder.', action: () => act(m => m.newWorkspaceFromChat(c.key, c.title)), disabled: !!why },
    { label: organization?.pinned ? 'Unpin conversation' : 'Pin conversation', action: () => organize(organization?.pinned ? 'unpin' : 'pin') },
    organization?.snoozedUntil || organization?.settled
      ? { label: 'Wake conversation', action: () => organize('wake') }
      : { label: off('Snooze until…', hideWhy), action: () => organize('snooze'), disabled: !!hideWhy },
    { label: 'Automatic settlement…', action: () => organize('automatic') },
  ];
  // M5: Move to runner, once a paired Mac takes chats (store/chatMoves.ts).
  const home = chatHomes()[c.key]?.state, moving = c.archived ? 'archived' : home === 'leaving' || movingKeys().has(c.key) ? 'moving' : home === 'away' ? 'moved' : null;
  if (moveRunner()) items.push({ label: off('Move to runner', moving), disabled: !!moving,
    action: () => void import('./store/chatMoves').then(m => m.moveToRunner(c.key, c.shown), () => lifecycleFailed('That action didn’t load. Try again.')) });
  items.push(
    { separator: true },
    { label: off('Open conversation', c.openBlocked), action: () => void c.open(), disabled: !!c.openBlocked },
  );
  if (ws) items.push(isQuickChats(ws) ? { label: off('New chat', sending), action: () => startQuickChat(), disabled: !!sending }
    : { label: off('New conversation in project', sending), action: () => newWorkspaceSession(ws), disabled: !!sending });
  if (ws && isQuickChats(ws)) items.push({ label: 'Scratch folder', title: 'This chat’s own folder for working files. Turn into project carries them into the project.', action: () => act(m => m.revealChatScratch(c.key)) });
  items.push({ label: off('Schedule in this chat…', c.archived ? 'archived' : null), action: () => act(m => m.scheduleInChat(c.key, c.shown)), disabled: !!c.archived },
    { label: 'Copy conversation ID', action: () => copy(c.key, 'Conversation ID') },
    { separator: true },
    c.archived ? { label: 'Restore conversation', action: () => act(m => m.restoreConversation(c.key, c.after)) }
      : { label: off('Settle conversation', hideWhy), title: 'Archive this conversation, with Undo. Pending questions close.', action: () => organize('settle'), disabled: !!hideWhy });
  return items;
}
/** The menu's changing actions load on first use (the App chunk's budget). */
function act(run: (m: typeof import('./conversationActions')) => unknown) {
  void import('./conversationActions').then(run, () => lifecycleFailed('That action didn’t load. Try again.'));
}

/** A conversation's menu beside its open title. `rename` renames in place;
 *  `hooks` run before its Archive. */
export function sessionMenuItems(w: WorkWorkspace, s: AgentSessionSummary, title: string, rename?: () => void,
  hooks: Pick<ConversationTarget, 'beforeArchive'> = {}): ContextMenuItem[] {
  return conversationMenuItems({ key: s.key, title: s.title, shown: title, workspaceId: w.id, fresh: true, rename, ...hooks,
    open: () => pickConversation(w.id, s.key), openBlocked: canLeaveConversation() ? null : SENDING });
}

/** A Conversation history row's menu, and the ⋯ beside the open
 *  conversation's title there: the page opens it its own way. */
export function historyMenuItems(s: AgentSessionSummary, opts: { archived: boolean; open: () => unknown; openBlocked: string | null; after: () => unknown; rename?: () => void }): ContextMenuItem[] {
  return conversationMenuItems({ key: s.key, title: s.title, shown: s.title || 'Untitled conversation', workspaceId: workspaceOfSession(s.key, s.cwd), ...opts });
}
