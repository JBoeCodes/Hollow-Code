// App-wide overlays any component opens through a signal: the context menu,
// the confirm modal (showConfirm / showPrompt / showChecklist), the command
// palette and the keyboard-shortcuts card. No store imports.
// Moved verbatim out of store/runtime.ts (plan L3 X19, 2026-10-14); runtime.ts
// re-exports every symbol, so `store/runtime` imports keep working.
import { createSignal } from 'solid-js';
import { parseTicked, type ChecklistRow } from '../checklist';

// ─────────────────────────────────────────────────────────────
// CONTEXT MENU — a single global signal drives a positioned
// overlay. Any component can call openContextMenu(e, items)
// from an onContextMenu handler.
// ─────────────────────────────────────────────────────────────

export interface ContextMenuItem {
  /** Display label. Ignored when `separator` is true. */
  label?: string;
  /** Full explanation retained when an action moves into a menu. */
  title?: string;
  /** Click handler. Skipped if disabled. */
  action?: () => void | Promise<void>;
  /** Right-aligned shortcut hint, e.g. `⌘N`. */
  shortcut?: string;
  /** Render as a divider row instead of a button. */
  separator?: boolean;
  /** Style as a destructive action (red). */
  danger?: boolean;
  disabled?: boolean;
}

export interface ContextMenuState {
  x: number;
  y: number;
  items: ContextMenuItem[];
  /** The element that was right-clicked, and where it was then. A scroll
   *  closes the menu only when it moved this anchor (ContextMenu.tsx). */
  anchor?: Element;
  anchorTop?: number;
  anchorLeft?: number;
}

export const [contextMenu, setContextMenu] = createSignal<ContextMenuState | null>(null);

export function openContextMenu(e: MouseEvent, items: ContextMenuItem[]) {
  e.preventDefault();
  e.stopPropagation();
  const anchor = e.currentTarget instanceof Element ? e.currentTarget : undefined;
  const rect = anchor?.getBoundingClientRect();
  setContextMenu({ x: e.clientX, y: e.clientY, items, anchor, anchorTop: rect?.top, anchorLeft: rect?.left });
}

/** Open the context menu under a visible button (its "⋯"), at the same
 *  place for a click and for Enter/Space (consolidation D: every right-click
 *  menu also has a visible button). */
export function openContextMenuAt(anchor: Element, items: ContextMenuItem[]) {
  const rect = anchor.getBoundingClientRect();
  setContextMenu({ x: Math.round(rect.left), y: Math.round(rect.bottom + 4), items, anchor, anchorTop: rect.top, anchorLeft: rect.left });
}

export function closeContextMenu() {
  setContextMenu(null);
}

// ─────────────────────────────────────────────────────────────
// CONFIRM MODAL — in-app replacement for `window.confirm`.
// Any callsite that needs a yes/no decision calls `showConfirm`
// and awaits the boolean. The dialog renders globally (App.tsx
// mounts <ConfirmDialog/>); Escape cancels, Enter confirms.
// ─────────────────────────────────────────────────────────────

export interface ConfirmOptions {
  title: string;
  /** One line above the body that the dialog is described by, so a screen
   *  reader reads the title and this, not a long body (P1-38: a merge). */
  summary?: string;
  /** Optional body — supports plain text with `\n` newlines. */
  body?: string;
  /** Why the action can't run now: shown, and the confirm button is off
   *  (P1-38: a merge a working conversation blocks). */
  blockedReason?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  alternateLabel?: string;
  /** Style the confirm button red (destructive / danger mode). */
  danger?: boolean;
  /** Start on Cancel without the red: for a costly but not destructive
   *  choice, so a stray Enter does nothing (resuming chats sends billed
   *  messages; overnight B8). Danger dialogs already start there. */
  focusCancel?: boolean;
  /** Show a one-line text field (see `showPrompt`). */
  input?: { value: string; label: string; placeholder?: string; maxLength?: number; /** A date and time picker (`datetime-local`); its value is a local `YYYY-MM-DDTHH:mm`. */ type?: 'text' | 'datetime-local'; min?: string };
  /** Show ticks, one per row (see `showChecklist`). */
  checklist?: ChecklistRow[];
  /** Rows listed under the ticks with one button each and nothing to tick
   *  (P1-40: agent-started chats to open). A button cancels the dialog,
   *  then runs `onAction` with its row's id. */
  aside?: { heading: string; note?: string; actionLabel: string; rows: { id: string; label: string; detail?: string }[]; onAction: (id: string) => void };
  /** No confirm button: the aside's rows are the only choices, and focus
   *  starts on the first (2026-11-10: which of several files to open). */
  hideConfirm?: boolean;
}

