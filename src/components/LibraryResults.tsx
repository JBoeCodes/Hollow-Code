import { For, Show, Suspense, batch, createEffect, createMemo, createResource, createSignal, lazy, on, onCleanup, untrack } from 'solid-js';
import { api } from '../api';
import { Icon } from '../icons';
import { Segmented, StatePanel, Skeleton, ErrorText } from './ui';
import { Select } from './Select';
import { relativeWhen } from '../homeReturn';
import { openWorkspaceResult, visitWorkspace, workspaceListLoaded, workspaces } from '../store/workspaces';
import { unsuspended } from '../stableResource';
import { keyedList } from '../keyedList';
import { canvasOpenRequest, canvasStartWorkspace, resultsType, setResultsType } from '../store/libraryMode';
import { takeGalleryCanvas } from '../canvas/galleryRequest';
import { THUMBNAILS, vaultChangeUnderAnyOutputs } from '../resultsCards';
import { LIBRARY_RESULTS_ASK, LIBRARY_RESULTS_CAP, partialNote, resultsCapped } from '../libraryListModel';
import { filterMixedResults, mixedLibraryResults, resetMixedFilters, type MixedResult } from '../libraryResultsModel';
import { canvasStoreListing } from '../libraryCanvasListing';
import type { GalleryItem } from '../canvasGallery';
import { vaultChange } from '../store/vaultChanges';
import { openPreview } from '../store/preview';
import { vaultRoot } from '../store/chat';
import { joinPath } from '../chatFileLinks';
import { latestRequest } from '../latestRequest';
import { errMsg } from '../utils';
import { shortcutsEnabled } from '../shortcuts';
import { commandPaletteOpen, confirmDialog } from '../store';
import { modalOpen } from '../modalGuard';
import { leaderPending } from '../store/leader';
import { attachCanvasTarget } from '../store/canvasAttach';
import { CanvasThumb, ResultThumb } from './ResultThumb';
import { PreviewWhenNear } from './PreviewWhenNear';

const GalleryCanvas = lazy(() => import('./CanvasGallery').then(m => ({ default: m.GalleryCanvas })));
const RESULT_TYPES = [{ value: 'all', label: 'All' }, { value: 'files', label: 'Files' }, { value: 'canvases', label: 'Canvases' }] as const;
let seenOpenRequest = 0;

