import { createEffect } from 'solid-js';
import { lifecycleFailed } from '../store/workspaces';
import { bulkRunning, clearSelection, pruneSelection, selectAll, selectedKeys, setBulkRunning, stopSelecting } from '../store/rowSelection';

/** The bar over the Quick chats list while chats are ticked: Settle, Snooze, Move, Select all, Done.
 *  It acts on the ticked rows that are shown, one action at a time. */
export function RailBulkBar(props: { visible: () => string[] }) {
  const keys = () => props.visible().filter(k => selectedKeys().has(k));
  // A tick on a row that left the list (filtered out, settled elsewhere) is dropped.
  createEffect(() => pruneSelection(props.visible()));
  const run = async (f: (m: typeof import('../conversationBulk'), ks: string[]) => unknown) => {
    const ks = keys();
    if (!ks.length || bulkRunning()) return;
    setBulkRunning(true);
    try { await import('../conversationBulk').then(m => f(m, ks), () => lifecycleFailed('That action did not load. Try again.')); }
    finally { setBulkRunning(false); }
  };
  const none = () => !keys().length || bulkRunning();
  return <div class="rail-bulk" role="group" aria-label="Selected chats" aria-busy={bulkRunning()}>
    <span class="rail-bulk-count" role="status" aria-live="polite">{keys().length ? `${keys().length} selected` : 'None selected'}</span>
    <button type="button" class="btn ghost small" disabled={none()} onClick={() => void run((m, ks) => m.bulkOrganize(ks, 'settle'))}>Settle</button>
    <button type="button" class="btn ghost small" disabled={none()} onClick={() => void run((m, ks) => m.bulkOrganize(ks, 'snooze'))}>Snooze…</button>
    <button type="button" class="btn ghost small" disabled={none()} onClick={() => void run((m, ks) => m.bulkMove(ks))}>Move…</button>
    <button type="button" class="btn ghost small" disabled={bulkRunning()} onClick={() => keys().length && keys().length === props.visible().length ? clearSelection() : selectAll(props.visible())}>{keys().length && keys().length === props.visible().length ? 'Clear' : 'All'}</button>
    <button type="button" class="btn ghost small" disabled={bulkRunning()} onClick={stopSelecting}>Done</button>
  </div>;
}
