import { createRoot, createEffect, createSignal, on } from 'solid-js';
import { guardNavigation } from './navigationGuard';
import { bindDestinationNav, bindReviewNav, resetWorkView } from './destinations';
import { bindWorkspaceNav, launchLanded, learnConversation, quietly } from './navHistory';
import { withViewTransition } from '../viewTransitions';
import { turnsFromConversation } from '../chatHydrate';
import { listen } from '../nativeEvents';
import type { AgentSessionSummary, VaultChange } from '../types';
import { api } from '../api';
import { unsavedText, worktreeUnsaved, type WorkWorkspace, type WorkspaceDetail, type WorkspaceIcon, type WorkspaceSessions, type RailGitState, type SessionTarget } from '../workspaceTypes';
import { keepUnchanged, keepUnchangedRecord, samePayload } from '../keyedList';
import {
  chatAgentSessionKey, chatCwd, chatBusy, chatInput, chatAttachments, chatTurns,
  setChatInput, setChatAttachments, resetChatState, applyNewChatPermissionMode, setChatCwdPersistent,
  setChatAgentPersistent, setChatSessionIdPersistent, setChatAgentSessionKeyPersistent, setChatPermissionMode, chatPermissionMode,
  setChatTurns, rebuildChatArtifacts, setChatSessionLoading, setActivePage,
  chatSessionLoading, setChatPanelOpen, requestChatFocus, KNOWN_AGENTS, type ChatAgent,
  showConfirm, showPrompt, flashFooter, flashRetry, openCommandPalette, activePage, shownPage,
  canLeaveConversation, backgroundRunningChat, restoreBackgroundChat, isBackgrounded, backgroundedKeyForCwd, backgroundChatCwd,
  chatStoredTranscriptPending, showStoredTranscript, beginStoredTranscriptRead, storedTranscriptReadApplies, markStoredTranscriptApplied,
  backgroundChatKeys, flashWithUndo, dismissSnackbar, trackTask, dropChatInteractionsFor, pendingChatSend, setPendingChatSend, openMovePalette, openMergePalette,
  type PageId, type ChatTurn, vaultRoot,
} from './runtime';
import { errMsg } from '../utils';
import { latestRequest } from '../latestRequest';
import { conversationPermission } from '../permModes';
import { conversationDrafts, bindFirstDraftSession, bindBeforeDraftReset } from './conversationDrafts';
import { projectChoices } from '../projectDestination';
import { runQueue } from './runQueue';
import { openPreview, togglePreview } from './preview';
import { inVault } from '../projectFiles';
import { QUICK_CHATS_ID, QUICK_CHATS_LABEL, MAIN_AGENT_TITLE, isBuiltinWorkspace, isMainAgent, isQuickChats, unarchiveStatus } from '../workspaceTypes';
import { registerSessionDropHandler, type SessionDragPayload } from '../sessionDrag';
import { sessionWorkspaceId } from '../sessionMove';
import { SHARED_DOCS, archiveBody, archiveDoneMessage, archiveSummary, hasMoves, type SharedDocArchive } from '../archiveOlderEntries';
import { mergeConfirmBody, mergeConfirmSummary, mergeDoneMessage, unmergeDoneMessage } from '../workspaceMerge';
import { currentPage } from '../navigation';
import { setAppSessionRequest } from './sessionRequest';
import { refreshRunQueue } from './runQueue';
import { railGroups, railWorkspaceOrder } from '../railWorkspaces';
import { pinOrder } from '../workspacePins';
import { movedNotice, recordMove } from '../movedNotice';
import { LAST_CONVERSATION_KEY, launchLanding, launchTarget, parseLastConversation, workspaceConversationKeys, type LastConversation } from '../lastConversation';

const saved = (key: string) => { try { return localStorage.getItem(key); } catch { return null; } };
export const [workspaces, setWorkspaces] = createSignal<WorkWorkspace[]>([]);
export const [workspaceId, setWorkspaceId] = createSignal<string | null>(saved('hollow.workspace'));
export const [workspaceDetail, setWorkspaceDetail] = createSignal<WorkspaceDetail | null>(null);
/** The Work page's banner (A08): its text, and whether it is an error to put
 *  in plain words (the raw text under Details) or a line to show as written.
 *  Build-workspace warnings are Hollow's own sentences about the repo, so they
 *  go through `setWorkspaceWarning`; everything else is an error. */
const [workspaceBanner, setWorkspaceBanner] = createSignal<{ text: string; plain: boolean }>({ text: '', plain: true });
/** The banner's text; '' when there is none. */
export const workspaceError = () => workspaceBanner().text;
/** False for a warning, which the banner shows exactly as written. */
export const workspaceErrorPlain = () => workspaceBanner().plain;
/** Show an error in the banner (plain words, raw text under Details); '' clears it. */
export function setWorkspaceError(text: string): void { setWorkspaceBanner({ text, plain: true }); }
/** Show a warning in the banner as written, with no Details. */
export function setWorkspaceWarning(text: string): void { setWorkspaceBanner({ text, plain: false }); }
export const [workspaceLoading, setWorkspaceLoading] = createSignal(false);
export const [workspaceListLoaded, setWorkspaceListLoaded] = createSignal(false);
/** List failures belong to the rail, never the selected workspace's banner. */
export const [workspaceListError, setWorkspaceListError] = createSignal('');
export const [workspaceListLoading, setWorkspaceListLoading] = createSignal(false);
export const [workspaceLastVisited, setWorkspaceLastVisited] = createSignal<string | null>(null);
export const [workspaceRevision, setWorkspaceRevision] = createSignal(0);
/** The workspace open in flight: a newer open, or leaving for the overview,
 *  drops it; launch restore and `refreshWorkspace` join it (`peek`). */
const opens = latestRequest();
let applying = false;
/** An open (quiet or not) is reading its workspace (P1-42: Back cancels it). */
let opening = false;
let draftOwner: { workspace: string; cwd: string; key: string | null } | null = null;
let draftSaveFailed = false;
function draftSaved(ok: boolean) {
  if (!ok && !draftSaveFailed) flashFooter('Some draft text is kept in this window but could not be saved for restart.');
  draftSaveFailed = !ok;
}
function restoreDraft(id: string, key: string | null) {
  const draft = conversationDrafts.read(id, key);
  setChatInput(draft.text); setChatAttachments(draft.attachments);
}
function remember() {
  if (applying || !draftOwner) return;
  const w = workspaceDetail()?.workspace;
  if (!w || w.id !== draftOwner.workspace || chatCwd() !== draftOwner.cwd || chatAgentSessionKey() !== draftOwner.key) return;
  const key = chatAgentSessionKey();
  conversationDrafts.migrate(w.id, key);
  draftSaved(conversationDrafts.save(w.id, key, chatInput(), chatAttachments()));
}
bindFirstDraftSession((cwd, key) => {
  const w = workspaceDetail()?.workspace;
  if (applying || !draftOwner || draftOwner.key || !w || w.id !== draftOwner.workspace
    || w.cwd !== cwd || chatAgentSessionKey()) return;
  draftSaved(conversationDrafts.accept(w.id, key, chatInput(), chatAttachments()));
  draftOwner = { workspace: w.id, cwd: w.cwd, key };
});
bindBeforeDraftReset(() => { remember(); draftOwner = null; });
/** App artwork / GitHub owner per workspace id. Refetched only when the set
 *  of workspace ids changes, since artwork is data URLs. */
export const [workspaceIcons, setWorkspaceIcons] = createSignal<Record<string, WorkspaceIcon>>({});
let iconKey = '';
async function refreshWorkspaceIcons(list: WorkWorkspace[]) {
  const key = list.map(w => w.id).join('|');
  if (key === iconKey) return;
  iconKey = key;
  try {
    const icons = await api.workspaceIcons();
    setWorkspaceIcons(prev => keepUnchangedRecord(prev, Object.fromEntries(icons.map(icon => [icon.workspaceId, icon]))));
  } catch { iconKey = ''; /* icons are decoration; the list still works */ }
}
/** Five asks B4: one refresh at a time. The rail and the workspace page
 *  both refresh on mount, so launch ran the whole chain (list, icons, every
 *  workspace's conversations, git states) twice. `join` (mounts, the minute
 *  timer) shares a refresh already running. Any other call comes after
 *  something changed on disk, so it gets a refresh that starts after the
 *  running one: calls while one runs share that single follow-up. */
