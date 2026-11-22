/**
 * One dispatcher for the shared actions (plan completion L4, matrix N10).
 * `actionDefs.ts` says what each action is; this module is the one place
 * that runs it and decides whether it can run now. The palette's fixed rows,
 * the go-to keys (goTo.ts), the ⌘ shortcuts (App.tsx) and the native menu
 * (appMenu.ts) all call `runAction`, so a destination or behaviour is wired
 * once, whichever way the person reached it.
 *
 * An action whose condition doesn't hold (Stop with nothing running, Back
 * with nowhere to go) isn't run; reached anyway from a menu or key, it says
 * why in the footer, in the words `CONDITIONS` gives the route ledger.
 *
 * Rarely used actions load their module on first use, so the launch bundle
 * carries only the table.
 */
import { ACTION_BY_GO, ACTION_BY_ID, ACTION_BY_KEY, ACTION_BY_MENU, CONDITIONS, type ActionDef, type Condition } from './actionDefs';
import { focusWorkspaceConversation, goToPage, setNewWorkspaceRequest, startNewConversation, startQuickChat, stepConversation, toggleSidePane, workspaceDetail, workspaceId, chooseMergeTarget, openWorkspaceFile } from './store/workspaces';
import {
  activePage, agentsAvailable, canLeaveConversation, canNavBack, canNavForward, chatBusy, flashFooter, navBack, navForward, openCommandPalette,
  requestNewNote, openSettingsFromOutside, requestSearchChats, setActivePage, setModelMenuRequested, stopChatRun, toggleShortcutsOverlay, toggleSidebar,
} from './store';
import { canReopenClosedView, previewRemembered, previewShown, reopenClosedView } from './store/preview';
import { openModal } from './modals/TaskModal';
import { openAdd, openImportSource, openQuickCapture } from './modals/addState';
import { openMainAgent } from './store/mainAgent';
import { sideChatOpen, toggleSideChatChecked } from './store/sideChat';
import { openMissionControl } from './store/missionControl';
import { openPullRequests } from './store/destinations';
import { openCanvasGallery } from './store/canvasGallery';
import { openLibraryTab } from './store/libraryMode';
import { openWaitingOnYou } from './store/waitingOnYou';
import { openHomeAsk } from './store/ask';
import { openConflictCopies } from './store/conflictCopies';
import { lastCanvas } from './store/lastCanvas';
import { isMainAgent, isQuickChats } from './workspaceTypes';
import { canMergeFrom } from './workspaceMerge';
import { api } from './api';
import { openPreview } from './store/preview';
import { organizationUndoReady, undoLastOrganization } from './store/organizationUndo';

/** Whether each condition holds now. */
export const CONDITION_HOLDS: Readonly<Record<Condition, () => boolean>> = {
  'running-turn': () => chatBusy(),
  'chat-action-undo': () => organizationUndoReady(),
  'workspace-open': () => !!workspaceId(),
  // The Preview pane shows something, or has an item ⌘\ can bring back.
  'side-pane': () => previewShown() || previewRemembered() || (activePage() === 'today' && !!workspaceDetail()),
  // The sidebar greys New project out the same way.
  'can-leave': () => canLeaveConversation(),
  'build-workspace': () => !!workspaceDetail()?.workspace.build?.exists,
  'conversation-agent': () => agentsAvailable().length > 0 && !!workspaceId(),
  'can-merge': () => {
    const ws = workspaceDetail()?.workspace;
    return !!ws && !isQuickChats(ws) && !isMainAgent(ws) && canMergeFrom(ws) && canLeaveConversation();
  },
  'can-go-back': () => canNavBack(),
  'can-go-forward': () => canNavForward(),
  'recent-saved-canvas': () => !!lastCanvas()?.ref?.workspaceId,
  'closed-view': () => canReopenClosedView(),
  'side-chat': () => sideChatOpen() || (activePage() === 'today' && !!workspaceDetail() && !isMainAgent(workspaceDetail()!.workspace)),
};

/** Whether `def` can run now. */
export function actionAvailable(def: ActionDef): boolean {
  return !def.when || CONDITION_HOLDS[def.when]();
}

// Through openSettingsFromOutside (overnight 11-13 S3): a control that moved to another page opens there, with no Settings step in Back.
const settings = (section: string) => () => { openSettingsFromOutside(section); };

