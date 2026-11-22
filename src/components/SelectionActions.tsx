import { Show, createSignal, onCleanup, type JSX } from 'solid-js';
import { Icon } from '../icons';

/**
 * Actions on selected text in a reading surface (next-level P08): turn a
 * passage into a task, or ask the agent about it. The bar appears just above
 * the selection, only while there is a selection inside this region, and
 * every action is also reachable from the keyboard (select, then ⌥⌘T / ⌥⌘A
 * while focus is inside the region; ⌘⌥⇧T is the global quick capture). These
 * in-surface keys follow the local-command policy in docs/reference/keyboard-shortcuts.md:
 * they fire only with focus inside the surface, so they stay on when app
 * shortcuts are turned off. Nothing is created without the task
 * sheet or the composer, where the user confirms.
 */
export function SelectionActions(props: { children: JSX.Element; onTask: (text: string) => void; onAsk: (text: string) => void; class?: string }) {
  let region: HTMLDivElement | undefined;
  const [bar, setBar] = createSignal<{ x: number; y: number; text: string } | null>(null);
  const read = () => {
    const sel = window.getSelection();
    const text = sel?.toString().trim() ?? '';
    if (!sel || !text || !region || !sel.rangeCount || !region.contains(sel.getRangeAt(0).commonAncestorContainer)) { setBar(null); return; }
    const rect = sel.getRangeAt(0).getBoundingClientRect();
    const host = region.getBoundingClientRect();
    setBar({ x: Math.max(8, Math.min(rect.left - host.left + rect.width / 2, host.width - 8)), y: rect.top - host.top + region.scrollTop, text: text.slice(0, 4000) });
  };
  const onSelectionChange = () => { if (!window.getSelection()?.toString().trim()) setBar(null); };
  document.addEventListener('selectionchange', onSelectionChange);
  onCleanup(() => document.removeEventListener('selectionchange', onSelectionChange));
  const act = (fn: (t: string) => void) => { const b = bar(); if (b) { fn(b.text); setBar(null); window.getSelection()?.removeAllRanges(); } };
  return <div ref={region} class={`selection-actions-region${props.class ? ` ${props.class}` : ''}`} onMouseUp={() => setTimeout(read, 0)} onKeyUp={e => { if (e.shiftKey || e.key === 'Shift') read(); }}
    onKeyDown={e => {
      if (!(e.metaKey || e.ctrlKey) || !e.altKey) return;
      if (e.code === 'KeyT') { read(); if (bar()) { e.preventDefault(); act(props.onTask); } }
      if (e.code === 'KeyA') { read(); if (bar()) { e.preventDefault(); act(props.onAsk); } }
    }}>
    {props.children}
    <Show when={bar()}>{b =>
      <div class="selection-actions" role="toolbar" aria-label="Selected text" style={{ left: `${b().x}px`, top: `${b().y}px` }} onMouseDown={e => e.preventDefault()}>
        <button type="button" onClick={() => act(props.onTask)} title="Make a task from the selection (⌥⌘T)"><Icon name="checkSquare" size={13}/>Make a task</button>
        <button type="button" onClick={() => act(props.onAsk)} title="Ask the agent about the selection (⌥⌘A)"><Icon name="message" size={13}/>Ask about this</button>
      </div>}
    </Show>
  </div>;
}