let workspacesInFlight: Promise<void> | null = null;
let workspacesNext: Promise<void> | null = null;
export function refreshWorkspaces(opts: { join?: boolean } = {}): Promise<void> {
  if (workspacesInFlight) {
    if (opts.join) return workspacesInFlight;
    return workspacesNext ??= workspacesInFlight.then(() => { workspacesNext = null; return refreshWorkspaces(); });
  }
  setWorkspaceListLoading(true);
  workspacesInFlight = (async () => {
    try {
      // An unchanged re-read keeps the objects on screen (P4: the rail and
      // Work list every workspace; new objects redraw every row).
      const list = keepUnchanged(workspaces(), await api.listWorkspaces(), w => w.id);
      setWorkspaces(list);
      setWorkspaceListLoaded(true);
      setWorkspaceListError('');
      void refreshWorkspaceIcons(list);
      void refreshWorkspaceSessions();
      // Vault file changes don't move git or PR state; ask again only when
      // the build workspaces changed or it's 20 s old. A turn's end and the
      // build panel's commit/push/PR ask unthrottled.
      void refreshRailGitStates(20_000);
    } catch (e) { setWorkspaceListError(errMsg(e)); }
  })().finally(() => { workspacesInFlight = null; setWorkspaceListLoading(false); });
  return workspacesInFlight;
}

/** Build workspaces' branch and PR state, for the rail's git glyphs. */
export const [railGitStates, setRailGitStates] = createSignal<Record<string, RailGitState>>({});
const gitReads = latestRequest();
let gitAsked = { ids: '', at: 0 };
// Carry forced cache invalidations through overlapping asks until the latest
// response applies them. Otherwise a later cached read can hide a new PR.
let gitFreshVersion = 0;
const gitFresh = new Map<string, number>();
/** `maxAgeMs`: skip when the same build workspaces were asked about that
 *  recently (each ask spawns git and `gh` per worktree; five asks B4).
 *  The backend reuses each state for 2 min while the worktree's refs hold
 *  (a commit, checkout or fetch shows at once); `fresh` names workspaces it
 *  must read again regardless, e.g. one whose turn just ended and may have
 *  opened a pull request (overnight Y7). */
export async function refreshRailGitStates(maxAgeMs = 0, fresh: string[] = []) {
  const ids = workspaces().filter(w => w.build?.exists).map(w => w.id);
  if (!ids.length) { gitReads.cancel(); gitFresh.clear(); gitAsked = { ids: '', at: 0 }; setRailGitStates(prev => keepUnchangedRecord(prev, {})); return; }
  for (const id of gitFresh.keys()) if (!ids.includes(id)) gitFresh.delete(id);
  for (const id of fresh) if (ids.includes(id)) gitFresh.set(id, ++gitFreshVersion);
  const key = ids.join('|');
  if (!gitFresh.size && maxAgeMs > 0 && gitAsked.ids === key && Date.now() - gitAsked.at < maxAgeMs) return;
  gitAsked = { ids: key, at: Date.now() };
  const token = gitReads.start();
  const force = new Map(gitFresh);
  try {
    const states = await api.buildRailStates(ids, [...force.keys()]);
    if (gitReads.isLatest(token)) {
      setRailGitStates(prev => keepUnchangedRecord(prev, Object.fromEntries(states.map(s => [s.workspaceId, s]))));
      for (const [id, version] of force) if (gitFresh.get(id) === version) gitFresh.delete(id);
    }
  } catch { if (gitReads.isLatest(token)) gitAsked = { ids: '', at: 0 }; /* no glyph state; the rail still works */ }
}

/** Refresh the completed conversations' build workspaces, captured before any
 *  asynchronous read or navigation. The shared list refresh coalesces bursts. */
export async function refreshEndedBuildWorkspaces(cwds: readonly string[]) {
  if (!cwds.length) return;
  const ended = new Set(cwds);
  await refreshWorkspaces();
  const ids = workspaces().filter(w => w.build?.exists && ended.has(w.cwd)).map(w => w.id);
  if (ids.length) await refreshRailGitStates(0, ids);
}

/** Each workspace's unarchived conversations, for the rail tree. */
export const [workspaceSessions, setWorkspaceSessions] = createSignal<Record<string, WorkspaceSessions>>({});
/** Counts finished reads of [`workspaceSessions`], changed or not: a
 *  project's loaded older chats read again on each (overnight 11-04 S4b),
 *  since a rename or archive past the newest 25 leaves the groups alike. */
export const [workspaceSessionsRead, setWorkspaceSessionsRead] = createSignal(0);
const sessionReads = latestRequest();
let pendingSessionRead: Promise<boolean> | undefined;
function keepGroup(old: WorkspaceSessions, next: WorkspaceSessions): WorkspaceSessions {
  if (samePayload(old, next)) return old;
  return { ...next, sessions: keepUnchanged(old.sessions, next.sessions, s => s.key) };
}
export function refreshWorkspaceSessions(): Promise<boolean> {
  const token = sessionReads.start();
  const request = (async (): Promise<boolean> => { try {
    // The open chat (say an old one opened from Search chats) comes along
    // past its project's newest 25 (overnight 11-04).
    const open = chatAgentSessionKey();
    const groups = await api.workspaceSessions(open ? [open] : []);
    // Every workspace switch re-reads all of them (P4): unchanged groups, and
    // unchanged conversations inside a changed group, keep their objects.
    if (!sessionReads.isLatest(token)) return pendingSessionRead ?? false;
    setWorkspaceSessions(prev => keepUnchangedRecord(prev, Object.fromEntries(groups.map(g => [g.workspaceId, g])), keepGroup));
    setWorkspaceSessionsRead(n => n + 1);
    return true;
  } catch {
    return sessionReads.isLatest(token) ? false : pendingSessionRead ?? false;
  } })();
  pendingSessionRead = request;
  return request;
}

/** Workspaces shown in the rail: only those with a conversation, the ones used
 *  here first (`railWorkspaceOrder`; every-active-workspace was reverted
 *  2026-09-29). `hollow.railWorkspaces`
 *  is the "used here" list: a workspace joins it when a conversation is
 *  started or opened in it. `hollow.railHidden` is what "Hide from sidebar"
 *  took out; starting or opening a conversation there puts it back. Neither
 *  changes anything on disk: the status, brief and conversations stay. */
const RAIL_KEY = 'hollow.railWorkspaces';
const HIDDEN_KEY = 'hollow.railHidden';
const readIds = (key: string): string[] => {
  try { const v = JSON.parse(localStorage.getItem(key) ?? '[]'); return Array.isArray(v) ? v.filter(x => typeof x === 'string') : []; }
  catch { return []; }
};
export const [railWorkspaceIds, setRailWorkspaceIds] = createSignal<string[]>(readIds(RAIL_KEY));
export const [railHiddenIds, setRailHiddenIds] = createSignal<string[]>(readIds(HIDDEN_KEY));
function saveIds(key: string, ids: string[], set: (ids: string[]) => void) {
  set(ids);
  try { localStorage.setItem(key, JSON.stringify(ids)); } catch { /* memory still holds it */ }
}
/** The workspaces the rail lists under Projects, in order. Quick chats is
 *  the sidebar's Chats section below them, so it is never one of these. */
export const railWorkspaces = () => railWorkspaceOrder(
  projectWorkspaces(), railWorkspaceIds(), railHiddenIds(),
  Object.values(workspaceSessions()).filter(g => g.sessions.length > 0).map(g => g.workspaceId),
);
export const isInRail = (id: string) => railWorkspaces().some(w => w.id === id);
/** Newest first; a workspace already listed keeps its place. Un-hides it. */
export function addToRail(id: string) {
  if (!railWorkspaceIds().includes(id)) saveIds(RAIL_KEY, [id, ...railWorkspaceIds()], setRailWorkspaceIds);
  if (railHiddenIds().includes(id)) saveIds(HIDDEN_KEY, railHiddenIds().filter(x => x !== id), setRailHiddenIds);
}
export function removeFromRail(id: string) {
  if (railWorkspaceIds().includes(id)) saveIds(RAIL_KEY, railWorkspaceIds().filter(x => x !== id), setRailWorkspaceIds);
}
/** Rail "Hide from sidebar": take the workspace and its conversations out of the rail. */
export function archiveFromRail(w: WorkWorkspace) {
  removeFromRail(w.id);
  if (!railHiddenIds().includes(w.id)) saveIds(HIDDEN_KEY, [w.id, ...railHiddenIds()], setRailHiddenIds);
  flashFooter(`Hid ${w.title} from the sidebar. Its status is unchanged; the Projects page lists it.`);
}

/** The conversation last on screen, reopened at launch (frontier F4). */
function rememberLastConversation(value: LastConversation | null) {
  try {
    if (value) localStorage.setItem(LAST_CONVERSATION_KEY, JSON.stringify(value));
    else localStorage.removeItem(LAST_CONVERSATION_KEY);
  } catch { /* launch falls back to Home */ }
}
/** True when `key` is workspace `id`'s project main agent conversation and
 *  it now lives in another project (or none): the project keeps the key, but
 *  `openWorkspace` refuses to open it there, so launch and reopen fall back
 *  to the workspace's newest conversation instead (overnight 11-03 R4 review). */
