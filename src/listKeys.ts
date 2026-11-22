// Single-key list navigation (five asks A4, part 2), for lists that have no
// keys of their own: mark a list `data-key-list` and its rows
// `data-key-item` (each focusable). j / k (and ↓ / ↑ inside a list) move
// between rows, Enter opens one (a button's own Enter), `.` opens its menu
// (the context-menu event a right click sends, then the menu takes the
// keyboard), r opens it on Rename and e on Edit or Open, unless the row has
// its own `data-key-action="rename" | "edit"` button. Tasks and Waiting on
// you keep their own handlers and aren't marked. Letters go with Use
// keyboard shortcuts; never while typing, behind a modal or with a menu
// open. Pure parts here; `store/listKeys.ts` listens.

export type ListKey = 'next' | 'prev' | 'edit' | 'rename' | 'menu';

/** The list action of a plain key (`arrow`: ↓ / ↑, which work only inside
 *  a list and with shortcuts off too), or null. */
export function listKeyOf(e: { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean }): { action: ListKey; arrow: boolean } | null {
  if (e.metaKey || e.ctrlKey || e.altKey) return null;
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') return { action: e.key === 'ArrowDown' ? 'next' : 'prev', arrow: true };
  const action = ({ j: 'next', k: 'prev', e: 'edit', r: 'rename', '.': 'menu' } as Record<string, ListKey>)[e.key];
  return action ? { action, arrow: false } : null;
}

/** The menu item r and e land on. */
export const MENU_FOCUS: Readonly<Record<'rename' | 'edit', RegExp>> = { rename: /^rename/i, edit: /^(edit|open)/i };

/** Where focus sits among `items` when it is on none of them but inside
 *  the list (a toggle, a + button): the row before it, as a fractional
 *  index, so j goes to the next row and k to the one before. `before[i]`
 *  says whether row i precedes the focused element in the document. */
export function between(before: readonly boolean[]): number {
  const last = before.lastIndexOf(true);
  return last + 0.5;
}

/** The row to move to from `at` (an index, -1 for none) among `count`
 *  rows: j from none starts at the first, k from none at the last; the
 *  ends stop (no wrap). */
export function stepIndex(at: number, count: number, dir: 'next' | 'prev'): number {
  if (count <= 0) return -1;
  if (at === -1) return dir === 'next' ? 0 : count - 1;
  // Between rows (focus on a toggle, `between`): the neighbour that way.
  if (!Number.isInteger(at)) return dir === 'next' ? Math.min(count - 1, Math.max(0, Math.ceil(at))) : Math.max(0, Math.floor(at));
  return dir === 'next' ? Math.min(count - 1, at + 1) : Math.max(0, at - 1);
}

/** Whether an element is on screen (a hidden page's list isn't). */
export function isShown(el: Element): boolean {
  return (el as HTMLElement).getClientRects?.().length > 0;
}

/** The list a key applies to: the one holding focus, else (for j / k only,
 *  focus nowhere in a list) the page's own `data-key-default` list on
 *  screen. The rail is never a default: the page comes first. */
export function activeList(active: Element | null, doc: Pick<Document, 'querySelectorAll'>, action: ListKey, arrow = false): Element | null {
  const held = active?.closest?.('[data-key-list]') ?? null;
  if (held) return held;
  // The arrows never pull focus into a list: they scroll the page.
  if (arrow || (action !== 'next' && action !== 'prev')) return null;
  return [...doc.querySelectorAll('[data-key-list][data-key-default]')].find(isShown) ?? null;
}
