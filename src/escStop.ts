/**
 * Esc stops a running turn (frontier F6), but only when nothing else wants
 * that Escape and the composer is empty. Pure decision; ChatBody supplies
 * the DOM facts from a window capture listener, so it sees an open menu or
 * popup before that surface's own handler closes it.
 */
import { MODAL_SEL } from './modalGuard';

/** Anything open that owns Escape: modals and the palette (MODAL_SEL), the
 *  keymap card, menus, listboxes (Select, the model menu with its
 *  Permissions, the slash popup), dialogs, the usage ring and context menus. */
export const ESC_OWNER_SEL = `${MODAL_SEL}, .keymap-backdrop, [role="menu"], [role="listbox"], [role="dialog"], .ctx-menu, .slash-popup`;

export interface EscFacts {
  /** A turn is queued or running in the conversation on screen. */
  busy: boolean;
  /** Focus is inside the conversation (transcript or composer). */
  inConversation: boolean;
  /** The focused element is the composer. */
  onComposer: boolean;
  /** The focused element is some other text field (a feedback reason, a
   *  rename field): its Escape cancels that edit. */
  onOtherField: boolean;
  /** A surface in ESC_OWNER_SEL is open. */
  ownerOpen: boolean;
  /** The composer holds no text and no attachment. Esc stops a turn only
   *  then (P1-27's planning call), so it never throws away a draft's turn. */
  composerEmpty: boolean;
}

export function escStopsRun(
  e: { key: string; defaultPrevented: boolean; repeat?: boolean; isComposing?: boolean; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean },
  f: EscFacts,
): boolean {
  if (e.key !== 'Escape' || e.defaultPrevented || e.repeat || e.isComposing) return false;
  if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return false;
  if (!f.busy || !f.inConversation || f.ownerOpen || !f.composerEmpty) return false;
  return f.onComposer || !f.onOtherField;
}

/** Whether a surface that owns Escape is open. Not open: one still fading
 *  out after it closed (`.is-closing`, see components/exit.ts), a menu kept in
 *  the DOM inside a closed `<details>` (ActionMenu), or anything not rendered. */
export function escOwnerOpen(root: ParentNode = document): boolean {
  return [...root.querySelectorAll(ESC_OWNER_SEL)].some(el =>
    !el.closest('.is-closing') && !el.closest('details:not([open])') && el.getClientRects().length > 0);
}
