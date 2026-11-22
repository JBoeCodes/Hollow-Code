import { For, Show, batch, createEffect, createMemo, createResource, createSignal, on, onCleanup } from 'solid-js';
import { api } from '../api';
import { Icon, type IconName } from '../icons';
import { Segmented, StatePanel, ErrorText } from './ui';
import { AnimatedWidth, MotionCue, SkeletonRows } from './MotionKit';
import { relativeWhen } from '../homeReturn';
import { openWorkspaceResult, workspaceListLoaded, workspaces } from '../store/workspaces';
import { enableVaultTreeResource, refetchVaultItems, semanticSearchEnabled, vaultItemsData } from '../store/runtime';
import { libraryAllFilter, openLibraryCanvases, openLibraryNote, openLibraryTab, setLibraryAllFilter } from '../store/libraryMode';
import { requestRecording } from '../store/originFocus';
import { openPreview } from '../store/preview';
import { vaultRoot } from '../store/chat';
import { vaultChange } from '../store/vaultChanges';
import { joinPath } from '../chatFileLinks';
import { latestRequest } from '../latestRequest';
import { errMsg } from '../utils';
import { unsuspended } from '../stableResource';
import { canvasStoreListing } from '../libraryCanvasListing';
import { vaultChangeUnderAnyOutputs } from '../resultsCards';
import { mixedLibraryResults } from '../libraryResultsModel';
import { SemanticSearchLink } from './SemanticSearchLink';
import type { GalleryItem } from '../canvasGallery';
import { isWorkspaceVaultItem } from '../workspaceContent';
import { beginSearch, searchEmptyText, visibleResults, type SearchState } from '../searchState';
import type { SearchResult } from '../types';
import {
  FILTER_SHOWS, FILTER_SOURCES, LIBRARY_LIST_FILTERS, LIBRARY_RESULTS_ASK, LIBRARY_RESULTS_CAP, SOURCE_LABEL, buildLibraryList, isDamagedResultsStore, partialNote, resultsCapped, filterLibraryList, listEmpty, meaningRows,
  noteRows, limitShowing, readState, recordingRows, recordingsState, resultRows, stillReading, type LibraryListKind, type LibraryListRow, type LibraryListSource, type SourceState,
} from '../libraryListModel';

const KIND_ICON: Readonly<Record<LibraryListKind, IconName>> = {
  note: 'fileText', meeting: 'calendar', person: 'users', output: 'layers', canvas: 'appWindow', recording: 'mic',
};
const KIND_LABEL: Readonly<Record<LibraryListKind, string>> = {
  note: 'Note', meeting: 'Meeting note', person: 'Person', output: 'Result file', canvas: 'Canvas', recording: 'Recording',
};
/** Rows painted at once; Show more adds this many. */
const PAGE = 200;

/**
 * Library → All (overnight 11-12 L3): one searchable list of notes, meeting
 * notes, people, recordings, result files and canvases, newest first, with
 * Recent, Meetings, People and Outputs. Each source loads on its own, so
 * one failing reader shows its own Retry and the rest stays. A click
 * previews the item in the side panel; notes have Open in Notes for the
 * reader and editor. Nothing in the vault moves: this is a view.
 *
 * Overnight 11-12 L4: Notes, Results and Recordings left the tab strip and
 * open from here. New note and Browse folders (Notes' collections, folders,
 * saved views and templates) sit beside the search; Outputs offers Grid (the
 * Results grid) and Meetings offers Recordings (Plaud). Library owns the
 * views, so it hands in how to open each.
 */
