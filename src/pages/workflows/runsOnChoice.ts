// Owner Mac (M4 follow-up): where an automation runs, for the editor's Runs on
// choice and the list row's note. The rule mirrors Rust `runs_on::decide` and
// `Gate::new` (docs/reference/automation.md, Owner Mac): this Mac, a paired
// Mac (either side of M2's link: `peer_machines` and the iPhone app server's
// `machines`), or nowhere. Pure, so the node vitest suite loads it and the
// App chunk pays nothing.
import type { PeerMachinesView } from '../../generated/ipc';
import type { PhoneMachine } from '../../types';

/** A Mac as the owner check knows it. */
export interface OwnerMac {
  machineId: string | null;
  slug: string | null;
  name: string;
}

/** This Mac and the Macs paired with it. `thisMac` null: not read yet. */
export interface KnownMacs {
  thisMac: OwnerMac | null;
  peers: OwnerMac[];
}

/** The Runs on value for no owner. */
export const NO_OWNER = '';
/** The Runs on value for this Mac (`save_automation`'s `this`). */
export const THIS_MAC = 'this';

/** `m-` and 16 hex digits (Rust `phone_access::valid_machine_id`). */
export function isMachineId(value: string): boolean {
  return /^m-[0-9a-f]{16}$/.test(value);
}

const owns = (mac: OwnerMac, owner: string) => mac.machineId === owner || mac.slug === owner;

/** Both sides of M2's pairing, one entry per Mac (the client side's record
 *  wins: it carries the slug). */
export function knownMacs(view: PeerMachinesView | null, server: readonly PhoneMachine[] | null | undefined): KnownMacs {
  const peers: OwnerMac[] = (view?.machines ?? []).map(m => ({ machineId: m.machineId, slug: m.slug, name: m.name }));
  for (const m of server ?? []) {
    if (m.machineId && peers.some(p => p.machineId === m.machineId)) continue;
    peers.push({ machineId: m.machineId, slug: null, name: m.name });
  }
  const t = view?.thisMac;
  return { thisMac: t ? { machineId: t.machineId, slug: t.slug, name: t.name } : null, peers };
}

const ownerOf = (runsOn: string | null | undefined) => (runsOn ?? '').trim();

function isThisMac(macs: KnownMacs, owner: string): boolean {
  return !!owner && !!macs.thisMac && owns(macs.thisMac, owner);
}

function peerOf(macs: KnownMacs, owner: string): OwnerMac | null {
  return owner ? macs.peers.find(p => owns(p, owner)) ?? null : null;
}

/** Whether another Mac shares the vault (Rust `Gate::new`): a paired Mac, or
 *  an automation owned by another Mac's id (while this Mac is known) or by a
 *  paired Mac's slug. A typo or unknown slug is not evidence. */
export function vaultShared(macs: KnownMacs, runsOns: readonly (string | null | undefined)[]): boolean {
  if (macs.peers.length > 0) return true;
  return runsOns.some(raw => {
    const owner = ownerOf(raw);
    return !!owner && !isThisMac(macs, owner) && ((!!macs.thisMac && isMachineId(owner)) || !!peerOf(macs, owner));
  });
}

export type Placement =
  | { kind: 'here' }
  | { kind: 'elsewhere'; mac: string }
  | { kind: 'nowhere'; why: 'no-owner' | 'unknown'; owner: string };

/** Where an automation fires (Rust `runs_on::decide`). */
export function placement(runsOn: string | null | undefined, macs: KnownMacs, shared: boolean): Placement {
  const owner = ownerOf(runsOn);
  if (!owner) return shared ? { kind: 'nowhere', why: 'no-owner', owner } : { kind: 'here' };
  if (isThisMac(macs, owner)) return { kind: 'here' };
  const peer = peerOf(macs, owner);
  if (peer) return { kind: 'elsewhere', mac: peer.name };
  if (isMachineId(owner)) return { kind: 'elsewhere', mac: `another Mac (${owner})` };
  return { kind: 'nowhere', why: 'unknown', owner };
}

/** The list row's note, or null when the automation runs here (or the Macs
 *  aren't read yet). `warn` marks one that runs on no Mac. */
export function runsOnRowNote(runsOn: string | null | undefined, macs: KnownMacs | null, shared: boolean): { text: string; warn: boolean } | null {
  if (!macs?.thisMac) return null;
  const p = placement(runsOn, macs, shared);
  if (p.kind === 'here') return null;
  if (p.kind === 'elsewhere') return { text: `Runs on ${p.mac}`, warn: false };
  return p.why === 'no-owner'
    ? { text: 'Runs on no Mac: it has no owner and another Mac also has this automation. Open Edit and set Runs on.', warn: true }
    : { text: `Runs on no Mac: “${p.owner}” isn’t this Mac or a paired one. Open Edit and set Runs on.`, warn: true };
}

