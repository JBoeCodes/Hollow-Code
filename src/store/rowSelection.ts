// Which sidebar chat rows are ticked for a bulk action (3.5). One list at a time; a plain click
// opens a chat unless selecting; Cmd-click ticks one, Shift-click ticks the stretch from the last.
import { createSignal } from 'solid-js';

export const [selecting, setSelecting] = createSignal(false);
export const [selectedKeys, setSelectedKeys] = createSignal<ReadonlySet<string>>(new Set());
let anchor: string | null = null;

export function toggleSelected(key: string, on?: boolean) {
  setSelectedKeys(prev => {
    const next = new Set(prev);
    if (on ?? !next.has(key)) next.add(key); else next.delete(key);
    return next;
  });
  anchor = key;
}

/** Tick every row from the last ticked one to `key` in the order shown. */
export function selectRange(order: readonly string[], key: string) {
  const a = anchor && order.includes(anchor) ? order.indexOf(anchor) : order.indexOf(key);
  const b = order.indexOf(key);
  if (b < 0) return;
  const [from, to] = a <= b ? [a, b] : [b, a];
  setSelectedKeys(prev => { const next = new Set(prev); for (const k of order.slice(from, to + 1)) next.add(k); return next; });
  anchor = key;
}

/** Un-tick `keys` (the rows a finished bulk action handled); rows ticked meanwhile stay. */
export function unselect(keys: readonly string[]) { setSelectedKeys(prev => { const next = new Set(prev); for (const k of keys) next.delete(k); return next; }); }
/** Drop ticks for rows no longer shown. */
export function pruneSelection(shown: readonly string[]) { setSelectedKeys(prev => { const keep = [...prev].filter(k => shown.includes(k)); return keep.length === prev.size ? prev : new Set<string>(keep); }); }

/** True while a bulk action runs: its buttons wait, so two never overlap. */
export const [bulkRunning, setBulkRunning] = createSignal(false);

export function selectAll(keys: readonly string[]) { setSelectedKeys(new Set<string>([...keys])); }
export function clearSelection() { setSelectedKeys(new Set<string>()); anchor = null; }
export function startSelecting(key?: string) { setSelecting(true); if (key) toggleSelected(key, true); }
export function stopSelecting() { setSelecting(false); clearSelection(); }
