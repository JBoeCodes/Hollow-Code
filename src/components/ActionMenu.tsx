import { Index, Show, createSignal, createUniqueId, onCleanup, onMount, type JSX } from 'solid-js';
import { inSelectPopover } from './Select';
import { Icon } from '../icons';
import { typeaheadMatch } from '../selectModel';

export type ActionMenuItem = { label: string; action: () => unknown; disabled?: boolean; danger?: boolean; separatorBefore?: boolean;
  /** Read-only information that remains reachable with the menu's arrow keys. */
  informational?: boolean;
  /** Hover text: what the row does (the access chip's mode descriptions). */
  title?: string;
  /** Its key, shown at the end of the row (`⌥⌘U`) and announced as `aria-keyshortcuts` (`Alt+Meta+U`). */
  shortcut?: { label: string; aria: string } };

/** Rows are keyed by position (`Index`), not by item object, so a caller that
 * rebuilds its items (the access chip when a turn ends) updates the rows in
 * place and a focused row keeps focus.
 *
 * Overflow menu on a native disclosure. Enter/Space opens; arrows, Home/End
 * and typeahead move between actions; Escape closes and returns focus to the
 * trigger (R01 keyboard contract, shared with Select). Items name their
 * object ("Reveal file", "Delete workspace"), per audit H08. */
export function ActionMenu(props: { label: string; items: ActionMenuItem[]; icon?: JSX.Element; align?: 'start' | 'end' }) {
  let menu!: HTMLDetailsElement;
  let trigger!: HTMLElement;
  let typed = ''; let typedAt = 0;
  const triggerId = `action-menu-${createUniqueId()}`;
  // Named as a button with its state (consolidation D): a bare <summary>
  // reached the accessibility tree unnamed in Chromium and WebKit.
  const [open, setOpen] = createSignal(false);
  const buttons = () => [...menu.querySelectorAll<HTMLButtonElement>('.studio-action-menu-body button:not(:disabled)')];
  onMount(() => {
    const outside = (e: PointerEvent) => { if (!menu.contains(e.target as Node) && !inSelectPopover(e.target)) menu.open = false; };
    document.addEventListener('pointerdown', outside);
    onCleanup(() => document.removeEventListener('pointerdown', outside));
  });
  function close() { menu.open = false; trigger.focus(); }
  return <details ref={menu} class="studio-action-menu" classList={{ start: props.align === 'start' }}
    onToggle={() => {
      setOpen(menu.open);
      // The first item takes focus only while focus is still on the trigger: the
      // toggle event lands a task after the menu opens, and a fast ArrowDown
      // may already have moved to an item, which this must not undo (FIX-1,
      // keyboard-only.spec.ts, 11-13).
      const inBody = !!menu.querySelector('.studio-action-menu-body')?.contains(document.activeElement);
      if (menu.open && menu.contains(document.activeElement) && !inBody) queueMicrotask(() => buttons()[0]?.focus());
    }}
    on:keydown={(e: KeyboardEvent) => {
      if (!menu.open) { if (e.key === 'ArrowDown' && e.target === trigger) { e.preventDefault(); menu.open = true; } return; }
      const list = buttons(); const at = list.indexOf(document.activeElement as HTMLButtonElement);
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); }
      else if (e.key === 'ArrowDown') { e.preventDefault(); list[Math.min(list.length - 1, at + 1)]?.focus(); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); list[Math.max(0, at - 1)]?.focus(); }
      else if (e.key === 'Home') { e.preventDefault(); list[0]?.focus(); }
      else if (e.key === 'End') { e.preventDefault(); list[list.length - 1]?.focus(); }
      else if (e.key === 'Tab') menu.open = false;
      else if (e.key.length === 1 && e.key !== ' ' && !e.metaKey && !e.ctrlKey && !e.altKey) {
        const now = Date.now(); typed = now - typedAt > 600 ? e.key : typed + e.key; typedAt = now;
        const i = typeaheadMatch(list.map(b => ({ value: '', label: b.textContent ?? '' })), typed, at);
        if (i >= 0) list[i].focus();
      }
    }}>
    <summary ref={trigger} id={triggerId} role="button" aria-expanded={open()} aria-label={props.label} title={props.label} aria-haspopup="menu">{props.icon ?? <Icon name="more" size={16} />}</summary>
    <div class="studio-action-menu-body" role="menu" aria-labelledby={triggerId}><Index each={props.items}>{item => <>
      <Show when={item().separatorBefore}><div class="studio-action-menu-sep" role="separator" /></Show>
      <button type="button" role="menuitem" classList={{ danger: !!item().danger }} disabled={item().disabled} title={item().title} aria-disabled={item().informational ? 'true' : undefined} aria-keyshortcuts={item().shortcut?.aria} onClick={() => { if (item().informational) return; close(); item().action(); }}>{item().label}<Show when={item().shortcut}>{k => <kbd class="studio-action-menu-kbd" aria-hidden="true">{k().label}</kbd>}</Show></button>
    </>}</Index></div>
  </details>;
}
