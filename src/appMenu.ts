/**
 * Native menu actions (audit H14). `app_menu.rs` emits `app-menu` with an item
 * id; each id is a shared action's `menu` (actionDefs.ts), or a ⌘ command's
 * `key` from App.tsx's keymap, and runs the same dispatcher as its palette
 * row, `g` key and New menu item (actionRun.ts `runMenuOrKey`). A Rust test
 * checks every menu id is declared there.
 *
 * The action table and its dispatcher load on first use (App preloads them
 * when idle), so the launch bundle carries neither (plan completion L4).
 */
import { listen } from './nativeEvents';
import { MODAL_SEL, modalOpen } from './modalGuard';
import { summonAdd } from './modals/addState';
import { browserGo, browserTakesNav } from './store/browser';
import { runPaletteCommand } from './store/overlays';
import { flashFooter } from './store/snackbars';

const loadRun = () => import('./actionRun');

export async function copyBuildIdentity(): Promise<string> {
  return (await loadRun()).copyBuildIdentity();
}

/** The Add sheet's own system-wide keys and tray items (S5): quick capture
 *  and Import a source. */
export const ADD_KEYS: ReadonlySet<string> = new Set(['quick-capture', 'quick-ingest']);

/** The open dialog an Add key can't reach past, named for the notice
 *  ("“Edit task”", "the open dialog"), or null when the only one open is the
 *  Add sheet itself, which follows the key (S5 review). */
export function dialogBlockingAdd(doc: Document = document): string | null {
  const open = [...doc.querySelectorAll(MODAL_SEL)].filter(el => !el.classList.contains('add-sheet'));
  if (!open.length) return null;
  const dialog = open[0].matches('[role="dialog"]') ? open[0] : open[0].querySelector('[role="dialog"]');
  const labelledBy = dialog?.getAttribute('aria-labelledby');
  const name = (dialog?.getAttribute('aria-label') ?? (labelledBy ? doc.getElementById(labelledBy)?.textContent : null) ?? '').trim();
  return name ? `“${name}”` : 'the open dialog';
}

/** A native menu item or ⌘ shortcut by id. Behind a modal nothing runs
 *  (it says "Close the dialog first"), except copying the build identity
 *  (harmless), dictation (it only types into the focused field, so it works
 *  in a dialog too), the palette's own commands while it is open, and the
 *  Add keys while the open dialog is the Add sheet (it switches kind; any
 *  other dialog is named so the person knows what to close). An Add key's
 *  sheet steps back after a save when Hollow wasn't in front (lib.rs
 *  `show_main_with`). */
export function runMenuAction(id: string) {
  // The open palette answers its own: Search… closes it, Jump switches its
  // chip (P1-48).
  if (runPaletteCommand(id)) return;
  const addKey = ADD_KEYS.has(id);
  // Said, not silent (P1-49): a click in the menu bar has no other answer.
  if (modalOpen() && id !== 'copy-build' && id !== 'dictate') {
    if (!addKey) { flashFooter('Close the dialog first', 2500); return; }
    const blocking = dialogBlockingAdd();
    if (blocking) { flashFooter(`Close ${blocking} first, then press the key again`, 4000); return; }
  }
  void loadRun().then(m => { if (addKey) summonAdd(() => m.runMenuOrKey(id)); else m.runMenuOrKey(id); });
}

/** A native menu item, clicked or by its key. With the side panel's page in
 *  front, Back and Forward (⌘[ ⌘]) step the page, not Hollow (P1-47): a
 *  key pressed in the native page reaches only the menu. */
export function runNativeMenu(id: string, run: (id: string) => void = runMenuAction) {
  if ((id === 'go-back' || id === 'go-forward') && !modalOpen() && browserTakesNav()) {
    browserGo(id === 'go-back' ? 'back' : 'forward');
    return;
  }
  run(id);
}

export function listenAppMenu(): () => void {
  let off: (() => void) | undefined; let disposed = false;
  listen<string>('app-menu', e => runNativeMenu(e.payload)).then(u => { if (disposed) u(); else off = u; }).catch(() => { /* not in tauri */ });
  return () => { disposed = true; off?.(); };
}
