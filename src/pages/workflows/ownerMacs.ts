// The Macs the owner check knows (M4 follow-up), read lazily by the
// automations page: this Mac and its paired Macs from the same store the
// Macs panel uses (`store/peerSessions.ts`, `peer_machines`), plus the Macs
// paired with this one as a server (`phone_access_status`'s `machines`), so a
// mini that was paired from the MacBook still counts the MacBook.
import { createSignal } from 'solid-js';
import { invoke } from '../../api';
import { onNative } from '../../nativeEvents';
import { loadPeerMachines, peerMachines } from '../../store/peerSessions';
import type { PhoneAccessStatus, PhoneMachine } from '../../types';
import { knownMacs, type KnownMacs } from './runsOnChoice';

const [server, setServer] = createSignal<PhoneMachine[] | null>(null);
let reading: Promise<void> | null = null;

const readServer = () => invoke<PhoneAccessStatus>('phone_access_status').then(s => setServer(s.machines ?? []), () => setServer([]));

/** Read both lists (the automations page, when it opens). One read at a
 *  time; the store keeps the client side current after that. */
export function readOwnerMacs(): Promise<void> {
  reading ??= Promise.all([
    loadPeerMachines(false),
    readServer(),
  ]).then(() => undefined).finally(() => { reading = null; });
  return reading;
}

/** Read the server side again when a Mac pairs with this one or is revoked
 *  (as the Macs panel does); returns the unsubscribe. */
export function followOwnerMacs(): () => void {
  return onNative('phone-access-changed', () => void readServer());
}

/** This Mac and its paired Macs; null until `peer_machines` answered. */
export function ownerMacs(): KnownMacs | null {
  const view = peerMachines();
  return view ? knownMacs(view, server()) : null;
}