async function mainAgentMovedAway(id: string, detail: { sessions: readonly { key: string }[]; projectAgentKey?: string | null }, key: string | null | undefined): Promise<boolean> {
  if (!key || !unlistedMainAgent(detail, key)) return false;
  try {
    const owner = await api.sessionTarget(key);
    return !!owner && owner.workspaceId !== id;
  } catch { return false; }
}
/** The quick, synchronous half of `mainAgentMovedAway`: only then is the
 *  owner worth asking, since the await before the open lets a navigation
 *  happen first (callers check for one after it). */
function unlistedMainAgent(detail: { sessions: readonly { key: string }[]; projectAgentKey?: string | null }, key: string | null | undefined): boolean {
  return !!key && key === detail.projectAgentKey && !detail.sessions.some(s => s.key === key);
}
let launched = false;
/** Launch: reopen the last conversation. An archived, removed or moved
 *  conversation opens its workspace instead (a project's main agent reopens
 *  even archived or before its first turn, unless it moved to another
 *  project); a deleted workspace leaves you on Home. Runs
 *  once; later calls reopen the remembered workspace like before (Work's mount). */
export async function restoreLastConversation() {
  if (launched) { if (workspaceId() && !workspaceDetail()) await selectWorkspace(workspaceId()!); return; }
  launched = true;
  const target = launchTarget(parseLastConversation(saved(LAST_CONVERSATION_KEY)), saved('hollow.workspace'));
  if (!target) return;
  const startedOn = workspaceId();
  const token = opens.peek();
  setWorkspaceLoading(true);
  let keys: string[] | null = null;
  try {
    const detail = await api.readWorkspace(target.workspaceId);
    keys = workspaceConversationKeys(detail);
    if (unlistedMainAgent(detail, target.key) && await mainAgentMovedAway(target.workspaceId, detail, target.key)) keys = keys.filter(k => k !== target.key);
  } catch { keys = null; }
  // The user went somewhere else while this was reading: leave them there.
  if (!opens.isLatest(token) || workspaceId() !== startedOn || workspaceDetail()) { if (opens.isLatest(token)) setWorkspaceLoading(false); return; }
  const landing = launchLanding(target, keys);
  if (landing === 'home') { rememberLastConversation(null); showWorkOverview(); return; }
  // Nobody stepped from Work to here: Back has nowhere to go yet (P1-42).
  if (await openWorkspace(target.workspaceId, landing === 'open' ? target.key : undefined)) launchLanded(target.workspaceId);
}

/** ⌘N (frontier F4): a new conversation in the open workspace. With none
 *  open (2026-09-30), a new quick chat at once: no palette, no form. */
export function startNewConversation() {
  const id = workspaceId();
  if (id) { void newWorkspaceSession(id); return; }
  void startQuickChat();
}

// ── Quick chats (2026-09-30; back from the COO 2026-11-14) ─────────────
/** The built-in Quick chats workspace, once it exists. */
export const quickChatsWorkspace = () => workspaces().find(w => isQuickChats(w)) ?? null;
// The same array until the list changes (ST2): rows ask for it once each, and
// `distinguishing` indexes titles per array, so a fresh copy per call would
// rebuild that index for every row. Callers read it, never change it.
let projectFrom: WorkWorkspace[] | null = null;
let projectList: WorkWorkspace[] = [];
/** Every workspace but the built-in ones (Quick chats, the Main agent): the
 *  lists where they would be confusing (Work, automations' workspace
 *  pickers, the rail's Workspaces). */
export const projectWorkspaces = (): WorkWorkspace[] => {
  const all = workspaces();
  if (all !== projectFrom) { projectFrom = all; projectList = all.filter(w => !isBuiltinWorkspace(w)); }
  return projectList;
};

/** A new quick chat (New chat, ⌘N with no project open, Home's Ask
 *  anything, the sidebar's Quick chats +): a chat outside any project, in
 *  the Quick chats workspace, its composer focused (quick chats come back,
 *  2026-11-14; L8 had sent these to the COO, which keeps its orb). With
 *  `text`, that is sent as the first message through the normal composer.
 *  Creates the Quick chats workspace on first use. */
export async function startQuickChat(text?: string) {
  await openQuickChat(text);
}
/** A drop on Home opens a new quick chat (windowFileDrop.ts): true once
 *  it is open, so the files wait for its composer only then. */
export async function startQuickChatForFiles(): Promise<boolean> {
  return openQuickChat();
}
/** [`startQuickChat`], saying whether the quick chat now exists, is open and
 *  (with `text`) holds the message for its composer. */
async function openQuickChat(text?: string): Promise<boolean> {
  if (!canLeaveConversation()) { openFailed('Wait a moment: the current message is still being sent.'); return false; }
  let quick;
  try { quick = await api.ensureQuickChatsWorkspace(); }
  catch (e) { openFailed(`Couldn't open ${QUICK_CHATS_LABEL}: ${String(e)}`); return false; }
  if (!quickChatsWorkspace()) setWorkspaces([...workspaces(), quick]);
  if (!await openNewSession(QUICK_CHATS_ID)) return false;
  const message = text?.trim();
  if (message) setPendingChatSend({ text: message, cwd: quick.cwd });
  return true;
}

/** Work's Ask anything draft. It lives here, not in the component:
 *  Home unmounts while the quick chat opens ("Opening workspace…"), and a
 *  failed open must give the text back. Cleared only once the quick chat
 *  exists and holds the message (merged plan slice 4, B·F15). */
export const [quickAskDraft, setQuickAskDraft] = createSignal('');
export const [askProject, setAskProject] = createSignal<string | null>(null);
let quickAskSending = false;
/** Send the Ask anything draft (or `text`, the palette's Ask row, One Ask
 *  2026-11-03) as the first message of a new quick chat, or with a project chosen, that
 *  project's lead's (L8, 11-10). A second submit while
 *  the first is still opening does nothing (the draft is no longer cleared up
 *  front, which used to be the guard). */
export async function sendQuickAsk(text?: string): Promise<boolean> {
  const message = (text ?? quickAskDraft()).trim();
  if (!message || quickAskSending) return false;
  quickAskSending = true;
  try {
    const project = askProject();
    let opened: boolean;
    if (project) {
      const origin = {page:shownPage(), id:workspaceId(), cwd:chatCwd(), key:chatAgentSessionKey()};
      const stillHere = () => shownPage() === origin.page && workspaceId() === origin.id && chatCwd() === origin.cwd && chatAgentSessionKey() === origin.key && askProject() === project;
      const destination = await api.readWorkspace(project);
      if (!stillHere()) return false;
      if (!projectChoices([destination.workspace], {}).length) {
        openFailed('This project is no longer available. Choose another project.'); return false;
      }
      // L8 (11-10): a project's one conversation is its lead, so a prompt
      // for a project goes there (after its running turn, if one runs).
      opened = await openProjectAgent(project, stillHere);
      if (opened && workspaceId() === project && chatCwd() === destination.workspace.cwd && !pendingChatSend()) {
        setPendingChatSend({text:message,cwd:destination.workspace.cwd,sessionKey:chatAgentSessionKey(),preset:true});
      } else opened = false;
    } else opened = await openQuickChat(message);
    if (opened && quickAskDraft().trim() === message) setQuickAskDraft('');
    return opened;
  } catch (e) { openFailed(errMsg(e)); return false; }
  finally { quickAskSending = false; }
}

// ── Moving conversations (2026-09-30) ───────────────────────────────────
/** A conversation with a turn queued or running, on screen or not. */
export function sessionWorking(key: string): boolean {
  if (chatAgentSessionKey() === key && chatBusy()) return true;
  if (backgroundChatKeys().includes(key)) return true;
  return runQueue().some(item => item.source === 'chat' && item.sourceId === key
    && (item.status === 'queued' || item.status === 'claimed' || item.status === 'running'));
}
/** Work in this folder, including conversations that are not on screen.
 * Unmapped automation rows are covered by the backend status guard. */
export function workspaceWorking(cwd: string): boolean {
  if (chatCwd() === cwd && chatBusy()) return true;
  if (backgroundChatKeys().some(key => backgroundChatCwd(key) === cwd)) return true;
  return Object.values(workspaceSessions()).some(group => workspaces().some(w => w.id === group.workspaceId && w.cwd === cwd)
    && group.sessions.some(session => sessionWorking(session.key)));
}
/** Why conversation `key` can't move right now, or null. */
export function moveBlockedReason(key: string): string | null {
  if (!canLeaveConversation()) return 'Wait a moment: the current message is still being sent.';
  if (sessionWorking(key)) return 'This conversation is working. Move it when the turn finishes.';
  return null;
}
// Visible text only (toasts, task detail, merge prompts): the quick-chats folder shows as the COO; its stored title and id stay.
const workspaceTitle = (id: string) => isQuickChats(id) ? QUICK_CHATS_LABEL : isMainAgent(id) ? MAIN_AGENT_TITLE : workspaces().find(w => w.id === id)?.title ?? id;
/** The workspace a conversation is in (null for a vault-root chat). */
export const workspaceOfSession = (key: string, cwd?: string | null) => sessionWorkspaceId(key, cwd, workspaceSessions(), workspaces());

