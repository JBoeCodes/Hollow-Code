import { Show, createEffect, createUniqueId, onCleanup } from 'solid-js';
import { confirmDialog, flashFooter, resolveConfirm } from '../store';

/** A failed lazy chunk remains failed in this window. Every later request
 * must settle false too, including prompts/checklists, rather than hang. */
export function ConfirmUnavailable() {
  createEffect(() => {
    const request = confirmDialog();
    if (!request) return;
    queueMicrotask(() => {
      if (confirmDialog() !== request) return;
      resolveConfirm(false);
      const before = request.returnFocus;
      if (!confirmDialog() && before?.isConnected && (!document.activeElement || document.activeElement === document.body)) before.focus({ preventScroll: true });
      flashFooter('× Couldn’t open the confirm dialog; nothing was changed. Reload Hollow and try again.');
    });
  });
  return null;
}

/** The lightweight loading surface owns the keyboard before the full
 * dialog's code arrives. It can only cancel; no action is accepted here. */
export function ConfirmLoading() {
  const id = createUniqueId();
  let cancel: HTMLButtonElement | undefined;
  createEffect(() => {
    const request = confirmDialog();
    if (!request) return;
    const before = request.returnFocus;
    const key = (event: KeyboardEvent) => {
      event.stopPropagation();
      if (event.key === 'Escape' || event.key === 'Enter') { event.preventDefault(); resolveConfirm(false); }
      else if (event.key === 'Tab') { event.preventDefault(); cancel?.focus(); }
    };
    document.addEventListener('keydown', key, true);
    queueMicrotask(() => { if (confirmDialog() === request) cancel?.focus(); });
    onCleanup(() => {
      document.removeEventListener('keydown', key, true);
      queueMicrotask(() => {
        if (!confirmDialog() && before?.isConnected && (!document.activeElement || document.activeElement === document.body)) before.focus({ preventScroll: true });
      });
    });
  });
  return <Show when={confirmDialog()}>{request =>
    <div class="confirm-backdrop" onClick={() => resolveConfirm(false)}>
      <div class="confirm-dialog" role="dialog" aria-modal="true" aria-labelledby={`${id}-title`} aria-describedby={`${id}-body`} onClick={event => event.stopPropagation()}>
        <h2 id={`${id}-title`} class="confirm-title">{request().title}</h2>
        <p id={`${id}-body`} class="confirm-body" role="status">Opening confirmation…</p>
        <div class="confirm-actions"><button ref={cancel} type="button" class="btn" onClick={() => resolveConfirm(false)}>Cancel</button></div>
      </div>
    </div>
  }</Show>;
}
