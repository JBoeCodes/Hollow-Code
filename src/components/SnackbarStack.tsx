import { Portal } from 'solid-js/web';
import { toastMount } from '../toastPlacement';
import { For, Show, createEffect, on, untrack } from 'solid-js';
import { snackbars, leavingSnackbars, dismissSnackbar, performUndo, performSnackbarAction } from '../store';
import { heldSnackbars, holdSnackbar, releaseSnackbar } from '../store/runtime';
import { windowAway } from '../store/noticeTimers';

/// Undo snackbars live inside the uppermost full view, or bottom-left. Each
/// shows the message + an Undo button (when an undo callback is
/// provided) + a close button. Auto-dismiss is handled by the store.
///
/// The circular SVG ring around the close button visualizes the
/// dismiss countdown — its stroke-dashoffset animates from 0 (full
/// circle) to the full circumference (empty) over `ttlMs`, so the user
/// can see exactly when it'll vanish.
export function SnackbarStack() {
  // A move can remove a hovered/focused element without a leave event.
  // Resume only held timers; ordinary countdowns keep their original deadline.
  createEffect(on(toastMount, () => untrack(() => {
    for (const id of heldSnackbars()) {
      releaseSnackbar(id, 'focus'); releaseSnackbar(id, 'hover');
    }
  }), { defer: true }));
  // Ring geometry — small circle inside the 22px close-button slot.
  const R = 9;
  const C = 2 * Math.PI * R;
  return (
    <Portal mount={toastMount()}>
    <div class="snackbar-stack">
      <For each={snackbars()}>
        {(bar) => {
          return (
          <div
            class={`snackbar ${bar.kind ?? ''}`}
            classList={{ 'is-closing': leavingSnackbars().has(bar.id), 'is-held': heldSnackbars().has(bar.id) || windowAway(), 'is-sticky': (bar.ttlMs ?? 6000) <= 0 }}
            /* Focus or the pointer holds the countdown (overnight B8). */
            onFocusIn={() => holdSnackbar(bar.id, 'focus')}
            onFocusOut={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) releaseSnackbar(bar.id, 'focus'); }}
            onMouseEnter={() => holdSnackbar(bar.id, 'hover')}
            onMouseLeave={() => releaseSnackbar(bar.id, 'hover')}
            /* Failures interrupt; confirmations wait their turn. */
            role={bar.kind === 'error' ? 'alert' : 'status'}
          >
            <span class="snackbar-msg-wrap">
              <span class="snackbar-msg">{bar.message}</span>
              {/* The raw error behind a plain-language one (overnight C7). */}
              <Show when={bar.detail}>
                {(detail) => (
                  <details class="snackbar-details">
                    <summary>Details</summary>
                    <code class="snackbar-detail">{detail()}</code>
                  </details>
                )}
              </Show>
            </span>
            <Show when={bar.undo}>
              <button
                class="snackbar-action"
                onClick={() => performUndo(bar.id)}
              >{bar.actionLabel ?? 'Undo'}</button>
            </Show>
            <For each={bar.actions}>{(action, index) => <button class="snackbar-action" onClick={() => void performSnackbarAction(bar.id, index())}>{action.label}</button>}</For>
            <button
              class="snackbar-close"
              onClick={() => dismissSnackbar(bar.id)}
              aria-label="Dismiss"
              title="Dismiss"
            >
              <svg class="snackbar-progress" viewBox="0 0 22 22" aria-hidden="true">
                <circle
                  class="snackbar-progress-track"
                  cx="11" cy="11" r={R}
                />
                <circle
                  class="snackbar-progress-bar"
                  cx="11" cy="11" r={R}
                  stroke-dasharray={String(C)}
                  style={`--c:${C}; --ttl:${bar.ttlMs ?? 6000}ms`}
                />
              </svg>
              <span class="snackbar-close-x">×</span>
            </button>
          </div>
          );
        }}
      </For>
    </div>
    </Portal>
  );
}