/** The canvas store of the open conversation (Canvas C3): its workspace,
 *  Quick chats included, else its own app-data store by session key. Both
 *  null before a conversation has either (nothing is saved then). */
export function conversationCanvasScope(): { workspaceId: string | null; sessionKey: string | null } {
  const key = chatAgentSessionKey();
  const cwd = chatCwd();
  const workspaceId = key ? workspaceOfSession(key, cwd) : (cwd && workspaces().find(w => w.cwd === cwd)?.id) || null;
  return { workspaceId, sessionKey: key };
}

/** Move conversation `key` to workspace `target`. The open conversation (or
 *  `open: true`) follows: its target workspace opens with it on screen.
 *  Any other says so with an Open action. Both offer Undo, which moves it
 *  back (P1-39), unless it is an Undo itself or `noUndo`. Resolves whether
 *  it is in `target` now; a refusal or failure goes to `failed` when given,
 *  else to a toast. */
export async function moveSessionTo(key: string, target: string, opts: { open?: boolean; undo?: boolean; noUndo?: boolean; failed?: (why: string) => void } = {}): Promise<boolean> {
  const fail = opts.failed ?? ((why: string) => flashFooter(`× ${why}`, 5000));
  const why = moveBlockedReason(key);
  if (why) { fail(why); return false; }
  const follow = opts.open || chatAgentSessionKey() === key;
  let moved;
  try { moved = await trackTask('Move conversation', () => api.moveAgentSession(key, target), { detail: workspaceTitle(target) }); }
  catch (e) { fail(errMsg(e)); return false; }
  const title = workspaceTitle(target);
  const from = moved.from;
  // Undo moves it back, saying so without another Undo.
  const back = from && !opts.undo && !opts.noUndo ? () => void moveSessionTo(key, from, { undo: true }) : undefined;
  // A visible "moved" notice until the next turn has run (next-level P13):
  // a conversation with no native session gets no reset notice otherwise.
  if (moved.moved) {
    const turns = Object.values(workspaceSessions()).flatMap(g => g.sessions).find(s => s.key === key)?.turnCount ?? 0;
    recordMove(localStorage, key, title, turns);
  }
  if (!moved.moved) {
    if (follow) await selectWorkspace(target, key);
    else flashFooter(`Already in ${title}`);
    return true;
  }
  if (moved.expiredCards) dropChatInteractionsFor(key);
  if (!isBuiltinWorkspace(target)) addToRail(target);
  await refreshWorkspaces();
  const message = `${opts.undo ? 'Moved back' : 'Moved'} to ${title}`;
  if (follow) {
    await selectWorkspace(target, key);
    if (back) flashWithUndo({ message, kind: 'move', undo: back });
    else flashFooter(message);
  } else {
    if (workspaceId() && (workspaceId() === from || workspaceId() === target)) await refreshWorkspace().catch(() => {});
    // Open closes the bar, as it did when it held the Undo slot.
    const open = () => { dismissSnackbar(bar); return selectWorkspace(target, key); };
    const bar = flashWithUndo({ message, kind: 'move', ...(back ? { undo: back, actions: [{ label: 'Open', run: open }] } : { actionLabel: 'Open', undo: open }) });
  }
  for (const warning of moved.warnings) flashFooter(warning, 6000);
  return true;
}

/** Move to… (the conversation's menu): the palette lists where it can go. */
export function chooseMoveTarget(key: string) {
  const why = moveBlockedReason(key);
  if (why) { flashFooter(`× ${why}`, 4000); return; }
  openMovePalette(key);
}

// New workspace from this chat, rename, archive and restore are in
// conversationActions.ts, loaded on first use (P1-39).

// A dragged conversation dropped on a workspace header (dragdrop.ts).
registerSessionDropHandler<SessionDragPayload>('session', async (payload, target) => { await moveSessionTo(payload.key, target); });

/** ⌘⇧] / ⌘⇧[ (frontier F6): the next or previous project's lead in the
 *  sidebar's order, pinned projects first (L8, 11-10: a project's row is its
 *  lead, the sidebar lists no other conversation). From no project, the
 *  first or last. Does nothing at either end. */
export function stepConversation(dir: 1 | -1) {
  const groups = railGroups(railWorkspaces(), projectWorkspaces(), pinOrder(projectWorkspaces()).filter(id => !railHiddenIds().includes(id)));
  const ids = [...groups.pinned, ...groups.rest].map(w => w.id);
  const open = activePage() === 'today' && !!workspaceDetail() ? workspaceId() : null;
  const at = open ? ids.indexOf(open) : -1;
  const next = at < 0 ? (dir > 0 ? ids[0] : ids[ids.length - 1]) : ids[at + dir];
  if (next) void openProjectAgent(next);
}

/** ⌘\ (frontier F6; run 15 P1): hide the Preview pane, or bring back the
 *  last item it showed. The pane is app-wide, so no page changes. */
export function toggleSidePane() { togglePreview(chatAgentSessionKey()); }

/** Start a fresh conversation in a workspace and focus its composer. */
export async function newWorkspaceSession(id: string) {
  await openNewSession(id);
}
/** `newWorkspaceSession`, resolving to whether it opened (overnight F3:
 *  "Update brief" adds its request only to a conversation that opened). */
export function newWorkspaceSessionOpened(id: string, stillCurrent?: () => boolean): Promise<boolean> {
  return openNewSession(id, stillCurrent);
}
async function openNewSession(id: string, stillCurrent?: () => boolean): Promise<boolean> {
  const opened = await openWorkspace(id, undefined, true, false, stillCurrent);
  if (opened && workspaceId() === id) { setWorkspaceConversationVisible(true); requestChatFocus(); }
  return opened && workspaceId() === id;
}

// A legacy `notion-*` workspace is a plain one since Notion sync was removed (2026-11-10).
export const workspaceEditLock = (id: string) =>
  isBuiltinWorkspace(id) ? 'Built in' : '';
// Delete…'s lock is `workspaceDeleteLock` in workspaceTypes.ts (P0-13).

async function applyWorkspaceUpdate(id: string, update: { title?: string; status?: string }, done: string): Promise<boolean> {
  try {
    await api.updateWorkspace(id, update);
    await refreshWorkspaces();
    if (workspaceId() === id) await refreshWorkspace().catch(() => {});
    flashFooter(done);
    return true;
  } catch (e) { lifecycleFailed(String(e)); return false; }
}
export async function renameWorkspace(w: WorkWorkspace) {
  const title = await showPrompt({ title: 'Rename workspace', confirmLabel: 'Rename', input: { label: 'Name', value: w.title, maxLength: 100 } });
  if (title && title !== w.title) await applyWorkspaceUpdate(w.id, { title }, 'Workspace renamed');
}
/** The sidebar's rename in place: an empty or unchanged name is no change. True when renamed. */
export async function renameWorkspaceTo(w: WorkWorkspace, title: string): Promise<boolean> {
  const name = title.trim();
  if (!name || name === w.title) return false;
  return applyWorkspaceUpdate(w.id, { title: name }, 'Workspace renamed');
}
export async function setWorkspaceStatus(w: WorkWorkspace, status: string): Promise<void> {
  if (status !== 'done') { await applyWorkspaceUpdate(w.id, { status }, status === 'archived' ? `Archived ${w.title}` : `${w.title}: ${status}`); return; }
  // Mark as done offers Undo, which puts back the status it had (P1-38).
  const before = w.status.trim() || 'active';
  try { await api.updateWorkspace(w.id, { status }); }
  catch (e) { lifecycleFailed(String(e)); return; }
  await refreshWorkspaces();
  if (workspaceId() === w.id) await refreshWorkspace().catch(() => {});
  flashWithUndo({ message: `${w.title}: done`, kind: 'status', ttlMs: 10000, undo: () => void applyWorkspaceUpdate(w.id, { status: before }, `${w.title}: ${before}`) });
}
/** Unarchive (P1-38): back to the status it had when archived, and back in
 *  the sidebar. */
export async function unarchiveWorkspace(w: WorkWorkspace) {
  const status = unarchiveStatus(w);
  if (await applyWorkspaceUpdate(w.id, { status }, `Unarchived ${w.title} (back to ${status})`)) addToRail(w.id);
}
export async function deleteWorkspace(w: WorkWorkspace) {
  const ok = await showConfirm({
    title: `Delete “${w.title}”?`,
    body: 'The workspace folder (brief, plan, sources, Results and session records) moves to the vault’s .trash folder: Undo, or Restore under Projects → All projects → Recently deleted, brings it back. Its conversations stay in Conversation history.',
    confirmLabel: 'Move to trash', danger: true,
  });
  if (!ok) return;
  try {
    // Leave it only once the move is done: a refusal (a running turn, its
    // worktree) keeps you where you were, with the reason (P0-13).
    await api.trashWorkspace(w.id);
    if (workspaceId() === w.id) showWorkOverview();
    const inRail = railWorkspaceIds().includes(w.id);
    removeFromRail(w.id);
    if (parseLastConversation(saved(LAST_CONVERSATION_KEY))?.workspaceId === w.id) rememberLastConversation(null);
    await refreshWorkspaces();
    flashWithUndo({ message: `Moved ${w.title} to trash`, kind: 'delete', ttlMs: 10000, undo: () => restoreWorkspace(w.id, w.title, inRail) });
  } catch (e) { lifecycleFailed(String(e)); }
}

