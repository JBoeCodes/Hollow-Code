import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { render } from 'solid-js/web';
import { openFileInSidePane } from '../store/chat';
import '../styles/image-zoom.css';

/** Only an already-loaded data image: this dialog never fetches a URL. */
export function showImageZoom(src: string, alt: string, path?: string) {
  if (!/^data:image\//i.test(src)) return;
  const host = document.createElement('div'); document.body.append(host);
  let dispose: (() => void) | undefined;
  const close = () => { queueMicrotask(() => { dispose?.(); host.remove(); }); };
  dispose = render(() => <ImageZoom src={src} alt={alt} path={path} close={close} />, host);
}

function ImageZoom(props: { src: string; alt: string; path?: string; close: () => void }) {
  let dialog!: HTMLDialogElement;
  let stage!: HTMLDivElement;
  let image!: HTMLImageElement;
  const [scale, setScale] = createSignal(1);
  const [width, setWidth] = createSignal(1);
  const [fitted, setFitted] = createSignal(true);
  const fit = () => {
    setFitted(true);
    setWidth(image.naturalWidth || 1);
    if (image.naturalWidth) setScale(Math.min(1, stage.clientWidth / image.naturalWidth, stage.clientHeight / image.naturalHeight));
  };
  const zoom = (factor: number) => { setFitted(false); setScale(s => Math.max(.1, Math.min(4, s * factor))); };
  onMount(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.showModal();
    const observer = new ResizeObserver(() => { if (fitted()) fit(); });
    observer.observe(stage);
    onCleanup(() => { observer.disconnect(); dialog.close(); if (before?.isConnected) before.focus(); });
  });
  return <dialog ref={dialog} class="modal image-zoom-modal" aria-label={`Expanded image: ${props.alt}`} onCancel={e => { e.preventDefault(); props.close(); }} onClick={e => { if (e.target === dialog) props.close(); }}>
    <div class="image-zoom-content">
      <div class="image-zoom-controls">
        <button type="button" class="btn ghost" aria-label="Zoom out" onClick={() => zoom(1 / 1.25)}>Zoom out</button>
        <output aria-live="polite">{Math.round(scale() * 100)}%</output>
        <button type="button" class="btn ghost" aria-label="Zoom in" onClick={() => zoom(1.25)}>Zoom in</button>
        <button type="button" class="btn ghost" onClick={fit}>Fit</button>
        <button type="button" class="btn ghost" onClick={() => { setFitted(false); setScale(1); }}>Actual size</button>
        <Show when={props.path}><button type="button" class="btn ghost" onClick={() => { openFileInSidePane(props.path!); props.close(); }}>Open in Preview</button></Show>
        <button type="button" class="btn ghost" autofocus onClick={props.close}>Close</button>
      </div>
      <div ref={stage} class="image-zoom-stage"><img ref={image} src={props.src} alt={props.alt} onLoad={fit} style={{ width: `${width() * scale()}px` }} /></div>
    </div>
  </dialog>;
}
