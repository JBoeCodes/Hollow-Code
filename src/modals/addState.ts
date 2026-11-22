import { createSignal } from 'solid-js';
import type { CaptureKind } from '../captureClassify';

// The Add sheet's open state (next-level P12), apart from the sheet itself
// so the rail and palette can open it without loading it (P21).
export const [addOpen, setAddOpen] = createSignal(false);
export const [addSeed, setAddSeed] = createSignal('');
/** The kind this opening starts on, instead of the sheet's guess (S5). */
export const [addPreset, setAddPreset] = createSignal<CaptureKind | null>(null);
/** Counts openings, so an open sheet follows one made while it is open (S5 review). */
export const [addRequest, setAddRequest] = createSignal(0);
/** Whether this opening came from ⌘⌥⇧T, ⌘⌥⇧I or their tray items, which
 *  step back to where the person was after a save (S5 review). */
export const [addSummoned, setAddSummoned] = createSignal(false);
let summoning = false;
/** Open the Add sheet (rail, palette, Your work), optionally with text, and
 *  optionally set to a kind until the person picks another. */
export function openAdd(text = '', kind: CaptureKind | null = null) {
  setAddSeed(text); setAddPreset(kind); setAddSummoned(summoning);
  setAddRequest(n => n + 1); setAddOpen(true);
}
/** Run `open` as the system-wide key or tray item did (appMenu.ts). */
export function summonAdd(open: () => unknown) { summoning = true; try { open(); } finally { summoning = false; } }
/** Import a source (the palette, Library's ⋯, ⌘⌥⇧I and the tray): the Add
 *  sheet set to a Library source, with its Domain and Purpose (S5: the
 *  separate Import source sheet and window were folded into Add). */
export function openImportSource() { openAdd('', 'source'); }
/** Quick capture (⌘⌥⇧T and the tray): the Add sheet set to a task, parsed
 *  as you type (S5 folded the separate quick-capture window into Add). */
export function openQuickCapture() { openAdd('', 'task'); }