/** Bring a deleted workspace back from the vault's trash (P1-41: Delete's
 *  Undo, Recently deleted's Restore), with its conversations; back in the
 *  sidebar when Undo takes back a delete from there. */
export async function restoreWorkspace(id: string, title: string, toRail = false) {
  try { await api.restoreWorkspace(id); }
  catch (e) { lifecycleFailed(`Couldn’t restore ${title}: ${errMsg(e)}`); return; }
  if (toRail) addToRail(id);
  await refreshWorkspaces();
  await refreshWorkspaceSessions();
  flashFooter(`Restored ${title}`);
}

/** Archive one workspace after the person confirms (the workspace map's
 *  drag to the edge, overnight B4), with an Undo that restores its status.
 *  Resolves whether it was archived. */
export async function proposeArchive(id: string, title: string, status: string, onUndone?: () => void): Promise<boolean> {
  const ok = await showConfirm({
    title: `Archive “${title}”?`,
    body: 'Archiving only sets its status: nothing moves or is deleted, and Undo or Unarchive puts it back.',
    confirmLabel: 'Archive',
  });
  if (!ok) return false;
  try { await api.updateWorkspace(id, { status: 'archived' }); }
  catch (e) { lifecycleFailed(String(e)); return false; }
  await refreshWorkspaces();
  if (workspaceId() === id) await refreshWorkspace().catch(() => {});
  flashWithUndo({
    message: `Archived “${title}”`, kind: 'archive', ttlMs: 10000,
    undo: async () => {
      await api.updateWorkspace(id, { status: status || 'active' }).catch(e => lifecycleFailed(String(e)));
      await refreshWorkspaces();
      if (workspaceId() === id) await refreshWorkspace().catch(() => {});
      onUndone?.();
    },
  });
  return true;
}

/** Merge into… (master plan #18): the palette lists where `w` can merge. */
export function chooseMergeTarget(w: WorkWorkspace) {
  openMergePalette(w.id);
}

/** Merge workspace `from` into `into`: show every file and conversation
 *  that moves, then merge exactly that (the plan's hash binds the confirm).
 *  `from` is archived afterwards; History can put the files back. */
/** `onChanged` runs whenever workspaces may have moved: after the merge, a
 *  merge that failed part-way, and the snackbar's Undo. The workspace map
 *  refetches with it, which a Retry (overnight G2) needs: it runs outside
 *  the map's own handler. */
export async function mergeWorkspaceInto(from: string, into: string, onChanged?: () => void) {
  let plan;
  try { plan = await trackTask('Preview merge', () => api.previewWorkspaceMerge(from, into)); }
  catch (e) { flashRetry(`× Couldn’t preview the merge: ${errMsg(e)}`, () => mergeWorkspaceInto(from, into, onChanged)); return; }
  // A long list starts on Cancel, so a stray Enter merges nothing; a merge
  // a working conversation blocks says why before Merge, which is off (P1-38).
  const ok = await showConfirm({
    title: `Merge “${plan.fromTitle}” into “${plan.intoTitle}”?`,
    summary: mergeConfirmSummary(plan),
    body: mergeConfirmBody(plan),
    confirmLabel: 'Merge',
    focusCancel: true,
    blockedReason: plan.blocked ?? undefined,
  });
  if (!ok || plan.blocked) return;
  // Turn counts before the move, for each conversation's "moved" notice.
  const turns = new Map(Object.values(workspaceSessions()).flatMap(g => g.sessions).map(s => [s.key, s.turnCount]));
  let out;
  try { out = await trackTask('Merge workspace', () => api.mergeWorkspace(from, into, plan.hash), { detail: plan.intoTitle }); }
  catch (e) {
    // A merge that stopped part-way may have moved some of it.
    lifecycleFailed(String(e));
    await refreshWorkspaces();
    await refreshWorkspaceSessions();
    onChanged?.();
    return;
  }
  removeFromRail(from);
  if (!isBuiltinWorkspace(into)) addToRail(into);
  if (parseLastConversation(saved(LAST_CONVERSATION_KEY))?.workspaceId === from) rememberLastConversation(null);
  // Each moved conversation says where its next turn runs, as a single move does.
  for (const s of plan.sessions) recordMove(localStorage, s.key, plan.intoTitle, turns.get(s.key) ?? 0);
  if (out.expiredCards) for (const s of plan.sessions) dropChatInteractionsFor(s.key);
  await refreshWorkspaces();
  await refreshWorkspaceSessions();
  if (workspaceId() === from) await selectWorkspace(into);
  else if (workspaceId() === into) await refreshWorkspace().catch(() => {});
  onChanged?.();
  flashWithUndo({ message: mergeDoneMessage(out), kind: 'merge', ttlMs: 10000, undo: async () => { await undoWorkspaceMerge(from, plan.fromTitle); onChanged?.(); } });
  for (const warning of out.warnings) flashFooter(warning, 6000);
}

/** Archive older entries… (V5a, `workspaces.archive_shared_docs`): a dry run
 *  of WORKSPACE.md and PLAN.md shows what would move, then one confirm applies
 *  exactly that (each doc's hash binds its apply; a doc that changed since
 *  moves nothing). History & recovery can put it back. */
export async function archiveOlderEntries(w: WorkWorkspace) {
  let dry: SharedDocArchive[];
  try {
    dry = await trackTask('Preview archive', async () => {
      const rows: SharedDocArchive[] = [];
      for (const doc of SHARED_DOCS) {
        try { rows.push(await api.archiveSharedDocs(w.id, doc)); }
        catch (e) { if (!(doc === 'PLAN.md' && errMsg(e).includes('no PLAN.md'))) throw e; }
      }
      return rows;
    });
  } catch (e) { flashRetry(`× Couldn’t preview the archive: ${errMsg(e)}`, () => archiveOlderEntries(w)); return; }
  if (!dry.some(hasMoves)) { flashFooter(`Nothing in “${w.title}” is old enough to archive.`); return; }
  const busy = dry.some(r => r.busy);
  const ok = await showConfirm({
    title: `Archive older entries of “${w.title}”?`,
    summary: archiveSummary(dry),
    body: archiveBody(dry),
    confirmLabel: 'Archive',
    focusCancel: true,
    blockedReason: busy ? 'a conversation is working in this project; archive once it finishes' : undefined,
  });
  if (!ok || busy) return;
  const done: SharedDocArchive[] = [];
  try {
    for (const r of dry.filter(hasMoves)) done.push(await trackTask('Archive older entries', () => api.archiveSharedDocs(w.id, r.doc, 'apply', r.hash, r.today), { detail: r.doc }));
  } catch (e) { lifecycleFailed(String(e)); }
  if (workspaceId() === w.id) await refreshWorkspace().catch(() => {});
  if (done.length) flashFooter(archiveDoneMessage(done), 8000);
  for (const r of done) for (const warning of r.warnings) flashFooter(warning, 6000);
}

/** Undo merge… from the merged workspace's menu: asks first. */
export async function confirmUndoMerge(w: WorkWorkspace) {
  const into = workspaceTitle(w.mergedInto ?? '');
  const ok = await showConfirm({
    title: `Undo the merge of “${w.title}”?`,
    body: `Its files and conversations go back from “${into}”, the merged sections come out of “${into}”'s brief, plan and sources list (unless edited since), and “${w.title}” gets its old status back. Anything changed since stays where it is, and you'll be told what.`,
    confirmLabel: 'Undo merge',
  });
  if (ok) await undoWorkspaceMerge(w.id, w.title);
}

/** Undo merge (after the menu's confirm, or the snackbar right after a
 *  merge): puts back what its merge record lists. */
export async function undoWorkspaceMerge(from: string, fromTitle: string) {
  let out;
  try { out = await trackTask('Undo merge', () => api.undoWorkspaceMerge(from), { detail: fromTitle }); }
  catch (e) { lifecycleFailed(String(e)); await refreshWorkspaces(); await refreshWorkspaceSessions(); return; }
  addToRail(from);
  // Conversations that went back say so, replacing the merge's moved notice.
  const turns = new Map(Object.values(workspaceSessions()).flatMap(g => g.sessions).map(s => [s.key, s.turnCount]));
  for (const key of out.sessionKeys) recordMove(localStorage, key, fromTitle, turns.get(key) ?? 0);
  await refreshWorkspaces();
  await refreshWorkspaceSessions();
  if (workspaceId() === from || workspaceId() === out.into) await refreshWorkspace().catch(() => {});
  flashFooter(unmergeDoneMessage(out, fromTitle), 5000);
  for (const warning of out.warnings) flashFooter(warning, 8000);
}

