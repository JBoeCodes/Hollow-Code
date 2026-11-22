/**
 * The command palette's navigation rules, pure so a test can hold them (parity
 * lane 4.1, 2026-11-20): the `>` actions prefix, Backspace going back one
 * level, and the digit keys that choose a row while the palette is open.
 *
 * The digits are palette-local. Only the page digits (⌘1 to ⌘5) are app
 * shortcuts, and they stay that way: with the palette open the same keystroke
 * chooses the row that digit labels, and the native menu's page items answer
 * the same way (`digitOfMenuCommand`). Nothing here adds a chord to the keymap.
 */
import { chordKey, hasChordModifier, activeChord, type ChordEvent } from './shortcuts';
import type { PaletteSearchScope } from './paletteSearch';

/** Rows 1 to 9 carry a digit. */
export const PALETTE_DIGITS = 9;

/** `>` at the start of the box, then what follows it: the actions scope. */
export function splitActionPrefix(value: string): { actions: boolean; query: string } {
  const m = /^\s*>\s?/.exec(value);
  return m ? { actions: true, query: value.slice(m[0].length) } : { actions: false, query: value };
}

/** The digit a row at `index` (0-based, in displayed order) answers to, or null past the ninth. */
export function rowDigit(index: number): number | null {
  return index >= 0 && index < PALETTE_DIGITS ? index + 1 : null;
}

/** The row a digit chooses, or null when the list is shorter. */
export function rowForDigit(digit: number, count: number): number | null {
  return digit >= 1 && digit <= PALETTE_DIGITS && digit <= count ? digit - 1 : null;
}

/** ⌘ (Ctrl elsewhere) plus one digit 1 to 9, nothing else held: the digit a keystroke names. */
export function digitOfKey(e: ChordEvent & { isComposing?: boolean; repeat?: boolean }): number | null {
  if (e.isComposing || e.altKey || e.shiftKey || !hasChordModifier(e)) return null;
  const key = chordKey(e);
  return key && /^[1-9]$/.test(key) ? Number(key) : null;
}

/** A native menu item that carries a page digit now (go-work is ⌘1 …): the
 *  menu takes the accelerator before the page sees it, so the palette reads
 *  the digit from the item's current chord. Null for any other item, or a page
 *  key the person moved off a digit. */
export function digitOfMenuCommand(id: string): number | null {
  const m = /^CmdOrCtrl\+([1-9])$/.exec(activeChord(id));
  return m ? Number(m[1]) : null;
}

/** What Backspace on an empty box does: step back one level. A chip or the
 *  `>` scope returns to All; a page the palette opened from the root
 *  (New conversation…) returns to the root. Null at the root. */
export type PaletteBack = 'to-all' | 'to-root' | null;
export function backTarget(state: { query: string; scope: PaletteSearchScope; subpageFromRoot: boolean }): PaletteBack {
  if (state.query !== '') return null;
  if (state.subpageFromRoot) return 'to-root';
  return state.scope !== 'all' ? 'to-all' : null;
}
