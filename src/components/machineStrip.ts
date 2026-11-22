// M2 PR 4: the machine strip's words, the machine badge's words and the
// remote chat menu, as pure functions (`docs/reference/peers.md`, "The UI").
// RailMachines, MachineBadge and RemoteChatView render what these return;
// machineStrip.test.ts holds them.
//
// A Mac's state comes from the backend (`PeerState`): before the first call a
// Mac is `offline` with `checkedAt` null, which reads "Not checked yet", not
// "Offline". This Mac is always first.
import type { MachineRef, PeerMachine, PeerMachinesView, PeerSessionRow, PeerState } from '../generated/ipc';
import { visibleRows, type PeerRowsHeld } from '../store/peerMerge';

/** The status dot: this Mac (accent), answering (green), opening a tunnel
 *  (pulsing), not answering (hollow ring), needs the person (amber). */
export type StripDot = 'this-mac' | 'online' | 'connecting' | 'offline' | 'attention';
/** What a click on the row does: nothing (this Mac), its popover, or Pair a Mac. */
export type StripClick = 'none' | 'popover' | 'pair';

export interface StripRow {
  machineId: string;
  name: string;
  /** The tooltip: the name, the slug and the state. */
  title: string;
  state: PeerState;
  dot: StripDot;
  /** The short state after the name. */
  text: string;
  /** Chats running there now (the count chip; 0 hides it). */
  running: number;
  click: StripClick;
  /** The row's accessible name. */
  label: string;
}

const MINUTE = 60_000;

/** "just now", "4 min ago", "3 h ago", "2 days ago"; null for no time. */
export function agoLabel(at: string | null | undefined, now: number): string | null {
  if (!at) return null;
  const t = Date.parse(at);
  if (!Number.isFinite(t)) return null;
  const ms = Math.max(0, now - t);
  if (ms < MINUTE) return 'just now';
  if (ms < 60 * MINUTE) return `${Math.floor(ms / MINUTE)} min ago`;
  if (ms < 24 * 60 * MINUTE) return `${Math.floor(ms / (60 * MINUTE))} h ago`;
  const days = Math.floor(ms / (24 * 60 * MINUTE));
  return days === 1 ? '1 day ago' : `${days} days ago`;
}

/** The badge's word: the name's last word, lower case ("mini" for "Mac
 *  mini"), else the slug's first word. The plan named the slug, but a slug is
 *  often a hostname nobody reads as the Mac's name. */
export function machineWord(m: MachineRef): string {
  const words = m.name.trim().split(/\s+/).filter(Boolean);
  if (words.length) return words[words.length - 1].toLowerCase();
  const fromSlug = m.slug?.trim().split(/[-_.\s]+/).find(Boolean);
  return (fromSlug ?? 'mac').toLowerCase();
}

/** The badge's accessible name: where the chat lives, and whether it waits. */
export function badgeLabel(m: MachineRef, needsAnswer = false): string {
  return needsAnswer ? `On ${m.name}, waiting for your answer` : `On ${m.name}`;
}

/** A Mac's short state. */
export function stateText(m: PeerMachine, running: number, now: number): string {
  switch (m.state) {
    case 'this-mac': return 'This Mac';
    case 'online': return running > 0 ? `${running} running` : 'Online';
    case 'connecting': return 'Connecting…';
    case 'pairing-needed': return 'Pair again';
    case 'update-needed': return 'Needs an update';
    case 'offline': {
      if (m.checkedAt == null) return 'Not checked yet';
      const ago = agoLabel(m.lastSeenAt, now);
      return ago ? `Offline · ${ago}` : 'Offline';
    }
  }
}

const DOTS: Record<PeerState, StripDot> = {
  'this-mac': 'this-mac', online: 'online', connecting: 'connecting', offline: 'offline',
  'pairing-needed': 'attention', 'update-needed': 'attention',
};

/** The Mac's chats it shows now (fresh or stale), newest first. */
export function machineRows(held: PeerRowsHeld | undefined, now: number): { rows: PeerSessionRow[]; stale: boolean } {
  const shown = held ? visibleRows(held, now) : null;
  if (!shown) return { rows: [], stale: false };
  const time = (r: PeerSessionRow) => { const t = Date.parse(r.lastActivityAt); return Number.isFinite(t) ? t : 0; };
  return { rows: [...shown.rows].sort((a, b) => time(b) - time(a)), stale: shown.stale };
}

/** Chats running on the Mac now (fresh rows only). */
export function runningCount(held: PeerRowsHeld | undefined, now: number): number {
  const shown = machineRows(held, now);
  return shown.stale ? 0 : shown.rows.filter(r => r.state === 'running').length;
}

function row(m: PeerMachine, running: number, now: number): StripRow {
  const text = stateText(m, running, now);
  const named = m.slug && m.slug !== m.name ? `${m.name} (${m.slug})` : m.name;
  return {
    machineId: m.machineId,
    name: m.name,
    title: `${named} · ${text}`,
    state: m.state,
    dot: DOTS[m.state],
    text,
    running: m.state === 'online' || m.state === 'this-mac' ? running : 0,
    click: m.state === 'this-mac' ? 'none' : m.state === 'pairing-needed' ? 'pair' : 'popover',
    label: `${m.name}, ${text}`,
  };
}

