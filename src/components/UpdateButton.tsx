import { Show, createEffect, createSignal, createUniqueId, onCleanup, onMount } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Icon } from '../icons';
import {
  updateReady, updateRelease, updateWaiting, updateWork, updateError, updateState, updateInstalling, clearUpdateError,
  restartToUpdate, updateWhenChatsFinish, cancelUpdateWait, dismissUpdate, startUpdates,
  lastUpdateResult, ackLastUpdateResult,
} from '../store/updates';
import { flashWithUndo } from '../store/snackbars';
import { openSettingsFromOutside } from '../store';
import { updateResultText } from '../updatesText';
import { ErrorText } from './ui';
import '../styles/update-button.css';

let notifiedUpdateAt: string | undefined;

/** Nonmodal update disclosure. Installation decisions stay in the update store. */
export function UpdateButton() {
  const [open, setOpen] = createSignal(false);
  const [workRunning, setWorkRunning] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const id = `hollow-update-${createUniqueId()}`;
  let trigger: HTMLButtonElement | undefined;
  let panel: HTMLDivElement | undefined;
  let wasWaiting = false;
  createEffect(() => {
    const state = updateState();
    if (updateWork().length === 0 || state?.phase.phase !== 'ready' || (wasWaiting && !state.waiting)) setWorkRunning(false);
    wasWaiting = state?.waiting ?? false;
  });
  const installing = () => updateState()?.phase.phase === 'installing' || updateInstalling();
  createEffect(() => {
    if (open() && installing()) queueMicrotask(() => panel?.focus());
  });
  createEffect(() => {
    const result = lastUpdateResult();
    if (!result || notifiedUpdateAt === result.at) return;
    notifiedUpdateAt = result.at;
    flashWithUndo({
      message: updateResultText(result),
      kind: result.kind === 'updated' ? 'status' : 'error',
      ttlMs: 0,
      actionLabel: 'Open Updates',
      undo: () => { openSettingsFromOutside('updates'); },
      // The action also dismisses through performUndo, acknowledging once.
      onDismiss: () => { void ackLastUpdateResult(); },
    });
  });
  const hasWork = () => updateWork().length > 0 || workRunning();
  function close(restore = false) {
    setOpen(false);
    if (restore) trigger?.focus();
  }
  function toggle() {
    if (open()) { close(); return; }
    clearUpdateError();
    setOpen(true);
    queueMicrotask(() => panel?.focus());
  }
  async function act(action: () => Promise<unknown>) {
    if (busy()) return;
    setBusy(true);
    try { await action(); } finally { setBusy(false); }
  }
  async function restart() {
    const outcome = await restartToUpdate();
    setWorkRunning(outcome?.outcome === 'workRunning' || outcome?.outcome === 'workChanged');
  }
  function cancelWait() {
    setWorkRunning(false);
    return cancelUpdateWait();
  }
  onMount(() => {
    const stop = startUpdates();
    const outside = (event: Event) => {
      if (open() && !trigger?.contains(event.target as Node) && !panel?.contains(event.target as Node)) close();
    };
    const escape = (event: KeyboardEvent) => {
      if (open() && event.key === 'Escape' && !event.defaultPrevented) {
        event.preventDefault(); event.stopPropagation(); close(true);
      }
    };
    document.addEventListener('pointerdown', outside);
    document.addEventListener('focusin', outside);
    document.addEventListener('keydown', escape);
    onCleanup(() => {
      stop();
      document.removeEventListener('pointerdown', outside);
      document.removeEventListener('focusin', outside);
      document.removeEventListener('keydown', escape);
    });
  });
  return <Show when={updateReady() || installing()}>
    <button ref={trigger} type="button" class="update-button" aria-haspopup="dialog"
      aria-label={updateWaiting() ? 'Update waiting for chats' : 'Update'}
      aria-expanded={open()} aria-controls={open() ? id : undefined} onClick={toggle}>
      <Icon name="refresh" size={14} />
      <span>Update<Show when={updateWaiting()}><span class="update-waiting-label"> waiting for chats</span></Show></span>
    </button>
    <Show when={open()}><Portal>
      <div ref={panel} id={id} class="update-popover" role="dialog" aria-labelledby={`${id}-heading`} tabindex="-1">
        <Show when={updateRelease()}>{release => <>
          <header class="update-popover-heading">
            <Show when={release().local} fallback={
              <h2 id={`${id}-heading`}><span class="sr-only">Hollow update: </span>Hollow {release().version} {updateState()?.phase.phase === 'ready' || updateState()?.phase.phase === 'installing' ? 'is ready' : 'is being prepared'}</h2>
            }>{local => <h2 id={`${id}-heading`}><span class="sr-only">Hollow update: </span>Local build of {local().branch}</h2>}</Show>
            <span class="update-channel">{release().local ? 'Local' : release().channel === 'nightly' ? 'Nightly' : 'Main'}</span>
          </header>
          <Show when={release().local}>{local => <p>Version {release().version}, commit {(release().commit ?? '').slice(0, 12)}{local().dirty ? ', with uncommitted changes' : ''}</p>}</Show>
          <Show when={release().downgrade}><p>This is an older version than the one you have. You switched channel.</p></Show>
          <div class="update-notes" tabindex="0" role="region" aria-label="Release notes">{release().notes}</div>
        </>}</Show>
        <Show when={updateWaiting()} fallback={
          <Show when={hasWork()}><p role="status">Chats are running: {updateWork().map(work => work.name).join(', ') || 'Checking running chats'}</p></Show>
        }>
          <p role="status">Waiting for chats to finish. Hollow restarts by itself when they end, however long that takes.</p>
        </Show>
        <Show when={updateError()}>{error => <ErrorText text={error()} class="update-error" />}</Show>
        <Show when={installing()} fallback={<div class="update-actions">
          <Show when={updateWaiting()} fallback={
            <button type="button" class="btn primary" disabled={busy()}
              onClick={() => void act(hasWork() ? updateWhenChatsFinish : restart)}>
              {hasWork() ? 'Update when chats finish' : 'Restart to update'}
            </button>
          }>
            <button type="button" class="btn" disabled={busy()} onClick={() => void act(cancelWait)}>Cancel</button>
          </Show>
          <Show when={!updateWaiting()}><button type="button" class="btn ghost" disabled={busy()} onClick={() => {
            close(true);
            void act(dismissUpdate);
          }}>Not now</button></Show>
        </div>}>
          <p role="status">Restarting Hollow to update...</p>
        </Show>
      </div>
    </Portal></Show>
  </Show>;
}
