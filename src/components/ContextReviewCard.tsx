/** The review card for everything Hollow found that could become context
 *  (CX20): the context scan's instruction and memory files and the org facts
 *  chats queued. Each item is decided on its own (Accept, Change layer,
 *  Skip); there is no select-all and no bulk accept. Lazy: Library,
 *  Learnings mounts it at the top, and the first-run sheet (H6) can mount it
 *  with a scan id. Claude Code's auto-memory entries (PX6) show why they got
 *  their proposed layer (Home, Personal, Project or Stale). Words in
 *  `contextReview.ts`, requests in `contextReviewApi.ts`. */
import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { createStore } from 'solid-js/store';
import {
  contextReviewAccept, contextReviewList, contextReviewSkip, contextReviewUnskip, contextScanStatus, startContextScan,
  type ContextReviewItem, type ContextReviewLayer, type ContextReviewList,
} from '../contextReviewApi';
import {
  acceptBlocked, acceptedText, choiceFor, choiceText, duplicateText, flagText, itemOrigin, itemTitle, layerLabel, layerOptions, orgConfirm, scanText, sortText, splitItems, withheldText,
} from '../contextReview';
import { showConfirm } from '../store/overlays';
import { errMsg } from '../utils';
import { ErrorText } from './ui';
import { Select } from './Select';
import '../styles/contextReview.css';

/** How often a running scan is asked for its progress. */
const POLL_MS = 700;

