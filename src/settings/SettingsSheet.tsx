// A modal sheet for one of Settings' large tools (Settings PR 5, plan 3.1
// rule 8): the theme editor, the runner setup wizard, the connector form and
// catalog, the self-hosted model form, Add Claude account and teammate roles.
// It is the shared dialog shell (`rbk-backdrop` + `rbk-card`) with Tab held
// inside and Esc to close. Closing puts focus back on the row's own button:
// `returnTo` names it, since WKWebView doesn't focus a button on click, so
// "what had focus before" can be the page body. Mount it under a `<Show>`; the
// body is built only while the sheet is open.
import { onCleanup, onMount, type JSX } from 'solid-js';
import { Portal } from 'solid-js/web';
import { holdFocus } from '../focusHold';
import { confirmDialog } from '../store/overlays';
import '../styles/settings-sheet.css';

export function SettingsSheet(props: {
  title: string;
  onClose: () => void;
  /** The row's button that opened this sheet; it gets focus back on close. */
  returnTo?: () => HTMLElement | null | undefined;
  /** True while a save runs, so a stray Esc or backdrop click can't drop it. */
  locked?: () => boolean;
  /** The sheet's id, used by tests to find it. */
  id?: string;
  children: JSX.Element;
}) {
  let card!: HTMLDivElement;
  // Leaving a field commits it first: a text field saves on `change`, which
  // blur fires, and the sheet's unmount would otherwise drop the typed edit.
  const close = () => {
    if (props.locked?.()) return;
    const active = document.activeElement;
    if (active instanceof HTMLElement && card.contains(active)) active.blur();
    props.onClose();
  };
  onMount(() => {
    const release = holdFocus(card);
    card.focus();
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      // A select list or a confirm on top gets the key first.
      if (document.querySelector('.hs-pop') || confirmDialog()) return;
      e.preventDefault(); e.stopImmediatePropagation(); close();
    };
    document.addEventListener('keydown', key, true);
    onCleanup(() => {
      document.removeEventListener('keydown', key, true);
      release();
      // After this update settles: a row that swaps its button (Set up becomes
      // Set up again) has mounted the new one by then.
      queueMicrotask(() => { const row = props.returnTo?.(); if (row?.isConnected) row.focus(); });
    });
  });
  const titleId = () => `${props.id ?? 'settings-sheet'}-title`;
  return (
    <Portal>
      <div class="rbk-backdrop" onClick={e => { if (e.target === e.currentTarget) close(); }}>
        <div class="rbk-card settings-sheet" id={props.id} ref={card} role="dialog" aria-modal="true" aria-labelledby={titleId()} tabIndex={-1}>
          <div class="rbk-head">
            <h2 id={titleId()}>{props.title}</h2>
            <button type="button" class="btn ghost" disabled={props.locked?.()} onClick={close}>Close</button>
          </div>
          <div class="settings-sheet-body">{props.children}</div>
        </div>
      </div>
    </Portal>
  );
}

/** Remembers the row button that opened a sheet: call `mark` in its click
 *  handler, and hand `get` to the sheet's `returnTo`. `again` is a selector
 *  for the same row's button when the saved change replaced it. */
export function openerRef(again?: string) {
  let el: HTMLElement | null = null;
  let findAgain = again;
  return {
    /** `selector` overrides the constructor's, for one of several rows' buttons. */
    mark: (e: Event, selector?: string) => { el = e.currentTarget instanceof HTMLElement ? e.currentTarget : null; findAgain = selector ?? again; },
    get: (): HTMLElement | null => (el?.isConnected ? el : findAgain ? document.querySelector<HTMLElement>(findAgain) : null),
  };
}
