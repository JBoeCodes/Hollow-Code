import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js';
import { confirmDialog, resolveConfirm } from '../store';
import { createExitPhase } from './exit';

/// Global confirm dialog. Mounted once at the App level; renders when
/// `confirmDialog()` is non-null (set by `showConfirm` from the store).
/// Keyboard: Esc cancels; Enter activates the focused choice. Clicking the
/// backdrop cancels. Confirm receives initial focus for ordinary dialogs;
/// danger dialogs focus Cancel so Enter leaves the action unapplied.
export function ConfirmDialog() {
  let confirmBtnRef: HTMLButtonElement | undefined;
  let cancelBtnRef: HTMLButtonElement | undefined;
  let alternateBtnRef: HTMLButtonElement | undefined;
  let inputRef: HTMLInputElement | undefined;
  let bodyRef: HTMLParagraphElement | undefined;
  // A body taller than its box scrolls; it then takes focus so the keyboard
  // can scroll it before choosing (a merge's file list, a persona).
  const [bodyScrolls, setBodyScrolls] = createSignal(false);
  const [value, setValue] = createSignal('');
  // A checklist dialog's ticked ids (sent back as JSON in `value`).
  const [ticked, setTicked] = createSignal<string[]>([]);
  const checkRefs: HTMLInputElement[] = [];
  // The aside's per-row buttons (P1-40: agent-started chats to open).
  const asideRefs: HTMLButtonElement[] = [];
  const blocked = () => {
    const d = confirmDialog() ?? held();
    return !!d?.hideConfirm || !!d?.blockedReason || (!!d?.input && !value().trim()) || (!!d?.checklist && ticked().length === 0);
  };
  const accept = () => {
    const d = confirmDialog();
    if (!d || blocked()) return;
    resolveConfirm(true, d.checklist ? JSON.stringify(ticked()) : value());
  };
  const toggle = (id: string, on: boolean) => setTicked(t => on ? [...t.filter(x => x !== id), id] : t.filter(x => x !== id));

  // Exit choreography — the dialog state nulls on resolve, so hold the
  // last non-null value while the 140ms exit plays.
  const phase = createExitPhase(() => !!confirmDialog(), 140);
  const [held, setHeld] = createSignal<ReturnType<typeof confirmDialog>>(null);
  createEffect(() => {
    const d = confirmDialog();
    if (d) { setHeld(d); setValue(d.input?.value ?? ''); setTicked(d.checklist?.filter(r => r.checked).map(r => r.id) ?? []); checkRefs.length = 0; asideRefs.length = 0; }
  });

  // Global key handler — only active while a dialog is open.
  createEffect(() => {
    const d = confirmDialog();
    if (!d) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        resolveConfirm(false);
      } else if (e.key === 'Enter') {
        // Don't fire if the user is mid-text-entry somewhere (shouldn't
        // happen since we focus a button, but defensive against future
        // dialogs with form inputs).
        const tag = (document.activeElement?.tagName || '').toLowerCase();
        if (d.input && document.activeElement === inputRef) { e.preventDefault(); accept(); return; }
        if (tag === 'input' || tag === 'textarea') return;
        e.preventDefault();
        // On the scrolling body, Enter chooses nothing: only a button does.
        if (bodyRef && document.activeElement === bodyRef) return;
        // An aside row's own button does its own thing, never the confirm.
        const own = asideRefs.find(b => b === document.activeElement);
        if (own) { own.click(); return; }
        // Enter must activate the focused choice, including Cancel. Danger
        // dialogs intentionally focus Cancel to prevent an accidental action.
        if (document.activeElement === cancelBtnRef) resolveConfirm(false);
        else if (document.activeElement === alternateBtnRef) resolveConfirm(false, 'alternate');
        else accept();
      } else if (e.key === 'Tab') {
        // Simple focus trap: [input | ticks] → aside rows → Cancel → alternate → Confirm.
        e.preventDefault();
        const order = [bodyScrolls() ? bodyRef : undefined, inputRef, ...checkRefs, ...asideRefs, cancelBtnRef, alternateBtnRef, confirmBtnRef].filter(el => el && el.isConnected && !(el as HTMLButtonElement).disabled) as HTMLElement[];
        const at = order.indexOf(document.activeElement as HTMLElement);
        order[(at + (e.shiftKey ? order.length - 1 : 1)) % order.length]?.focus();
      }
    };
    document.addEventListener('keydown', onKey, true);
    // What had focus before; it gets it back when the dialog closes
    // (overnight B8), unless the choice took the person somewhere else.
    const before = d.returnFocus ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    // For danger dialogs, focus Cancel so a stray Enter doesn't fire the
    // destructive action. For non-danger, focus Confirm — that's the
    // expected default-accept behavior.
    queueMicrotask(() => {
      setBodyScrolls(!!bodyRef && bodyRef.scrollHeight > bodyRef.clientHeight + 1);
      if (d.input && inputRef) { inputRef.focus(); inputRef.select(); return; }
      // A checklist starts on its first tick, so a stray Enter applies nothing,
      // unless it asks for Cancel (a costly confirm: resuming chats, P1-40).
      if (d.checklist && checkRefs[0] && !d.focusCancel) { checkRefs[0].focus(); return; }
      (d.hideConfirm ? asideRefs[0] : d.danger || d.focusCancel ? cancelBtnRef : confirmBtnRef)?.focus();
    });
    onCleanup(() => {
      document.removeEventListener('keydown', onKey, true);
      queueMicrotask(() => {
        const now = document.activeElement;
        const stayed = !now || now === document.body || !!now.closest('.confirm-dialog');
        if (!confirmDialog() && stayed && before?.isConnected && !before.closest('.confirm-dialog')) before.focus({ preventScroll: true });
      });
    });
  });

  return (
    <Show when={phase() !== 'closed' ? held() : null}>
      {(dialog) => (
        <div class="confirm-backdrop" classList={{ 'is-closing': phase() === 'closing' }} onClick={() => resolveConfirm(false)}>
          <div
            class={`confirm-dialog ${dialog().danger ? 'danger' : ''}`}
            role="dialog"
            aria-modal="true"
            aria-labelledby="confirm-title"
            aria-describedby={[dialog().summary ? 'confirm-summary' : dialog().body ? 'confirm-body' : '', dialog().blockedReason ? 'confirm-blocked' : ''].filter(Boolean).join(' ') || undefined}
            onClick={(e) => e.stopPropagation()}
          >
            <h2 id="confirm-title" class="confirm-title">{dialog().title}</h2>
            <Show when={dialog().summary}><p class="confirm-body" id="confirm-summary">{dialog().summary}</p></Show>
            <Show when={dialog().blockedReason}><p class="confirm-body" id="confirm-blocked" role="note">{dialog().blockedReason}</p></Show>
            <Show when={dialog().body}>
              <p class="confirm-body" id="confirm-body" ref={bodyRef} tabIndex={bodyScrolls() ? 0 : undefined}>{dialog().body}</p>
            </Show>
            <Show when={dialog().input}>
              {(input) => (
                <label class="confirm-input">
                  <span>{input().label}</span>
                  <input
                    ref={inputRef}
                    type={input().type ?? 'text'}
                    min={input().min}
                    aria-label={input().label}
                    value={value()}
                    placeholder={input().placeholder}
                    maxLength={input().maxLength}
                    onInput={(e) => setValue(e.currentTarget.value)}
                  />
                </label>
              )}
            </Show>
            <Show when={dialog().checklist}>
              {(rows) => (
                <div class="confirm-checklist" role="group" aria-label={dialog().title}>
                  <For each={rows()}>
                    {(row) => (
                      <label class="confirm-check">
                        <input
                          ref={(el) => checkRefs.push(el)}
                          type="checkbox"
                          checked={ticked().includes(row.id)}
                          onChange={(e) => toggle(row.id, e.currentTarget.checked)}
                        />
                        <span class="confirm-check-text">
                          <span>{row.label}</span>
                          <Show when={row.detail}><small>{row.detail}</small></Show>
                        </span>
                      </label>
                    )}
                  </For>
                </div>
              )}
            </Show>
            <Show when={dialog().aside}>
              {(aside) => (
                <section class="confirm-aside" aria-labelledby="confirm-aside-heading">
                  <h3 id="confirm-aside-heading" class="confirm-aside-heading">{aside().heading}</h3>
                  <Show when={aside().note}><p class="confirm-aside-note">{aside().note}</p></Show>
                  <ul class="confirm-checklist">
                    <For each={aside().rows}>
                      {(row) => (
                        <li class="confirm-check">
                          <span class="confirm-check-text">
                            <span>{row.label}</span>
                            <Show when={row.detail}><small>{row.detail}</small></Show>
                          </span>
                          <button ref={(el) => asideRefs.push(el)} class="btn ghost small" aria-label={`${aside().actionLabel} ${row.label}`}
                            onClick={() => { if (!confirmDialog()) return; const run = aside().onAction; resolveConfirm(false); run(row.id); }}>{aside().actionLabel}</button>
                        </li>
                      )}
                    </For>
                  </ul>
                </section>
              )}
            </Show>
            <div class="confirm-actions">
              <button
                ref={cancelBtnRef}
                class="btn ghost"
                onClick={() => resolveConfirm(false)}
              >{dialog().cancelLabel ?? 'Cancel'}</button>
              <Show when={dialog().alternateLabel}>{label => <button ref={alternateBtnRef} class="btn ghost" onClick={() => resolveConfirm(false, 'alternate')}>{label()}</button>}</Show>
              <Show when={!dialog().hideConfirm}>
                <button
                  ref={confirmBtnRef}
                  class={`btn ${dialog().danger ? 'danger' : 'primary'}`}
                  disabled={blocked()}
                  onClick={accept}
                >{dialog().confirmLabel ?? 'Confirm'}</button>
              </Show>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
}
