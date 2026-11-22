// Context sync clash cards (CX6c): every open clash on every root (personal,
// vault), and one card for the selected clash with Keep mine, Keep theirs,
// Keep both and Edit (then Done). Opened from Waiting on you and from the
// Personal and Vault rows of Settings, Sources, Context ("N clashes"); a lazy
// chunk. Words in `syncClashes.ts`, calls in `syncClashesApi.ts`.
import { For, Show, createEffect, createResource, createSignal, on, onCleanup, onMount } from 'solid-js';
import { holdFocus } from '../focusHold';
import type { SyncClash } from '../generated/ipc';
import { choiceBlocked, choiceConfirm, clashKey, clashSummary, clashTitle, rootLabel, sideCaption } from '../syncClashes';
import { resolveSyncClash, syncClashes, type ClashChoice } from '../syncClashesApi';
import { commandPaletteOpen, confirmDialog, openVaultNote, showConfirm } from '../store';
import { closeSyncClashes, syncClashesOpen, syncClashesRoot } from '../store/syncClashes';
import { refreshWaitingOnYou } from '../store/waitingOnYou';
import { errMsg, fmtTime } from '../utils';
import { DiffView } from './DiffView';
import { createExitPhase } from './exit';
import { Button, PlainErrorText } from './ui';

const keyOf = clashKey;

/** One clash: both machines and times, what differs, the first lines of
 *  each version, and the person's four choices. */
export function ClashCard(props: { clash: SyncClash; busy: boolean; choose: (choice: ClashChoice) => void }) {
  const c = () => props.clash;
  const blocked = (choice: ClashChoice) => choiceBlocked(c(), choice);
  const id = () => `clash-${keyOf(c()).replace(/[^a-zA-Z0-9]+/g, '-')}`;
  return (
    <article aria-labelledby={`${id()}-title`}>
      <h3 id={`${id()}-title`} class="rbk-row-title">{clashTitle(c())}</h3>
      <p class="rbk-note">
        {rootLabel(c().root)} · <code>{c().path}</code> keeps this Mac’s version ({c().mineMachine}); <code>{c().copy}</code> holds {c().theirsMachine}’s. Clashed {fmtTime(c().at)}.
      </p>
      <Show when={clashSummary(c())}>{s => <p class="rbk-note">{s()}</p>}</Show>
      <div class="rbk-actions" role="group" aria-label={`Resolve ${clashTitle(c())}`}>
        <Show when={!c().editing} fallback={
          <>
            <Button variant="primary" size="sm" icon="check" disabled={props.busy} onClick={() => props.choose('done')}>Done</Button>
            <Button size="sm" disabled={props.busy} onClick={() => props.choose('edit')}>Open the file</Button>
          </>
        }>
          <Button variant="primary" size="sm" disabled={props.busy || !!blocked('keep_mine')} aria-describedby={blocked('keep_mine') ? `${id()}-why` : undefined} onClick={() => props.choose('keep_mine')}>Keep mine</Button>
          <Button size="sm" disabled={props.busy || !!blocked('keep_theirs')} aria-describedby={blocked('keep_theirs') ? `${id()}-why` : undefined} onClick={() => props.choose('keep_theirs')}>Keep theirs</Button>
          <Button size="sm" disabled={props.busy} onClick={() => props.choose('keep_both')}>Keep both</Button>
          <Button size="sm" variant="ghost" disabled={props.busy} onClick={() => props.choose('edit')}>Edit</Button>
        </Show>
      </div>
      {/* Said in words, not only by a disabled button. */}
      <Show when={blocked('keep_theirs') ?? blocked('keep_mine')}>{why => <p class="rbk-note" id={`${id()}-why`}>{why()}</p>}</Show>
      <Show when={c().editing}><p class="rbk-note">Editing: merge what you need from the copy into the file, then press Done. Deleting the copy closes this too.</p></Show>
      <Show when={c().mine && c().theirs}>
        <DiffView
          before={c().mine!.lines.join('\n')}
          after={c().theirs!.lines.join('\n')}
          beforeLabel={sideCaption(`This Mac (${c().mineMachine})`, c().mine)}
          afterLabel={sideCaption(c().theirsMachine, c().theirs)}
        />
      </Show>
    </article>
  );
}