/** Remove a build workspace's git worktree. The branch stays in the repo;
 *  unsaved work (uncommitted or unpushed) needs a second, explicit confirm. */
export async function removeWorktree(w: WorkWorkspace) {
  const build = w.build;
  if (!build?.exists) return;
  if (chatBusy() && chatCwd() === w.cwd) { lifecycleFailed('Stop the current agent turn before removing its worktree.'); return; }
  const ok = await showConfirm({
    title: `Remove the worktree for “${w.title}”?`,
    body: `This deletes ${build.worktree}. The branch ${build.branch} stays in ${build.repo}, and the workspace and its conversations stay; new conversations run in the workspace folder.`,
    confirmLabel: 'Remove worktree',
  });
  if (!ok) return;
  try {
    await api.removeBuildWorktree(w.id, false);
  } catch (e) {
    const unsaved = worktreeUnsaved(e);
    if (!unsaved) { lifecycleFailed(String(e)); return; }
    const force = await showConfirm({
      title: 'The worktree has unsaved work',
      body: `It has ${unsavedText(unsaved)}. Removing it now loses ${unsaved.uncommitted ? 'the uncommitted changes' : 'nothing on disk, but the unpushed commits stay only on the local branch'}.`,
      confirmLabel: 'Remove anyway', danger: true,
    });
    if (!force) return;
    try { await api.removeBuildWorktree(w.id, true); } catch (e2) { lifecycleFailed(String(e2)); return; }
  }
  await refreshWorkspaces();
  if (workspaceId() === w.id) await refreshWorkspace().catch(() => {});
  flashFooter(`Removed the worktree for ${w.title}`);
}

export async function refreshWorkspace() {
  const id = workspaceId(); if (!id) return;
  const generation = opens.peek();
  const detail = await api.readWorkspace(id);
  if (workspaceId() === id && opens.isLatest(generation)) { setWorkspaceDetail(detail); setWorkspaceRevision(v => v + 1); }
}
// Keep the recap and result shelf current after external edits as well as runs.
// Coalesce watcher bursts; refreshWorkspace guards against workspace switches.
let workspaceChangeTimer: ReturnType<typeof setTimeout> | undefined;
listen<VaultChange>('vault-changed', e => {
  if (!e.payload?.folders.includes('workspaces')) return;
  if (workspaceChangeTimer) clearTimeout(workspaceChangeTimer);
  workspaceChangeTimer = setTimeout(() => {
    void refreshWorkspaces();
    void refreshWorkspace().catch(error => setWorkspaceError(String(error)));
  }, 400);
}).catch(() => { /* browser preview */ });
export async function selectWorkspace(id: string, key?: string, fresh = false) {
  await openWorkspace(id, key, fresh);
}
/** How long a workspace open's page transition waits for the workspace to
 *  load, so a fast load morphs into the loaded page (overnight 10-11 B1). */
const WORKSPACE_SETTLE_MS = 120;
/** `selectWorkspace` for the person's own click or palette pick (overnight
 *  10-11 B1): the page column animates, and a `morphFrom` row grows into the
 *  page. Launch restores, merges, moves and agents' navigation stay
 *  `selectWorkspace`, still. */
export async function visitWorkspace(id: string, key?: string, options: { conversation?: boolean } = {}) {
  await withViewTransition(async () => {
    const opened = await (key ? pickConversation(id, key) : openWorkspace(id));
    if (options.conversation && opened && shownPage() === 'today' && workspaceId() === id
      && workspaceDetail()?.workspace.cwd === chatCwd() && (!key || chatAgentSessionKey() === key)) {
      setWorkspaceConversationVisible(true);
      requestChatFocus();
    }
  }, WORKSPACE_SETTLE_MS);
}
/** A sidebar row returns to its conversation; ordinary visits retain a
 *  requested section such as Results and do not take composer focus. */
export function visitWorkspaceConversation(id: string, key?: string) {
  return visitWorkspace(id, key, { conversation: true });
}
/** Explicit project main route. Existing task chats and default restores keep
 * their identities; opening this route never creates a paid run. */
export async function openProjectAgent(id: string, stillCurrent?: () => boolean): Promise<boolean> {
  const opened = await openWorkspace(id, undefined, false, false, stillCurrent, true);
  if (opened && workspaceId() === id) { addToRail(id); requestChatFocus(); }
  return opened && workspaceId() === id;
}
/** Open conversation `key` the person picked (a row, the palette, Open
 *  conversation): once open, its workspace is back in the rail even after
 *  Hide from sidebar (P1-38). Resolves whether it opened. */
export async function pickConversation(id: string, key: string, stillCurrent?: () => boolean): Promise<boolean> {
  const opened = await openWorkspace(id, key, false, false, stillCurrent);
  if (opened) addToRail(id);
  return opened;
}
/** `selectWorkspace`, resolving to whether it opened (morning M5: "Work on
 *  this" fills the composer only of a workspace that opened). */
export function selectWorkspaceOpened(id: string): Promise<boolean> {
  return openWorkspace(id);
}
/** Conversation `key` by its exact key when the workspace's own list (the
 *  newest 1,000) doesn't have it: archived and older conversations open too,
 *  but only in the workspace they belong to (merged plan slice 4, B·F15). */
async function exactSession(key: string, id: string): Promise<AgentSessionSummary | undefined> {
  try {
    const target = await api.sessionTarget(key);
    return target?.workspaceId === id ? target.session : undefined;
  } catch { return undefined; }
}
/** [`selectWorkspace`], saying whether workspace `id` (and conversation
 *  `key`) is now open. `quiet`: the workspace is already on screen and only
 *  its conversation comes back, so the page doesn't swap to "Opening
 *  workspace…" (which would unmount the side panel and the control that
 *  asked, A04). */
async function openWorkspace(id: string, key?: string, fresh = false, quiet = false, stillCurrent?: () => boolean, projectMain = false): Promise<boolean> {
  return await guardNavigation(() => openWorkspaceAfterGuard(id, key, fresh, quiet, stillCurrent, projectMain)) ?? false;
}

