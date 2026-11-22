import { Show, createEffect, createResource, createSignal, onCleanup, type JSX } from 'solid-js';
import { api } from '../api';
import { HtmlArtifact } from './HtmlArtifact';
import { CanvasPinButton } from './CanvasPinButton';
import { canvasGrantKey } from '../canvas/canvasGrants';
import { CanvasVersions, keepFocus } from './ChatCanvas';
import { CANVAS_FENCE_MODE, innerOf } from '../canvas/chatCanvas';
import { canvasState } from '../canvasBridge';
import type { Canvas } from '../generated/ipc';
import { errMsg } from '../utils';
import { Skeleton, ErrorText } from './ui';
import { olderLabel, savedWords } from '../canvas/hostContract';
import { openCanvasGallery } from '../store/canvasGallery';
import { visitWorkspace } from '../store/workspaces';
import { relativeWhen } from '../homeReturn';
import { QUICK_CHATS_ID } from '../workspaceTypes';
import type { ActionMenuItem } from './ActionMenu';
import { canvasSourceSession, openCanvasSource } from '../store/canvasSource';
import { openAttachCanvas } from '../store/canvasAttach';
import '../styles/chat-canvas.css';
import '../styles/results.css';

/**
 * A saved canvas opened from Results (Canvas C5b): the store's current
 * revision in Canvas mode (scripts on, no network; outside Tauri it falls
 * back to Offline as every canvas does), with its revision menu. Picking an
 * earlier revision shows it read-only from `.revisions/`; while one shows,
 * `onViewing` reports it, so the review's Accept, which records the visible
 * file (the current revision), is held back.
 *
 * Viewing writes nothing: no `record_canvas` (the result is already saved)
 * and no `save_canvas_state` (the frame gets its saved state through `init`,
 * and what the person changes here isn't kept). When the store has no
 * record of the canvas, or its visible copy was edited outside Hollow (so the
 * file Accept records is not the saved canvas), `fallback` shows the file in
 * the document view instead, with a line that says why.
 *
 * Host contract (plan L3 X7/X8, 2026-10-14; review U canvases): the head says
 * where it is saved and when it was updated, marks an older version
 * "Older version (vN of M) · read only", and ⋯ offers Compare versions in
 * the gallery (in the gallery itself, its own Compare button).
 */
