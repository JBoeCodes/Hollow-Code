// Browser-mode fixtures for the context sync clash cards (CX6c): one clash
// on the personal root, in memory per page load, only while the page was
// seeded with `localStorage.mockSyncClash` (the route ledger's seed); a
// choice removes it as the real resolve does, and Edit marks it editing.
import type { ClashResolved, SyncClash } from './generated/ipc';

let clashes: SyncClash[] | null = null;

function seeded(): SyncClash[] {
  if (clashes) return clashes;
  clashes = typeof localStorage !== 'undefined' && localStorage.getItem('mockSyncClash') ? [{
    root: 'personal', path: 'me/Identity.md', copy: 'me/Identity (from studio-mac).md', mineMachine: 'this-mac', theirsMachine: 'studio-mac',
    at: '2026-11-15T09:00:00Z', replaceable: true, editing: false, changed: false,
    mine: { lines: ['# Identity', 'Role: product lead'], totalLines: 2, unreadable: null },
    theirs: { lines: ['# Identity', 'Role: head of product'], totalLines: 2, unreadable: null },
    onlyMine: 1, onlyTheirs: 1,
  }] : [];
  return clashes;
}

export function syncClashesFixture(command: string, args: Record<string, unknown> | undefined): { value: unknown } | null {
  switch (command) {
    case 'sync_clashes':
      return { value: seeded() };
    case 'resolve_sync_clash': {
      const list = seeded();
      const i = list.findIndex(c => c.root === args?.root && c.path === args?.path && c.copy === args?.copy);
      if (i < 0) throw new Error(`${String(args?.path)} has no open clash.`);
      const c = list[i];
      if (args?.choice === 'edit') {
        list[i] = { ...c, editing: true };
        return { value: { path: c.path, text: `Editing ${c.path}.`, open: true } satisfies ClashResolved };
      }
      list.splice(i, 1);
      return { value: { path: c.path, text: `Resolved ${c.path}.`, open: false } satisfies ClashResolved };
    }
  }
  return null;
}
