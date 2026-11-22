// Meeting prep pack (overnight 10-11 A4): who's coming, what's open with
// them, related work, earlier notes, and open questions and talking points
// your agent wrote from those (each point cites its sources). Opened from
// Today's Prepare and the Agenda's Prep pack; saved as a note in the
// matching workspace's outputs/prep/.
import { For, Show, createEffect, on, onCleanup, onMount } from 'solid-js';
import { holdFocus } from '../focusHold';
import { commandPaletteOpen, confirmDialog } from '../store';
import { openVaultNote } from '../store/runtime';
import { closePrepPack, packBody, prepBusy, prepError, prepMeeting, prepPack, prepPackOpen, prepRecap, useRecapRecording, retryPrepPack, rewritePrepPack } from '../store/prepPack';
import { runGoTarget } from '../goTo';
import { createExitPhase } from './exit';
import { NoteRenderer } from './NoteRenderer';
import { Icon } from '../icons';
import '../styles/ask-hollow.css';
import { ErrorText } from './ui';

export function PrepPackDialog() {
  const isRecap = () => prepMeeting()?.kind === 'recap';
  const choices = () => { const r = prepRecap(); return r?.kind === 'needsRecording' ? r : null; };
  const counts = () => { const r = prepRecap(); return r?.kind === 'recap' ? r : null; };
  /** Opening a note leaves the pack. Today was cut (overnight 11-08 S3). */
  const leave = () => { closePrepPack(); };
  const phase = createExitPhase(prepPackOpen, 140);
  let card: HTMLDivElement | undefined;
  let release: (() => void) | undefined;
  createEffect(on(prepPackOpen, isOpen => {
    if (isOpen) queueMicrotask(() => { if (!card || !prepPackOpen()) return; release?.(); release = holdFocus(card); card.focus(); });
    else { release?.(); release = undefined; }
  }));
  onCleanup(() => release?.());
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!prepPackOpen() || confirmDialog() || commandPaletteOpen()) return;
      if (e.key === 'Escape') { e.preventDefault(); e.stopImmediatePropagation(); closePrepPack(); }
    };
    document.addEventListener('keydown', onKey, true);
    onCleanup(() => document.removeEventListener('keydown', onKey, true));
  });
  const when = () => {
    const p = prepPack();
    return p?.start ? new Date(p.start).toLocaleString([], { weekday: 'short', hour: 'numeric', minute: '2-digit' }) : '';
  };
  return (
    <Show when={phase() !== 'closed'}>
      <div class="rbk-backdrop" classList={{ 'is-closing': phase() === 'closing' }} onClick={(e) => { if (e.target === e.currentTarget) closePrepPack(); }}>
        <div class="ask-card prep-card" ref={card} tabIndex={-1} role="dialog" aria-modal="true" aria-labelledby="prep-title">
          <div class="ask-head">
            <span class="ask-mark" aria-hidden="true"><Icon name="calendar" size={16} /></span>
            <h2 id="prep-title" class="prep-title">{isRecap() ? 'Recap' : 'Prep'}: {prepMeeting()?.title}<Show when={when()}><small> · {when()}</small></Show></h2>
            <button type="button" class="btn ghost" onClick={closePrepPack} aria-label={isRecap() ? 'Close the meeting recap (esc)' : 'Close the prep pack (esc)'}>Close</button>
          </div>
          <span class="sr-only" role="status">{prepBusy() ? (isRecap() ? 'Reading the recording…' : 'Preparing the pack…') : prepPack() ? (isRecap() ? 'The meeting recap is ready.' : 'The prep pack is ready.') : ''}</span>
          <div class="ask-body" aria-busy={prepBusy()}>
            <Show when={prepBusy() && !prepPack()}>
              <div class="ask-thinking" aria-hidden="true"><span /><span /><span /></div>
              <p class="ask-hint">{isRecap() ? 'Reading the recording…' : "Reading who's coming, what's open with them, related work and earlier notes, then writing talking points…"}</p>
            </Show>
            <Show when={prepError()}>
              <ErrorText class="ask-note is-error" text={prepError()} />
            </Show>
            <Show when={choices()}>{r => <section aria-label="Choose a meeting recording"><h3>Recordings from {r().day}</h3><For each={r().candidates} fallback={<p>No Plaud recordings that day.</p>}>{c => <p>{c.name} · {c.duration ?? 'Duration unavailable'} <button type="button" class="btn" disabled={prepBusy()} onClick={() => useRecapRecording(c.id)}>Use this recording</button></p>}</For></section>}</Show>
            <Show when={prepPack()}>{p => <div class="prep-body"><NoteRenderer content={packBody(p().markdown)} onWikilinkClick={name => { leave(); openVaultNote(name.endsWith('.md') ? name : `${name}.md`); }} /></div>}</Show>
            <Show when={counts()}>{r => <Show when={!r().kept}><p class="ask-note">{r().queued} {r().queued === 1 ? 'proposal' : 'proposals'} added to Needs you<Show when={r().dropped}> · {r().dropped} dropped</Show><Show when={r().queued === 0}>. Unchanged transcript proposals aren't added again.</Show></p></Show>}</Show>
          </div>
          <div class="ask-actions">
            <Show when={counts()}><button type="button" class="btn ghost" onClick={() => { leave(); runGoTarget('waiting'); }}>Open Needs you</button></Show>
            <Show when={prepPack()}>{p => <>
              <span class="ask-meta" title={p().path}>{p().kept ? 'Kept from earlier' : 'Written now'}<Show when={!isRecap()}> · `{p().path}`</Show></span>
            </>}</Show>
            <button type="button" class="btn ghost" aria-disabled={prepBusy()} onClick={() => { if (!prepBusy()) prepPack() ? rewritePrepPack() : retryPrepPack(); }}>{prepBusy() ? 'Preparing…' : prepPack() ? 'Write it again' : 'Try again'}</button>
            <Show when={prepPack()}>{p => <button type="button" class="btn" onClick={() => { const path = p().path; leave(); openVaultNote(path); }}>Open as a note</button>}</Show>
            <button type="button" class="btn primary" onClick={closePrepPack}>Done</button>
          </div>
        </div>
      </div>
    </Show>
  );
}