interface ConfirmState extends ConfirmOptions {
  /** Local DOM origin survives lazy loading and replacement requests; never serialized. */
  returnFocus?: HTMLElement | null;
  resolve: (ok: boolean, value?: string) => void;
}

export const [confirmDialog, setConfirmDialog] = createSignal<ConfirmState | null>(null);

function confirmReturnFocus(existing: ConfirmState | null): HTMLElement | null {
  if (existing) return existing.returnFocus ?? null;
  return typeof document !== 'undefined' && document.activeElement instanceof HTMLElement ? document.activeElement : null;
}

/// Show a confirm dialog and resolve to `true` when the user accepts,
/// `false` when they cancel/press Escape/click the backdrop.
///
/// If a confirm is already open, it gets auto-cancelled (resolves false)
/// before the new one shows so the queue stays linear.
export function showConfirm(opts: ConfirmOptions): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    const existing = confirmDialog();
    if (existing) existing.resolve(false);
    setConfirmDialog({ ...opts, returnFocus: confirmReturnFocus(existing), resolve });
  });
}

/** Three explicit choices, sharing the confirm dialog's focus and cancellation. */
export function showChoice(opts: ConfirmOptions & { alternateLabel: string }): Promise<'confirm' | 'alternate' | 'cancel'> {
  return new Promise(resolve => {
    const existing = confirmDialog();
    existing?.resolve(false);
    setConfirmDialog({ ...opts, returnFocus: confirmReturnFocus(existing), resolve: (ok, value) => resolve(ok ? 'confirm' : value === 'alternate' ? 'alternate' : 'cancel') });
  });
}

/// Ask for one line of text (a rename). Resolves to the trimmed text, or
/// `null` when cancelled. Uses the same global dialog as `showConfirm`.
export function showPrompt(opts: ConfirmOptions & { input: NonNullable<ConfirmOptions['input']> }): Promise<string | null> {
  return new Promise<string | null>((resolve) => {
    const existing = confirmDialog();
    if (existing) existing.resolve(false);
    setConfirmDialog({ ...opts, returnFocus: confirmReturnFocus(existing), resolve: (ok, value) => resolve(ok && value?.trim() ? value.trim() : null) });
  });
}

/// Ask the person to tick rows (Suggest workspaces to archive…). Resolves to
/// the ticked ids in row order, or `null` when cancelled. The confirm button
/// is disabled while nothing is ticked.
export function showChecklist(opts: ConfirmOptions & { checklist: ChecklistRow[] }): Promise<string[] | null> {
  return new Promise<string[] | null>((resolve) => {
    const existing = confirmDialog();
    if (existing) existing.resolve(false);
    setConfirmDialog({ ...opts, returnFocus: confirmReturnFocus(existing), resolve: (ok, value) => resolve(ok ? parseTicked(value, opts.checklist) : null) });
  });
}

/// Internal — called by the ConfirmDialog component on user action.
export function resolveConfirm(ok: boolean, value?: string) {
  const cur = confirmDialog();
  if (!cur) return;
  setConfirmDialog(null);
  cur.resolve(ok, value);
}

// ─────────────────────────────────────────────────────────────
// COMMAND PALETTE — Cmd+K / Ctrl+K. Renders globally over the
// app. Items are produced fresh from signals each time it opens,
// so they reflect current state (recent notes, cached tasks, etc.)
// ─────────────────────────────────────────────────────────────

export const [commandPaletteOpen, setCommandPaletteOpen] = createSignal(false);
/** What the palette lists: everything, only "which workspace gets the new
 *  conversation" (the palette's New conversation…, frontier F4), or (a
 *  conversation's Move to…, 2026-09-30) where `paletteMoveKey` goes. */