async function openWorkspaceAfterGuard(id: string, key?: string, fresh = false, quiet = false, stillCurrent?: () => boolean, projectMain = false): Promise<boolean> {
  if(stillCurrent && !stillCurrent())return false;
  // A running turn keeps running in the background (build programme PR B4);
  // only a turn with no conversation key yet (its send is still being
  // accepted) or a loading history pins the conversation on screen.
  if (!canLeaveConversation()) { openFailed('Wait a moment: the current message is still being sent.'); return false; }
  const token = opens.start(); opening = true;
  remember(); if (!(quiet && id === workspaceId() && workspaceDetail())) setWorkspaceLoading(true); setWorkspaceError('');
  // Go there now, from any page: a click in the rail from Settings or
  // Library shows "Opening workspace…" instead of seeming to do nothing
  // while the workspace loads (or fails to). Quietly: the step Back and
  // Forward record is the workspace once it has loaded (P1-42).
  quietly(() => setActivePage('today'));
  try {
    const detail = await api.readWorkspace(id);
    if (projectMain) {
      if (!detail.projectAgentKey) throw new Error('This workspace has no project lead.');
      key = detail.projectAgentKey;
    }
    // "Already open" needs its turns on screen: at launch the key and cwd are
    // restored from localStorage but the transcript is not, so it must load.
    // A project's lead with nothing sent yet has no transcript to load: once
    // this run has opened it (its detail is held), its sidebar row brings
    // the same conversation back instead of a remount (L8, 11-10).
    const emptyLeadShown = !!key && key === detail.projectAgentKey && workspaceId() === id && !!workspaceDetail()
      && !detail.sessions.some(s => s.key === key);
    // Turns on screen or a busy composer don't prove the transcript loaded:
    // after a reload mid-turn the run-queue adoption or a live event gets
    // there first (overnight 11-13 c-11), so the pending mark decides.
    const storedPending = (k: string | null) => !!k && chatStoredTranscriptPending() === k;
    const sameConversation = !fresh && !isBackgrounded(key ?? chatAgentSessionKey() ?? '') && (key ? key === chatAgentSessionKey() && chatCwd() === detail.workspace.cwd && !storedPending(key) && (chatTurns.length > 0 || chatBusy() || emptyLeadShown)
      : chatCwd() === detail.workspace.cwd && !!chatAgentSessionKey() && !storedPending(chatAgentSessionKey()) && chatTurns.length > 0);
    // Opening a workspace brings back the conversation left running there.
    const backgrounded = !fresh && !key ? backgroundedKeyForCwd(detail.workspace.cwd) : null;
    const session = fresh ? undefined : key ? detail.sessions.find(s => s.key === key) ?? await exactSession(key, id) : detail.sessions[0];
    if (key && !session && backgroundChatCwd(key) !== detail.workspace.cwd) {
      if (key !== detail.projectAgentKey) throw new Error("This conversation is no longer available in this workspace.");
      const owner = await api.sessionTarget(key);
      if (owner) throw new Error('This lead conversation belongs to another project. Move it back from Conversation history before continuing.');
    }
    const target = backgrounded ?? key ?? session?.key ?? null;
    // A conversation still running in the background comes back from its
    // snapshot, not from the store (which has only finished turns).
    const readKey = !sameConversation && target && !isBackgrounded(target) && session ? session.key : null;
    const readGeneration = readKey ? beginStoredTranscriptRead(readKey) : 0;
    const conversation = readKey ? await api.readAgentConversation(readKey) : null;
    // A creation flow may have lost its page while this read was pending.
    // Check before remember/reset can claim the newer page's composer draft.
    if (!opens.isLatest(token) || (stillCurrent && !stillCurrent())) return false;
    // A newer read of the same transcript (its turn's end re-read it) already
    // put it on screen: this older copy must not replace it (Codex review).
    // A newer read that never wrote the screen does not count.
    const keepScreen = sameConversation || (!!readKey && readKey === chatAgentSessionKey() && !storedTranscriptReadApplies(readKey, readGeneration));
    // That conversation is already on screen with live turns but not its
    // stored ones: put them in front, keeping the running turn as it is.
    const hydrateInPlace = !keepScreen && !!conversation && !!target && target === chatAgentSessionKey() && storedPending(target)
      && chatCwd() === detail.workspace.cwd && (chatBusy() || chatTurns.length > 0);
    if (!keepScreen && !hydrateInPlace && !backgroundRunningChat()) { openFailed('Wait a moment: the current message is still being sent.'); return false; }
    remember();
    // The legacy slot belonged to the previously selected/default chat,
    // never whichever historical row is clicked first after this upgrade.
    const legacyOwner = chatCwd() === detail.workspace.cwd ? chatAgentSessionKey() : detail.sessions[0]?.key ?? null;
    draftSaved(conversationDrafts.migrate(id, legacyOwner));
    applying = true;
    if (hydrateInPlace) {
      showStoredTranscript(target!, turnsFromConversation(conversation!));
      markStoredTranscriptApplied(readKey!, readGeneration);
    } else if (!keepScreen && target && restoreBackgroundChat(target)) {
      restoreDraft(id, target);
    } else if (!keepScreen) {
      resetChatState();
      if (conversation) {
        const s = conversation.summary;
        if (!KNOWN_AGENTS.includes(s.agent as ChatAgent)) throw new Error(`Unavailable agent: ${s.agent}`);
        setChatAgentPersistent(s.agent as ChatAgent);
        setChatAgentSessionKeyPersistent(s.key);
        const permission = conversationPermission(s.key, conversation.permissionMode, s.agent);
        setChatPermissionMode(permission.mode);
        setChatSessionIdPersistent(permission.resumeNative && s.resumable === 'native' ? s.harnessSessionId : null);
        const turns: ChatTurn[] = turnsFromConversation(conversation);
        const notice = movedNotice(localStorage, s.key, s.turnCount);
        if (notice) turns.push({ role: 'system', text: notice, tools: [], hydratedNotice: true });
        setChatTurns(turns); rebuildChatArtifacts(turns);
        if (readKey) markStoredTranscriptApplied(readKey, readGeneration);
      } else {
        // A fresh COO conversation keeps the chosen agent like any chat
        // (L6, 11-10: any agent may run the COO); reopened ones keep theirs.
        applyNewChatPermissionMode();
        if (target && target === detail.projectAgentKey) {
          setChatAgentSessionKeyPersistent(target);
          // A choice made for this key before its first send (S3) outranks
          // the new-chat default just applied.
          setChatPermissionMode(conversationPermission(target, chatPermissionMode()).mode);
        }
      }
      setChatCwdPersistent(detail.workspace.cwd);
      restoreDraft(id, target);
    }
    // The old queue adoption could pair this session key with another cwd.
    // Restoring its snapshot must not restore that mismatch indefinitely.
    // Explicit keys were checked against this workspace's sessions above;
    // implicit background targets were found by this workspace's cwd.
    setChatCwdPersistent(detail.workspace.cwd);
    // Hide from sidebar holds until a conversation is picked
    // (`pickConversation`) or started there (P1-38): an open, a launch
    // restore or a refresh lists only a workspace that isn't hidden.
    if (target && !railHiddenIds().includes(id)) addToRail(id);
    if (workspaceId() !== id || !workspaceDetail()) {
      setWorkspaceLastVisited(saved(`hollow.visited.${id}`));
      try { localStorage.setItem(`hollow.visited.${id}`, new Date().toISOString()); } catch { /* recap still works without a visit comparison */ }
    }
    setWorkspaceId(id); setWorkspaceDetail(detail); setChatPanelOpen(false);
    draftOwner = { workspace: id, cwd: detail.workspace.cwd, key: chatAgentSessionKey() };
    localStorage.setItem('hollow.workspace', id); setActivePage('today');
    // A turn first sent here may not have been listed before we left it.
    void refreshWorkspaceSessions();
    refreshRunQueue(0);
    return true;
  } catch (e) {
    if (opens.isLatest(token) && (!stillCurrent || stillCurrent())) {
      const title = workspaces().find(w => w.id === id)?.title;
      openFailed(title ? `Couldn't open ${title}: ${String(e)}` : String(e));
    }
    return false;
  }
  finally { applying = false; if (opens.isLatest(token)) { opening = false; setWorkspaceLoading(false); } }
}

/** Hand conversation `session`, which no workspace holds (a vault-root
 *  chat), to Conversation history (Library → Conversations, page id `chat`)
 *  with its summary, so it opens the conversation even when its
 *  own capped list doesn't carry it (older than its 300 newest, or archived).
 *  The one path for Home, Activity and an agent's `navigate('session')`. */
function openOnChatPage(session: AgentSessionSummary) {
  goToPage('chat');
  setAppSessionRequest({ key: session.key, session });
}
/** Open conversation `key` by its exact key, wherever it lives (B·F15):
 *  never through the rail's capped lists, a title or the newest match. One
 *  in no workspace opens in Conversation history. False, with the reason on screen,
 *  when it can't be opened. */
export async function openSessionByKey(key: string, stillCurrent?: () => boolean): Promise<boolean> {
  if (stillCurrent && !stillCurrent()) return false;
  const request = opens.peek();
  let target: SessionTarget | null;
  try { target = await api.sessionTarget(key); }
  catch (e) { if (!stillCurrent || stillCurrent()) openFailed(`Couldn't find the conversation: ${String(e)}`); return false; }
  if (stillCurrent && (!stillCurrent() || !opens.isLatest(request))) return false;
  if (!target) { openFailed('This conversation no longer exists.'); return false; }
  if (!target.workspaceId) { openOnChatPage(target.session); return true; }
  // Every caller is a person's click on one conversation: a pick (P1-38).
  return pickConversation(target.workspaceId, key, stillCurrent);
}
/** Where an agent's `navigate('session', key)` opens conversation `key`
 *  (B·F15): its workspace, found by exact key when the rail's 25 newest don't
 *  list it, or Conversation history (`'chat'`) for a conversation in no workspace.
 *  Throws when no conversation has the key or its workspace won't open, so
 *  the agent is told instead of nothing happening. */
export async function openSessionForAgent(key: string): Promise<'workspace' | 'chat'> {
  let id = Object.values(workspaceSessions()).find(g => g.sessions.some(s => s.key === key))?.workspaceId ?? null;
  if (!id) {
    const target = await api.sessionTarget(key);
    if (!target) throw new Error(`navigate: no conversation has the key ${key}.`);
    if (!target.workspaceId) { openOnChatPage(target.session); return 'chat'; }
    id = target.workspaceId;
  }
  if (!await openWorkspace(id, key)) throw new Error(`navigate: couldn't open conversation ${key}: ${workspaceError() || 'another open took its place'}`);
  return 'workspace';
}
/** A workspace that won't open says so where the user is looking: the Work
 *  page's banner sits at the top, out of view from a scrolled list, so the
 *  failure also shows as a toast. */
function openFailed(message: string) { lifecycleFailed(message); }
/** Any workspace or conversation lifecycle failure (open, rename, status,
 *  archive, merge, delete, restore, worktree; P1-41): the Work page's banner
 *  plus a toast, since the menus that start them work from every page and
 *  only the Work page shows the banner. */
