import { For, Show, createEffect, onCleanup } from 'solid-js';
import { contextMenu, closeContextMenu, type ContextMenuItem } from '../store';
import { takeMenuFocus } from '../store/listKeys';

/**
 * Single global overlay driven by the `contextMenu` signal.
 * Mount once near the root of the app. Components attach their own
 * `onContextMenu` handlers and call `openContextMenu(event, items)`.
 *
 * Closes on outside click, Escape, a wheel/touch scroll outside the menu, a
 * scroll that moves the right-clicked element, or after an item is clicked.
 * Position is adjusted so the menu never spills past the viewport.
 */

/** A `scroll` event closes the menu only when it moved the anchor. Layout
 *  scrolls that keep the row in place (scroll anchoring while rows above it
 *  animate in or re-render) used to close it a few ms after it opened. */
export function scrollMovedAnchor(m: { anchor?: Element; anchorTop?: number; anchorLeft?: number }): boolean {
  if (!m.anchor || m.anchorTop === undefined || m.anchorLeft === undefined) return true;
  if (!m.anchor.isConnected) return false;
  const r = m.anchor.getBoundingClientRect();
  return Math.abs(r.top - m.anchorTop) > 2 || Math.abs(r.left - m.anchorLeft) > 2;
}
export function ContextMenu() {
  let menuEl: HTMLDivElement | undefined;
  // Opened from the keyboard (five asks A4: `.`, r or e on a list row): the
  // menu takes focus, the arrows walk it, and closing puts focus back.
  let opener: HTMLElement | null = null;
  const items = () => [...(menuEl?.querySelectorAll<HTMLButtonElement>('.ctx-item:not(:disabled)') ?? [])];
  createEffect(() => {
    const m = contextMenu();
    if (!m) {
      const back = opener;
      opener = null;
      if (back?.isConnected) queueMicrotask(() => back.focus());
      return;
    }
    const hint = takeMenuFocus();
    if (!hint) return;
    opener = document.activeElement as HTMLElement | null;
    queueMicrotask(() => {
      const list = items();
      const target = hint === true ? list[0] : list.find(b => hint.test(b.textContent?.trim() ?? '')) ?? list[0];
      target?.focus();
    });
  });
  const onMenuKey = (e: KeyboardEvent) => {
    const list = items();
    const at = list.indexOf(document.activeElement as HTMLButtonElement);
    const to = e.key === 'ArrowDown' || e.key === 'j' ? at + 1 : e.key === 'ArrowUp' || e.key === 'k' ? at - 1
      : e.key === 'Home' ? 0 : e.key === 'End' ? list.length - 1 : null;
    if (to === null) { if (e.key === 'Tab') { e.preventDefault(); closeContextMenu(); } return; }
    e.preventDefault();
    e.stopPropagation();
    list[(to + list.length) % list.length]?.focus();
  };

  // Close on document-level interactions when the menu is open.
  createEffect(() => {
    const m = contextMenu();
    if (!m) return;
    const onDown = (e: MouseEvent) => {
      if (menuEl && !menuEl.contains(e.target as Node)) closeContextMenu();
    };
    const onKey = (e: KeyboardEvent) => {
      // Taken, so a page's own Esc (Tasks clears its selection) doesn't also run.
      if (e.key === 'Escape') { e.preventDefault(); closeContextMenu(); }
    };
    const onScroll = (e: Event) => {
      if (menuEl?.contains(e.target as Node)) return;
      if (scrollMovedAnchor(m)) closeContextMenu();
    };
    const onWheel = (e: Event) => {
      if (menuEl && !menuEl.contains(e.target as Node)) closeContextMenu();
    };
    const onResize = () => closeContextMenu();
    // mousedown (not click) so we close before the underlying element
    // sees a click that would re-open another menu.
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    document.addEventListener('scroll', onScroll, true);
    document.addEventListener('wheel', onWheel, { capture: true, passive: true });
    document.addEventListener('touchmove', onWheel, { capture: true, passive: true });
    window.addEventListener('resize', onResize);
    onCleanup(() => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
      document.removeEventListener('scroll', onScroll, true);
      document.removeEventListener('wheel', onWheel, true);
      document.removeEventListener('touchmove', onWheel, true);
      window.removeEventListener('resize', onResize);
    });
  });

  // After render, nudge the menu so it stays on-screen.
  createEffect(() => {
    const m = contextMenu();
    if (!m || !menuEl) return;
    queueMicrotask(() => {
      if (!menuEl) return;
      // Entrance transforms shrink getBoundingClientRect; use layout size so
      // the final row still fits when the animation reaches full size.
      const width = menuEl.offsetWidth, height = menuEl.offsetHeight;
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      let x = m.x;
      let y = m.y;
      if (x + width + 8 > vw) x = Math.max(8, vw - width - 8);
      if (y + height + 8 > vh) y = Math.max(8, vh - height - 8);
      menuEl.style.left = `${x}px`;
      menuEl.style.top = `${y}px`;
    });
  });

  return (
    <Show when={contextMenu()}>
      <div
        ref={el => (menuEl = el)}
        class="ctx-menu"
        role="menu"
        style={{ left: `${contextMenu()!.x}px`, top: `${contextMenu()!.y}px` }}
        onContextMenu={e => e.preventDefault()}
        onKeyDown={onMenuKey}
      >
        <For each={contextMenu()!.items}>
          {item => <Item item={item} />}
        </For>
      </div>
    </Show>
  );
}

function Item(props: { item: ContextMenuItem }) {
  const it = () => props.item;
  if (it().separator) return <div class="ctx-sep" />;
  return (
    <button
      type="button"
      role="menuitem"
      class={`ctx-item ${it().danger ? 'danger' : ''}`}
      // An item that is off with a reason (its title) stays focusable, so the
      // arrows reach it and a screen reader reads why; it does nothing (P1-17).
      disabled={it().disabled && !it().title}
      aria-disabled={it().disabled && it().title ? 'true' : undefined}
      title={it().title}
      aria-description={it().disabled ? it().title : undefined}
      // Native, so a click on an off item stops here, as a disabled button's
      // would, before App's document click-to-close sees it.
      on:click={e => { if (it().disabled) { e.preventDefault(); e.stopPropagation(); } }}
      onClick={async () => {
        if (it().disabled) return;
        const fn = it().action;
        closeContextMenu();
        if (fn) await fn();
      }}
    >
      <span class="ctx-label">{it().label}</span>
      <Show when={it().shortcut}>
        <span class="ctx-shortcut">{it().shortcut}</span>
      </Show>
    </button>
  );
}
