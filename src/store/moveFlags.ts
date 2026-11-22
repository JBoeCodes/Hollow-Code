// M5 UI: the two facts the shell's conversation menu reads about chat moves
// (`workspaceMenu.ts`, Move to runner), kept tiny because that menu is in the
// App chunk. The lazy `chatMoves.ts` and `peerSessions.ts` fill them.
import { createSignal } from 'solid-js';
import type { HomeRow } from '../generated/ipc';

/** The paired Mac Move to runner moves to, by name; null hides the item. */
export const [moveRunner, setMoveRunner] = createSignal<string | null>(null);

/** Chats whose Move to runner is in flight from this window (the menu
 *  reads "moving"; a second click does nothing). */
export const [movingKeys, setMovingKeys] = createSignal<ReadonlySet<string>>(new Set());

/** This Mac's chats that are moving or moved (`chat_move_homes`), by key. */
export const [chatHomes, setChatHomes] = createSignal<Readonly<Record<string, HomeRow>>>({});

const SEEN = 'hollow.chatHomes';
/** Whether this Mac's home records are worth reading: a Mac is paired, or
 *  the last read found some (a chat moved to a Mac since forgotten). Keeps
 *  the read off a launch that has neither (the launch IPC budget). */
export function homesWorthReading(paired: boolean): boolean {
  return paired || localStorage.getItem(SEEN) === '1';
}
/** The last read found records (or none). */
export function noteHomesSeen(any: boolean): void {
  if (any) localStorage.setItem(SEEN, '1'); else localStorage.removeItem(SEEN);
}
