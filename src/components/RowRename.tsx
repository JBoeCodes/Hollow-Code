import { onCleanup, onMount, type JSX } from 'solid-js';
import { Show } from 'solid-js';
import { renamingKey, setRenamingKey } from '../store/rowRename';

/** A sidebar row's name as a field (3.6): Enter or leaving the field saves, Escape keeps the old
 *  name. An empty or unchanged name is no change (the commit callbacks decide). */
export function RowRename(props: { value: string; label: string; maxLength?: number; onCommit: (name: string) => unknown }) {
  let input!: HTMLInputElement;
  let done = false;
  // A blur the person did not cause (the chat view grabbing focus after the row opened it, the
  // menu handing focus back to its button) must not end the edit: only a click outside, Tab, or
  // Enter and Escape do.
  let leaving = false;
  const finish = (save: boolean, restoreFocus = false) => {
    if (done) return;
    done = true;
    const head = input.closest('.work-tree-head');
    const name = input.value.trim();
    document.removeEventListener('pointerdown', outside, true);
    setRenamingKey(null);
    if (restoreFocus) queueMicrotask(() => head?.querySelector<HTMLElement>('[data-key-item]')?.focus());
    if (save && name && name !== props.value.trim()) void props.onCommit(name);
  };
  const outside = (e: PointerEvent) => { if (!input.contains(e.target as Node)) leaving = true; };
  onMount(() => { document.addEventListener('pointerdown', outside, true); input.focus(); input.select(); });
  onCleanup(() => document.removeEventListener('pointerdown', outside, true));
  return <input ref={input} class="work-row-rename" type="text" value={props.value} maxLength={props.maxLength ?? 120} aria-label={props.label}
    onKeyDown={e => {
      if (e.key === 'Enter') { e.preventDefault(); e.stopPropagation(); finish(true, true); }
      else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); finish(false, true); }
      else { if (e.key === 'Tab') leaving = true; e.stopPropagation(); }
    }}
    onBlur={() => {
      if (done) return;
      // A blur the person did not cause hands focus straight back; switching apps keeps the edit.
      if (!leaving) { if (document.hasFocus()) queueMicrotask(() => { if (!done && input.isConnected) input.focus(); }); return; }
      finish(true);
    }}
    onClick={e => e.stopPropagation()}
    onDblClick={e => e.stopPropagation()}/>;
}

/** The field while `id` is being renamed, else the row's usual content. */
export function Renamable(props: { id: string; value: string; label: string; maxLength?: number; commit: (name: string) => unknown; children: JSX.Element }) {
  return <Show when={renamingKey() === props.id} fallback={props.children}>
    <RowRename value={props.value} label={props.label} maxLength={props.maxLength} onCommit={props.commit}/>
  </Show>;
}

/** Save a chat's new name from its row; `after` re-reads the list the row came from. */
export function renameChatFromRow(key: string, name: string, after?: () => unknown) {
  return import('../conversationActions').then(m => m.renameConversationTo(key, name, after), () => undefined);
}
