// Context sync clash cards (CX6c): the pure words of one card. Two Macs
// changed the same lines of one file; this Mac kept its version at the
// file's name and the other Mac's sits beside it as `Name (from <machine>)`.
// Rendered by `components/SyncClashesDialog.tsx`; unit-tested in
// syncClashes.test.ts.
import type { ClashSide, SyncClash } from './generated/ipc';
import type { ClashChoice } from './syncClashesApi';

/** A root's name on a card. */
export function rootLabel(root: string): string {
  return root === 'personal' ? 'Personal' : root === 'vault' ? 'Vault' : root;
}

/** One card's key: its root, file and copy together, as the backend finds
 *  the record (two clashes of one file have different copies). */
export function clashKey(c: Pick<SyncClash, 'root' | 'path' | 'copy'>): string {
  return `${c.root}:${c.path}:${c.copy}`;
}

/** The card's heading: the file's own name. */
export function clashTitle(c: Pick<SyncClash, 'path'>): string {
  return c.path.split('/').pop() || c.path;
}

/** "1 clash" or "N clashes"; empty for none. */
export function clashCountText(n: number): string {
  return n <= 0 ? '' : n === 1 ? '1 clash' : `${n} clashes`;
}

/** Open cards of one root. */
export function clashesOf(list: readonly SyncClash[] | undefined, root: string): number {
  return (list ?? []).filter(c => c.root === root).length;
}

/** The short summary under the heading: what differs, in lines. */
export function clashSummary(c: Pick<SyncClash, 'onlyMine' | 'onlyTheirs' | 'mine' | 'theirs'>): string {
  if (!c.mine || !c.theirs) return '';
  if (c.mine.unreadable || c.theirs.unreadable) return 'One version can’t be shown here; open the file to compare.';
  const lines = (n: number) => (n === 1 ? '1 line' : `${n} lines`);
  if (c.onlyMine === 0 && c.onlyTheirs === 0) return 'Same lines in both, in a different order or spacing.';
  return `${lines(c.onlyMine)} only in this Mac’s version, ${lines(c.onlyTheirs)} only in the other’s.`;
}

/** One side's caption: whose, and how much of it shows. */
export function sideCaption(who: string, side: ClashSide | null): string {
  if (!side) return who;
  if (side.unreadable) return `${who}: ${side.unreadable}`;
  const more = side.totalLines - side.lines.length;
  return more > 0 ? `${who}: first ${side.lines.length} of ${side.totalLines} lines` : `${who}: ${side.totalLines === 1 ? '1 line' : `${side.totalLines} lines`}`;
}

/** Why a choice can't be pressed now, or null when it can. */
export function choiceBlocked(c: Pick<SyncClash, 'replaceable' | 'changed'>, choice: ClashChoice): string | null {
  if ((choice === 'keep_mine' || choice === 'keep_theirs') && !c.replaceable) {
    return 'A folder now holds this name, so one version can’t replace the other: keep both, or edit by hand.';
  }
  if (choice === 'keep_theirs' && c.changed) {
    return 'The file changed since the clash, so Keep theirs would refuse: look at it again, or keep both.';
  }
  return null;
}

/** The confirm before a choice that moves a file; null for one that doesn't. */
export function choiceConfirm(c: Pick<SyncClash, 'path' | 'copy' | 'theirsMachine'>, choice: ClashChoice): { title: string; body: string; confirmLabel: string } | null {
  if (choice === 'keep_mine') {
    return {
      title: 'Keep this Mac’s version?',
      body: `“${c.copy}” (${c.theirsMachine}’s version) moves to the vault’s trash; “${c.path}” stays as it is.`,
      confirmLabel: 'Keep mine',
    };
  }
  if (choice === 'keep_theirs') {
    return {
      title: `Keep ${c.theirsMachine}’s version?`,
      body: `“${c.path}” moves to the vault’s trash and “${c.copy}” takes its name. The other Mac gets the change at its next sync.`,
      confirmLabel: 'Keep theirs',
    };
  }
  return null;
}
