/** Storeless export document. Only this trusted host can call the capture job;
 *  the agent HTML stays in an opaque-origin, network-free artifact iframe. */
import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { acceptFrameMessage, initMessage, themeMessage } from '../canvasBridge';
import { artifactDocUrl, loadCanvasKit, loadCanvasKitAsk, NO_KIT, SANDBOX, sandboxDocParts, themePairs } from '../components/htmlSandbox';
import { ARTIFACT_STYLES } from './artifactStyles';
import { canvasFrameBody, parseCanvasFence } from './fence';
import type { CanvasExportRequest, CaptureJobMeasure, MeasureText } from './canvasExport';

/** Repo ideas 2026-10-27 #4: what a Measure on the picture pass adds to the
 *  host's styles. Every pass freezes animation and transition (the four
 *  pictures must match), and every pass but `shown` sets only the text's own
 *  fill (`-webkit-text-fill-color`, never `color`, so anything painted in
 *  `currentColor` stays put), with shadows, strokes, decorations and
 *  emphasis marks out of the way. The id-weighted selector outranks the
 *  agent's own `!important` rules; an inline `!important` one still wins,
 *  and that text then scores as it is. */
const MEASURE_ALL = ':not(#hc-measure#hc-measure#hc-measure#hc-measure)';
export function measureCss(text: MeasureText): string {
  const every = `${MEASURE_ALL},${MEASURE_ALL}::before,${MEASURE_ALL}::after`;
  const frozen = `${every}{animation:none!important;transition:none!important}`;
  if (text === 'shown') return frozen;
  const fill = text === 'hidden' ? 'transparent' : text === 'black' ? '#000' : '#fff';
  return `${frozen}${every}{-webkit-text-fill-color:${fill}!important;-webkit-text-stroke-color:${fill}!important;text-shadow:none!important;text-decoration-color:transparent!important;text-emphasis-color:transparent!important}`;
}

export function CaptureHost() {
  const [url, setUrl] = createSignal('');
  const [height, setHeight] = createSignal(80);
  let frame: HTMLIFrameElement | undefined;
  let job: { request: CanvasExportRequest; width: number; measure?: CaptureJobMeasure } | undefined;
  let canvasId = '', kitVersion = '', done = false, disposed = false;
  let colors: Array<[string,string]> = [];
  const fail = () => { if (!done) { done = true; void invoke('canvas_capture_failed').catch(() => {}); } };
  const init = () => {
    if (!job || !frame) return;
    const r = job.request;
    frame.contentWindow?.postMessage(themeMessage(r.scheme, colors), '*');
    if (job.measure) {
      // Measure on the picture draws as the reply's frame did: texture only
      // ever shows with motion on, so its texture choice sets both.
      const texture = job.measure.texture;
      frame.contentWindow?.postMessage({ __hc: 1, kind: 'motion', reducedMotion: !texture, textureAllowed: texture }, '*');
      frame.contentWindow?.postMessage(initMessage({ canvasId, kit: kitVersion, locale: navigator.language, reducedMotion: !texture, state: r.state ?? null }), '*');
      return;
    }
    // Export is deterministic/plain: explicitly veto texture, even if the
    // normal frame watcher is added here later. Source requests stay intact.
    frame.contentWindow?.postMessage({ __hc: 1, kind: 'motion', reducedMotion: true, textureAllowed: false }, '*');
    frame.contentWindow?.postMessage(initMessage({ canvasId, kit: kitVersion, locale: navigator.language, reducedMotion: true, state: r.state ?? null }), '*');
  };
  const onMessage = (event: MessageEvent) => {
    if (!frame || !job || done) return;
    const parsed = acceptFrameMessage(event, frame, canvasId);
    if (!parsed.ok) return;
    const m = parsed.message;
    if (m.kind === 'error') { fail(); return; }
    if (m.kind !== 'size' && m.kind !== 'settled') return;
    const cap = job.request.format === 'pdf' ? 14400 : 8000;
    if (m.kind === 'size') { setHeight(Math.min(cap, Math.max(80, m.h))); return; }
    done = true;
    setHeight(Math.min(cap, Math.max(job.request.preset === 'slide' ? 720 : 80, m.h)));
    // Let the trusted host's layout catch up before Rust snapshots the view.
    requestAnimationFrame(() => requestAnimationFrame(() => { void invoke('canvas_capture_ready', { height: m.h }).catch(() => {}); }));
  };
  onMount(() => {
    window.addEventListener('message', onMessage);
    void (async () => {
      job = await invoke('canvas_capture_job');
      if (!job || disposed) return;
      const r = job.request, parsed = parseCanvasFence(r.source, true);
      if (parsed.state !== 'ready') throw new Error('Invalid canvas');
      canvasId = parsed.root.id;
      document.documentElement.dataset.theme = r.scheme;
      document.documentElement.dataset.motion = 'reduced';
      colors = Object.entries(themeMessage(r.scheme, r.look === 'match' ? r.theme : themePairs()).vars);
      const background = colors.find(([k]) => k === '--app-canvas')?.[1];
      if (background) document.body.style.backgroundColor = background;
      const kit = await loadCanvasKit(), ask = parsed.root.ask ? await loadCanvasKitAsk() : NO_KIT;
      if (!kit.js || (parsed.root.ask && !ask.js)) throw new Error('Canvas kit unavailable');
      kitVersion = kit.version;
      const styles = job.measure ? ARTIFACT_STYLES + measureCss(job.measure.text) : ARTIFACT_STYLES;
      const next = await artifactDocUrl(sandboxDocParts(styles, 'canvas', kit, canvasFrameBody(parsed), ask,
        { scheme: r.scheme, pairs: colors, adjusted: [] }), 'canvas');
      if (!next) throw new Error('Canvas document unavailable');
      await invoke('canvas_capture_job', { documentUrl: next });
      if (!disposed) setUrl(next);
    })().catch(fail);
  });
  onCleanup(() => { disposed = true; window.removeEventListener('message', onMessage); });
  return <Show when={url()}><iframe ref={frame} title="Canvas export" sandbox={SANDBOX} src={url()} onLoad={init}
    style={{ border: '0', display: 'block', width: `${job?.width ?? 800}px`, height: `${height()}px` }} /></Show>;
}