/** What each action does. Every `ACTION_DEFS` id has one (tested). */
export const ACTION_RUN: Readonly<Record<string, () => unknown>> = {
  // Create
  'new-conversation': () => startNewConversation(),
  'new-quick-chat': () => startQuickChat(),
  // Your work's form, leaving any open workspace (the sidebar's Projects +).
  'new-workspace': () => { goToPage('today'); setNewWorkspaceRequest(v => v + 1); },
  'jump': () => openCommandPalette('jump'),
  // ⌘⇧M and View → Main Agent: the drawer they toggled is gone (run 15 P3);
  // the id stays so a custom binding keeps working.
  'main-drawer': () => openMainAgent(),
  // The system-wide key and tray item (lib.rs show_main_with) always land
  // on it; the key never sends you away.
  'main-agent': () => openMainAgent(),
  // Places. Today, Incoming, Commitments and Weekly review were cut (overnight 11-08 S3).
  // Push-to-talk (overnight 10-11 B7): types into the focused field; loaded on first use.
  'dictate': () => import('./store/dictation').then(m => m.dictateFromShortcut()),
  // Your work's Ask field, focused (One Ask, 2026-11-03; the id is Ask your vault's).
  'ask-vault': () => openHomeAsk(),
  'activity': () => openMissionControl(),
  'pull-requests': () => openPullRequests(),
  'canvases': () => openCanvasGallery(),
  'library-all': () => openLibraryTab('all'),
  'library-notes': () => openLibraryTab('notes'),
  'library-results': () => openLibraryTab('results'),
  'library-recordings': () => openLibraryTab('recordings'),
  'library-skills': () => openLibraryTab('skills'),
  'library-history': () => openLibraryTab('history'),
  'conflict-copies': () => openConflictCopies(),
  'save-canvas-image': () => { const c = lastCanvas(); if (c?.ref?.workspaceId) void import('./store/canvasExport').then(m => m.openCanvasExport(c)); },
  'new-note': () => { setActivePage('vault'); requestNewNote(); },
  'add': () => openAdd(),
  'new-task': () => openModal(),
  // Quick capture (⌘⌥⇧T, the tray): the Add sheet set to a task (S5); the
  // id stays so a custom binding keeps working.
  'quick-capture': () => openQuickCapture(),
  // Import a source (palette, ⌘⌥⇧I, the tray): the Add sheet set to a
  // Library source (S5); the id stays so a custom binding keeps working.
  'quick-ingest': () => openImportSource(),
  'connectors': settings('connectors'),
  'appearance': settings('interface'),
  'toggle-sidebar': () => toggleSidebar(),
  'shortcuts': () => toggleShortcutsOverlay(),
  'save-diagnostics': () => import('./store/diagnostics').then(m => m.saveDiagnosticsBundle()),
  'export-settings': () => import('./store/settingsTransfer').then(m => m.exportSettings()),
  'import-settings': () => import('./store/settingsTransfer').then(m => m.importSettings()),
  'clear-debug-logs': async () => {
    try {
      const n = await api.clearDebugLogs();
      flashFooter(n > 0 ? `Cleared ${n} log file${n === 1 ? '' : 's'}` : 'No debug logs to clear', 2200);
    } catch (e) {
      flashFooter(`× ${(e as Error)?.message || e}`, 3500);
    }
  },
  // This conversation and workspace
  'stop-run': () => stopChatRun(),
  'undo-chat-action': () => undoLastOrganization(),
  'switch-model': async () => {
    const onConversation = activePage() === 'today' && !!workspaceDetail();
    if (!onConversation && !(await focusWorkspaceConversation())) return;
    setModelMenuRequested(true);
  },
  // A build workspace's Changes and a project's notes open in the Preview
  // pane (COO rebuild L4: they were workspace panel sections).
  'open-changes': () => { const id = workspaceDetail()?.workspace.build?.exists ? workspaceDetail()!.workspace.id : null; if (id) openPreview({ kind: 'changes', workspaceId: id }); },
  'open-project-files': () => { const w = workspaceDetail()?.workspace; if (w && !isQuickChats(w)) void openWorkspaceFile(`${w.path}/WORKSPACE.md`); },
  'toggle-side-pane': () => toggleSidePane(),
  'reopen-closed-view': () => { reopenClosedView(); },
  'toggle-side-chat': () => toggleSideChatChecked(),
  'merge-workspace': () => { const ws = workspaceDetail()?.workspace; if (ws) queueMicrotask(() => chooseMergeTarget(ws)); },
  'prev-conversation': () => stepConversation(-1),
  'next-conversation': () => stepConversation(1),
  // Moving a conversation needs which one: the palette's move list runs it.
  'new-workspace-from-chat': () => openCommandPalette(),
  // Navigation
  'go-back': () => navBack(),
  'go-forward': () => navForward(),
  'go-work': () => goToPage('today'),
  'go-tasks': () => goToPage('tasks'),
  'go-library': () => goToPage('vault'),
  'go-automations': () => goToPage('workflows'),
  'review': () => openWaitingOnYou(null),
  'history': () => goToPage('chat'),
  'usage': () => { openSettingsFromOutside('usage'); },
  'settings': () => goToPage('settings'),
  'workspace-conversation': () => focusWorkspaceConversation(),
  'search': () => openCommandPalette(),
  'search-alt': () => openCommandPalette(),
  'search-chats': () => requestSearchChats(document.activeElement instanceof HTMLElement && document.activeElement !== document.body ? document.activeElement : null),
  'diagnostics': () => { openSettingsFromOutside('system'); },
  'copy-build': () => copyBuildIdentity(),
};

export async function copyBuildIdentity(): Promise<string> {
  const id = await api.buildIdentity();
  try { await navigator.clipboard.writeText(id.text); flashFooter('Build identity copied', 1800); }
  catch { flashFooter(id.text, 6000); }
  return id.text;
}

/** A native menu item's or ⌘ command's id (appMenu.ts), run as its shared
 *  action. False when no action has that id. */
export function runMenuOrKey(id: string): boolean {
  const def = ACTION_BY_MENU[id] ?? ACTION_BY_KEY[id];
  return def ? runAction(def.id) : false;
}

/** A go-to target (goTo.ts), run as its shared action. */
export function runGo(id: string): boolean {
  const def = ACTION_BY_GO[id];
  return def ? runAction(def.id) : false;
}

/** Run action `id`. False when it doesn't exist or can't run now (and then
 *  the footer says why). */
export function runAction(id: string): boolean {
  const def = ACTION_BY_ID[id];
  const run = ACTION_RUN[id];
  if (!def || !run) return false;
  if (!actionAvailable(def)) {
    flashFooter(CONDITIONS[def.when!].unavailable, 3500);
    return false;
  }
  Promise.resolve(run()).catch(err => flashFooter(`× ${(err as Error)?.message || err}`, 3500));
  return true;
}
