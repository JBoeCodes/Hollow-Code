// Conflict copies (overnight A10): the pure parts of the person's resolve.
// Keeping the original moves the copy to the vault's .trash/; keeping the
// copy moves the original there and gives the copy its name. Nothing is
// deleted, and a file changed since the diff refuses.
import type { ConflictCopy } from './generated/ipc';

/** The confirm for keeping one side. */
export function keepConfirm(c: Pick<ConflictCopy, 'copy' | 'original'>, keepCopy: boolean): { title: string; body: string; confirmLabel: string } {
  return keepCopy
    ? {
        title: 'Keep the copy?',
        body: `“${c.original}” moves to the vault's trash and “${c.copy}” takes its name. You can move it back out of .trash/ by hand.`,
        confirmLabel: 'Keep the copy',
      }
    : {
        title: 'Keep the original?',
        body: `“${c.copy}” moves to the vault's trash; “${c.original}” stays as it is. You can move it back out of .trash/ by hand.`,
        confirmLabel: 'Keep the original',
      };
}

/** The list row's second line. */
export function copyMeta(c: Pick<ConflictCopy, 'original' | 'identical' | 'copyModified' | 'originalModified'>): string {
  const day = (iso: string) => iso.slice(0, 10);
  const same = c.identical ? 'same contents' : 'differs';
  return `copy of ${c.original} · ${same} · copy ${day(c.copyModified)}, original ${day(c.originalModified)}`;
}