export interface RunsOnOption { value: string; label: string }

/** The select's value for a saved owner: no owner, this Mac, or the saved
 *  text itself (a paired Mac, or one this Mac doesn't know). */
export function runsOnValue(runsOn: string | null | undefined, macs: KnownMacs | null): string {
  const owner = ownerOf(runsOn);
  if (!owner) return NO_OWNER;
  return macs && isThisMac(macs, owner) ? THIS_MAC : owner;
}

/** The choices: no owner, this Mac, each paired Mac, and the saved owner when
 *  it is none of those (so the select still shows it). A paired Mac's value is
 *  its machine id (what Hollow writes), or the saved text when the saved owner
 *  names it by slug, so an unchanged choice saves nothing. */
export function runsOnOptions(saved: string | null | undefined, macs: KnownMacs | null): RunsOnOption[] {
  const owner = ownerOf(saved);
  const options: RunsOnOption[] = [
    { value: NO_OWNER, label: 'Any Mac that holds it (until you pair another Mac)' },
    { value: THIS_MAC, label: macs?.thisMac ? `This Mac (${macs.thisMac.name})` : 'This Mac' },
  ];
  let savedListed = !owner || (!!macs && isThisMac(macs, owner));
  for (const p of macs?.peers ?? []) {
    const value = owner && owns(p, owner) ? owner : p.machineId ?? p.slug;
    if (!value) continue;
    if (value === owner) savedListed = true;
    options.push({ value, label: p.name });
  }
  if (!savedListed) {
    options.push({ value: owner, label: isMachineId(owner) ? `Another Mac (${owner})` : `Unknown Mac “${owner}” (runs nowhere)` });
  }
  return options;
}

/** The owner a choice stands for, as the stamp compares it: the saved text
 *  when the choice is unchanged, else what the save will write. */
export function runsOnStamp(value: string, saved: string | null | undefined, macs: KnownMacs | null): string | null {
  if (value === runsOnValue(saved, macs)) return ownerOf(saved) || null;
  if (value === NO_OWNER) return null;
  if (value === THIS_MAC) return macs?.thisMac ? macs.thisMac.machineId ?? macs.thisMac.slug : THIS_MAC;
  return value;
}

/** What a save sends: the choice when it changed (`''` removes the owner,
 *  `this` is this Mac), else nothing (keep it). */
export function runsOnForSave(value: string, saved: string | null | undefined, macs: KnownMacs | null): string | undefined {
  return value === runsOnValue(saved, macs) ? undefined : value;
}

/** A Mac's display name for the stamp's change line. */
export function ownerLabel(owner: string | null, macs: KnownMacs | null): string {
  const o = ownerOf(owner);
  if (!o) return 'no owner';
  if (o === THIS_MAC || (macs && isThisMac(macs, o))) return 'this Mac';
  return (macs && peerOf(macs, o)?.name) || o;
}

/** The line under the choice: what it means, and, when another Mac also
 *  has the automation and it has no owner, that it won't run until it has
 *  one. Before the Macs are read (or when they can't be) it warns of
 *  nothing. */
export function runsOnHint(value: string, macs: KnownMacs | null, shared: boolean): { text: string; warn: boolean } {
  if (value === NO_OWNER) {
    return shared
      ? { text: 'Won’t run on any Mac until it has an owner: another Mac also has this automation. Pick This Mac or a paired Mac.', warn: true }
      : { text: 'No owner: it runs on this Mac until another Mac also has this automation. Once you pair another Mac it stops until you pick one.', warn: false };
  }
  if (value === THIS_MAC) {
    return { text: 'Runs only on this Mac. Another Mac that also has it leaves it alone.', warn: false };
  }
  if (!macs?.thisMac) return { text: 'Reading your Macs.', warn: false };
  const p = placement(value, macs, shared);
  if (p.kind === 'nowhere') {
    return { text: `“${value}” isn’t this Mac or a paired one, so it runs nowhere. Pick a Mac to fix it.`, warn: true };
  }
  const name = p.kind === 'elsewhere' ? p.mac : 'that Mac';
  return { text: `Runs only on ${name}: this Mac’s schedule leaves it alone. Run now still runs it here.`, warn: false };
}

/** Said beside the choice: moving it is a permission change. */
export const RUNS_ON_APPROVAL = 'Moving it to a Mac is a permission change: Save asks you to approve it, because each Mac has its own logins.';
