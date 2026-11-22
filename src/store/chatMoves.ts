// M5 UI: the person's chat moves (`docs/reference/peers.md`, "Moving a
// chat"): Move to runner from a chat's menu, Keep it here and Release here
// from its notice, and this Mac's home records (`chat_move_homes`) behind
// the notice and the moved badge. Lazy: the menu loads it on first use, and
// the strip, the notice and Settings read it. The words are chatMoveWords.ts.
import { invoke } from '../api';
import { dismissSnackbar, flashFooter, flashWithUndo, showConfirm } from '../store';
import type { ChatMoveResult, HomeRow } from '../generated/ipc';
import { errMsg } from '../utils';
import { chatHomes, movingKeys, noteHomesSeen, setChatHomes, setMovingKeys } from './moveFlags';
import { isPeerFailure, loadPeerMachines, peerMachines, refreshPeerSessions } from './peerSessions';
import { openRemoteChat } from './peerChat';
import { macName, moveConfirm, moveNotice, releaseConfirm } from './chatMoveWords';
import { loadOfflineRows } from './chatOffline';
import { offlineRows } from './offlineFlags';

let poll: number | undefined;
/** Reads started; only the newest one's answer is applied. */
let reads = 0;

/** Read this Mac's home records again. While a chat is still moving it is
 *  read again every 30 s: the backend retries a move every minute, and a
 *  chat that settles on its own should not keep its notice. */
export async function loadChatHomes(): Promise<readonly HomeRow[]> {
  const mine = ++reads;
  let rows: HomeRow[];
  try {
    rows = await invoke<HomeRow[]>('chat_move_homes');
  } catch {
    return Object.values(chatHomes());
  }
  // An older read that answered late never overwrites a newer one.
  if (mine !== reads) return Object.values(chatHomes());
  setChatHomes(Object.fromEntries(rows.map(r => [r.key, r])));
  noteHomesSeen(rows.length > 0);
  // Offline copies continue only a chat that moved away (`away`); once
  // any are known they are read again too (a copy's original may be back,
  // or its sync settled).
  if (rows.some(r => r.state === 'away') || offlineRows().length > 0) void loadOfflineRows();
  window.clearTimeout(poll);
  if (rows.some(r => r.state === 'leaving')) poll = window.setTimeout(() => void loadChatHomes(), 30_000);
  return rows;
}

const failure = (e: unknown) => (isPeerFailure(e) ? e.message : errMsg(e));

/** A move command's answer, said plainly; a move offers Open at its new home. */
function tell(r: ChatMoveResult, machineId: string | null): void {
  const n = moveNotice(r);
  flashWithUndo({
    message: n.text,
    kind: n.failed ? 'error' : 'status',
    ttlMs: n.failed ? 10_000 : 7000,
    ...(n.open && machineId ? { actionLabel: 'Open', undo: () => void openRemoteChat(machineId, r.key) } : {}),
  });
}

/** After any move command: the records, and the other Mac's list (the chat
 *  shows there, or no longer does). */
function after(machineId: string | null): void {
  void loadChatHomes();
  if (machineId) void refreshPeerSessions(machineId);
}

/** Move to runner (a chat's menu): asks first, then moves it to the paired
 *  Mac the backend picks, saying what happened. A running turn stops at its
 *  next safe point first, so this can take half a minute. */
export async function moveToRunner(key: string, title: string): Promise<void> {
  // One move per chat at a time: a second click while the first waits for a
  // safe point would send a second move under a new id.
  if (movingKeys().has(key)) return;
  const view = peerMachines() ?? await loadPeerMachines(false);
  const to = view?.moveTo ?? null;
  const name = to ? macName(view, to) : null;
  if (!to || !name) {
    flashFooter('× No paired Mac takes chats yet. Pair one in Settings → Devices → Macs.');
    return;
  }
  if (movingKeys().has(key)) return;
  setMovingKeys(s => new Set(s).add(key));
  try {
    if (!await showConfirm({ ...moveConfirm(title, name), confirmLabel: 'Move' })) return;
    await sendMove(key, title, to, name);
  } finally {
    setMovingKeys(s => { const next = new Set(s); next.delete(key); return next; });
  }
}

async function sendMove(key: string, title: string, to: string, name: string): Promise<void> {
  const working = flashWithUndo({ message: `Moving “${title}” to ${name}…`, kind: 'status', ttlMs: 0 });
  try {
    tell(await invoke<ChatMoveResult>('chat_move_to_runner', { key, expect: to }), to);
  } catch (e) {
    flashFooter(`× ${failure(e)}`);
  } finally {
    dismissSnackbar(working);
    after(to);
  }
}

/** Keep it here (the notice on a chat that is moving or waiting). */
export async function keepHere(key: string): Promise<void> {
  const peer = chatHomes()[key]?.peer ?? null;
  try {
    tell(await invoke<ChatMoveResult>('chat_move_keep_here', { key }), peer);
  } catch (e) {
    flashFooter(`× ${failure(e)}`);
  } finally {
    after(peer);
  }
}

/** Release here, after a confirm that says what it does. */
export async function releaseHere(key: string, title: string): Promise<void> {
  const peer = chatHomes()[key]?.peer ?? null;
  const name = peer ? macName(peerMachines(), peer) : null;
  if (!await showConfirm({ ...releaseConfirm(name, title), confirmLabel: 'Release here', danger: true })) return;
  try {
    tell(await invoke<ChatMoveResult>('chat_move_release_here', { key }), null);
  } catch (e) {
    flashFooter(`× ${failure(e)}`);
  } finally {
    after(peer);
  }
}
