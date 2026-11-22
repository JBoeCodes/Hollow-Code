// M5 offline UI: what the window says about a chat continued offline
// (`docs/reference/peers.md`, "Offline sync"). Pure: no IPC, no signals;
// chatOfflineWords.test.ts holds them. ChatMoveNotice.tsx shows them, with
// chatMoveWords.ts's home notice; RailQuickChats shows the badge.
import type { HomeRow, OfflineRow } from '../generated/ipc';

/** The offline badge's word and accessible name: a chat continued offline
 *  (`forkKind` `offline`), here or on the other Mac. */
export const OFFLINE_BADGE = 'offline copy';
export function offlineBadgeLabel(forked: boolean): string {
  return forked ? 'Offline copy, its own chat next to the original' : 'Offline copy of a chat that moved to another Mac';
}

/** The newest offline record of chat `key`: its own (an offline copy) or,
 *  for a chat that moved, the newest copy continuing it. */
export function offlineOf(rows: readonly OfflineRow[], key: string): { own: OfflineRow | null; copy: OfflineRow | null } {
  const own = rows.find(r => r.offlineKey === key) ?? null;
  // The rows come oldest first.
  const copy = [...rows].reverse().find(r => r.key === key) ?? null;
  return { own, copy };
}

/** What the offline part of a chat's notice shows. `text` replaces the home
 *  notice's sentence when set; the buttons add to its own. */
export interface OfflineNotice {
  kind: 'offline' | 'synced' | 'forked' | 'kept' | 'fork-here' | 'moved';
  text: string | null;
  /** The offline badge (this chat is an offline copy). */
  badge: boolean;
  /** Continue here offline, on a chat that moved (none open yet). */
  canContinue: boolean;
  /** The open offline copy to go back to, on the chat it continues. */
  openCopy: string | null;
  /** The chat this copy continues, to open here (its copy on this Mac). */
  openOriginal: string | null;
}

/** The offline notice for chat `key`, or null for none. `home` is its home
 *  record, `name` the other Mac's name (null when not paired), and
 *  `fork` its own `forkKind` and `parentKey` (a fork made on this Mac by
 *  another Mac's offline sync). */
export function offlineNotice(
  key: string,
  home: HomeRow | undefined,
  rows: readonly OfflineRow[],
  name: string | null,
  fork?: { forkKind: string | null; parentKey: string | null },
): OfflineNotice | null {
  const { own, copy } = offlineOf(rows, key);
  const mac = name ?? 'the other Mac';
  const none = { badge: false, canContinue: false, openCopy: null, openOriginal: null };
  if (own) {
    const back = { ...none, badge: true, openOriginal: own.key };
    switch (own.state) {
      case 'open':
        // While it is sent, the home notice (moving, waiting) says so.
        if (home) return { ...back, kind: 'offline', text: null, openOriginal: null };
        // A long stretch syncs in parts: some are there already.
        return own.partly
          ? { ...back, kind: 'offline', text: `Part of its turns synced to ${mac} already. The rest follows when ${mac} answers next, and it runs on this Mac until then.` }
          : { ...back, kind: 'offline', text: `Continued here while ${mac} was out of reach. It syncs to ${mac} once ${mac} answers, and runs on this Mac until then.` };
      case 'forked':
        return { ...back, kind: 'forked', openOriginal: null, text: `${mac} went on with the original while you were offline, so these turns became their own chat there, next to it.` };
      case 'kept':
        return { ...back, kind: 'kept', text: `Kept on this Mac: ${own.note ?? `${mac} didn’t take its turns.`} It is an ordinary chat here now.` };
      default:
        return null;
    }
  }
  if (home?.state === 'arrived' && fork?.forkKind === 'offline') {
    return { ...none, badge: true, kind: 'fork-here', openOriginal: fork.parentKey, text: `Continued offline on ${name ?? 'another Mac'}. The original went on here meanwhile, so these turns are their own chat.` };
  }
  if (home?.state !== 'away') return null;
  if (copy?.state === 'open') {
    return { ...none, kind: 'moved', text: null, openCopy: copy.offlineKey };
  }
  const moved = { ...none, kind: 'moved' as const, canContinue: true };
  switch (copy?.state) {
    case 'appended':
      return { ...moved, kind: 'synced', text: `The turns you added offline synced into it on ${mac}. It keeps going there.` };
    case 'forked':
      return { ...moved, kind: 'forked', text: `${mac} had gone on with it, so the turns you added offline became their own chat there.` };
    case 'kept':
      return { ...moved, kind: 'kept', text: `Your offline copy stayed on this Mac: ${copy.note ?? `${mac} didn’t take its turns.`}` };
    default:
      return { ...moved, text: null };
  }
}

/** Continue here offline's confirm: what it makes, and when it syncs. */
export function continueConfirm(title: string, name: string | null): { title: string; body: string } {
  const mac = name ?? 'the other Mac';
  return {
    title: `Continue “${title}” here offline?`,
    body: `This starts a copy of it on this Mac with the turns this Mac has, for while ${mac} is out of reach. When ${mac} answers again, the copy’s new turns sync there: into the chat if ${mac} didn’t go on with it, else as a chat of their own next to it.`,
  };
}
