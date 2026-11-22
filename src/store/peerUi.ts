// M2 PR 4: what the peer UI shares (`docs/reference/peers.md`, "The UI"):
// the strip's request to Settings, Devices, Macs (a Mac that needs pairing
// again opens Pair a Mac with its alias filled; Settings takes it once), and
// one clock for "4 min ago" and the 24-hour drop of stale rows.
import { createSignal } from 'solid-js';

export interface PairRequest { alias: string; at: number }

const [request, setRequest] = createSignal<PairRequest | null>(null);

/** The latest Pair a Mac request the strip made, until Settings takes it. */
export const pairRequest = request;

/** Ask Settings to open Pair a Mac with `alias` filled. */
export function askToPair(alias: string): void {
  setRequest({ alias, at: Date.now() });
}

/** Settings took the request. */
export function takePairRequest(): PairRequest | null {
  const r = request();
  setRequest(null);
  return r;
}

const [libraryAsk, setLibraryAsk] = createSignal<string | null>(null);

/** The Mac whose chats Library, Conversations should list (the strip's Open
 *  all in Library), until Conversations takes it. */
export const libraryMachineRequest = libraryAsk;

/** Ask Library, Conversations to show "On <Mac>" for `machineId`. */
export function askLibraryMachine(machineId: string): void {
  setLibraryAsk(machineId);
}

/** Conversations took the request (or it was dropped). */
export function takeLibraryMachineRequest(): string | null {
  const id = libraryAsk();
  setLibraryAsk(null);
  return id;
}

const [tick, setTick] = createSignal(0);
let ticking = false;

/** Now, in ms: read again every 30 s (and on any other change of the reader). */
export function peerNow(): number {
  if (!ticking && typeof window !== 'undefined') {
    ticking = true;
    window.setInterval(() => setTick(t => t + 1), 30_000);
  }
  tick();
  return Date.now();
}
