// M5 offline UI: Continue here offline on a chat that moved to another Mac
// (`chat_continue_offline`, user-only), and this Mac's offline records
// (`chat_offline_rows`) behind the notices and the offline badge
// (`docs/reference/peers.md`, "Offline sync"). Lazy: the chat's notice and
// chatMoves.ts load it. The words are chatOfflineWords.ts.
import { invoke } from '../api';
import { flashFooter, flashWithUndo, showConfirm } from '../store';
import type { OfflineRow } from '../generated/ipc';
import { errMsg } from '../utils';
import { onNative } from '../nativeEvents';
import { setOfflineRows } from './offlineFlags';
import { continueConfirm } from './chatOfflineWords';
import { pickConversation, refreshWorkspaceSessions, workspaceId } from './workspaces';

/** Reads started; only the newest one's answer is applied. */
let reads = 0;

// The backend says when a sync folded in or a copy was kept here: the homes
// are read again, and with them the records (`loadChatHomes` reads those
// once a copy is known; read here only when that can't load).
onNative('chat-offline-changed', () => {
  void import('./chatMoves').then(m => m.loadChatHomes(), () => loadOfflineRows());
}, () => {});

/** Read this Mac's offline records again. */
export async function loadOfflineRows(): Promise<readonly OfflineRow[]> {
  const mine = ++reads;
  try {
    const rows = await invoke<OfflineRow[]>('chat_offline_rows');
    if (mine === reads) setOfflineRows(rows);
    return rows;
  } catch {
    return [];
  }
}

/** Open chat `key` of this Mac's in the open project (an offline copy and
 *  the chat it continues share their folder). */
export async function openHere(key: string): Promise<void> {
  const id = workspaceId();
  if (!id) return;
  await refreshWorkspaceSessions();
  if (!await pickConversation(id, key)) flashFooter('× That chat didn’t open. Try again from the sidebar.');
}

/** Continue here offline (the notice on a chat that moved): asks first,
 *  then makes the copy (or finds the open one) and opens it. */
export async function continueOffline(key: string, title: string, name: string | null): Promise<void> {
  if (!await showConfirm({ ...continueConfirm(title, name), confirmLabel: 'Continue here' })) return;
  let copy: string;
  try {
    copy = await invoke<string>('chat_continue_offline', { key });
  } catch (e) {
    flashFooter(`× ${errMsg(e)}`);
    return;
  }
  await loadOfflineRows();
  flashWithUndo({ message: `“${title}” continues here offline. It syncs once ${name ?? 'the other Mac'} answers.`, kind: 'status', ttlMs: 7000 });
  await openHere(copy);
}
