import { For, Show, createMemo, createSignal, onCleanup } from 'solid-js';
import { api, type DirectoryConnection } from '../api';
import { Icon } from '../icons';
import { showConfirm } from '../store';
import { contentsIndexedLine, exclusionConsequence, refreshedLabel, summarizeCoverage } from '../sourceCoverage';
import type { ConnectedFileState, ConnectedIndexStatus } from '../types';
import { ErrorText } from './ui';

/**
 * One compact coverage line for connected projects (audit H03), replacing a
 * stack of warning cards. Details list the capped projects with their
 * largest folders to leave out, folders already left out, listed files Hollow
 * doesn't read (too large, needs a reader; next-level P07), the PDF and
 * Office files it reads itself (repo features R14a) and any other scan
 * problems. Leaving a folder out is Hollow-side only and confirmed first.
 */
export function SourceCoverage(props: { connection?: DirectoryConnection; contentIndex?: ConnectedIndexStatus | null; busy?: boolean; onChange: (next: DirectoryConnection) => void; onRefresh: () => void; onShowState?: (state: ConnectedFileState) => void }) {
  // Repo features R14b: how much of the listed text the content index holds.
  const indexedLine = () => contentsIndexedLine(props.contentIndex);
  const summary = createMemo(() => summarizeCoverage(props.connection));
  // Re-render the relative refresh time each minute.
  const [now, setNow] = createSignal(Date.now());
  const tick = setInterval(() => setNow(Date.now()), 60_000);
  onCleanup(() => clearInterval(tick));
  const refreshed = () => props.connection?.refreshedAt ? refreshedLabel(props.connection.refreshedAt, now()) : '';
  const unread = () => summary().unread.tooLarge + summary().unread.needsReader;
  const n = (x: number) => x.toLocaleString('en-US');
  const [working, setWorking] = createSignal('');
  const [error, setError] = createSignal('');
  async function exclude(relative: string, files: number, on: boolean) {
    if (on && !(await showConfirm({ title: `Leave “${relative}” out of Hollow?`, body: exclusionConsequence(relative, files), confirmLabel: 'Leave folder out' }))) return;
    setWorking(relative); setError('');
    try { props.onChange(await api.setDirectoryExclusion(relative, on)); }
    catch (e) { setError(String(e)); }
    finally { setWorking(''); }
  }
  return <section class="source-coverage" data-state={summary().state} aria-label="Source coverage">
    <div class="source-coverage-line">
      <Icon name={summary().state === 'complete' ? 'check' : 'alert'} size={15} />
      <p role={summary().state === 'partial' ? 'status' : undefined}>{summary().headline}<Show when={indexedLine()}>{line => <> {line()}.</>}</Show><Show when={refreshed()}><small class="source-coverage-refreshed" title={props.connection?.refreshedAt}>{refreshed()}</small></Show></p>
      <button class="btn small ghost" disabled={props.busy} onClick={props.onRefresh}>{props.busy ? 'Refreshing…' : 'Refresh projects'}</button>
    </div>
    <Show when={summary().capped.length || summary().other.length || unread() || summary().extracted || (props.connection?.coverage?.excluded.length ?? 0)}>
      <details class="source-coverage-details" open={summary().state === 'partial' && summary().capped.length > 0 && summary().capped.length <= 3}>
        <summary>What’s missing and how to fix it</summary>
        <Show when={error()}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
        <For each={summary().capped}>{p =>
          <div class="source-coverage-project">
            <div class="source-coverage-project-head"><strong>{p.title}</strong><span>{p.omitted === undefined ? `at least ${n(p.found)} supported files · first ${n(p.indexed)} indexed` : `${n(p.found)} supported files · ${n(p.indexed)} listed · ${n(p.omitted)} not listed, still found by name`}</span></div>
            <Show when={p.folders.length} fallback={<p class="work-muted">No large folders to suggest. Add rules to the project’s .hollowignore to choose which files matter.</p>}>
              <p class="work-muted">Leave out folders you don’t need so the rest of {p.title} fits:</p>
              <ul class="source-coverage-folders"><For each={p.folders}>{f =>
                <li><span title={f.relative}>{f.relative.split('/').slice(1).join('/') || f.relative}</span><small>{f.files.toLocaleString('en-US')} files</small>
                  <button class="btn small ghost" disabled={!!working()} onClick={() => void exclude(f.relative, f.files, true)}>{working() === f.relative ? 'Leaving out…' : 'Leave out'}</button></li>
              }</For></ul>
            </Show>
          </div>
        }</For>
        <Show when={unread()}>
          <div class="source-coverage-project"><div class="source-coverage-project-head"><strong>Listed but not read</strong><span>Hollow never treats these as empty</span></div>
            <ul class="source-coverage-folders">
              <Show when={summary().unread.tooLarge}><li><span>Larger than 1 MB: open the original, or export a smaller text file</span><small>{n(summary().unread.tooLarge)} {summary().unread.tooLarge === 1 ? 'file' : 'files'}</small>
                <Show when={props.onShowState} fallback={<i />}><button class="btn small ghost" onClick={() => props.onShowState?.('tooLarge')}>Show</button></Show></li></Show>
              <Show when={summary().unread.needsReader}><li><span>Images: need a reader or a text export</span><small>{n(summary().unread.needsReader)} {summary().unread.needsReader === 1 ? 'file' : 'files'}</small>
                <Show when={props.onShowState} fallback={<i />}><button class="btn small ghost" onClick={() => props.onShowState?.('needsReader')}>Show</button></Show></li></Show>
            </ul></div>
        </Show>
        <Show when={summary().extracted}>
          <div class="source-coverage-project"><div class="source-coverage-project-head"><strong>Read by Hollow</strong><span>Agents can cite their text</span></div>
            <ul class="source-coverage-folders"><li><span>PDF and Office: read by Hollow</span><small>{n(summary().extracted)} {summary().extracted === 1 ? 'file' : 'files'}</small>
              <Show when={props.onShowState} fallback={<i />}><button class="btn small ghost" onClick={() => props.onShowState?.('extracted')}>Show</button></Show></li></ul></div>
        </Show>
        <Show when={props.connection?.coverage?.excluded.length}>
          <div class="source-coverage-project"><div class="source-coverage-project-head"><strong>Left out by you</strong><span>Hollow skips these; the folders are unchanged</span></div>
            <ul class="source-coverage-folders"><For each={props.connection?.coverage?.excluded}>{r =>
              <li><span title={r}>{r}</span><button class="btn small ghost" disabled={!!working()} onClick={() => void exclude(r, 0, false)}>{working() === r ? 'Including…' : 'Include again'}</button></li>
            }</For></ul></div>
        </Show>
        <Show when={summary().other.length}>
          <div class="source-coverage-project"><div class="source-coverage-project-head"><strong>Other scan notes</strong></div>
            <ul class="source-coverage-notes"><For each={summary().other}>{w => <li>{w}</li>}</For></ul></div>
        </Show>
      </details>
    </Show>
  </section>;
}