export function lifecycleFailed(message: string) {
  setWorkspaceError(message);
  flashFooter(`× ${message}`);
}
export function showWorkOverview() {
  return guardNavigation(showWorkOverviewAfterGuard);
}
function showWorkOverviewAfterGuard() {
  remember(); opens.cancel(); opening = false; setWorkspaceId(null); setWorkspaceDetail(null);
  localStorage.removeItem('hollow.workspace'); setWorkspaceLoading(false); setActivePage('today');
}
/** The one user-facing page jump shared by the rail, Cmd+digit, the palette and
 *  the native menu. "Work" always means the overview: a bare
 *  setActivePage('today') keeps the open workspace, which made Cmd+1 and the
 *  rail disagree (audit H01). The workspace draft is remembered either way. */
// Work's views (store/destinations.ts) ask here whether the overview is
// on screen and come here to show it (consolidation D).
// goHome keeps the view: it is how a view opened from another page (Today,
// Running, the map) reaches Work, so it must not reset to the overview.
bindDestinationNav({ onHome: () => activePage() === 'today' && !workspaceId(), goHome: onNavigate => goToPage('today', { keepWorkView: true, onNavigate }) });
bindReviewNav({ onReview: () => activePage() === 'review', goReview: () => goToPage('review') });
// Back and Forward step through workspaces and conversations (P1-42): an
// entry of Work's page names the workspace and conversation on screen,
// and going back to one reopens it (its page change then matches the entry).
const navCurrent = () => {
  const id = workspaceId(), key = chatAgentSessionKey();
  if (!id) return {};
  return key && workspaceDetail()?.workspace.cwd === chatCwd() ? { workspaceId: id, sessionKey: key } : { workspaceId: id };
};
bindWorkspaceNav({
  current: navCurrent,
  restore: e => {
    // An open still loading for another entry would land over this one.
    // (A quiet reopen never shows loading, so `opening` says it is pending.)
    if (opening || workspaceLoading()) { opens.cancel(); opening = false; setWorkspaceLoading(false); }
    if (currentPage(e.page) !== 'today') return;
    if (!e.workspaceId) { if (workspaceId()) showWorkOverviewAfterGuard(); return; }
    const now = navCurrent();
    if (workspaceDetail() && now.workspaceId === e.workspaceId && now.sessionKey === e.sessionKey) return;
    // Back and Forward have passed the navigation guard already. No key: the
    // new conversation it showed, never sent.
    void openWorkspaceAfterGuard(e.workspaceId, e.sessionKey, !e.sessionKey);
  },
});
/** The open workspace's title: what the top bar's return button and New
 *  conversation go to (P1-42); none on Work's overview. */
export const openWorkspaceTitle = () => {
  const id = workspaceId();
  return id ? workspaces().find(w => w.id === id)?.title ?? workspaceDetail()?.workspace.title : undefined;
};

/** P1-05: every route to Work (rail rows, ⌘1, the palette, the menu)
 *  lands on its overview tab, not the one last open; `keepWorkView` is only
 *  for a view that asked to be shown. */
/** `onNavigate` commits an accepted destination before page exposure. A caller
 * selecting a Work view must also set `keepWorkView: true`. */
export function goToPage(page: PageId, opts: { keepWorkView?: boolean; onNavigate?: () => void } = {}) {
  const jump = () => {
    // Select the accepted destination before revealing its page. Otherwise
    // reactive observers can briefly mount Home and start its data reads.
    opts.onNavigate?.();
    if (currentPage(page) === 'today') { if (!opts.keepWorkView) resetWorkView(); showWorkOverviewAfterGuard(); }
    else { remember(); setActivePage(page); }
  };
  // Already there: nothing to animate (overnight 10-11 B1).
  const there = activePage() === currentPage(page) && (currentPage(page) !== 'today' || !workspaceId());
  if (there) return currentPage(page) === 'today' ? guardNavigation(jump) : jump();
  return guardNavigation(jump, () => withViewTransition(jump));
}
createRoot(() => {
  createEffect(on(backgroundChatKeys, (keys, previous) => {
    // Done entries keep their snapshot; restoring/dropping a running entry
    // removes it first, so those removals must not masquerade as completion.
    const cwds = [...new Set((previous ?? []).filter(key => !keys.includes(key))
      .map(key => backgroundChatCwd(key)).filter((cwd): cwd is string => !!cwd))];
    if (cwds.length) void refreshEndedBuildWorkspaces(cwds);
  }));
  createEffect(() => {
    chatInput(); chatAttachments(); chatAgentSessionKey(); chatCwd(); workspaceDetail();
    remember();
  });
  // Whatever workspace conversation is on screen is the one launch reopens.
  // Leaving for Home or another page keeps it (detail is null on Home).
  createEffect(() => {
    const open = workspaceDetail()?.workspace;
    const key = chatAgentSessionKey();
    if (open && workspaceId() === open.id && chatCwd() === open.cwd) {
      rememberLastConversation({ workspaceId: open.id, key });
      // A new conversation's first send: its Back/Forward step learns the
      // key (an open sets the key while `applying`, and records its own).
      if (key && !applying) learnConversation(open.id, key);
    }
  });
  let busy = false;
  createEffect(() => {
    const current = chatBusy();
    // A send in a workspace starts a session there: list it in the rail.
    const open = workspaceDetail()?.workspace;
    if (!busy && current && open && chatCwd() === open.cwd) addToRail(open.id);
    // A turn that just ended may have committed, pushed or opened a pull
    // request in its worktree: ask for git state now, not throttled.
    if (busy && !current && !backgroundChatKeys().includes(chatAgentSessionKey() ?? '')) {
      // Moving a still-running chat off screen is not completion.
      const cwd = chatCwd();
      void refreshWorkspace().catch(e => setWorkspaceError(String(e)));
      void refreshEndedBuildWorkspaces(cwd ? [cwd] : []);
    }
    busy = current;
  });
});

export const [newWorkspaceRequest, setNewWorkspaceRequest] = createSignal(0);
export function requestNewWorkspace() {
  setActivePage('today');
  setNewWorkspaceRequest(v => v + 1);
}

export const [workspaceConversationVisible, setWorkspaceConversationVisible] = createSignal(true);
/** Bring the open workspace's conversation back on screen when another
 *  context's is loaded (the Work page's "Open workspace conversation", a
 *  canvas view's Refresh, overnight 2026-10-16 A04): the conversation on
 *  screen, if it is one of this workspace's, else its default. Never moves
 *  focus. True when this workspace's conversation is on screen afterwards. */
export async function reopenWorkspaceConversation(): Promise<boolean> {
  const id = workspaceId();
  if (!id) return false;
  const key = chatAgentSessionKey();
  const detail = workspaceDetail();
  let selected = key && detail && workspaceConversationKeys(detail).includes(key) ? key : undefined;
  if (selected && detail && unlistedMainAgent(detail, selected)) {
    const token = opens.peek(), page = shownPage();
    const moved = await mainAgentMovedAway(id, detail, selected);
    // Nothing is opening yet, so a navigation meanwhile cancels no token:
    // whatever the person did while the owner was read wins (A04).
    if (!opens.isLatest(token) || shownPage() !== page || workspaceId() !== id || chatAgentSessionKey() !== key) return false;
    if (moved) selected = undefined;
  }
  await openWorkspace(id, selected, false, true);
  return workspaceDetail()?.workspace.cwd === chatCwd();
}
export async function focusWorkspaceConversation(): Promise<boolean> {
  const id = workspaceId();
  if (!id) { requestNewWorkspace(); return false; }
  if (workspaceDetail()?.workspace.cwd === chatCwd()) setActivePage('today');
  else await selectWorkspace(id);
  if (workspaceDetail()?.workspace.cwd !== chatCwd()) return false;
  setWorkspaceConversationVisible(true);
  requestChatFocus();
  return true;
}

// ── Results and project notes in Files (COO rebuild L4) ─────────────────
/** Open a workspace's Result `path` (vault-relative) in the Preview pane's
 *  Files tab with its review bar (Needs you, Library, Home, a chat chip,
 *  search). The workspace opens first, so Ask for a revision drafts in its
 *  own conversation; the file opens once it has. It was the workspace
 *  panel's Results section (2026-10-03 to 2026-11-10). */
export async function openWorkspaceResult(id: string, path: string) {
  await guardNavigation(async () => {
    if (workspaceId() !== id || !workspaceDetail()) { if (!await openWorkspace(id)) return; }
    else setActivePage('today');
    const root = await vaultRoot();
    if (root) openPreview({ kind: 'file', path: path.startsWith('/') ? path : inVault(root, path) });
  });
}
/** Open a workspace's own file `rel` (PLAN.md, WORKSPACE.md, SOURCES.md;
 *  vault-relative), at `line` if given, in the Files tab. */
export async function openWorkspaceFile(rel: string, line?: number) {
  const root = await vaultRoot();
  if (root) openPreview({ kind: 'file', path: inVault(root, rel), line });
}