export function ResultCanvas(props: {
  workspaceId: string;
  id: string;
  /** Changes when the listed file changes, so the canvas is read again. */
  stamp: string;
  fallback: () => JSX.Element;
  onViewing?: (revision: number | null) => void;
  /** The current revision's body (the file's text), for Compare. */
  onLoaded?: (text: string) => void;
  /** Shown in the canvas gallery (F3): no Accept there, so the saved canvas
   *  renders even when its file was edited outside Hollow, and nothing says
   *  what Accept records. */
  gallery?: boolean;
}) {
  const scope = () => ({ workspaceId: props.workspaceId, sessionKey: null });
  const [canvas] = createResource(() => props.stamp, () => api.readCanvas(scope(), props.id));
  const [viewing, setViewing] = createSignal<number | null>(null);
  const [sourceError, setSourceError] = createSignal('');
  let disposed = false;
  onCleanup(() => { disposed = true; });
  const openSource = (c: Canvas, revision: number) => {
    setSourceError('');
    void openCanvasSource(c, revision).catch(e => { if (!disposed) setSourceError(errMsg(e)); });
  };
  createEffect(() => props.onViewing?.(viewing()));
  onCleanup(() => props.onViewing?.(null));
  const [other] = createResource(() => viewing() ?? false, async n => {
    try { const r = await api.readCanvasRevision(scope(), props.id, n); return { n, body: r.html }; }
    catch (e) { return { n, error: errMsg(e) }; }
  });
  const loaded = (): Canvas | null | undefined => canvas.state === 'ready' || canvas.state === 'refreshing' ? canvas.latest : canvas.error ? null : undefined;
  const usable = () => { const c = loaded(); return c && (!c.htmlChanged || props.gallery) ? c : null; };
  createEffect(() => { const c = usable(); if (c) props.onLoaded?.(c.html); });
  const shown = (): { n: number; body: string } | null => {
    const c = usable(); if (!c) return null;
    const v = viewing(); if (v === null || v === c.revision) return { n: c.revision, body: c.html };
    const o = other.latest; return o && o.body !== undefined && o.n === v ? { n: o.n, body: o.body } : null;
  };
  const otherError = () => { const o = other.latest; return o && o.error !== undefined && o.n === viewing() ? o.error : null; };
  return <Show when={loaded() !== undefined} fallback={<Skeleton rows={4} label="Opening the canvas"/>}>
    <Show when={usable()} fallback={<>
      <p class="result-note result-canvas-note" role="status">{loaded()?.htmlChanged
        ? 'This canvas’s file was edited outside Hollow after it was saved, so it opens as the file: that is what Accept records.'
        : `This canvas isn’t in the workspace’s canvas store${canvas.error ? ` (${errMsg(canvas.error)})` : ''}, so it opens as its file.`}</p>
      {props.fallback()}
    </>}>{c => {
      // Picking a version or going back keeps focus on the version label,
      // and the change is said in a live region (C6 P2, as ChatCanvas).
      let root: HTMLDivElement | undefined;
      const [picked, setPicked] = createSignal(false);
      const pick = (n: number | null) => { setPicked(true); setViewing(n); keepFocus(root?.querySelector<HTMLElement>('button.chat-artifact-version')); };
      const said = () => !picked() ? '' : viewing() === null || viewing() === c().revision ? `Showing v${c().revision}, the current version.` : otherError() ? `Couldn’t open v${viewing()}.` : shown() ? `Showing v${viewing()}, an earlier version, read-only.` : `Opening v${viewing()}…`;
      const saved = () => { const w = savedWords({ store: props.workspaceId === QUICK_CHATS_ID ? 'quick' : 'workspace', saved: true, revision: c().revision, latest: c().revision }); const when = relativeWhen(c().updatedAt); return when ? { ...w, text: `${w.text} · updated ${when}` } : w; };
      const older = () => { const v = viewing(); return v !== null && v !== c().revision ? olderLabel(v, c().revision) : null; };
      const compare = (): ActionMenuItem[] => [{ label: 'Open source chat', disabled: !canvasSourceSession(c(), shown()?.n),
        action: () => openSource(c(), shown()?.n ?? c().revision) },
      // A19: a copy of the version shown on an issue; the canvas stays here.
      { label: 'Attach to issue…',
        action: () => openAttachCanvas({ workspaceId: props.workspaceId, canvasId: c().id, revision: shown()?.n ?? c().revision, title: c().title }) }, ...(props.gallery || c().revisions.length < 2 ? [] : [{
        label: 'Compare versions in the gallery',
        action: () => {
          const ws = props.workspaceId;
          void openCanvasGallery(ws, { workspaceId: ws, id: c().id, compare: true, back: { label: 'Back to the workspace', go: () => void visitWorkspace(ws) } });
        },
      }])];
      const badge = <><CanvasVersions title={c().title} shown={viewing() ?? c().revision} own={c().revision} latest={c().revision} revisions={c().revisions}
        onOpen={() => {}} onPick={n => pick(n === c().revision ? null : n)}/><Show when={viewing() === null}><CanvasPinButton canvas={c()}/></Show></>;
      return <div class="result-canvas" ref={root} data-canvas-id={c().id} data-revision={viewing() ?? c().revision}>
        <span class="chat-canvas-live sr-only" role="status">{said()}</span>
        <Show when={sourceError()}><ErrorText class="chat-canvas-note" text={sourceError()} /></Show>
        <Show when={shown()} keyed fallback={<Show when={otherError()} fallback={<Skeleton rows={4} label={`Opening v${viewing()}`}/>}>{e => <ErrorText class="result-note" text={`Couldn’t open v${viewing()}: ${e()}`} />}</Show>}>{s =>
          <HtmlArtifact lookWorkspace={props.workspaceId} code={`\n${innerOf(s.body, '')}\n`} defaultMode={CANVAS_FENCE_MODE}
            canvas={{ title: c().title, source: s.body, badge, ref: { workspaceId: props.workspaceId, revision: s.n, state: canvasState(s.n === c().revision ? c().state : null) } }}
            bridge={{ canvasId: c().id, state: () => (s.n === c().revision ? canvasState(c().state) ?? null : null) }}
            stateTarget={{ revision: c().revision, open: canvasSourceSession(c()) ? () => openSource(c(), c().revision) : undefined }}
            grantKey={canvasGrantKey(scope(), c().id)}
            initialHeight={c().height ?? undefined} fill
            host={props.gallery ? 'gallery' : 'result'} saved={saved()} older={older()} actions={compare()}/>
        }</Show>
        <Show when={viewing() !== null && viewing() !== c().revision}>
          <p class="chat-canvas-note">Showing v{viewing()}, an earlier version, read-only.{props.gallery ? '' : ` Accept records the current version, v${c().revision}.`} <button type="button" class="chat-canvas-back" onClick={() => pick(null)}>Back to v{c().revision}</button></p>
        </Show>
      </div>;
    }}</Show>
  </Show>;
}
