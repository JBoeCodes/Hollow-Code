// Conflict copies (overnight A10): every iCloud/Finder copy beside its
// original, the diff of the selected pair, and the person's Keep original /
// Keep copy (after a confirm; the other goes to the vault's .trash/), or Not
// a copy (morning M7: never offered again, with Undo). Opened from Waiting on
// you or the palette; a lazy chunk.
import { For, Show, createEffect, createResource, createSignal, on, onCleanup, onMount } from 'solid-js';
import { holdFocus } from '../focusHold';
import { api } from '../api';
import type { ConflictCopy } from '../generated/ipc';
import { copyMeta, keepConfirm } from '../conflictCopies';
import { commandPaletteOpen, confirmDialog, showConfirm } from '../store';
import { closeConflictCopies, conflictCopiesOpen } from '../store/conflictCopies';
import { refreshWaitingOnYou } from '../store/waitingOnYou';
import { errMsg } from '../utils';
import { DiffView } from './DiffView';
import { createExitPhase } from './exit';
import { Button, ErrorText, PlainErrorText } from './ui';

export function ConflictCopiesDialog() {
  const phase = createExitPhase(conflictCopiesOpen, 140);
  const [list, { refetch }] = createResource(() => conflictCopiesOpen() || undefined, () => api.conflictCopies());
  const copies = (): ConflictCopy[] => (list.error ? [] : list() ?? []);
  const [selected, setSelected] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  // Said here, not in the footer: the backdrop covers it (overnight S5).
  const [note, setNote] = createSignal<{ text: string; error: boolean; undo?: () => Promise<void> } | null>(null);
  const [pair, { refetch: refetchPair }] = createResource(() => selected() ?? undefined, (copy) => api.conflictCopyPair(copy));
  // Read both files again (the list may say the same, the files may not).
  const reload = async () => { await refetch(); if (selected()) await refetchPair(); };
  // Only the selected copy's settled pair is shown or acted on.
  const shown = () => {
    if (pair.error || pair.loading) return undefined;
    const p = pair();
    return p && p.copy === selected() ? p : undefined;
  };
  const current = () => copies().find(x => x.copy === selected());
  let card: HTMLDivElement | undefined;

  createEffect(on(copies, rows => { if (!rows.some(r => r.copy === selected())) setSelected(rows[0]?.copy ?? null); }));
  // Tab stays in the card, and closing puts focus back (overnight B8).
  let release: (() => void) | undefined;
  createEffect(on(conflictCopiesOpen, isOpen => {
    if (isOpen) { setNote(null); queueMicrotask(() => { if (!card || !conflictCopiesOpen()) return; release?.(); release = holdFocus(card); card.focus(); }); }
    else { release?.(); release = undefined; }
  }));
  onCleanup(() => release?.());
  // The dialog stays mounted between opens: the list re-reads on its own
  // (its source turns on again); the selected pair is read again here.
  createEffect(on(conflictCopiesOpen, isOpen => { if (isOpen && selected()) void refetchPair(); }, { defer: true }));

  async function keep(keepCopy: boolean) {
    const p = shown();
    const c = copies().find(x => x.copy === selected());
    if (!p || !c || busy()) return;
    if (!(await showConfirm(keepConfirm(c, keepCopy)))) return;
    setBusy(true); setNote(null);
    try {
      const kept = await api.resolveConflictCopy({ copy: p.copy, keepCopy, copyHash: p.copyHash, originalHash: p.originalHash });
      setNote({ text: `Kept ${kept}; the other is in the vault's trash.`, error: false });
      await refetch();
      refreshWaitingOnYou(true);
    } catch (e) {
      setNote({ text: `Couldn't resolve it: ${errMsg(e)}`, error: true });
      // Show what the files hold now, so "look again" means something.
      await reload();
    } finally {
      setBusy(false);
    }
  }

  /** "Not a copy" (morning M7): never offered again; nothing in the vault
   *  changes, so no confirm, and Undo sits in the note (the footer's
   *  snackbar is under the backdrop). */
  async function notACopy() {
    const copy = selected();
    if (!copy || busy()) return;
    setBusy(true); setNote(null);
    try {
      const original = await api.dismissConflictCopy(copy);
      setNote({
        text: `Marked ${copy} as not a copy of ${original}; Hollow won't offer it again.`,
        error: false,
        undo: async () => {
          if (busy()) return;
          setBusy(true); setNote(null);
          try {
            await api.undismissConflictCopy(copy); await refetch(); setSelected(copy); refreshWaitingOnYou(true);
            setNote({ text: `${copy} is offered again.`, error: false });
          } catch (e) { setNote({ text: `Couldn't undo it: ${errMsg(e)}`, error: true }); }
          finally { setBusy(false); }
        },
      });
      await refetch();
      refreshWaitingOnYou(true);
      // Focus goes to Undo, so a second Enter can't mark the next pair.
      queueMicrotask(() => card?.querySelector<HTMLButtonElement>('.ccp-undo')?.focus());
    } catch (e) {
      setNote({ text: `Couldn't mark it: ${errMsg(e)}`, error: true });
      await reload();
    } finally {
      setBusy(false);
    }
  }

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!conflictCopiesOpen() || confirmDialog() || commandPaletteOpen()) return;
      if (e.key === 'Escape') { e.preventDefault(); e.stopImmediatePropagation(); closeConflictCopies(); }
    };
    document.addEventListener('keydown', onKey, true);
    onCleanup(() => document.removeEventListener('keydown', onKey, true));
  });

  return (
    <Show when={phase() !== 'closed'}>
      <div class="rbk-backdrop ccp-backdrop" classList={{ 'is-closing': phase() === 'closing' }} onClick={(e) => { if (e.target === e.currentTarget) closeConflictCopies(); }}>
        <div class="rbk-card" ref={card} tabIndex={-1} role="dialog" aria-modal="true" aria-labelledby="ccp-title">
          <div class="rbk-head">
            <h2 id="ccp-title">Conflict copies</h2>
            <span class="rbk-legend">Copies iCloud or Finder kept beside a note. Keep one; the other goes to the vault's trash.</span>
            <button type="button" class="btn ghost" onClick={() => void reload()} aria-label="Refresh conflict copies">Refresh</button>
            <button type="button" class="btn ghost" onClick={closeConflictCopies} aria-label="Close conflict copies (esc)">Close</button>
          </div>
          <Show when={note()}>{n => <p class="rbk-note" classList={{ 'is-error': n().error }} role={n().error ? 'alert' : 'status'}><PlainErrorText text={n().text} plain={n().error} /><Show when={n().undo}>{undo => <>{' '}<button type="button" class="btn-link ccp-undo" onClick={() => void undo()()}>Undo</button></>}</Show></p>}</Show>
          <div class="rbk-body">
            <ul class="rbk-list" role="list" aria-label="Conflict copies">
              <Show when={list.loading && !list()}><li class="rbk-note"><span role="status">Looking for copies…</span></li></Show>
              <Show when={list.error}><li class="rbk-note is-error"><span role="alert">Couldn't read the vault: <PlainErrorText text={errMsg(list.error)} /></span></li></Show>
              <Show when={!list.loading && !list.error && copies().length === 0}><li class="rbk-note">No conflict copies in the vault.</li></Show>
              <For each={copies()}>
                {c => (
                  <li>
                    <button type="button" class="rbk-row" classList={{ 'is-active': c.copy === selected() }} aria-pressed={c.copy === selected()} onClick={() => setSelected(c.copy)}>
                      <span class="rbk-row-title">{c.copy}</span>
                      <span class="rbk-row-meta">{copyMeta(c)}</span>
                    </button>
                  </li>
                )}
              </For>
            </ul>
            <section class="rbk-detail">
              <Show when={selected()} fallback={<p class="rbk-note">Nothing to resolve.</p>}>
                <div class="rbk-actions">
                  <Button variant="primary" size="sm" icon="check" disabled={busy() || !shown()} onClick={() => void keep(false)}>Keep the original</Button>
                  <Button size="sm" disabled={busy() || !shown() || !!current()?.originalProtected} aria-describedby={current()?.originalProtected ? 'ccp-protected' : undefined} onClick={() => void keep(true)}>Keep the copy</Button>
                  <Button size="sm" variant="ghost" disabled={busy() || !current()} title="Two different notes that only look like a copy (Chapter 2 beside Chapter): never offered again" onClick={() => void notACopy()}>Not a copy</Button>
                </div>
                {/* Said in words, not only a tooltip on a disabled button. */}
                <Show when={current()?.originalProtected}><p class="rbk-note" id="ccp-protected">The original is a protected file, so the copy can't replace it: compare the two and edit it by hand.</p></Show>
                <Show when={pair.loading}><p class="rbk-note" role="status">Reading both…</p></Show>
                <Show when={pair.error}><ErrorText class="rbk-note is-error" text={errMsg(pair.error)} /></Show>
                <Show when={shown()}>
                  {p => <DiffView before={p().originalText} after={p().copyText} beforeLabel={p().original} afterLabel={p().copy} />}
                </Show>
              </Show>
            </section>
          </div>
        </div>
      </div>
    </Show>
  );
}
