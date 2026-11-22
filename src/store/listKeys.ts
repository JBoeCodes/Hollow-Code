import { createSignal } from 'solid-js';
import { MENU_FOCUS, activeList, between, isShown, listKeyOf, stepIndex } from '../listKeys';
import { isTypingTarget } from '../keyboardLeader';
import { recordingShortcut, shortcutsEnabled } from '../shortcuts';
import { modalOpen } from '../modalGuard';
import { contextMenu, shortcutsOverlayOpen } from './runtime';

// Single-key list navigation (five asks A4, part 2): App.tsx listens on the
// document (bubbling), after the go-to leader (window capture) and before
// a page's own window listeners (Tasks), whose lists aren't marked.

/** A menu the keyboard opened: the item it should focus (true: the first),
 *  read once by `ContextMenu` when it opens. */
const [menuFocus, setMenuFocus] = createSignal<RegExp | true | null>(null);
/** The next menu to open takes focus on its first item (a "⋯" opened with
 *  Enter or Space). */
export function focusNextMenu() { setMenuFocus(true); }
export function takeMenuFocus(): RegExp | true | null {
  const hint = menuFocus();
  setMenuFocus(null);
  return hint;
}

const OWN_ACTION: Record<'edit' | 'rename', string> = { edit: '[data-key-action="edit"]', rename: '[data-key-action="rename"]' };

/** One keydown; whether a list took it. */
export function listKeydown(e: KeyboardEvent): boolean {
  const key = listKeyOf(e);
  if (!key || e.defaultPrevented || contextMenu()) return false;
  const { action, arrow } = key;
  if (e.repeat && action !== 'next' && action !== 'prev') return false;
  if ((!arrow && !shortcutsEnabled()) || recordingShortcut() || modalOpen() || shortcutsOverlayOpen() || isTypingTarget(e.target)) return false;
  const active = document.activeElement;
  const list = activeList(active, document, action, arrow);
  if (!list) return false;
  const items = [...list.querySelectorAll<HTMLElement>('[data-key-item]')].filter(el => isShown(el) && !(el as HTMLButtonElement).disabled);
  let at = items.findIndex(el => el === active || el.contains(active));
  // Focus on a toggle or + inside the list: keep the place.
  if (at < 0 && active && list.contains(active)) {
    at = between(items.map(el => !!(el.compareDocumentPosition(active) & Node.DOCUMENT_POSITION_FOLLOWING)));
  }
  if (action === 'next' || action === 'prev') {
    const to = items[stepIndex(at, items.length, action)];
    if (!to) return false;
    e.preventDefault();
    to.focus();
    to.scrollIntoView?.({ block: 'nearest' });
    return true;
  }
  const item = Number.isInteger(at) ? items[at] : undefined;
  if (!item) return false;
  if (action !== 'menu') {
    const own = item.querySelector<HTMLElement>(OWN_ACTION[action]) ?? (item.matches(OWN_ACTION[action]) ? item : null);
    if (own) { e.preventDefault(); own.click(); return true; }
  }
  // The same event a right click sends, at the row; the menu then takes the
  // keyboard on the item asked for. A row with no menu leaves the key alone.
  setMenuFocus(action === 'menu' ? true : MENU_FOCUS[action]);
  const r = item.getBoundingClientRect();
  const opened = !item.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: r.left + Math.min(24, r.width / 2), clientY: r.top + r.height / 2 }));
  if (!opened) { setMenuFocus(null); return false; }
  e.preventDefault();
  return true;
}
