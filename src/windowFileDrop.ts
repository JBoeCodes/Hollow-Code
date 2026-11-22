/**
 * A file dragged from Finder or the desktop onto any part of Hollow's
 * window attaches to the chat (2026-11-04, Alex: "I want to be able to
 * drag and drop any file into hollow and have it show in chat and send to
 * the agent"). Until then only the message box took a drop; anywhere else
 * the webview's own default ran (load the file as a page), which the main
 * window's navigation guard blocks, so the drop silently did nothing.
 *
 * WindowFileDrop (components/) listens on the window after every inner
 * target: a target that takes file drops itself (the message box, Skills'
 * import) cancels the event first and keeps it. What is left comes here:
 * - the chat is on screen: the files attach to its message box;
 * - a project's chat is open behind another page or panel: it comes back
 *   (`focusWorkspaceConversation`) and the files attach there;
 * - no chat is open (Home): a new quick chat opens (`startQuickChatForFiles`)
 *   and the files attach once its message box shows.
 * Files go through the composer's own attach path (C5: any file, 20 MB
 * each, 10 and 50 MB a message, folders refused in words), so a drop here
 * and one on the message box can't differ. Nothing is sent until you send.
 */
import { FOLDER_REFUSED, splitDrop } from './composerDrop';
import type { DroppedItem } from './fileDrag';
import { isQuickChats } from './workspaceTypes';
import { activePage, attachChatImage, chatComposerOnScreen, chatConversationVersion, chatCwd, conversationHistoryShown, flashFooter, requestChatFocus, setActivePage } from './store';
import { focusWorkspaceConversation, startQuickChatForFiles, workspaceDetail, workspaceId } from './store/workspaces';

/** What the drop veil says while files are dragged over the window. */
export function dropTargetLabel(): string {
  if (chatComposerOnScreen()) return 'Drop to attach to this chat';
  const open = workspaceId() ? workspaceDetail()?.workspace : null;
  if (open && open.id === workspaceId()) {
    return isQuickChats(open) ? 'Drop to attach to your open chat' : `Drop to attach to the chat in ${open.title}`;
  }
  return 'Drop to attach to a new quick chat';
}

/** Where a drop landed, taken in the drop handler itself, before any chunk
 *  loads (review, 2026-11-04): the conversation and project on screen then,
 *  and whether its message box was showing. */
export interface DropOrigin { version: number; workspace: string | null; onScreen: boolean }

/** A drop read by the window's listener (`readDrop`): split into files and
 *  folders as the message box splits its own drops, then routed. */
export function routeDroppedItems(items: readonly DroppedItem[], origin: DropOrigin): Promise<void> {
  const { files, folders } = splitDrop({ items: items.map(i => ({ kind: 'file', getAsFile: () => i.file, webkitGetAsEntry: () => i.folder === null ? null : { isDirectory: i.folder, name: i.name } })) });
  return routeDroppedFiles(files, folders, origin);
}

/** How long a dropped file waits for its chat's message box to show (a
 *  workspace still opening, or a page asking about unsaved edits). */
const SHOW_WAIT_MS = 10_000;

/** Attach dropped files to the chat, bringing it on screen first (or opening
 *  a new one). Folders are refused in words, as on the message box. The
 *  files go only to the chat the drop meant: nothing changed since the drop,
 *  the page was left (its unsaved-edits question answered), and the chat it
 *  opened is still the one showing; never into a queue another conversation
 *  could empty (review, 2026-11-04). */
export async function routeDroppedFiles(files: readonly { file: File; known: boolean }[], folders: readonly string[], origin: DropOrigin = { version: chatConversationVersion(), workspace: workspaceId(), onScreen: chatComposerOnScreen() }): Promise<void> {
  if (folders.length > 0) flashFooter(FOLDER_REFUSED, 6000);
  if (files.length === 0) return;
  const one = files.length === 1;
  const fail = (why: string) => flashFooter(`× ${one ? 'The file wasn’t' : 'The files weren’t'} attached: ${why}. Drop ${one ? 'it' : 'them'} again.`, 6000);
  if (chatConversationVersion() !== origin.version || workspaceId() !== origin.workspace) { fail('the chat changed first'); return; }
  if (!origin.onScreen || !chatComposerOnScreen()) {
    // Leave the page first: a page with unsaved edits asks, and kept means no drop.
    if (activePage() !== 'today' && !conversationHistoryShown()) {
      await setActivePage('today');
      if (activePage() !== 'today') { fail('you stayed on this page'); return; }
    }
    // A project's chat comes back only when it is the conversation already
    // held (behind another page or panel): reopening another would let a
    // conversation picked meanwhile take the files (review, 2026-11-04).
    if (origin.workspace && workspaceDetail()?.workspace.cwd !== chatCwd()) { fail('its chat isn’t open; open it first'); return; }
    // Each says why itself when it can't (a message still being sent).
    const open = origin.workspace ? await focusWorkspaceConversation() : await startQuickChatForFiles();
    if (!open) { fail('no chat could open'); return; }
    // The held conversation must be unchanged; with no project, the new
    // quick chat is the chat now open.
    if (origin.workspace && chatConversationVersion() !== origin.version) { fail('the chat changed first'); return; }
    const dest = workspaceId(), version = chatConversationVersion();
    const meant = origin.workspace ? dest === origin.workspace : isQuickChats(dest);
    const same = () => meant && workspaceId() === dest && chatConversationVersion() === version;
    if (!await shows(same)) { fail('its chat didn’t show'); return; }
  }
  requestChatFocus();
  for (const f of files) attachChatImage(f.file, false, f.known);
}

/** True once a message box is on screen while `same()` holds; false when
 *  another workspace opens meanwhile or the wait runs out (a page that asked
 *  about unsaved edits and was kept). */
async function shows(same: () => boolean): Promise<boolean> {
  const until = Date.now() + SHOW_WAIT_MS;
  while (Date.now() < until) {
    if (!same()) return false;
    if (chatComposerOnScreen()) return true;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  return false;
}
