import { For, Match, Show, Switch, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { changedOnly, lineDiff } from '../lineDiff';
import { capturedWhen, isSourceChanged } from '../sourceAuthority';
import { formatBytes } from '../sourcesModel';
import type { SourceRefreshPreview, WorkspaceDetail } from '../workspaceTypes';
import { Button, ErrorText } from './ui';
import '../styles/sources-cards.css';

/** Changed lines shown before "N more"; the rest are in the files themselves. */
const SHOWN_LINES = 300;

type Step =
  | { kind: 'loading' }
  | { kind: 'failed'; error: string }
  | { kind: 'ready'; preview: SourceRefreshPreview }
  | { kind: 'replacing'; preview: SourceRefreshPreview }
  | { kind: 'conflict' }
  | { kind: 'done' }
  | { kind: 'kept' };

/**
 * Check for changes on one snapshot (plan L3 X5, 2026-10-14; review U
 * library, ASG 5): compares it with its original by content
 * (`preview_source_refresh`, which reads only), shows the changed lines, and
 * replaces it only on the person's Replace snapshot, with the digests that
 * preview showed (`apply_source_refresh`). If either side moved on since, the
 * backend refuses with `SOURCE_CHANGED:` and this says to check again. The
 * earlier snapshot is kept in `sources/.previous/` and SOURCES.md records the
 * refresh. Lazy: loaded only when someone clicks Check for changes.
 */
export function SourceRefresh(props: { workspaceId: string; path: string; name?: string; onReplaced?: (detail: WorkspaceDetail) => void; onClose?: () => void }) {
  const [step, setStep] = createSignal<Step>({ kind: 'loading' });
  let alive = true; const checks = latestRequest();
  onCleanup(() => { alive = false; });
  const name = () => props.name ?? props.path.replace(/^sources\//, '');
  async function check() {
    const mine = checks.start(); setStep({ kind: 'loading' });
    try { const preview = await api.previewSourceRefresh(props.workspaceId, props.path); if (alive && checks.isLatest(mine)) setStep({ kind: 'ready', preview }); }
    catch (e) { if (alive && checks.isLatest(mine)) setStep({ kind: 'failed', error: String(e) }); }
  }
  async function replace(preview: SourceRefreshPreview) {
    if (!preview.origin) return;
    setStep({ kind: 'replacing', preview });
    try {
      const detail = await api.applySourceRefresh(props.workspaceId, props.path, preview.origin.sha256, preview.snapshot.sha256);
      if (!alive) return;
      setStep({ kind: 'done' }); props.onReplaced?.(detail);
    } catch (e) {
      if (!alive) return;
      setStep(isSourceChanged(e) ? { kind: 'conflict' } : { kind: 'failed', error: String(e) });
    }
  }
  function keep() { setStep({ kind: 'kept' }); props.onClose?.(); }
  onMount(() => void check());
  const preview = () => { const s = step(); return s.kind === 'ready' || s.kind === 'replacing' ? s.preview : null; };
  return <section class="src-refresh" aria-label={`Check for changes: ${name()}`} aria-busy={step().kind === 'loading' || step().kind === 'replacing'}>
    <Switch>
      <Match when={step().kind === 'loading'}><p class="src-refresh-status" role="status">Comparing the snapshot with its original…</p></Match>
      <Match when={step().kind === 'failed' ? step() as Extract<Step, { kind: 'failed' }> : null}>{s =>
        <><ErrorText class="src-refresh-error" text={s().error}/><div class="src-refresh-actions"><Button size="sm" onClick={() => void check()}>Check again</Button><Show when={props.onClose}><Button size="sm" variant="ghost" onClick={() => props.onClose!()}>Close</Button></Show></div></>
      }</Match>
      <Match when={step().kind === 'conflict'}>
        <p class="src-refresh-status" role="alert">The original or the snapshot changed after you checked. Check again.</p>
        <div class="src-refresh-actions"><Button size="sm" variant="primary" onClick={() => void check()}>Check again</Button></div>
      </Match>
      <Match when={step().kind === 'done'}>
        <p class="src-refresh-status" role="status">Snapshot replaced. SOURCES.md records the refresh.</p>
        <p class="src-refresh-detail">The earlier snapshot is kept in this workspace’s sources/.previous/ folder, and its entry stays in SOURCES.md.</p>
      </Match>
      <Match when={step().kind === 'kept'}><p class="src-refresh-status" role="status">Kept this snapshot. Nothing changed.</p></Match>
      <Match when={preview()}>{p => <Review preview={p()} busy={step().kind === 'replacing'} onReplace={() => void replace(p())} onKeep={keep}/>}</Match>
    </Switch>
  </section>;
}

function Review(props: { preview: SourceRefreshPreview; busy: boolean; onReplace: () => void; onKeep: () => void }) {
  const p = () => props.preview;
  const when = () => capturedWhen(p().recorded.captured);
  const lines = createMemo(() => p().text && p().contentChanged ? changedOnly(lineDiff(p().text!.before, p().text!.after)) : []);
  return <Switch>
    <Match when={!p().available}>
      <p class="src-refresh-status" role="status">The original can’t be found, so there is nothing to compare. The snapshot is unchanged.</p>
      <p class="src-refresh-detail">Recorded original: {p().original}</p>
    </Match>
    <Match when={!p().contentChanged}>
      <p class="src-refresh-status" role="status">The original matches the snapshot (same content).</p>
      <p class="src-refresh-detail">Compared by content, not just size and date. From {p().original}</p>
    </Match>
    <Match when={p().origin}>{origin => <>
      {/* Only the copy was edited when the original still hashes to what
          was captured (L3 review): replacing then restores the capture. */}
      <p class="src-refresh-status" role="status">{origin().sha256 === p().recorded.sha256
        ? `The original is unchanged; the snapshot was edited since it was captured. Replacing restores the original.${when() ? ` Captured ${when()}.` : ''}`
        : `The original changed: ${formatBytes(p().snapshot.size)} → ${formatBytes(origin().size)}.${when() ? ` Captured ${when()}.` : ''}`}</p>
      <p class="src-refresh-detail">From {p().original}</p>
      <Show when={p().snapshotEdited}><p class="src-refresh-warn" role="note">The snapshot was edited since it was captured; replacing it drops those edits.</p></Show>
      <Show when={p().text} fallback={<p class="src-refresh-detail">Can’t show the changes for this file type.</p>}>
        <ol class="src-refresh-diff" aria-label="Changed lines">
          <For each={lines().slice(0, SHOWN_LINES)}>{l => <li data-kind={l.kind}><span class="src-refresh-mark" aria-hidden="true">{l.kind === 'added' ? '+' : l.kind === 'removed' ? '−' : ' '}</span><span class="sr-only">{l.kind === 'added' ? 'Added: ' : l.kind === 'removed' ? 'Removed: ' : ''}</span>{l.text || ' '}</li>}</For>
        </ol>
        <Show when={lines().length > SHOWN_LINES}><p class="src-refresh-detail">{(lines().length - SHOWN_LINES).toLocaleString('en-US')} more changed lines aren’t shown.</p></Show>
      </Show>
      <div class="src-refresh-actions">
        <Button size="sm" variant="primary" disabled={props.busy} onClick={props.onReplace}>{props.busy ? 'Replacing…' : 'Replace snapshot'}</Button>
        <Button size="sm" variant="ghost" disabled={props.busy} onClick={props.onKeep}>Keep this snapshot</Button>
      </div>
      <p class="src-refresh-detail">Replacing keeps the earlier snapshot in sources/.previous/ and adds a Refreshed entry to SOURCES.md.</p>
    </>}</Match>
  </Switch>;
}
