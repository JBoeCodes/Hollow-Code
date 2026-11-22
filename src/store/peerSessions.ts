// M2 PR 3: the paired Macs and their chat rows, in the webview's memory only
// (`docs/reference/peers.md`). The pure reducers below are what the tests
// hold; the store wraps them around `peer_machines`, `peer_sessions` and the
// `peer-machines-changed` / `peer-chat-event` (kind `list`) events. Nothing
// here is written anywhere: rows are dropped when Hollow quits, and a Mac's
// rows when it is forgotten or needs pairing.
import { createSignal } from 'solid-js';
import { invoke } from '../api';
import { onNative } from '../nativeEvents';
import type { MachineRef, PeerChatEvent, PeerFailure, PeerMachine, PeerMachinesView, PeerSessionsPage, PeerState } from '../generated/ipc';
import type { PeerRowsHeld } from './peerMerge';
import { homesWorthReading, setMoveRunner } from './moveFlags';
import { moveRunnerName } from './chatMoveWords';

export type HeldRows = Readonly<Record<string, PeerRowsHeld>>;

/** Whether `e` is a peer command's error (what `invoke` rejects with). */
export function isPeerFailure(e: unknown): e is PeerFailure {
  return typeof e === 'object' && e !== null && typeof (e as PeerFailure).code === 'string'
    && typeof (e as PeerFailure).message === 'string' && typeof (e as PeerFailure).state === 'string';
}

const refOf = (m: PeerMachine): MachineRef => ({ machineId: m.machineId, slug: m.slug, name: m.name });

/** Macs that turned online with rows from before: their rows are old
 *  until fetched again ([`heldAfterMachines`] keeps them stale till then). */
export function cameOnline(held: HeldRows, view: PeerMachinesView): string[] {
  return view.machines
    .filter(m => m.state === 'online' && held[m.machineId] && held[m.machineId].state !== 'online')
    .map(m => m.machineId);
}

/** The Macs' list changed: states follow it, a Mac no longer paired goes
 *  with its rows, a Mac that needs pairing keeps none. A Mac that turned
 *  online keeps its earlier state on its rows (so they stay stale) until a
 *  fresh list replaces them ([`heldAfterRows`]). */
export function heldAfterMachines(held: HeldRows, view: PeerMachinesView): HeldRows {
  const next: Record<string, PeerRowsHeld> = {};
  for (const m of view.machines) {
    const was = held[m.machineId];
    const cleared = m.state === 'pairing-needed';
    const old = m.state === 'online' && was && was.state !== 'online' && was.fetchedAt != null;
    next[m.machineId] = {
      machine: refOf(m),
      state: old ? was.state : m.state,
      rows: cleared ? [] : was?.rows ?? [],
      fetchedAt: cleared ? null : was?.fetchedAt ?? null,
    };
  }
  return next;
}

/** A fresh list from `page.home` (it answered, so it is online). A page
 *  for a Mac not in the last list of Macs (just forgotten) is dropped. */
export function heldAfterRows(held: HeldRows, page: PeerSessionsPage, now: number): HeldRows {
  if (!held[page.home.machineId]) return held;
  return {
    ...held,
    [page.home.machineId]: { machine: page.home, state: 'online', rows: page.sessions, fetchedAt: now },
  };
}

/** A failed fetch: the Mac's state from the error; its rows stay (stale)
 *  unless it needs pairing. An error the Mac answered (busy, internal) means
 *  it is online: the rows keep their time, the error its text (in
 *  `peerErrors`), and it no longer counts as coming back online. */
export function heldAfterFailure(held: HeldRows, machineId: string, failure: PeerFailure): HeldRows {
  const was = held[machineId];
  if (!was) return held;
  const state: PeerState = failure.state;
  if (state === 'online') return was.state === 'online' ? held : { ...held, [machineId]: { ...was, state } };
  const cleared = state === 'pairing-needed';
  return {
    ...held,
    [machineId]: { ...was, state, rows: cleared ? [] : was.rows, fetchedAt: cleared ? null : was.fetchedAt },
  };
}

// ── The store ────────────────────────────────────────────────────────────

const [machines, setMachines] = createSignal<PeerMachinesView | null>(null);
const [held, setHeld] = createSignal<HeldRows>({});
const [errors, setErrors] = createSignal<Readonly<Record<string, string>>>({});

/** This Mac and the paired Macs with their state (null until read). */
export const peerMachines = machines;
/** Every paired Mac's rows as held now (`peerMerge.mergeChatRows` reads them). */
export const peerRowsHeld = (): PeerRowsHeld[] => Object.values(held());
/** The last fetch error per Mac, a plain sentence. */
export const peerErrors = errors;

/** The paired Macs' ids the homes were last read for (null: never). */
let lastPaired: string | null = null;

