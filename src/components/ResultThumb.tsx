import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, on, onCleanup } from 'solid-js';
import { api } from '../api';
import { Icon, type IconName } from '../icons';
import { latestRequest } from '../latestRequest';
import { joinPath } from '../chatFileLinks';
import { vaultRoot } from '../store/chat';
import { canvasStructure, structureSummary, type CanvasStructure } from '../canvas/structure';
import type { ResultCard } from '../resultsCards';
import { resultCover } from '../resultCover';
import '../styles/result-thumb.css';

/** Shared host-drawn preview. Callers bound enabled cards; disabled ones never read. */
export function ResultThumb(props: { card: ResultCard; workspaceId: string; enabled: boolean }) {
  const c = () => props.card;
  const stamp = () => `${props.workspaceId}:${c().path}:${c().modified ?? ''}:${c().size}:${c().canvas?.revision ?? ''}`;
  return <Switch fallback={c().thumb ? <KindThumb icon={c().icon}/> : <DocumentCover card={c()}/>}>
    <Match when={props.enabled && c().thumb === 'image'}><ImageThumb path={c().path} stamp={stamp()} icon={c().icon}/></Match>
    <Match when={props.enabled && c().thumb === 'poster'}><PosterThumb path={c().path} stamp={stamp()} icon={c().icon}/></Match>
    <Match when={props.enabled && c().thumb === 'canvas' && c().canvas}><CanvasThumb workspaceId={props.workspaceId} id={c().canvas!.id} stamp={stamp()}/></Match>
  </Switch>;
}

function DocumentCover(props: { card: ResultCard }) {
  const cover = () => resultCover(props.card);
  return <div class="res-thumb res-thumb-document" aria-hidden="true">
    <div class="res-cover-sheet"><Icon name={props.card.icon} size={22}/><strong>{cover().format}</strong><Show when={cover().size}>{size => <span>{size()}</span>}</Show></div>
  </div>;
}

function KindThumb(props: { icon: IconName }) {
  return <div class="res-thumb res-thumb-icon" aria-hidden="true"><Icon name={props.icon} size={32}/></div>;
}

/** Raster previews only; failed and obsolete reads leave the kind icon. */
function RasterThumb(props: { path: string; stamp: string; icon: IconName; poster?: boolean }) {
  const [src, setSrc] = createSignal<string | null>(null);
  const reads = latestRequest();
  onCleanup(() => reads.cancel());
  createEffect(on(() => `${props.path}:${props.stamp}`, () => {
    setSrc(null);
    const ticket = reads.start();
    void (async () => {
      const root = await vaultRoot();
      if (!root || !reads.isLatest(ticket)) return;
      const path = joinPath(root, props.path);
      const url = props.poster ? await api.mediaPoster(path) : (await api.chatPreviewFile(path)).dataUrl;
      if (reads.isLatest(ticket)) setSrc(url && /^data:image\/(png|jpeg|gif|webp);base64,/i.test(url) ? url : null);
    })().catch(() => { if (reads.isLatest(ticket)) setSrc(null); });
  }));
  return <Show when={src()} fallback={<KindThumb icon={props.icon}/>}>{url =>
    <div class={`res-thumb ${props.poster ? 'res-thumb-poster' : 'res-thumb-image'}`} aria-hidden="true">
      <img src={url()} alt="" onError={() => setSrc(null)}/>
      <Show when={props.poster}><span class="res-thumb-play"><Icon name="play" size={16}/></span></Show>
    </div>
  }</Show>;
}
export function ImageThumb(props: { path: string; stamp: string; icon: IconName }) { return <RasterThumb {...props}/>; }
export function PosterThumb(props: { path: string; stamp: string; icon: IconName }) { return <RasterThumb {...props} poster/>; }

/** The fence is parsed only for structure. Agent markup never reaches this DOM. */
export function CanvasThumb(props: { workspaceId: string; id: string; stamp: string }) {
  const key = () => `${props.workspaceId}:${props.id}:${props.stamp}`;
  const [canvas] = createResource(key, async () => ({
    key: key(), value: await api.readCanvas({ workspaceId: props.workspaceId, sessionKey: null }, props.id).catch(() => null),
  }));
  // Reading .latest while unresolved registers with an outer Suspense.
  // Thumbnails must never hide the gallery's controls behind a pending read.
  const c = () => (canvas.state === 'ready' || canvas.state === 'refreshing') && canvas.latest?.key === key() ? canvas.latest.value : null;
  const structure = createMemo(() => { const v = c(); return v ? canvasStructure(v.html) : null; });
  return <Show when={c()} fallback={<KindThumb icon="appWindow"/>}>{v =>
    <CanvasSketch structure={structure()} label={`Canvas ${v().title}, v${v().revision}: ${structure() ? structureSummary(structure()!) : 'unreadable'}`}/>
  }</Show>;
}

/** Also used for unsaved conversation fences: no invented workspace/store ID. */
export function CanvasSketch(props: { structure: CanvasStructure | null; label: string }) {
  return <div class="res-thumb res-thumb-canvas" role="img" aria-label={props.label}>
      <svg viewBox="0 0 200 104" preserveAspectRatio="xMidYMid meet" aria-hidden="true">
        <For each={props.structure?.parts.slice(0, 4) ?? [{label:'Text and layout',count:1}]}>{(part, i) =>
          <g transform={`translate(${8 + i() % 2 * 96} ${8 + Math.floor(i() / 2) * 46})`} data-part={part.label}>
            <rect class="res-sketch-panel" width="88" height="38" rx="4"/>
            <Switch fallback={<path d="M8 10h62M8 18h48M8 26h56"/>}>
              <Match when={/stats?/.test(part.label)}><For each={Array.from({length:Math.min(3,part.count)})}>{(_, j) => <g transform={`translate(${8+j()*25} 9)`}><rect class="res-sketch-fill" width="17" height="13" rx="2"/><path d="M0 20h17"/></g>}</For></Match>
              <Match when={/chart|bar|progress/.test(part.label)}><path d="M8 7v24h72"/><path class="res-sketch-accent" d="M20 27V19M37 27V10M54 27V15M71 27V6"/></Match>
              <Match when={/table/.test(part.label)}><path d="M8 8h72v23H8ZM8 16h72M8 24h72M34 8v23M58 8v23"/></Match>
              <Match when={/timeline|diagram/.test(part.label)}><path d="M18 19h52"/><For each={[10,36,62]}>{x => <rect class="res-sketch-fill" x={x} y="12" width="16" height="14" rx="3"/>}</For></Match>
            </Switch>
          </g>
        }</For>
      </svg>
    </div>;
}