export function SyncClashesDialog() {
  const phase = createExitPhase(syncClashesOpen, 140);
  const [list, { refetch }] = createResource(() => syncClashesOpen() || undefined, () => syncClashes());
  const clashes = (): SyncClash[] => {
    const all = list.error ? [] : list() ?? [];
    const only = syncClashesRoot();
    return only ? all.filter(c => c.root === only) : all;
  };
  const [selected, setSelected] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  // Said here, not in the footer: the backdrop covers it.
  const [note, setNote] = createSignal<{ text: string; error: boolean } | null>(null);
  const current = () => clashes().find(c => keyOf(c) === selected());
  let card: HTMLDivElement | undefined;

  createEffect(on(clashes, rows => { if (!rows.some(r => keyOf(r) === selected())) setSelected(rows[0] ? keyOf(rows[0]) : null); }));
  // Tab stays in the card, and closing puts focus back. The list re-reads
  // on each open (its source turns on again).
  let release: (() => void) | undefined;
  createEffect(on(syncClashesOpen, isOpen => {
    if (isOpen) { setNote(null); queueMicrotask(() => { if (!card || !syncClashesOpen()) return; release?.(); release = holdFocus(card); card.focus(); }); }
    else { release?.(); release = undefined; }
  }));
  onCleanup(() => release?.());

  async function choose(choice: ClashChoice) {
    const c = current();
    if (!c || busy()) return;
    const confirm = choiceConfirm(c, choice);
    if (confirm && !(await showConfirm(confirm))) return;
    setBusy(true); setNote(null);
    try {
      const done = await resolveSyncClash(c.root, c.path, c.copy, choice);
      setNote({ text: done.text, error: false });
      refreshWaitingOnYou(true);
      if (choice === 'edit') { closeSyncClashes(); openVaultNote(done.path); return; }
      await refetch();
    } catch (e) {
      setNote({ text: `Couldn’t resolve it: ${errMsg(e)}`, error: true });
      await refetch();
    } finally {
      setBusy(false);
    }
  }

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!syncClashesOpen() || confirmDialog() || commandPaletteOpen()) return;
      if (e.key === 'Escape') { e.preventDefault(); e.stopImmediatePropagation(); closeSyncClashes(); }
    };
    document.addEventListener('keydown', onKey, true);
    onCleanup(() => document.removeEventListener('keydown', onKey, true));
  });

  return (
    <Show when={phase() !== 'closed'}>
      <div class="rbk-backdrop" classList={{ 'is-closing': phase() === 'closing' }} onClick={(e) => { if (e.target === e.currentTarget) closeSyncClashes(); }}>
        <div class="rbk-card" ref={card} tabIndex={-1} role="dialog" aria-modal="true" aria-labelledby="scl-title">
          <div class="rbk-head">
            <h2 id="scl-title">Context sync clashes</h2>
            <span class="rbk-legend">Two Macs changed the same lines. This Mac kept its version; the other Mac’s sits beside it. Choose for each file; the rest keeps syncing.</span>
            <button type="button" class="btn ghost" onClick={() => void refetch()} aria-label="Refresh context sync clashes">Refresh</button>
            <button type="button" class="btn ghost" onClick={closeSyncClashes} aria-label="Close context sync clashes (esc)">Close</button>
          </div>
          <Show when={note()}>{n => <p class="rbk-note" classList={{ 'is-error': n().error }} role={n().error ? 'alert' : 'status'}><PlainErrorText text={n().text} plain={n().error} /></p>}</Show>
          <div class="rbk-body">
            <ul class="rbk-list" role="list" aria-label="Context sync clashes">
              <Show when={list.loading && !list()}><li class="rbk-note"><span role="status">Looking for clashes…</span></li></Show>
              <Show when={list.error}><li class="rbk-note is-error"><span role="alert">Couldn’t read the clashes: <PlainErrorText text={errMsg(list.error)} /></span></li></Show>
              <Show when={!list.loading && !list.error && clashes().length === 0}><li class="rbk-note">No clashes waiting.</li></Show>
              <For each={clashes()}>
                {c => (
                  <li>
                    <button type="button" class="rbk-row" classList={{ 'is-active': keyOf(c) === selected() }} aria-pressed={keyOf(c) === selected()} onClick={() => setSelected(keyOf(c))}>
                      <span class="rbk-row-title">{c.path}</span>
                      <span class="rbk-row-meta">{rootLabel(c.root)} · {c.theirsMachine} · {fmtTime(c.at)}{c.editing ? ' · editing' : ''}</span>
                    </button>
                  </li>
                )}
              </For>
            </ul>
            <section class="rbk-detail">
              <Show when={current()} fallback={<p class="rbk-note">Nothing to resolve.</p>}>
                {c => <ClashCard clash={c()} busy={busy()} choose={choice => void choose(choice)} />}
              </Show>
            </section>
          </div>
        </div>
      </div>
    </Show>
  );
}
