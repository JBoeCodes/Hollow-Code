/** Import conversations (H6 with H5a; design `sources-and-import-2026-11-13`
 *  section 4): one card per source the backend lists (Claude Code and Codex
 *  only when their folders exist; an export source such as ChatGPT only
 *  when `session_import_sources` returns one). Each card asks before reading
 *  (Look), inspects into staging, polls the pass, lists what it found with
 *  Select all and Select none (default: everything not imported yet) and
 *  imports the picked ones with a result line. Lazy: the first-run sheet's
 *  Find what's already here step mounts it, and Library can reuse it. Words
 *  and selection rules in `importConversations.ts`. */
import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { api, type ImportCommitResult, type ImportJob, type ImportManifest, type ImportSource } from '../api';
import {
  STATUS_LABELS, broughtIn, countsText, defaultSelection, entryMeta, importLabel, isExportSource, jobFinished, jobListable, pickable,
  progressText, resultText, scanNotice, selectAll, sourceLine, visibleSources,
} from '../importConversations';
import { errMsg } from '../utils';
import { ErrorText } from './ui';
import '../styles/firstRun.css';

/** How often a running inspect pass is asked for its progress. */
const POLL_MS = 700;

function SourceCard(props: { source: ImportSource; onImported?: (result: ImportCommitResult, source: ImportSource) => void }) {
  const [importId, setImportId] = createSignal<string | null>(null);
  const [job, setJob] = createSignal<ImportJob | null>(null);
  const [manifest, setManifest] = createSignal<ImportManifest | null>(null);
  const [picked, setPicked] = createSignal<Set<string>>(new Set());
  const [busy, setBusy] = createSignal(false);
  const [result, setResult] = createSignal<ImportCommitResult | null>(null);
  const [error, setError] = createSignal('');
  let timer: ReturnType<typeof setTimeout> | undefined;
  let alive = true;
  onCleanup(() => { alive = false; if (timer) clearTimeout(timer); });

  async function poll(id: string) {
    try {
      const status = await api.sessionImportStatus(id);
      if (!alive || importId() !== id) return;
      const current = status.jobs.find(j => j.importId === id) ?? null;
      setJob(current);
      if (!jobFinished(current)) { timer = setTimeout(() => void poll(id), POLL_MS); return; }
      if (jobListable(current)) {
        const list = await api.listSessionImport(id);
        if (!alive || importId() !== id) return;
        setManifest(list); setPicked(defaultSelection(list));
      } else {
        // Failed or cancelled: Look (or Choose file) comes back to retry.
        setImportId(null);
      }
    } catch (e) {
      // A failed read leaves no pass to wait on: Look comes back to retry.
      if (alive && importId() === id) { setError(errMsg(e)); setImportId(null); }
    }
  }

  async function look(root?: string | null) {
    if (busy()) return;
    setError(''); setResult(null); setManifest(null); setJob(null); setImportId(null);
    setBusy(true);
    try {
      const started = await api.inspectSessionImport(props.source.source, root ?? null);
      if (!alive) return;
      setImportId(started.importId);
      void poll(started.importId);
    } catch (e) { setError(errMsg(e)); }
    finally { setBusy(false); }
  }

  async function chooseFile() {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const path = await open({ multiple: false, directory: false, title: `Choose the ${props.source.label} export` });
      if (typeof path === 'string') await look(path);
    } catch (e) { setError(errMsg(e)); }
  }

  async function cancel() {
    const id = importId();
    if (!id) return;
    if (timer) clearTimeout(timer);
    try { await api.cancelSessionImport(id); } catch { /* the pass may have just ended */ }
    if (alive) { setImportId(null); setJob(null); setManifest(null); }
  }

  function toggle(id: string, on: boolean) {
    setPicked(prev => { const next = new Set(prev); if (on) next.add(id); else next.delete(id); return next; });
  }

  async function commit() {
    const id = importId();
    if (!id || busy() || !picked().size) return;
    setBusy(true); setError('');
    try {
      const done = await api.commitSessionImport(id, { ids: [...picked()] });
      if (!alive) return;
      setResult(done);
      props.onImported?.(done, props.source);
    } catch (e) { setError(errMsg(e)); }
    finally { setBusy(false); }
  }

  const inspecting = () => !!importId() && !jobFinished(job());

  return <section class="import-source" data-import-source={props.source.source} aria-label={props.source.label}>
    <div class="import-source-head">
      <h4>{props.source.label}</h4>
      <Show when={!importId()}>
        <Show when={isExportSource(props.source)} fallback={<button type="button" class="btn small" disabled={busy()} onClick={() => void look()}>Look</button>}>
          <button type="button" class="btn small" disabled={busy()} onClick={() => void chooseFile()}>Choose file</button>
        </Show>
      </Show>
      <Show when={inspecting()}>
        <button type="button" class="btn small ghost" onClick={() => void cancel()}>Cancel</button>
      </Show>
    </div>
    <Show when={!importId()}><p class="settings-row-v">{sourceLine(props.source)}</p></Show>
    <Show when={inspecting() || (job() && !jobListable(job()))}><p class="settings-row-v" role="status">{progressText(job())}</p></Show>
    <Show when={manifest()}>{list => <>
      <p class="settings-row-v" role="status">{countsText(list())}</p>
      <Show when={scanNotice(list())}>{notice => <p class="settings-row-v" role="status">{notice()}</p>}</Show>
      <Show when={list().note}>{note => <p class="settings-row-v">{note()}</p>}</Show>
      <Show when={!result() && list().conversations.length}>
        <div class="work-actions" role="group" aria-label={`${props.source.label} selection`}>
          <button type="button" class="btn small ghost" onClick={() => setPicked(selectAll(list()))}>Select all</button>
          <button type="button" class="btn small ghost" onClick={() => setPicked(new Set<string>())}>Select none</button>
        </div>
        <ul class="import-list" aria-label={`${props.source.label} conversations`}>
          <For each={list().conversations}>{entry =>
            <li class="import-entry" data-status={entry.status}>
              <label>
                <input type="checkbox" checked={picked().has(entry.id)} disabled={!pickable(entry)} onChange={e => toggle(entry.id, e.currentTarget.checked)} />
                <span class="import-entry-title">{entry.title || 'Untitled conversation'}</span>
              </label>
              <small>{entryMeta(entry)}{entry.status === 'new' ? '' : ` · ${STATUS_LABELS[entry.status]}`}</small>
              <Show when={entry.otherBranches}><small>Latest branch only; {entry.otherBranches} other branches remain in the source.</small></Show>
              <Show when={entry.malformedLines || entry.truncated || entry.shortenedTurns}><small>This history is partial. {entry.malformedLines} unreadable lines; {entry.shortenedTurns} earlier turns omitted.{entry.truncated ? ' The final line is incomplete.' : ''}</small></Show>
            </li>}
          </For>
        </ul>
        <div class="work-actions">
          <button type="button" class="btn small primary" disabled={busy() || !picked().size} onClick={() => void commit()}>{busy() ? 'Importing' : importLabel(picked().size)}</button>
        </div>
      </Show>
    </>}</Show>
    <Show when={result()}>{r => <>
      <p class="import-result" role="status">{resultText(r())}</p>
      <Show when={r().failed.length}>
        <ul class="import-failed"><For each={r().failed}>{f => <li>{f.title}: {f.error}</li>}</For></ul>
      </Show>
      <Show when={broughtIn(r()) > 0}><p class="settings-row-v">They are in Library, Conversations.</p></Show>
    </>}</Show>
    <Show when={error()}><ErrorText class="work-error" text={error()} /></Show>
  </section>;
}

export function ImportConversations(props: { onImported?: (result: ImportCommitResult, source: ImportSource) => void }) {
  const [sources, setSources] = createSignal<ImportSource[] | null>(null);
  const [error, setError] = createSignal('');
  onMount(async () => {
    try { setSources(visibleSources(await api.sessionImportSources())); } catch (e) { setError(errMsg(e)); }
  });
  return <div class="import-conversations">
    <Show when={sources()} fallback={<Show when={!error()}><p class="settings-loading">Looking for other apps’ folders…</p></Show>}>
      {list => <Show when={list().length} fallback={<p class="settings-row-v">No supported conversation folders or exports found.</p>}>
        <For each={list()}>{source => <SourceCard source={source} onImported={props.onImported} />}</For>
      </Show>}
    </Show>
    <Show when={error()}><ErrorText class="work-error" text={error()} /></Show>
  </div>;
}

export default ImportConversations;