function noteMachines(view: PeerMachinesView): void {
  const back = cameOnline(held(), view);
  setMachines(view);
  setHeld(h => heldAfterMachines(h, view));
  // M5 UI: the conversation menu's Move to runner, and the moved chats'
  // records (a Mac paired or forgotten changes what they say).
  setMoveRunner(moveRunnerName(view));
  // Read only when the paired set changed, or a Mac came back online (after
  // a sleep, chats may have moved meanwhile); not on every state change.
  const paired = view.machines.map(m => m.machineId).sort().join(' ');
  if ((paired !== lastPaired || back.length > 0) && homesWorthReading(view.machines.length > 0)) {
    void import('./chatMoves').then(m => m.loadChatHomes(), () => {});
  }
  lastPaired = paired;
  if (readers === 0) return;
  for (const id of back) void refreshPeerSessions(id);
  // Watch every paired Mac's list (again): the hub keeps a running stream,
  // so the call is cheap, and one a pairing couldn't restart starts here.
  for (const m of view.machines) if (m.state !== 'pairing-needed') void inOrder(m.machineId, 'peer_watch');
}

/** Read the Macs (`check`: ask each now, Settings' and the strip's Check). */
export async function loadPeerMachines(check = false): Promise<PeerMachinesView | null> {
  try {
    const view = await invoke<PeerMachinesView>('peer_machines', { check });
    noteMachines(view);
    return view;
  } catch {
    return null;
  }
}

/** Cancel connecting to a Mac (its tunnel stays closed until Retry); the
 *  Macs are read again. Rejects with the command's error. */
export async function cancelPeerConnect(machineId: string): Promise<void> {
  await invoke<PeerMachine>('peer_cancel_connect', { machineId });
  await loadPeerMachines(false);
}

const fetching = new Map<string, Promise<void>>();
const again = new Set<string>();

/** When the Mac was paired, as the last list of Macs says. */
const pairedAt = (machineId: string): string | null =>
  machines()?.machines.find(m => m.machineId === machineId)?.pairedAt ?? null;

async function fetchOnce(machineId: string): Promise<void> {
  const paired = pairedAt(machineId);
  try {
    const page = await invoke<PeerSessionsPage>('peer_sessions', { machineId });
    setHeld(h => heldAfterRows(h, page, Date.now()));
    setErrors(({ [machineId]: _gone, ...rest }) => rest);
  } catch (e) {
    if (!isPeerFailure(e)) return;
    // A fetch that started before the Mac was paired again speaks for the
    // old pairing (its 401 is the old token's): dropped.
    if (pairedAt(machineId) !== paired) return;
    setHeld(h => heldAfterFailure(h, machineId, e));
    setErrors(all => ({ ...all, [machineId]: e.message }));
  }
}

/** Fetch a Mac's chat list; on failure the held rows turn stale (or go, for
 *  pairing needed) and the error is kept for the strip. One fetch per Mac at
 *  a time: a call while one runs gets that one's promise, and the list is
 *  fetched once more after it (a change it was asked for may be newer). */
export function refreshPeerSessions(machineId: string): Promise<void> {
  const running = fetching.get(machineId);
  if (running) {
    again.add(machineId);
    return running;
  }
  const run = (async () => {
    do {
      again.delete(machineId);
      await fetchOnce(machineId);
    } while (again.has(machineId));
  })().finally(() => fetching.delete(machineId));
  fetching.set(machineId, run);
  return run;
}

/** Each Mac's watch and unwatch calls, one after another, so an unwatch
 *  never overtakes the watch it ends (the backend would keep that stream). */
const streamCalls = new Map<string, Promise<unknown>>();
function inOrder(machineId: string, command: 'peer_watch' | 'peer_unwatch'): Promise<unknown> {
  const next = (streamCalls.get(machineId) ?? Promise.resolve())
    .then(() => invoke(command, { machineId, key: null }))
    .catch(() => {});
  streamCalls.set(machineId, next);
  return next;
}

let readers = 0;
/** Bumped at every first reader and every last disposer: a load that
 *  finishes after its readers left starts nothing. */
let epoch = 0;
let stops: (() => void)[] = [];

/** Follow the Macs and their lists while something shows them (the strip,
 *  Library). Returns the disposer; the last one stops the streams. */
export function followPeerSessions(): () => void {
  if (readers++ === 0) {
    stops.push(onNative<PeerMachinesView>('peer-machines-changed', noteMachines, () => {}));
    stops.push(onNative<PeerChatEvent>('peer-chat-event', e => {
      if (e.key === null && e.kind === 'list') setHeld(h => heldAfterRows(h, e.data as PeerSessionsPage, Date.now()));
    }, () => {}));
    const mine = ++epoch;
    // No `check`: the lists' own calls say who answers, and a mount must
    // not skip a tunnel's backoff (Check now in the UI asks with it).
    // Its streams start in `noteMachines`, which a reader is here for.
    void loadPeerMachines(false).then(view => {
      if (mine !== epoch || readers === 0) return;
      for (const m of view?.machines ?? []) {
        if (m.state === 'pairing-needed') continue;
        void refreshPeerSessions(m.machineId);
      }
    });
  }
  let done = false;
  return () => {
    if (done) return;
    done = true;
    if (--readers > 0) return;
    epoch++;
    for (const stop of stops) stop();
    stops = [];
    for (const m of machines()?.machines ?? []) {
      void inOrder(m.machineId, 'peer_unwatch');
    }
  };
}
