import { Show, createSignal, onCleanup, onMount } from 'solid-js';
import { carriesFiles, readDrop } from '../fileDrag';
import { modalOpen } from '../modalGuard';
import { chatComposerOnScreen, chatConversationVersion } from '../store';
import { workspaceId } from '../store/workspaces';

/**
 * Files dropped anywhere on Hollow's window attach to the chat
 * (windowFileDrop.ts, 2026-11-04). The listeners sit on the window, after
 * every inner target: one that takes file drops itself (the message box,
 * Skills' import) cancels the event first, and this leaves it alone. The
 * veil only shows where a drop lands and never takes the pointer, so those
 * targets keep their own highlight; while it shows, frames (reply canvases)
 * let the drag through to the page, or a drop on one would do nothing.
 */
export function WindowFileDrop() {
  const [label, setLabel] = createSignal<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const show = (text: string) => { document.documentElement.dataset.fileDrag = ''; setLabel(text); };
  const hide = () => { clearTimeout(timer); delete document.documentElement.dataset.fileDrag; setLabel(null); };
  // A resting pointer still repeats dragover (WebKit and Chromium), so the
  // veil goes once they stop: the drag left, ended or was cancelled.
  const hideSoon = (ms: number) => { clearTimeout(timer); timer = setTimeout(hide, ms); };
  onMount(() => {
    const over = (e: DragEvent) => {
      if (!carriesFiles(e.dataTransfer) || modalOpen()) return;
      if (e.defaultPrevented) { hide(); return; }
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
      // Where it goes, from the routing's own chunk (the launch chunk's budget).
      if (!label()) { show('Drop files to attach'); void import('../windowFileDrop').then(m => { if (label()) setLabel(m.dropTargetLabel()); }); }
      hideSoon(700);
    };
    // Leaving the window (no element under the pointer).
    const leave = (e: DragEvent) => { if (!e.relatedTarget) hideSoon(100); };
    const drop = (e: DragEvent) => {
      hide();
      if (e.defaultPrevented || !carriesFiles(e.dataTransfer) || modalOpen()) return;
      e.preventDefault();
      // Read while the event is live: the DataTransfer empties after it.
      const items = readDrop(e.dataTransfer);
      // Where it landed, now: the routing chunk loads after (review).
      const origin = { version: chatConversationVersion(), workspace: workspaceId(), onScreen: chatComposerOnScreen() };
      void import('../windowFileDrop').then(m => m.routeDroppedItems(items, origin));
    };
    window.addEventListener('dragenter', over);
    window.addEventListener('dragover', over);
    window.addEventListener('dragleave', leave);
    window.addEventListener('drop', drop);
    window.addEventListener('dragend', hide);
    onCleanup(() => {
      window.removeEventListener('dragenter', over);
      window.removeEventListener('dragover', over);
      window.removeEventListener('dragleave', leave);
      window.removeEventListener('drop', drop);
      window.removeEventListener('dragend', hide);
      hide();
    });
  });
  return <Show when={label()}>{text =>
    <div class="window-file-drop" aria-hidden="true"><span class="window-file-drop-label">{text()}</span></div>}
  </Show>;
}
