import { For, Show, createEffect, onCleanup, onMount } from 'solid-js';
import { GO_TARGETS, LEADER, leaderKeyOf } from '../keyboardLeader';
import { shortcutsOverlayOpen, closeShortcutsOverlay, type PageId } from '../store';
import { COMMANDS, MOD, displayChord, keyFor, shortcutSettings, shortcutsEnabled } from '../shortcuts';
import { LOCAL_COMMANDS } from '../localKeys';
import { openSettingsFromOutside } from '../store';
import { createExitPhase } from './exit';
import { keymapSections, pageKeys, type KeySection } from '../keymapSections';


/// The keymap card — press `?` anywhere you're not typing and a lifted
/// stone tablet rises with every shortcut as real keycaps. The app-wide
/// rows are the user's current keys (shortcuts.ts COMMANDS, which Settings,
/// Keyboard shortcuts changes), so they cannot drift from the truth; the
/// page-local keys are the PAGE_KEYS registry in keymapSections.ts (kept in
/// sync with the page listeners, per docs/reference/keyboard-shortcuts.md).
export function ShortcutsOverlay() {
  const phase = createExitPhase(shortcutsOverlayOpen, 140);
  let cardRef: HTMLDivElement | undefined;

  onMount(() => {
    // Capture phase, for two reasons: (1) Escape must stopImmediate-
    // Propagation so it closes ONLY this card — every modal also listens
    // for Escape at the document level, and one press must never reach
    // an open task or note editor behind us (unsaved work loss).
    // (2) The card is an overlay, so list-nav keys (j/k/arrows/Enter)
    // are swallowed while it's open — without this they keep driving
    // the page underneath (Enter would open a TaskModal UNDER the
    // backdrop). Capture on document runs before the pages' window
    // bubble listeners.
    const onKey = (e: KeyboardEvent) => {
      if (!shortcutsOverlayOpen()) return;
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopImmediatePropagation();
        closeShortcutsOverlay();
        return;
      }
      if (['j', 'k', 'ArrowDown', 'ArrowUp', 'Enter'].includes(e.key) && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        e.stopPropagation();
      }
    };
    document.addEventListener('keydown', onKey, true);
    onCleanup(() => document.removeEventListener('keydown', onKey, true));
  });

  // Focus the card on open so it's scrollable by keyboard and the
  // previously-focused element isn't left receiving input.
  createEffect(() => {
    if (phase() === 'open') queueMicrotask(() => cardRef?.focus());
  });

  // Derived, not hand-written (overnight F4): app-wide rows are COMMANDS
  // with the keys in effect now, page-local keys come from PAGE_KEYS
  // (keymapSections.ts); the unit tests check every row has a label.
  const sections = (): KeySection[] => keymapSections(
    COMMANDS.map(c => ({ label: c.label, group: c.group, key: keyFor(c.id) })),
    pageKeys(MOD, keyFor('search')),
    shortcutsEnabled(),
    LOCAL_COMMANDS.map(l => ({ surface: l.surface, label: l.label, key: displayChord(l.chord) })),
    GO_TARGETS.map(t => ({ label: t.label, leader: LEADER, key: leaderKeyOf(t.id, shortcutSettings().leader ?? {}) })),
  );

  const openSettings = () => {
    closeShortcutsOverlay();
    openSettingsFromOutside('shortcuts');
  };

  return (
    <Show when={phase() !== 'closed'}>
      <div
        class="keymap-backdrop"
        classList={{ 'is-closing': phase() === 'closing' }}
        onClick={closeShortcutsOverlay}
      >
        <div
          class="keymap-card"
          ref={cardRef}
          tabindex="-1"
          role="dialog"
          aria-modal="true"
          aria-label="Keyboard shortcuts"
          onClick={e => e.stopPropagation()}
        >
          <div class="keymap-head">
            <h2>Keyboard shortcuts</h2>
            <span class="keymap-esc"><kbd>esc</kbd> close</span>
          </div>
          <Show when={!shortcutsEnabled()}>
            <p class="keymap-note">Shortcuts are off. The arrow keys, Enter and the chat's send keys still work.</p>
          </Show>
          <button type="button" class="keymap-change" onClick={openSettings}>Change shortcuts…</button>
          <div class="keymap-grid">
            <For each={sections()}>
              {sec => (
                <section class="keymap-section">
                  <div class="ledger-cap keymap-cap">{sec.title}</div>
                  <For each={sec.rows}>
                    {row => (
                      <div class="keymap-row">
                        <span class="keymap-label">{row.label}</span>
                        <span class="keymap-keys">
                          <For each={row.keys}>{k => <kbd>{k}</kbd>}</For>
                        </span>
                      </div>
                    )}
                  </For>
                </section>
              )}
            </For>
          </div>
        </div>
      </div>
    </Show>
  );
}
