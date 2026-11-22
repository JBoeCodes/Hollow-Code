import { Show, createEffect, createSignal, onCleanup } from 'solid-js';
import { api } from '../api';
import { createPalettePreview, previewText, type PalettePreviewTarget } from '../palettePreview';

/** Read-only and lazy. The parent mounts only under the shared experimental
 * policy; query, selection, close and policy changes cancel pending delivery. */
export default function PalettePreview(props: { target?: PalettePreviewTarget; query: string; returnFocus: () => void }) {
  let region!: HTMLElement;
  const [text, setText] = createSignal('');
  const [state, setState] = createSignal('');
  const reader = createPalettePreview(path => api.vaultNote(path).then(note => note.content));
  createEffect(() => {
    const target = props.target;
    void props.query;
    reader.cancel();
    setText('');
    setState(target?.path ? 'Reading preview…' : '');
    if (!target) return;
    if (!target.path) { setText(previewText(target.body ?? '')); return; }
    const timer = setTimeout(() => reader.load(target.path!, result => {
      setState(result.ok ? '' : 'Preview unavailable. Open the result to read it.');
      if (result.ok) setText(previewText(result.value));
    }), 180);
    onCleanup(() => { clearTimeout(timer); reader.cancel(); });
  });
  onCleanup(() => { reader.cancel(); if (region?.contains(document.activeElement)) props.returnFocus(); });
  return <aside ref={region} class="palette-preview" aria-label="Result preview" tabIndex={0}>
    <p role="status" class="palette-preview-status">{state()}</p>
    <Show when={props.target} fallback={<p class="palette-preview-empty">Highlight a workspace, task or note to preview it here.</p>}>
      {target => <>
        <div class="palette-preview-kind">{target().kind} <span>Preview</span></div>
        <h2>{target().title}</h2>
        <Show when={target().facts}><p class="palette-preview-facts">{target().facts}</p></Show>
        <Show when={text()} fallback={!state() ? <p>No preview text.</p> : undefined}>
          <div class="palette-preview-text">{text()}</div>
        </Show>
        <p class="palette-preview-caption">{target().path ?? 'From the loaded task list'} · Open for full details</p>
      </>}
    </Show>
  </aside>;
}
