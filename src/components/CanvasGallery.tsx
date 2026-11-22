import { Show, createMemo, createResource, createSignal, onCleanup } from 'solid-js';
import { api } from '../api';
import { canvasGrantKey } from '../canvas/canvasGrants';
import { canvasState } from '../canvasBridge';
import { CANVAS_FENCE_MODE, innerOf } from '../canvas/chatCanvas';
import { comparePair, type ComparePair } from '../canvasGallery';
import type { MixedCanvas } from '../libraryResultsModel';
import { canvasCompareMode } from '../canvasCompareMode';
import { Icon } from '../icons';
import { visitWorkspace } from '../store/workspaces';
import { errMsg } from '../utils';
import { DiffView } from './DiffView';
import { HtmlArtifact } from './HtmlArtifact';
import { canvasSourceSession, openCanvasSource } from '../store/canvasSource';
import { ResultCanvas } from './ResultCanvas';
import { Select } from './Select';
import { Skeleton, ErrorText } from './ui';
import '../styles/canvas-gallery.css';

/** The selected canvas: as Results shows it, or two revisions compared. */
export function GalleryCanvas(props: { item: MixedCanvas; compareRequest: number; onCompareConsumed: () => void }) {
  const [sourceError, setSourceError] = createSignal('');
  let disposed = false;
  onCleanup(() => { disposed = true; });
  const openSource = (revision: number) => {
    const c = saved(); if (!c) return;
    setSourceError('');
    void openCanvasSource(c, revision).catch(e => { if (!disposed) setSourceError(errMsg(e)); });
  };
  const scope = () => ({ workspaceId: props.item.workspaceId, sessionKey: null });
  const [canvas] = createResource(() => `${props.item.revision}:${props.item.updatedAt ?? ''}`, () => api.readCanvas(scope(), props.item.id));
  /** The saved canvas once read (null: not in the store). Never reads an
   *  errored resource's value, which would throw out of the view. */
  const saved = () => (canvas.state === 'ready' || canvas.state === 'refreshing' ? canvas.latest : undefined);
  const [comparing, setComparing] = canvasCompareMode(() => props.compareRequest, () => props.onCompareConsumed());
  // Owned by the selected canvas, not a revision pair or comparison mount.
  const [sourceOpen, setSourceOpen] = createSignal(false);
  const [chosen, setChosen] = createSignal<ComparePair | null>(null);
  const pair = createMemo(() => { const c = saved(); return c ? comparePair(c.revisions.map(r => r.revision), c.revision, chosen()) : null; });
  /** Picking a side's version the other side shows swaps the two. */
  const pick = (side: 'before' | 'after', n: number) => {
    const p = pair(); if (!p) return;
    const other = side === 'before' ? p.after : p.before;
    setChosen(side === 'before' ? (n === other ? { before: n, after: p.before } : { before: n, after: p.after })
      : (n === other ? { before: p.after, after: n } : { before: p.before, after: n }));
  };
  const body = async (n: number) => { const c = saved()!; return n === c.revision ? c.html : (await api.readCanvasRevision(scope(), props.item.id, n)).html; };
  const [sides] = createResource(() => (comparing() && pair()) || false, async p => {
    try { const [before, after] = await Promise.all([body(p.before), body(p.after)]); return { before: { n: p.before, body: before }, after: { n: p.after, body: after } }; }
    catch (e) { return { error: errMsg(e) }; }
  });
  // Leaving Library: Results starts at All next time (store/libraryMode.ts).
  const openWorkspace = () => { void visitWorkspace(props.item.workspaceId); };
  const revisionOptions = () => (saved()?.revisions ?? []).map(r => r.revision).sort((a, b) => b - a).map(n => ({ value: String(n), label: `v${n}${n === saved()?.revision ? ' (current)' : ''}` }));
  const frame = (title: string, source: string, revision: number) =>
    <HtmlArtifact lookWorkspace={props.item.workspaceId} code={`\n${innerOf(source, '')}\n`} defaultMode={CANVAS_FENCE_MODE} canvas={{ title, source, ref: { workspaceId: props.item.workspaceId, revision } }}
      bridge={{ canvasId: props.item.id, state: () => revision === saved()?.revision ? canvasState(saved()?.state) ?? null : null }}
      stateTarget={saved() ? { revision: saved()!.revision, open: canvasSourceSession(saved()!) ? () => openSource(saved()!.revision) : undefined } : undefined}
      actions={[{ label: 'Open source chat', disabled: !saved() || !canvasSourceSession(saved()!, revision), action: () => openSource(revision) }]}
      grantKey={canvasGrantKey(scope(), props.item.id)} fill host="gallery"/>;
  return <div class="cg-canvas" role="region" aria-label="Canvas detail">
    <Show when={sourceError()}><ErrorText class="chat-canvas-note" text={sourceError()} /></Show>
    <div class="cg-canvas-head">
      <div class="cg-canvas-title"><h3>{saved()?.title ?? props.item.title}</h3><span>{props.item.workspaceTitle}{saved() ? ` · v${saved()!.revision}` : ''}</span></div>
      <div class="cg-actions" role="group" aria-label="Canvas actions">
        <Show when={(saved()?.revisions.length ?? 0) > 1}><button type="button" class="btn small ghost" aria-pressed={comparing()} onClick={() => setComparing(v => !v)}><Icon name="columns" size={13}/>Compare versions</button></Show>
        <button type="button" class="btn small" onClick={openWorkspace}><Icon name="folder" size={13}/>Open workspace</button>
      </div>
    </div>
    <Show when={canvas.state !== 'errored'} fallback={<ErrorText class="cg-empty" text={`This canvas couldn’t be read: ${errMsg(canvas.error)}`} />}>
      <Show when={comparing() && pair()} fallback={
        <ResultCanvas gallery workspaceId={props.item.workspaceId} id={props.item.id} stamp={`${props.item.revision}:${props.item.updatedAt ?? ''}`}
          fallback={() => <p class="cg-empty">Only its file remains; open the workspace’s Results to read it.</p>}/>
      }>{p => <>
        <div class="cg-compare-pick">
          <Select label="Before" size="sm" value={String(p().before)} onChange={v => pick('before', Number(v))} options={revisionOptions()}/>
          <Icon name="arrowRight" size={14}/>
          <Select label="After" size="sm" value={String(p().after)} onChange={v => pick('after', Number(v))} options={revisionOptions()}/>
        </div>
        <Show when={!sides.loading && sides.latest} fallback={<Skeleton rows={4} label="Opening both versions"/>}>{s => {
          const v = s();
          return 'error' in v ? <ErrorText class="cg-empty" text={`Couldn’t open a version: ${v.error}`} /> : <>
            <div class="cg-compare">
              <figure><figcaption>Before · v{v.before.n}</figcaption>{frame(saved()?.title ?? props.item.title, v.before.body, v.before.n)}</figure>
              <figure><figcaption>After · v{v.after.n}</figcaption>{frame(saved()?.title ?? props.item.title, v.after.body, v.after.n)}</figure>
            </div>
            <details class="cg-source" open={sourceOpen()} onToggle={e => setSourceOpen(e.currentTarget.open)}>
              <summary>View source changes</summary>
              <Show when={sourceOpen()}>
                <DiffView class="cg-diff" before={v.before.body} after={v.after.body} beforeLabel={`v${v.before.n}`} afterLabel={`v${v.after.n}`}/>
              </Show>
            </details>
          </>;
        }}</Show>
      </>}</Show>
    </Show>
  </div>;
}
