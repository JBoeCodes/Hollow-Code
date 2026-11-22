// M2 PR 3: this Mac's chat rows and other Macs' rows as one list
// (`docs/reference/peers.md`, "Merged chat list"). Pure: no IPC, no signals.
//
// - A row's identity is its home and its key: the same key on two Macs is
//   two rows, each with its badge; nothing is hidden or merged.
// - Newest `lastActivityAt` first, across every Mac.
// - A remote row whose workspace id is known here sits in that project's list
//   (`placement: 'workspace'`); one whose workspace is unknown here, or has
//   none, sits under its Mac (`'machine'`).
// - A Mac that is offline (or connecting, or needs an update) keeps the last
//   rows fetched this session, marked `stale`; after 24 hours without a fresh
//   list they are dropped. A Mac that needs pairing shows no rows.
// - M5: a chat that moved between Macs shows one row, at its new home. The
//   side that says `movedTo` is hidden while the Mac it names lists the same
//   key (a local row says it through this Mac's home record, `chat_homes`
//   `away`); with no such row there (that Mac offline past a day, or
//   forgotten) the old side stays, marked moved. The new home's row carries
//   `movedFrom`, so its badge says moved.
import type { MachineRef, PeerSessionRow, PeerState } from '../generated/ipc';

/** Rows of an unreachable Mac are dropped this long after they were fetched. */
export const STALE_DROP_MS = 24 * 60 * 60 * 1000;

/** What the merge needs from a local chat row. */
export interface LocalChatRow {
  key: string;
  title: string | null;
  lastActivityAt: string;
  /** Its workspace id here, or null (a chat outside one). */
  workspaceId: string | null;
  /** M5: it moved from this Mac to this machine id (`chat_homes` `away`). */
  movedTo?: string | null;
}

/** One Mac's rows as the webview holds them (memory only). */
export interface PeerRowsHeld {
  machine: MachineRef;
  state: PeerState;
  rows: readonly PeerSessionRow[];
  /** When `rows` were fetched (ms since the epoch); null: never. */
  fetchedAt: number | null;
}

export interface MergedChatRow<L extends LocalChatRow = LocalChatRow> {
  /** `<home machineId>:<key>`; unique across Macs. */
  id: string;
  key: string;
  /** null: this Mac. */
  home: MachineRef | null;
  title: string | null;
  lastActivityAt: string;
  workspaceId: string | null;
  placement: 'workspace' | 'machine';
  /** From a Mac that isn't reachable now: shown dimmed, read-only. */
  stale: boolean;
  needsAnswer: boolean;
  /** The remote row's `idle` / `queued` / `running`; null for a local row. */
  state: string | null;
  local: L | null;
  remote: PeerSessionRow | null;
  /** M5: it lives on this machine id now (shown only when that Mac doesn't
   *  list it). */
  movedTo: string | null;
  /** M5: it moved here from this machine id (the moved badge). */
  movedFrom: string | null;
}

const THIS_MAC = 'this-mac';

function time(at: string): number {
  const t = Date.parse(at);
  return Number.isFinite(t) ? t : 0;
}

/** The rows a Mac shows now, and whether they are stale; null: none. */
export function visibleRows(held: PeerRowsHeld, now: number): { rows: readonly PeerSessionRow[]; stale: boolean } | null {
  if (held.state === 'pairing-needed' || held.state === 'this-mac') return null;
  if (held.fetchedAt == null) return null;
  const stale = held.state !== 'online';
  if (stale && now - held.fetchedAt > STALE_DROP_MS) return null;
  return { rows: held.rows, stale };
}

/** This Mac's rows and every other Mac's, newest first. */
export function mergeChatRows<L extends LocalChatRow>(
  local: readonly L[],
  peers: readonly PeerRowsHeld[],
  opts: { thisMac: MachineRef | null; knownWorkspaces: ReadonlySet<string>; now: number },
): MergedChatRow<L>[] {
  const here = opts.thisMac?.machineId ?? THIS_MAC;
  const out: MergedChatRow<L>[] = local.map(row => ({
    id: `${here}:${row.key}`,
    key: row.key,
    home: null,
    title: row.title,
    lastActivityAt: row.lastActivityAt,
    workspaceId: row.workspaceId,
    placement: 'workspace',
    stale: false,
    needsAnswer: false,
    state: null,
    local: row,
    remote: null,
    movedTo: row.movedTo ?? null,
    movedFrom: null,
  }));
  for (const held of peers) {
    // A record naming this Mac itself never shows twice.
    if (held.machine.machineId === here) continue;
    const shown = visibleRows(held, opts.now);
    if (!shown) continue;
    for (const row of shown.rows) {
      const workspaceId = row.workspace?.id ?? null;
      out.push({
        id: `${held.machine.machineId}:${row.key}`,
        key: row.key,
        home: held.machine,
        title: row.title,
        lastActivityAt: row.lastActivityAt,
        workspaceId,
        placement: workspaceId && opts.knownWorkspaces.has(workspaceId) ? 'workspace' : 'machine',
        stale: shown.stale,
        needsAnswer: row.needsAnswer,
        state: row.state,
        local: null,
        remote: row,
        movedTo: row.movedTo ?? null,
        movedFrom: row.movedFrom ?? null,
      });
    }
  }
  const listed = new Set(out.map(r => `${r.home?.machineId ?? here}:${r.key}`));
  return out.filter(r => !r.movedTo || !listed.has(`${r.movedTo}:${r.key}`)).sort((a, b) => time(b.lastActivityAt) - time(a.lastActivityAt) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

/** A project's chat list: its own rows and remote rows in the same workspace. */
export function rowsForWorkspace<L extends LocalChatRow>(rows: readonly MergedChatRow<L>[], workspaceId: string): MergedChatRow<L>[] {
  return rows.filter(r => r.placement === 'workspace' && r.workspaceId === workspaceId);
}

/** A Mac's rows that have no project here (the strip's popover, Library's "On <Mac>"). */
export function rowsForMachine<L extends LocalChatRow>(rows: readonly MergedChatRow<L>[], machineId: string): MergedChatRow<L>[] {
  return rows.filter(r => r.home?.machineId === machineId && r.placement === 'machine');
}

/** How many of a Mac's chats run now (the strip's count chip). */
export function runningOn(rows: readonly MergedChatRow[], machineId: string): number {
  return rows.filter(r => r.home?.machineId === machineId && !r.stale && r.state === 'running').length;
}
