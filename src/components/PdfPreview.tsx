import { Show, createMemo, createResource, createSignal, untrack } from 'solid-js';
import { api } from '../api';
import { unlessErrored } from '../stableResource';
import { PlainErrorText } from './ui';
import '../styles/pdf-preview.css';

type Position = { page: number; zoom: number; top?: number; left?: number };
const positions = new Map<string, Position>();

/** A read-only native page image: no PDF script, embedded attachment, link
 * action or external app runs. Closing the parent leaves the chat untouched. */
export function PdfPreview(props: { path: string; nonce?: number; sessionKey?: string | null }) {
  let surface: HTMLDivElement | undefined;
  const restoreScroll = () => requestAnimationFrame(() => { const saved=untrack(position); if(surface){surface.scrollTop=saved.top??0;surface.scrollLeft=saved.left??0;} });
  const key = () => JSON.stringify([props.sessionKey ?? '', props.path]);
  const [selection, setSelection] = createSignal<{ key: string; position: Position }>();
  const position = () => selection()?.key === key() ? selection()!.position : positions.get(key()) ?? { page: 0, zoom: 1 };
  const remember = (update: Partial<Position>) => {
    const value = { ...position(), ...update };
    positions.delete(key()); positions.set(key(), value);
    while (positions.size > 32) positions.delete(positions.keys().next().value!);
    setSelection({ key: key(), position: value });
  };
  const page = createMemo(() => position().page);
  const zoom = createMemo(() => position().zoom);
  const setZoom = (next: number | ((value: number) => number)) => remember({ zoom: typeof next === 'number' ? next : next(zoom()) });
  const [read, { refetch }] = createResource(
    () => ({ path: props.path, nonce: props.nonce ?? 0, page: page(), session: props.sessionKey }),
    async source => ({ ...source, value: await api.pdfPreviewPage(source.path, source.page, source.session) }),
  );
  const loaded = () => unlessErrored(read);
  const document = () => {
    const value = loaded();
    return value?.path === props.path && value.session === props.sessionKey ? value : null;
  };
  const current = createMemo(() => {
    const value = document();
    return value?.page === page() && value.nonce === (props.nonce ?? 0) ? value.value : null;
  });
  const valid = () => current()?.dataUrl.startsWith('data:image/png;base64,') && current()!.width > 0 && current()!.height > 0;
  const pages = () => document()?.value.pages ?? 0;
  const name = () => props.path.slice(props.path.lastIndexOf('/') + 1);
  const change = (next: number) => { remember({ page: next, top: 0, left: 0 }); };
  // Refresh/retry keeps the reading position. A shortened document exposes
  // a separate, explicit way to recover from an out-of-range page.
  const retry = () => { void refetch(); };
  return <section class="pdf-preview" aria-label={`PDF preview: ${name()}`} aria-busy={read.loading}>
    <div class="pdf-preview-toolbar" role="group" aria-label="PDF controls">
      <button type="button" class="btn ghost small" aria-label="Previous PDF page" disabled={read.loading || page() === 0} onClick={() => change(page() - 1)}>Previous</button>
      <span class="pdf-preview-count" role="status" aria-label="PDF page count">Page {page() + 1}<Show when={pages()}> of {pages()}</Show></span>
      <button type="button" class="btn ghost small" aria-label="Next PDF page" disabled={read.loading || !pages() || page() + 1 >= pages()} onClick={() => change(page() + 1)}>Next</button>
      <span class="pdf-preview-zoom" role="group" aria-label="PDF zoom">
        <button type="button" class="btn ghost small" aria-label="Zoom out PDF" disabled={zoom() <= 1} onClick={() => setZoom(z => Math.max(1, z - .25))}>−</button>
        <button type="button" class="btn ghost small" aria-label={`${zoom() === 1 ? 'Fit' : `${Math.round(zoom() * 100)}% · Fit`} PDF to width`} onClick={() => setZoom(1)}>{zoom() === 1 ? 'Fit' : `${Math.round(zoom() * 100)}%`}</button>
        <button type="button" class="btn ghost small" aria-label="Zoom in PDF" disabled={zoom() >= 2} onClick={() => setZoom(z => Math.min(2, z + .25))}>+</button>
      </span>
      <button type="button" class="btn ghost small" aria-label="Refresh PDF preview" disabled={read.loading} onClick={retry}>Refresh</button>
    </div>
    <Show when={read.error && !read.loading}>
      <div class="pdf-preview-message" role="alert"><strong>Couldn’t preview this PDF</strong><PlainErrorText text={String(read.error)}/><button type="button" class="btn small" onClick={retry}>Retry PDF preview</button><Show when={page() > 0}><button type="button" class="btn small" onClick={() => change(0)}>Go to first PDF page</button></Show></div>
    </Show>
    <Show when={read.loading}><p class="pdf-preview-message" role="status">Loading PDF page {page() + 1}…</p></Show>
    <Show when={!read.loading && !read.error && current()}>
      <Show when={valid()} fallback={<p class="pdf-preview-message" role="alert">This PDF page could not be displayed. Use Refresh to try again.</p>}>
        <div ref={surface} class="pdf-preview-surface" role="region" aria-label={`PDF page ${page() + 1} of ${pages()}`} tabIndex={0} onScroll={e=>{if(!read.loading && valid() && e.currentTarget.isConnected && e.currentTarget.getClientRects().length) remember({top:e.currentTarget.scrollTop,left:e.currentTarget.scrollLeft});}}><img onLoad={restoreScroll} class="pdf-preview-page" style={{ width: `${zoom() * 100}%`, 'aspect-ratio': `${current()!.width} / ${current()!.height}` }} src={current()!.dataUrl} alt={`${name()}, page ${page() + 1} of ${pages()}`}/></div>
        <details class="pdf-preview-text"><summary>Read page text</summary><Show when={current()?.text} fallback={<p>No text layer on this page.</p>}><pre>{current()?.text}</pre></Show><Show when={current()?.textTruncated}><p>Page text is shortened; the visual page remains available.</p></Show></details>
      </Show>
    </Show>
  </section>;
}