export function ContextReviewCard(props: { scanId?: string | null; showScan?: boolean }) {
  const [list, setList] = createSignal<ContextReviewList | null>(null);
  const [scanId, setScanId] = createSignal<string | null>(props.scanId ?? null);
  const [scan, setScan] = createSignal<{ state: 'idle' | 'scanning' | 'ready' | 'cancelled' | 'failed'; found: number; error?: string | null }>({ state: 'idle', found: 0 });
  const [loadError, setLoadError] = createSignal('');
  const [busy, setBusy] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal('');
  /** The layer picked with Change layer, by item id; else its proposed one. */
  const [picked, setPicked] = createStore<Record<string, ContextReviewLayer | undefined>>({});
  const [errors, setErrors] = createStore<Record<string, string | undefined>>({});
  let timer: ReturnType<typeof setTimeout> | undefined;
  let alive = true;
  onCleanup(() => { alive = false; if (timer) clearTimeout(timer); });

  async function load() {
    try {
      const next = await contextReviewList(scanId());
      if (!alive) return;
      setList(next); setLoadError('');
      if (!scanId() && next.scanId) setScanId(next.scanId);
    } catch (e) { if (alive) setLoadError(errMsg(e)); }
  }
  onMount(() => void load());

  async function poll(id: string) {
    try {
      const status = await contextScanStatus(id);
      if (!alive) return;
      if (!status || status.state === 'scanning') { timer = setTimeout(() => void poll(id), POLL_MS); return; }
      setScan({ state: status.state, found: status.found, error: status.error });
      if (status.state === 'ready') { setScanId(id); await load(); }
    } catch (e) { if (alive) setScan({ state: 'failed', found: 0, error: errMsg(e) }); }
  }

  async function startScan() {
    if (scan().state === 'scanning') return;
    setNotice(''); setScan({ state: 'scanning', found: 0 });
    try { const started = await startContextScan(); if (alive) void poll(started.scanId); }
    catch (e) { setScan({ state: 'failed', found: 0, error: errMsg(e) }); }
  }

  const layerOf = (item: ContextReviewItem) => picked[item.id] ?? item.proposedLayer;

  /** Accept ONE item into its chosen layer; Org confirms first (a pull request is outward). */
  async function accept(item: ContextReviewItem) {
    const layer = layerOf(item);
    if (busy() || acceptBlocked(item, layer)) return;
    const choice = choiceFor(item, layer);
    if (!choice) return;
    if (choice.outward && !(await showConfirm(orgConfirm(item, choice)))) return;
    setBusy(item.id); setErrors(item.id, undefined);
    try {
      const done = await contextReviewAccept(item.id, layer, scanId());
      setNotice(acceptedText(done));
      await load();
    } catch (e) { setErrors(item.id, errMsg(e)); }
    finally { setBusy(null); }
  }

  async function skip(item: ContextReviewItem, undo = false) {
    if (busy()) return;
    setBusy(item.id); setErrors(item.id, undefined);
    try { await (undo ? contextReviewUnskip(item.id) : contextReviewSkip(item.id)); await load(); }
    catch (e) { setErrors(item.id, errMsg(e)); }
    finally { setBusy(null); }
  }

  const items = () => list()?.items ?? [];
  const parts = () => splitItems(items());

  const Item = (p: { item: ContextReviewItem }) => {
    const item = () => p.item;
    const layer = () => layerOf(item());
    const choice = () => choiceFor(item(), layer());
    const why = () => acceptBlocked(item(), layer());
    const [changing, setChanging] = createSignal(false);
    return <article class="review-card context-review-item" data-review-id={item().id} aria-label={`${itemTitle(item())}: ${item().relPath}`}>
      <strong>{itemTitle(item())}: {item().relPath}</strong>
      <small class="context-review-path">{itemOrigin(item())} <code>{item().path}</code></small>
      <Show when={duplicateText(item(), items())}>{text => <p role="note">{text()}</p>}</Show>
      <Show when={item().excerpt} fallback={<Show when={withheldText(item().withheld)}>{text => <p class="context-review-withheld">{text()}</p>}</Show>}>
        {text => <blockquote>{text()}</blockquote>}
      </Show>
      <Show when={sortText(item())}>{text => <p class="context-review-sort" role="note">{text()}</p>}</Show>
      <Show when={item().flags.length || item().scanNotes.length}>
        <ul class="context-review-flags" aria-label="Scan flags">
          <For each={item().flags}>{f => <li data-severity={f.severity}>{flagText(f)}</li>}</For>
          <For each={item().scanNotes}>{n => <li>{n}</li>}</For>
        </ul>
      </Show>
      <Show when={!item().skipped} fallback={<div class="work-actions">
        <button class="btn small ghost" disabled={!!busy()} onClick={() => void skip(item(), true)}>Unskip</button>
      </div>}>
        <dl class="review-provenance">
          <dt>Layer</dt><dd>{layerLabel(layer())}{layer() === item().proposedLayer ? ' (suggested)' : ''}</dd>
          <Show when={choice()}>{c => <><dt>Where</dt><dd>{choiceText(item(), c())}</dd></>}</Show>
        </dl>
        <Show when={changing()}>
          <Select label={`Layer for ${item().relPath}`} size="sm" value={layer()} options={layerOptions(item())} onChange={v => { setPicked(item().id, v as ContextReviewLayer); setChanging(false); }}/>
        </Show>
        <Show when={why()}>{text => <p class="context-review-why" role="note">{text()}</p>}</Show>
        <div class="work-actions" role="group" aria-label="Review actions">
          <button class="btn small" disabled={!!busy() || !!why()} onClick={() => void accept(item())}>{busy() === item().id ? 'Working' : choice()?.outward ? 'Accept and open pull request' : 'Accept'}</button>
          <button class="btn small ghost" disabled={!!busy() || !!item().blocked} aria-expanded={changing()} onClick={() => setChanging(v => !v)}>Change layer</button>
          <button class="btn small ghost" disabled={!!busy()} onClick={() => void skip(item())}>Skip</button>
        </div>
      </Show>
      <Show when={errors[item().id]}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
    </article>;
  };

  return <section class="context-review" aria-label="Found context">
    <h3>Found context</h3>
    <p class="settings-row-v">Instructions and memory Hollow found on this Mac, and org facts learned in chats. Nothing is copied until you accept one item.</p>
    <Show when={props.showScan !== false}>
      <div class="work-actions">
        <button class="btn small" disabled={scan().state === 'scanning'} onClick={() => void startScan()}>{scan().state === 'scanning' ? 'Scanning' : 'Scan this Mac'}</button>
      </div>
      <p class="settings-row-v" role="status">{scanText(scan().state, scan().found, scan().error)}</p>
    </Show>
    <Show when={notice()}><p class="context-review-notice" role="status">{notice()}</p></Show>
    <Show when={loadError()}><ErrorText class="work-error" text={loadError()}/></Show>
    <For each={parts().open}>{item => <Item item={item}/>}</For>
    <Show when={list() && !parts().open.length && (list()!.scanId || scan().state === 'ready')}><p class="settings-row-v">Nothing left to review.</p></Show>
    <Show when={parts().skipped.length}>
      <details class="context-review-skipped"><summary>Skipped ({parts().skipped.length})</summary>
        <For each={parts().skipped}>{item => <Item item={item}/>}</For>
      </details>
    </Show>
  </section>;
}

export default ContextReviewCard;