/** The strip: this Mac first, then each paired Mac in the order paired.
 *  Empty when no Mac is paired (the strip isn't shown then). */
export function stripRows(
  view: PeerMachinesView | null,
  held: readonly PeerRowsHeld[],
  localRunning: number,
  now: number,
): StripRow[] {
  if (!view || view.machines.length === 0) return [];
  const byId = new Map(held.map(h => [h.machine.machineId, h]));
  const out: StripRow[] = [];
  const here = view.thisMac;
  out.push(row({
    machineId: here?.machineId ?? 'this-mac', slug: here?.slug ?? null, name: here?.name || 'This Mac',
    sshHosts: [], remotePort: 0, pairedAt: '', lastSeenAt: null, lastVersion: null, state: 'this-mac', message: null, checkedAt: null, alias: null, moves: false,
  }, localRunning, now));
  for (const m of view.machines) {
    if (here && m.machineId === here.machineId) continue;
    out.push(row(m, runningCount(byId.get(m.machineId), now), now));
  }
  return out;
}

/** The popover's lines under a Mac's chats: what it says and offers. */
export interface PopoverInfo {
  heading: string;
  /** The state in a sentence (offline's names when it was last seen). */
  status: string;
  /** ssh's or the Mac's own sentence, when there is one. */
  detail: string | null;
  /** Rows shown dimmed and read-only. */
  stale: boolean;
  /** Offer Retry (check it now). */
  retry: boolean;
  /** Offer Cancel (its tunnel is opening now). */
  cancel?: boolean;
}

export function popoverInfo(m: PeerMachine, now: number, error: string | null): PopoverInfo {
  const seen = agoLabel(m.lastSeenAt, now);
  const detail = m.message || error || null;
  switch (m.state) {
    case 'online':
      return { heading: m.name, status: seen ? `Online, last seen ${seen}` : 'Online', detail: error, stale: false, retry: false };
    case 'connecting':
      return { heading: m.name, status: `Connecting to ${m.name}…`, detail, stale: true, retry: false, cancel: true };
    case 'update-needed':
      return { heading: m.name, status: m.lastVersion ? `Needs an update. ${m.name} runs Hollow ${m.lastVersion}.` : 'Needs an update: this Mac and it speak different versions of the link.', detail, stale: true, retry: true };
    case 'pairing-needed':
      return { heading: m.name, status: `${m.name} needs pairing again.`, detail, stale: true, retry: false };
    default:
      if (m.checkedAt == null) return { heading: m.name, status: `Not checked yet. Retry asks ${m.name} now.`, detail, stale: true, retry: true };
      return { heading: m.name, status: seen ? `${m.name} is offline, last seen ${seen}.` : `${m.name} is offline.`, detail, stale: true, retry: true };
  }
}

/** The right-click menu of a remote chat row: Open, Stop while it runs, Copy
 *  conversation ID; what only its home can do is listed, disabled. */
export interface RemoteMenuEntry { label: string; enabled: boolean; title?: string; kind: 'open' | 'stop' | 'copy' | 'home-only' }
export function remoteMenu(row: Pick<PeerSessionRow, 'state'>, home: MachineRef, stale: boolean): RemoteMenuEntry[] {
  const there = `Do this on ${home.name}`;
  const out: RemoteMenuEntry[] = [{ label: 'Open', enabled: true, kind: 'open' }];
  if (row.state === 'running' || row.state === 'queued') out.push({ label: 'Stop', enabled: !stale, kind: 'stop', ...(stale ? { title: `Can't reach ${home.name}` } : {}) });
  out.push({ label: 'Copy conversation ID', enabled: true, kind: 'copy' });
  for (const label of ['Rename…', 'Move to project…', 'Archive']) out.push({ label, enabled: false, title: there, kind: 'home-only' });
  return out;
}

/** The open remote chat's Mac is no longer paired (forgotten, here or
 *  elsewhere): the chat must close and its stream stop. A list of Macs not
 *  read yet says nothing. */
export function chatHomeGone(view: PeerMachinesView | null, open: { machineId: string } | null): boolean {
  if (!view || !open) return false;
  return !view.machines.some(m => m.machineId === open.machineId);
}

/** Library, Conversations' "On <Mac>" choices: each paired Mac by its own
 *  name from the registry (never this Mac, never a fixed "Mac mini"). */
export function libraryMacs(view: PeerMachinesView | null): { machineId: string; label: string }[] {
  const here = view?.thisMac?.machineId;
  return (view?.machines ?? []).filter(m => m.machineId !== here).map(m => ({ machineId: m.machineId, label: `On ${m.name}` }));
}

/** A Mac's chats as Library, Conversations lists them: newest first, only
 *  workspace `workspaceId`'s when the list is one project's (null: all), and
 *  matching `query` in the title or preview. */
export function libraryRows(
  held: PeerRowsHeld | undefined,
  now: number,
  opts: { workspaceId: string | null; query: string },
): { rows: PeerSessionRow[]; stale: boolean } {
  const shown = machineRows(held, now);
  const q = opts.query.trim().toLowerCase();
  const rows = shown.rows.filter(r => (!opts.workspaceId || r.workspace?.id === opts.workspaceId)
    && (!q || [r.title, r.preview].some(v => v?.toLowerCase().includes(q))));
  return { rows, stale: shown.stale };
}