export function LibraryList(props: { onNewNote: () => void; onBrowse: () => void; onGrid: () => void; onRecordings: () => void }) {
  // Notes: the vault's classified index (shared with the background warmer).
  enableVaultTreeResource();
  // Result files and canvases: the reads Library → Results makes.
  const [files, { refetch: refetchFiles }] = createResource(() => api.listAllResultsPeek(null, LIBRARY_RESULTS_ASK));
  const filesShown = unsuspended(files);
  const [canvases, setCanvases] = createSignal<GalleryItem[]>([]);
  const [storesLoading, setStoresLoading] = createSignal(true);
  const [failedStores, setFailedStores] = createSignal<string[]>([]);
  const stores = canvasStoreListing(id => api.listCanvases({ workspaceId: id, sessionKey: null }), value => batch(() => {
    setCanvases(value.items); setFailedStores(value.failed); setStoresLoading(false);
  }));
  const readStores = () => { if (!workspaceListLoaded()) return; setStoresLoading(true); stores.load(workspaces().map(w => ({ id: w.id, title: w.title }))); };
  const workspaceStamp = createMemo(() => JSON.stringify(workspaces().map(w => [w.id, w.title])));
  createEffect(on([workspaceListLoaded, workspaceStamp], () => readStores()));
  // Recordings: Plaud's last 30 days (its two-minute cache; Retry asks again).
  let fresh = false;
  const [recordings, { refetch: rereadRecordings }] = createResource(() => { const f = fresh; fresh = false; return api.listRecordings(30, f); });
  const retryRecordings = () => { fresh = true; void rereadRecordings(); };
  // Every source is read without suspending (lane S re-review of #1319): a
  // first read through `.latest` or the stable accessor would hand the whole
  // All view to Library's "Opening All" fallback until the slowest source
  // answered (Plaud's CLI can take 90 s), as `files` already avoided.
  const recordingsShown = unsuspended(recordings);
  const notesShown = unsuspended(vaultItemsData);
  let changeTimer: ReturnType<typeof setTimeout> | undefined;
  createEffect(on(vaultChange, change => {
    if (!vaultChangeUnderAnyOutputs(change)) return;
    clearTimeout(changeTimer);
    changeTimer = setTimeout(() => void refetchFiles(), 400);
  }, { defer: true }));
  const opening = latestRequest();
  onCleanup(() => { clearTimeout(changeTimer); stores.dispose(); opening.cancel(); });

  const state = (source: LibraryListSource): SourceState => {
    if (source === 'notes') return readState(vaultItemsData.error ? errMsg(vaultItemsData.error) : null, notesShown.latest);
    if (source === 'results') {
      if (files.error) return { status: 'failed', error: errMsg(files.error) };
      return files.loading && filesShown.latest === undefined ? { status: 'loading' } : { status: 'ready' };
    }
    if (source === 'canvases') {
      if (failedStores().length) return { status: 'failed', error: failedStores().join(', ') };
      return storesLoading() || !workspaceListLoaded() ? { status: 'loading' } : { status: 'ready' };
    }
    return recordingsState(recordings.error ? errMsg(recordings.error) : null, recordingsShown.latest);
  };
  const retry = (source: LibraryListSource) => {
    if (source === 'notes') void refetchVaultItems();
    else if (source === 'results') void refetchFiles();
    else if (source === 'canvases') readStores();
    else retryRecordings();
  };
  const refresh = () => { (['notes', 'results', 'canvases', 'recordings'] as const).forEach(retry); };

  // Guarded like the results read: a failed index shows its own Retry and
  // never throws into RecoverableView, which would blank the whole view.
  // (The unsuspended read keeps the last good list through an error.)
  const filesCapped = () => !files.error && resultsCapped(filesShown.latest?.length);
  const noteItems = createMemo(() => (notesShown.latest ?? []).filter(isWorkspaceVaultItem));
  const notes = createMemo(() => noteRows(noteItems()));
  const results = createMemo(() => resultRows(mixedLibraryResults(files.error ? [] : (filesShown.latest ?? []).slice(0, LIBRARY_RESULTS_CAP), canvases())));
  const recs = createMemo(() => recordingRows(recordings.error ? [] : recordingsShown.latest?.recordings ?? []));
  const all = createMemo(() => buildLibraryList({ notes: notes(), results: results(), recordings: recs(), items: noteItems() }));

  // In the store, so Back to All returns to the filter a view was opened from.
  const filter = libraryAllFilter, setFilter = setLibraryAllFilter;
  const [query, setQuery] = createSignal('');
  const [showOlder, setShowOlder] = createSignal(false);
  const [limit, setLimit] = createSignal(PAGE);
  const [selected, setSelected] = createSignal<string | null>(null);
  const [openError, setOpenError] = createSignal('');
  createEffect(on([filter, query, showOlder], () => setLimit(PAGE), { defer: true }));

  // By meaning, as Notes has it: the on-device index, debounced; only the
  // latest request answers, and earlier hits stay while it runs.
  const [byMeaning, setByMeaning] = createSignal(false);
  const [meaning, setMeaning] = createSignal<SearchState<SearchResult>>({ status: 'idle' });
  const meaningOn = () => byMeaning() && semanticSearchEnabled() && query().trim().length >= 3;
  const order = latestRequest();
  let meaningTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => { clearTimeout(meaningTimer); order.cancel(); });
  createEffect(() => {
    const q = query().trim();
    clearTimeout(meaningTimer);
    if (!(byMeaning() && semanticSearchEnabled()) || q.length < 3) { order.cancel(); setMeaning({ status: 'idle' }); return; }
    const ticket = order.start(); setMeaning(prev => beginSearch(prev, q));
    meaningTimer = setTimeout(() => {
      api.semanticSearch(q, 60)
        .then(r => { if (order.isLatest(ticket)) setMeaning({ status: 'done', query: q, results: r.filter(h => isWorkspaceVaultItem({ path: h.path })) }); })
        .catch(e => { if (order.isLatest(ticket)) setMeaning({ status: 'error', query: q, message: String(e) }); });
    }, 300);
  });

  const view = createMemo(() => meaningOn()
    ? { rows: meaningRows(all(), visibleResults(meaning()) ?? [], filter()), older: 0 }
    : filterLibraryList(all(), { filter: filter(), query: query(), showOlder: showOlder(), now: Date.now() }));
  const sources = () => FILTER_SOURCES[filter()];
  const reading = () => stillReading(sources(), state);
  const loading = () => reading().length > 0;
  const failed = () => sources().some(s => state(s).status === 'failed');
  const empty = () => {
    if (meaningOn()) {
      const e = searchEmptyText(meaning(), 'meaning');
      return e ? { kind: 'search' as const, title: e.title, detail: e.detail }
        : { kind: 'search' as const, title: 'Nothing matches your search', detail: filter() === 'recent' ? 'Nothing in the index is close to that description.' : `What the index found for that description isn’t in ${LIBRARY_LIST_FILTERS.find(f => f.value === filter())!.label}.` };
    }
    return listEmpty({ total: all().filter(r => FILTER_SOURCES[filter()].includes(r.source)).length, filter: filter(), query: query(), older: view().older, failed: failed(), loading: loading() });
  };

  let search: HTMLInputElement | undefined;
  let list: HTMLDivElement | undefined;
  const clearSearch = () => { setQuery(''); search?.focus(); };
  // A button that removes itself on click (Show older, Show everything, Show
  // more on its last page) hands focus to the rows it brought in, else the
  // first row, else the search box, never to the page (lane S re-review).
  // Show older resets the page to its first 200 (the effect above), so the
  // page grows to include the row that takes focus before it is looked up.
  const focusRows = (from = 0) => queueMicrotask(() => {
    if (from > 0) setLimit(l => limitShowing(from, PAGE, l));
    const rows = list?.querySelectorAll<HTMLElement>('[data-key-item]');
    (rows?.[from] ?? rows?.[0] ?? search)?.focus();
  });
  const showOlderRows = () => { const from = view().rows.length; setShowOlder(true); focusRows(from); };
  const showMore = () => { const from = limit(); setLimit(n => n + PAGE); if (view().rows.length <= limit()) focusRows(from); };
  const open = async (row: LibraryListRow) => {
    setOpenError('');
    setSelected(row.key);
    // Every branch takes a ticket first, so the newest click wins: a preview
    // still opening from an earlier click must not land over this one.
    const ticket = opening.start();
    if (row.kind === 'recording' && row.recordingId) { requestRecording(row.recordingId); void openLibraryTab('recordings'); return; }
    // Every canvas, this one open: Library's Back to All leads back (L4), so
    // the request carries no back of its own.
    if (row.kind === 'canvas' && row.workspaceId && row.canvasId) {
      void openLibraryCanvases('', { workspaceId: row.workspaceId, id: row.canvasId, compare: false, back: null });
      return;
    }
    if (!row.path) return;
    try {
      const root = await vaultRoot();
      if (!opening.isLatest(ticket)) return;
      if (!root) throw new Error('The Hollow folder is unavailable. Try opening this again.');
      openPreview({ kind: 'file', path: row.path.startsWith('/') ? row.path : joinPath(root, row.path) });
    } catch (e) { if (opening.isLatest(ticket)) setOpenError(errMsg(e)); }
  };
  const openLabel = (row: LibraryListRow) => row.kind === 'recording' ? 'Opens in Recordings' : row.kind === 'canvas' ? 'Opens in Results' : 'Previews in the side panel';

  return <section class="library-all" aria-label="All" aria-description="Everything in your Library, newest first. A click previews it in the side panel.">
    <div class="library-all-tools">
      <label class="work-search"><Icon name="search" size={15}/><input ref={search} type="search" aria-label="Search your Library"
        placeholder={byMeaning() && semanticSearchEnabled() ? 'Describe what you’re looking for…' : 'Search titles and folders…'} value={query()} onInput={e => setQuery(e.currentTarget.value)}/></label>
      <Show when={semanticSearchEnabled()}><button type="button" class="btn small ghost library-meaning" aria-pressed={byMeaning()} title="Find notes by what they mean, using the on-device index" onClick={() => setByMeaning(v => !v)}>By meaning</button></Show><SemanticSearchLink/>
      <button type="button" class="btn small" aria-label="Refresh the list" onClick={refresh}><Icon name="refresh" size={13}/>Refresh</button>
      <button type="button" class="btn small" title="Notes' collections, folders, saved views and records, with the reader and editor" onClick={() => props.onBrowse()}><Icon name="folder" size={13}/>Browse folders</button>
      <button type="button" class="btn small primary" onClick={() => props.onNewNote()}>New note</button>
    </div>
    <div class="library-all-filters">
      <Segmented level="secondary" class="library-all-filter" label="Library filter" value={filter()} onChange={v => { setFilter(v); setShowOlder(false); }} items={LIBRARY_LIST_FILTERS}/>
      <Show when={filter() === 'outputs'}><button type="button" class="btn small ghost" title="Results as a grid: files and canvases, by workspace, with review and Compare versions" onClick={() => props.onGrid()}><Icon name="layers" size={13}/>Grid</button></Show>
      <Show when={filter() === 'meetings'}><button type="button" class="btn small ghost" title="Your Plaud recordings: range, transcripts, search, linking and proposed tasks" onClick={() => props.onRecordings()}><Icon name="mic" size={13}/>Recordings</button></Show>
      <span class="library-all-count" role="status"><AnimatedWidth>{view().rows.length} {view().rows.length === 1 ? 'item' : 'items'}</AnimatedWidth>{partialNote({ loading: loading(), failed: failed(), capped: filesCapped() && sources().includes('results') })}{meaningOn() ? ' by meaning' : ''}</span>
    </div>
    <p class="library-all-shows">{FILTER_SHOWS[filter()]}</p>
    <For each={sources()}>{s => <>
      <Show when={state(s).status === 'failed'}>
        <StatePanel compact kind="failed" title={s === 'results' && isDamagedResultsStore((state(s) as { error: string }).error) ? 'Results review state is damaged' : `${SOURCE_LABEL[s]} couldn’t be read`} error={(state(s) as { error: string }).error}
          actions={<button type="button" class="btn small" onClick={() => retry(s)}>Retry {SOURCE_LABEL[s].toLowerCase()}</button>}>The rest of the list is still here.</StatePanel>
      </Show>
      <Show when={state(s).status === 'off'}>
        <p class="library-all-note">{(state(s) as { reason: string }).reason} <button type="button" class="btn-link" onClick={() => void openLibraryTab('recordings')}>Open Recordings</button></p>
      </Show>
    </>}</For>
    <Show when={loading() && view().rows.length}><p class="library-all-note" role="status"><MotionCue kind="activity" active={loading()}>Still reading {reading().map(s => SOURCE_LABEL[s].toLowerCase()).join(', ')}…</MotionCue></p></Show>
    <Show when={openError()}><ErrorText class="library-all-note" text={openError()}/></Show>
    <div ref={list} class="library-all-list" role="list" aria-label="Library items" data-key-list data-key-default aria-busy={meaningOn() && meaning().status === 'pending'}>
      <For each={view().rows.slice(0, limit())} fallback={<Show when={empty()}>{e => <Show when={e().kind !== 'loading'} fallback={<SkeletonRows rows={6} detail label="Loading your Library"/>}>
        <StatePanel kind={e().kind === 'failed' ? 'failed' : e().kind === 'empty' ? 'empty' : 'filtered'} title={e().title}
          actions={<>
            <Show when={query().trim()}><button type="button" class="btn small" onClick={clearSearch}>Clear search</button></Show>
            <Show when={!query().trim() && e().kind === 'filter' && view().older}><button type="button" class="btn small" onClick={showOlderRows}>Show older</button></Show>
            <Show when={!query().trim() && e().kind === 'filter' && filter() !== 'recent'}><button type="button" class="btn small" onClick={() => { setFilter('recent'); focusRows(); }}>Show everything</button></Show>
          </>}>{e().detail}</StatePanel>
      </Show>}</Show>}>
        {row => <div role="listitem" class="library-all-row" classList={{ selected: selected() === row.key }}>
          <button type="button" class="library-all-open" data-key-item data-row-key={row.key} aria-current={selected() === row.key ? 'true' : undefined}
            aria-description={`${KIND_LABEL[row.kind]}. ${row.where}${row.modified ? `. ${relativeWhen(row.modified)}` : ''}. ${openLabel(row)}`}
            title={row.path ?? row.title} onClick={() => void open(row)}>
            <span class="library-all-icon" data-kind={row.kind} aria-hidden="true"><Icon name={KIND_ICON[row.kind]} size={16}/></span>
            <span class="library-all-main"><strong>{row.title}</strong><small>{KIND_LABEL[row.kind]} · {row.where}</small></span>
            <span class="library-all-when">{row.modified ? relativeWhen(row.modified) : ''}</span>
          </button>
          <Show when={row.source === 'notes' && row.path}>{p =>
            <button type="button" class="btn small ghost library-all-action" data-key-action="edit" aria-label={`Open ${row.title} in Notes`} title="Open in Notes, to read and edit" onClick={() => void openLibraryNote(p())}>Open</button>}</Show>
          <Show when={row.source === 'results' && row.workspaceId && row.path}>{p =>
            <button type="button" class="btn small ghost library-all-action" data-key-action="edit" aria-label={`Open ${row.title} in its workspace`} title="Open in its workspace, with its review bar"
              onClick={() => void openWorkspaceResult(row.workspaceId!, p()).catch(e => setOpenError(errMsg(e)))}>Open</button>}</Show>
        </div>}
      </For>
    </div>
    <Show when={view().rows.length > limit()}><button type="button" class="btn small ghost library-all-more" onClick={showMore}>Show more ({view().rows.length - limit()} left)</button></Show>
    <Show when={view().rows.length && view().older && !query().trim()}><button type="button" class="btn small ghost library-all-more" onClick={showOlderRows}>Show older ({view().older})</button></Show>
  </section>;
}
