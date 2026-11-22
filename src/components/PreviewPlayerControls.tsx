import { Show, createEffect, createSignal, on, onCleanup, onMount } from 'solid-js';
import { closePreview, previewTarget } from '../store/preview';
import '../styles/preview-player.css';

/** The same pane and content stay mounted in both sizes, including native
 * web tabs and media playback. Only its layout and toolbar visibility change. */
export function PreviewPlayerControls() {
  let controls!: HTMLDivElement;
  let pane: HTMLElement | null = null;
  const [mini, setMini] = createSignal(false);
  onMount(() => {
    pane = controls.closest<HTMLElement>('.preview-pane');
    createEffect(() => pane?.classList.toggle('preview-mini', mini()));
  });
  // A new explicit target opens at normal size, with all controls available.
  createEffect(on(previewTarget, () => setMini(false), { defer: true }));
  onCleanup(() => pane?.classList.remove('preview-mini'));
  return <div ref={controls} class="preview-player-controls">
    <Show when={mini()}><span class="preview-player-title">Preview</span></Show>
    <button type="button" class="btn small ghost" aria-label={mini() ? 'Expand preview' : 'Minimize preview'} aria-expanded={!mini()}
      onClick={() => setMini(v => !v)}>{mini() ? 'Expand' : 'Mini player'}</button>
    <Show when={mini()}><button type="button" class="chat-panel-iconbtn" aria-label="Close preview" title="Close preview (Esc)" onClick={closePreview}>×</button></Show>
  </div>;
}
