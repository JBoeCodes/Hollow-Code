import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import { api } from '../../api';
import { flashFooter } from '../../store';
import { refreshRunQueue, runQueue } from '../../store/runQueue';
import { cancelLabel, isLive, sourceLabel, waitingText } from '../../runQueue';
import type { RunQueueItem } from '../../types';
import { errMsg, fmtTime } from '../../utils';

/** Finished rows listed under the live ones. */
const RECENT_SHOWN = 10;

/// Automations → Run queue (Paperclip PR 7; moved from Settings in PR 4, 11-12): every automation
/// waiting for the vault, in the order the pump will start them, with a
/// Cancel/Skip/Stop action, then the last few that finished. Reads the shared
/// `runQueue` signal (never a resource, so a refetch never blanks the page).
export function RunQueuePanel() {
  onMount(() => refreshRunQueue(0));
  const live = createMemo(() => runQueue().filter(isLive));
  const recent = createMemo(() => runQueue().filter(item => !isLive(item)).slice(0, RECENT_SHOWN));
  const [busy, setBusy] = createSignal<number | null>(null);

  async function act(item: RunQueueItem) {
    setBusy(item.id);
    try {
      await api.cancelQueuedRun(item.id);
      refreshRunQueue(0);
    } catch (e) {
      flashFooter(`× ${errMsg(e)}`, 3500);
    } finally {
      setBusy(null);
    }
  }

  return (
    <div class="run-queue-panel">
      <Show
        when={live().length > 0}
        fallback={<p class="settings-note">Nothing is queued. Automations wait here while another run holds the vault.</p>}
      >
        <For each={live()}>
          {item => (
            <div class="settings-row run-queue-row" data-status={item.status}>
              <div class="settings-row-label">
                <div class="settings-row-k">{item.label}</div>
                <div class="settings-row-v">
                  <span class="settings-state-chip" data-state={item.status === 'running' ? 'ok' : item.heldReason ? 'warn' : undefined}>
                    {sourceLabel(item.source)}
                  </span>{' '}
                  <span class="run-queue-waiting">{waitingText(item)}</span>
                  <Show when={item.reason}>{' · '}{item.reason}</Show>
                </div>
              </div>
              <Show when={cancelLabel(item)}>
                {label => (
                  <div class="settings-actions">
                    <button class="btn" disabled={busy() === item.id} onClick={() => void act(item)}>{label()}</button>
                  </div>
                )}
              </Show>
            </div>
          )}
        </For>
      </Show>
      <Show when={recent().length > 0}>
        <details class="run-queue-recent">
          <summary class="settings-row-v">Recently finished ({recent().length})</summary>
          <For each={recent()}>
            {item => (
              <div class="settings-row run-queue-row" data-status={item.status}>
                <div class="settings-row-label">
                  <div class="settings-row-k">{item.label}</div>
                  <div class="settings-row-v">
                    {sourceLabel(item.source)} · {item.status}
                    <Show when={item.finishedAt}>{' · '}{fmtTime(item.finishedAt)}</Show>
                    <Show when={item.error}>{' · '}{item.error}</Show>
                  </div>
                </div>
              </div>
            )}
          </For>
        </details>
      </Show>
    </div>
  );
}
