// A saved canvas's "Attach to issue…" (A19). The person finds an issue of
// the tracker's team in the local mirror, picks it, and confirms on Hollow's
// card; Hollow then uploads the canvas's HTML and attaches it to the issue
// (`attach_canvas_to_issue`). The canvas stays in Hollow; the issue gets a
// copy. Attaching the same version to the same issue again adds nothing.
import { For, Show, createEffect, createResource, createSignal, on, onCleanup, onMount } from 'solid-js';
import { holdFocus } from '../focusHold';
import { commandPaletteOpen, confirmDialog, showConfirm } from '../store';
import { attachCanvasTarget, closeAttachCanvas } from '../store/canvasAttach';
import { attachCanvasToIssue, trackerStatus, trackerView } from '../trackerApi';
import type { TrackerViewRow } from '../generated/ipc';
import { errMsg } from '../utils';
import { createExitPhase } from './exit';
import { ErrorText, PlainErrorText } from './ui';
import '../styles/canvas-attach.css';

export function AttachCanvasDialog() {
  const open = () => !!attachCanvasTarget();
  const phase = createExitPhase(open, 140);
  const [query, setQuery] = createSignal('');
  const [debounced, setDebounced] = createSignal('');
  const [issue, setIssue] = createSignal<TrackerViewRow | null>(null);
  const [sending, setSending] = createSignal(false);
  const [said, setSaid] = createSignal('');
  const [error, setError] = createSignal('');
  let timer: ReturnType<typeof setTimeout> | undefined;
  createEffect(on(query, q => { clearTimeout(timer); timer = setTimeout(() => setDebounced(q.trim()), 200); }));
  onCleanup(() => clearTimeout(timer));
  // A new canvas starts over.
  createEffect(on(attachCanvasTarget, () => { setQuery(''); setDebounced(''); setIssue(null); setSaid(''); setError(''); }));
  const close = () => { if (!sending()) closeAttachCanvas(); };

  const [status] = createResource(() => (open() ? true : undefined), () => trackerStatus().catch(() => null));
  const name = () => status()?.displayName ?? 'the tracker';
  const [issues] = createResource(() => (open() ? { q: debounced() } : undefined),
    ({ q }) => trackerView({ search: q || null, limit: 30 }).then(v => v.rows.filter(r => !r.archived)));

  let card: HTMLDivElement | undefined;
  let release: (() => void) | undefined;
  createEffect(on(open, isOpen => {
    if (isOpen) queueMicrotask(() => { if (!card || !open()) return; release?.(); release = holdFocus(card); card.querySelector<HTMLInputElement>('input')?.focus(); });
    else { release?.(); release = undefined; }
  }));
  onCleanup(() => release?.());
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!open() || confirmDialog() || commandPaletteOpen()) return;
      if (e.key === 'Escape') { e.preventDefault(); e.stopImmediatePropagation(); close(); }
    };
    document.addEventListener('keydown', onKey, true);
    onCleanup(() => document.removeEventListener('keydown', onKey, true));
  });

  async function attach() {
    const t = attachCanvasTarget(), i = issue();
    if (!t || !i || sending()) return;
    // Hollow's confirm card: the outward write, named in full.
    const ok = await showConfirm({
      title: `Attach “${t.title || 'Canvas'}” to ${i.key}?`,
      body: `This uploads a copy of the canvas (v${t.revision}, its HTML) to ${name()} and attaches it to ${i.key}: ${i.title}. Everyone on the team can open it there. The canvas stays in Hollow.`,
      confirmLabel: `Attach to ${i.key}`,
      focusCancel: true,
    });
    if (!ok || attachCanvasTarget() !== t || issue() !== i) return;
    setSending(true); setError(''); setSaid('');
    try {
      const out = await attachCanvasToIssue(t.workspaceId, t.canvasId, t.revision, i.id);
      if (attachCanvasTarget() === t) setSaid(out.note);
    } catch (e) {
      if (attachCanvasTarget() === t) setError(errMsg(e));
    } finally { setSending(false); }
  }

  return (
    <Show when={phase() !== 'closed'}>
      <div class="rbk-backdrop" classList={{ 'is-closing': phase() === 'closing' }} onClick={(e) => { if (e.target === e.currentTarget) close(); }}>
        <div class="rbk-card aci-card" ref={card} tabIndex={-1} role="dialog" aria-modal="true" aria-labelledby="aci-title">
          <div class="rbk-head">
            <h2 id="aci-title">Attach “{attachCanvasTarget()?.title || 'Canvas'}” to an issue</h2>
            <button type="button" class="btn ghost" onClick={close} disabled={sending()} aria-label="Close (esc)">Close</button>
          </div>
          <div class="aci-body">
            <label class="aci-label" for="aci-search">Issue</label>
            <input id="aci-search" class="input" type="search" placeholder={`Find an issue in ${name()} by key or title`} value={query()} onInput={(e) => setQuery(e.currentTarget.value)} />
            <span class="sr-only" role="status">{issues.loading ? 'Searching issues…' : issues() ? `${issues()!.length} issues` : ''}</span>
            <Show when={issues.error}><ErrorText class="rbk-note is-error" text={`Couldn’t read the issues: ${errMsg(issues.error)}`} /></Show>
            <ul class="aci-issues" aria-label="Issues">
              <For each={issues() ?? []} fallback={<li class="rbk-note">{issues.loading ? 'Searching…' : 'No issues match. Sync the tracker if one is missing.'}</li>}>
                {(row) => (
                  <li>
                    <button type="button" class="aci-issue" classList={{ on: issue()?.id === row.id }} aria-pressed={issue()?.id === row.id} disabled={sending()}
                      onClick={() => { setIssue(row); setSaid(''); setError(''); }}><span class="aci-key">{row.key}</span><span>{row.title}</span></button>
                  </li>
                )}
              </For>
            </ul>
            <p class="rbk-note">The issue gets a copy of this version’s HTML; the canvas stays in Hollow. Nothing is sent until you confirm.</p>
          </div>
          <div class="aci-foot">
            <span class="aci-said" role="status" classList={{ 'is-error': !!error() }}>{error() ? <PlainErrorText text={`Couldn’t attach it: ${error()}`} /> : said()}</span>
            <button type="button" class="btn primary" disabled={!issue() || sending()} onClick={() => void attach()}>{sending() ? 'Attaching…' : issue() ? `Attach to ${issue()!.key}…` : 'Attach…'}</button>
          </div>
        </div>
      </div>
    </Show>
  );
}