/** One metadata list, independent file/store coverage, and a lazy selected detail. */
export function LibraryResults() {
  const [list, { refetch }] = createResource(() => api.listAllResults(null, LIBRARY_RESULTS_ASK));
  const snapshot = unsuspended(list);
  const listed = () => list.error ? undefined : snapshot.latest?.slice(0, LIBRARY_RESULTS_CAP);
  const [canvases, setCanvases] = createSignal<GalleryItem[]>([]);
  const [storesLoading, setStoresLoading] = createSignal(true);
  const [failedStores, setFailedStores] = createSignal<string[]>([]);
  const stores = canvasStoreListing(id => api.listCanvases({ workspaceId: id, sessionKey: null }), value => batch(() => {
    setCanvases(value.items); setFailedStores(value.failed); setStoresLoading(false);
  }));
  const readStores = () => {
    if (!workspaceListLoaded()) return;
    setStoresLoading(true);
    stores.load(workspaces().map(w => ({ id: w.id, title: w.title })));
  };
  const workspaceStamp = createMemo(() => JSON.stringify(workspaces().map(w => [w.id, w.title])));
  createEffect(on([workspaceListLoaded, workspaceStamp], () => readStores()));
  const refresh = () => { void refetch(); readStores(); };
  let changeTimer: ReturnType<typeof setTimeout> | undefined;
  let firstChange = 0;
  createEffect(on(vaultChange, change => {
    if (!vaultChangeUnderAnyOutputs(change)) return;
    clearTimeout(changeTimer);
    if (!firstChange) firstChange = Date.now();
    changeTimer = setTimeout(() => { firstChange = 0; refresh(); }, Math.max(0, Math.min(400, firstChange + 2_000 - Date.now())));
  }, { defer: true }));
  const opening = latestRequest();
  onCleanup(() => { clearTimeout(changeTimer); stores.dispose(); opening.cancel(); });
  const loaded = keyedList(createMemo(() => mixedLibraryResults(listed() ?? [], canvases())), r => r.key);
  const byKey = createMemo(() => new Map(loaded().map(r => [r.key, r])));
  const [query, setQuery] = createSignal('');
  const [workspace, setWorkspace] = createSignal('');
  const [selected, setSelected] = createSignal<string | null>(null);
  const [compare, setCompare] = createSignal<{ key: string; request: number } | null>(null);
  const [back, setBack] = createSignal<{ label: () => string; go: () => void } | null>(null);
  const [openError, setOpenError] = createSignal('');
  const filters = () => ({ query: query(), workspace: workspace(), type: resultsType() });
  const shown = createMemo(() => filterMixedResults(loaded(), filters()));
  const keys = createMemo(() => shown().map(r => r.key));
  const current = createMemo(() => shown().find(r => r.key === selected() && r.canvas)
    ?? (!selected() && resultsType() === 'canvases' ? shown().find(r => r.canvas) : null));
  const thumbed = createMemo(() => new Set(shown().filter(r => r.canvas || r.card?.thumb)
    .slice(0, THUMBNAILS - (current() ? 2 : 0)).map(r => r.key)));
  const workspaceOptions = createMemo(() => [{ value: '', label: 'All workspaces' }, ...[...new Map(loaded().map(r => [r.workspaceId, r.workspaceTitle]))]
    .sort((a, b) => a[1].localeCompare(b[1])).map(([value, label]) => ({ value, label }))]);
  let search: HTMLInputElement | undefined;
  let toolbar: HTMLDivElement | undefined;
  let rows: HTMLDivElement | undefined;
  const recover = (scope: 'search' | 'filters') => {
    const next = resetMixedFilters(filters(), scope);
    batch(() => { setQuery(next.query); setWorkspace(next.workspace); setResultsType(next.type); });
    (scope === 'search' ? search : toolbar?.querySelector<HTMLButtonElement>('.library-results-type button'))?.focus();
  };
  createEffect(on(canvasOpenRequest, request => {
    if (!request || request === seenOpenRequest) return;
    seenOpenRequest = request;
    const r = takeGalleryCanvas(), ws = canvasStartWorkspace();
    const key = r ? JSON.stringify(['canvas', r.workspaceId, r.id]) : null;
    batch(() => {
      setWorkspace(ws); setQuery(''); setSelected(key);
      setCompare(r?.compare && key ? { key, request } : null);
      const title = () => workspaces().find(w => w.id === ws)?.title ?? 'the workspace';
      setBack(r?.back ? { label: () => r.back!.label, go: r.back.go }
        : ws ? { label: () => `Back to ${title()}`, go: () => void visitWorkspace(ws) } : null);
    });
  }));
  const open = async (r: MixedResult) => {
    const ticket = opening.start();
    setOpenError('');
    if (compare()?.key !== r.key) setCompare(null);
    if (r.canvas) { setSelected(r.key); return; }
    try {
      const root = await vaultRoot();
      if (!opening.isLatest(ticket)) return;
      if (!root || !r.card) throw new Error('The Hollow folder is unavailable. Try opening this result again.');
      setSelected(null);
      openPreview({ kind: 'file', path: r.card.path.startsWith('/') ? r.card.path : joinPath(root, r.card.path) });
    } catch (e) { if (opening.isLatest(ticket)) setOpenError(errMsg(e)); }
  };
  const closeDetail = (focusKey = current()?.key) => {
    opening.cancel();
    batch(() => { setSelected(null); setCompare(null); });
    queueMicrotask(() => [...(rows?.querySelectorAll<HTMLButtonElement>('[data-result-button]') ?? [])]
      .find(button => button.dataset.resultButton === focusKey)?.focus());
  };
  const move = (e: KeyboardEvent) => {
    if (confirmDialog() || commandPaletteOpen() || modalOpen() || leaderPending() || attachCanvasTarget() || document.querySelector('.hs-pop')) return;
    if (e.metaKey || e.ctrlKey || e.altKey || (e.key.length === 1 && !shortcutsEnabled())) return;
    const target = (e.target as HTMLElement | null)?.closest<HTMLElement>('[data-result-button]');
    if (!target || !rows?.contains(target)) return;
    if (e.key === 'Escape' && current() && resultsType() !== 'canvases') {
      e.preventDefault(); e.stopPropagation(); closeDetail(target.dataset.resultButton); return;
    }
    const step = e.key === 'j' || e.key === 'ArrowDown' ? 1 : e.key === 'k' || e.key === 'ArrowUp' ? -1 : 0;
    if (!step) return;
    const buttons = [...rows.querySelectorAll<HTMLButtonElement>('[data-result-button]')];
    const next = buttons[(buttons.indexOf(target as HTMLButtonElement) + step + buttons.length) % buttons.length];
    e.preventDefault(); next?.focus();
    // Arrow/j/k moves focus only; Enter/click opens a detail. Holding a key
    // must not start body reads and frame construction for every passed row.
  };
  const kindLabel = (r: MixedResult) => r.canvas ? `Canvas${r.canvas.revision === null ? ' · revision unknown' : ` v${r.canvas.revision}`}` : r.card?.kindLabel ?? 'File';
  const reviewLabel = (r: MixedResult) => r.card ? `${r.card.reviewLabel}${r.canvas && r.card.canvas?.revision !== r.canvas.revision ? ' (listed file revision)' : ''}` : 'Review not recorded';
  const pending = () => list.loading || storesLoading() || !workspaceListLoaded();
  const failed = () => !!list.error || failedStores().length > 0;
  return <section class="library-results-view" aria-label="Results" aria-description="Workspace output files and saved canvases, newest first.">
    <div class="library-results-tools" ref={toolbar}>
      <label class="work-search"><Icon name="search" size={15}/><input ref={search} type="search" aria-label="Find a result" placeholder="Find a result by title or workspace…" value={query()} onInput={e => setQuery(e.currentTarget.value)}/></label>
      <Segmented level="secondary" class="library-results-type" label="Result type" value={resultsType()} onChange={setResultsType} items={RESULT_TYPES}/>
      <Select size="sm" class="library-results-workspace" label="Workspace" value={workspace()} onChange={setWorkspace} options={workspaceOptions()}/>
      <span class="library-results-count">{shown().length} of {loaded().length}{partialNote({ loading: pending(), failed: failed(), capped: resultsCapped(list.error ? undefined : snapshot.latest?.length) })}</span>
      <button type="button" class="btn small" aria-label="Refresh results" disabled={list.loading} onClick={refresh}><Icon name="refresh" size={13}/>Refresh</button>
    </div>
    <Show when={back()}>{b => <button type="button" class="btn small ghost library-results-back" onClick={() => b().go()}><Icon name="arrowLeft" size={13}/>{b().label()}</button>}</Show>
    <Show when={pending()}><p class="library-results-coverage" role="status">{!workspaceListLoaded() ? 'Waiting for the workspace list…' : storesLoading() ? 'Reading canvas stores…' : 'Reading result files…'}</p></Show>
    <Show when={list.error}><StatePanel kind="failed" title="Result files couldn’t be listed" error={list.error} actions={<button class="btn small" onClick={() => void refetch()}>Retry files</button>}/></Show>
    <Show when={failedStores().length}><StatePanel kind="failed" title="Some canvas stores couldn’t be read" actions={<button class="btn small" onClick={readStores}>Retry canvases</button>}>{failedStores().join(', ')}. Last known canvases, when available, remain visible; the list may be incomplete or out of date.</StatePanel></Show>
    <Show when={openError()}><ErrorText class="library-results-coverage" text={openError()}/></Show>
    <div class="library-results-body" classList={{ 'has-detail': !!current() }}>
      <div class="library-results-list" ref={rows} role="list" onKeyDown={move}>
        <For each={keys()} fallback={<Show when={!pending()} fallback={<Skeleton rows={4} label="Loading results"/>}>
          <StatePanel kind={loaded().length ? 'filtered' : failed() ? 'failed' : 'empty'} title={loaded().length ? 'Nothing matches' : failed() ? 'Results are unavailable' : 'No results yet'}
            actions={<><Show when={query().trim()}><button class="btn small" onClick={() => recover('search')}>Clear search</button></Show><Show when={workspace() || resultsType() !== 'all'}><button class="btn small" onClick={() => recover('filters')}>Reset filters</button></Show></>}>
            {loaded().length ? 'Clear the search or filters.' : failed() ? 'Retry the failed listings above.' : 'Workspace output files and saved canvases appear here.'}
          </StatePanel>
        </Show>}>
          {key => {
            const initial = untrack(() => byKey().get(key)!);
            const r = createMemo(() => byKey().get(key) ?? initial);
            return <div role="listitem" class="library-result-card" data-result-key={key}>
              <button type="button" class="library-result-row" classList={{ selected: current()?.key === key }} data-result-button={key}
                aria-label={r().title} aria-description={`${r().workspaceTitle}. ${kindLabel(r())}. ${reviewLabel(r())}${r().canvas?.error ? '. Unreadable canvas metadata' : ''}`}
                aria-pressed={r().canvas ? current()?.key === key : undefined} title={`${r().title} · ${r().workspaceTitle} · ${kindLabel(r())} · ${reviewLabel(r())}`} onClick={() => void open(r())}>
                <PreviewWhenNear allowed={thumbed().has(key)}>{enabled => <Show when={r().canvas} fallback={<ResultThumb card={r().card!} workspaceId={r().workspaceId} enabled={enabled}/>}>{c =>
                  <Show when={enabled} fallback={<div class="res-thumb res-thumb-icon" aria-hidden="true"><Icon name="appWindow" size={32}/></div>}><CanvasThumb workspaceId={r().workspaceId} id={c().id} stamp={`${c().revision}:${c().updatedAt ?? ''}`}/></Show>
                }</Show>}</PreviewWhenNear>
                <span class="library-result-main"><strong>{r().title}</strong><small>{r().workspaceTitle} · {kindLabel(r())}{r().modified ? ` · ${relativeWhen(r().modified!)}` : ''}{r().canvas?.error ? ' · unreadable' : ''}</small></span>
                <span class="library-result-review" data-review={r().card?.review} title={reviewLabel(r())}><Icon name={r().card?.review === 'accepted' ? 'check' : r().card?.review === 'changed' ? 'refresh' : r().card?.review === 'ready' ? 'eye' : 'fileText'} size={14}/><span class="sr-only">{reviewLabel(r())}</span></span>
              </button>
              <Show when={r().card && !r().canvas}><button type="button" class="btn small ghost library-result-workspace"
                aria-label={`Open ${r().title} in its workspace`} title="Open in its workspace, with its review bar"
                onClick={() => void openWorkspaceResult(r().workspaceId, r().card!.path).catch(e => setOpenError(errMsg(e)))}><Icon name="folder" size={14}/><span class="sr-only">Open in its workspace</span></button></Show>
            </div>;
          }}
        </For>
      </div>
      <Show when={current()?.key} keyed>{key => {
        const initial = untrack(() => current()!.canvas!);
        const item = () => byKey().get(key)?.canvas ?? initial;
        return <section class="library-results-detail" aria-label="Selected canvas">
          <Show when={resultsType() !== 'canvases'}><button type="button" class="btn small ghost library-results-back" onClick={() => closeDetail()}><Icon name="arrowLeft" size={13}/>Back to all results</button></Show>
          <Suspense fallback={<Skeleton rows={4} label="Opening the canvas"/>}><GalleryCanvas item={item()} compareRequest={compare()?.key === key ? compare()!.request : 0} onCompareConsumed={() => setCompare(null)}/></Suspense>
        </section>;
      }}</Show>
    </div>
    <Show when={selected() && !current() && !pending()}><p role="status" class="library-results-coverage">The selected canvas is missing from this listing or hidden by the filters.</p></Show>
  </section>;
}