export type PaletteScope = 'all' | 'new-conversation' | 'move-session' | 'merge-workspace';
export const [paletteScope, setPaletteScope] = createSignal<PaletteScope>('all');
/** The conversation the move-session scope moves. */
export const [paletteMoveKey, setPaletteMoveKey] = createSignal<string | null>(null);
/** The workspace the merge-workspace scope merges (Merge into…, master plan #18). */
export const [paletteMergeFrom, setPaletteMergeFrom] = createSignal<string | null>(null);
/** Also used directly as an onClick handler, so anything but a scope
 *  string (a MouseEvent) means the full palette. */
/** The search chip the palette opens on, once (`'jump'`: Workspaces). */
export const [paletteSearchRequest, setPaletteSearchRequest] = createSignal<'workspaces' | null>(null);
/** Set when the palette opened a page of its own (New conversation…) from its
 *  root, so Backspace in the empty box returns there (paletteNav.ts). */
export const [paletteReturnsToRoot, setPaletteReturnsToRoot] = createSignal(false);
export function openCommandPalette(scope?: unknown) {
  setPaletteReturnsToRoot(false);
  if (scope === 'move-session') { const key = paletteMoveKey(); if (key) setProjectDestinationTarget({key}); return; }
  setPaletteScope(scope === 'new-conversation' || scope === 'move-session' || scope === 'merge-workspace' ? scope : 'all');
  // Jump to a workspace or chat (five asks A4): the palette on its
  // Workspaces chip (workspaces, conversations and Results).
  setPaletteSearchRequest(scope === 'jump' ? 'workspaces' : null);
  // The keys card (`?`) sits above every view; a view the palette opens would
  // land under it (cross-PR review S2).
  setShortcutsOverlayOpen(false);
  setCommandPaletteOpen(true);
}
/** Words the palette opens with, once: the sidebar's Search everything
 *  (overnight 11-04 S2) carries what was typed in Search chats. */
export const [paletteQueryRequest, setPaletteQueryRequest] = createSignal<string | null>(null);
export function openCommandPaletteWith(query: string) {
  setPaletteQueryRequest(query.trim() || null);
  openCommandPalette();
}
/** Move to…: the palette listing where conversation `key` can go. */
export function openMovePalette(key: string) {
  setPaletteMoveKey(key);
  setProjectDestinationTarget({ key });
}
/** One Add to project picker for existing chats and Home's next chat. */
export interface ProjectDestinationTarget {
  key: string | null;
  value?: string | null;
  pick?: (id: string | null) => void;
}
export const [projectDestinationTarget, setProjectDestinationTarget] = createSignal<ProjectDestinationTarget | null>(null);
/** Merge into…: the palette listing where workspace `from` can merge. */
export function openMergePalette(from: string) {
  setPaletteMergeFrom(from);
  openCommandPalette('merge-workspace');
}
export function closeCommandPalette() { setPaletteReturnsToRoot(false); setCommandPaletteOpen(false); }
/** Set while the palette is mounted: runs one of its own commands (Command
 *  palette closes it, Jump switches it to Workspaces) and says whether it
 *  took it (P1-48), so the menu reaches them behind the palette too. */
let paletteCommand: ((id: string) => boolean) | null = null;
export function registerPaletteCommand(run: (id: string) => boolean): () => void {
  paletteCommand = run;
  return () => { if (paletteCommand === run) paletteCommand = null; };
}
export function runPaletteCommand(id: string): boolean {
  return commandPaletteOpen() && !!paletteCommand?.(id);
}

// The keymap card — `?` anywhere you're not typing (App.tsx keydown).
// Generated from the same PAGE_KEY/ACTION_SHORTCUT maps that drive the
// real bindings, so it can't drift from the truth.
export const [shortcutsOverlayOpen, setShortcutsOverlayOpen] = createSignal(false);
export function toggleShortcutsOverlay() { setShortcutsOverlayOpen(v => !v); }
export function closeShortcutsOverlay() { setShortcutsOverlayOpen(false); }
