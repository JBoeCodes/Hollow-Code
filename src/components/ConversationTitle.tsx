import { Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from 'solid-js';
import { chatAgentSessionKey, chatCwd } from '../store';
import { lifecycleFailed, workspaceDetail, workspaceSessions } from '../store/workspaces';
import { chatTitleShown, renamedTitle } from '../conversationTools';
import { sessionMenuItems } from '../workspaceMenu';
import { MoreButton } from './MoreButton';
import type { ContextMenuItem } from '../store/overlays';

/** The name as a field: Enter or leaving it saves, Escape cancels, as the
 *  sidebar's row does (`Rail.tsx` RenameField). `keyboard` is true when
 *  Enter or Escape ended it. */
function TitleField(props: { value: string; restored: boolean; onInput: (v: string) => void; onDone: (title: string | null, keyboard: boolean) => void }) {
  let input: HTMLInputElement | undefined;
  let done = false, armed = false, leaving = false;
  // Read once, never as a binding: a refresh of the sessions list rebuilds the
  // saved title, and Solid would write it over what is being typed.
  const start = props.value;
  // A draft handed back after a remount keeps its caret at the end: selected,
  // the next key would replace what was typed.
  const restored = props.restored;
  const finish = (title: string | null, keyboard = false) => { if (done) return; done = true; props.onDone(title, keyboard); };
  // After the closing menu hands focus back to its opener (a microtask), so
  // that hand-back can't blur the field and end the edit at once.
  onMount(() => {
    const t = setTimeout(() => {
      if (!input?.isConnected) return;
      input.focus();
      if (restored) input.setSelectionRange(input.value.length, input.value.length); else input.select();
      armed = true;
    }, 0);
    onCleanup(() => clearTimeout(t));
  });
  onCleanup(() => { leaving = true; });
  return <input ref={input} class="chat-title-input" aria-label="Conversation name" value={start} maxLength={480}
    onInput={e => props.onInput(e.currentTarget.value)}
    onKeyDown={e => {
      if (e.isComposing) return;
      if (e.key === 'Enter') { e.preventDefault(); finish(e.currentTarget.value, true); }
      // Escape cancels the edit, not the run: it stops here, as Find's does.
      else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); finish(null, true); }
    }}
    // A blur from the field being taken out (a remount) saves nothing.
    onBlur={e => { if (armed && !leaving && e.currentTarget.isConnected) finish(e.currentTarget.value); }} />;
}

/**
 * The open conversation's name above its chat (overnight 11-07, D1), as in
 * Claude and ChatGPT: the title, a ⋯ with the same conversation menu as its
 * sidebar row (Rename…, Pin, Move to project…, Archive), and a double-click
 * to rename in place. It shows only where the sidebar's list holds this
 * chat (`workspaceSessions` also carries the open one past the newest 25),
 * so it never names the wrong one. Library → Conversations and the Main
 * agent's space have their own heading and don't mount it.
 */
export function ConversationTitle(props: { onDone?: () => void; items?: () => ContextMenuItem[]; showTitle?: boolean }) {
  const found = createMemo(() => {
    const key = chatAgentSessionKey();
    const ws = workspaceDetail()?.workspace;
    if (!key || !ws || ws.cwd !== chatCwd()) return null;
    const session = workspaceSessions()[ws.id]?.sessions.find(s => s.key === key);
    return session ? { ws, session } : null;
  });
  const shown = () => chatTitleShown(found()?.session);
  const [editing, setEditing] = createSignal(false);
  // What has been typed so far. It outlives the field, so a refresh that takes
  // the row away and brings it back doesn't lose it (the sidebar keeps one too).
  let draft: string | null = null;
  // Another conversation opened (a key that doesn't blur the field): the edit
  // was for the old one, so it ends instead of renaming the new one.
  createEffect(on(chatAgentSessionKey, () => { draft = null; setEditing(false); }, { defer: true }));

  function finish(title: string | null, keyboard: boolean) {
    draft = null;
    setEditing(false);
    // Enter or Escape gives the box its cursor back at once, as the sidebar's
    // row does for its row; the rename runs behind it, so typing straight on
    // never lands on the page while it saves.
    if (keyboard) queueMicrotask(() => props.onDone?.());
    const f = found();
    const name = f && title != null ? renamedTitle(title, f.session.title) : null;
    if (f && name) void import('../conversationActions').then(m => m.renameConversationTo(f.session.key, name), () => lifecycleFailed('That action didn’t load. Try again.'));
  }

  return (
    <Show when={props.showTitle !== false && shown()} fallback={<MoreButton class="chat-actions-menu" label="Actions for this conversation" items={() => props.items?.() ?? []} />}>
      {title => (
        <div class="chat-title-row" role="group" aria-label="Conversation">
          <Show when={editing()} fallback={<>
            <span class="chat-title" title={title()} onDblClick={() => setEditing(true)}>{title()}</span>
            <MoreButton class="chat-actions-menu" label="Actions for this conversation" items={() => {
              const f = found();
              return [...(props.items?.() ?? []), { separator: true }, ...(f ? sessionMenuItems(f.ws, f.session, title(), () => setEditing(true)) : [])];
            }} />
          </>}>
            <TitleField value={draft ?? found()?.session.title?.trim() ?? ''} restored={draft != null} onInput={v => { draft = v; }} onDone={finish} />
          </Show>
        </div>
      )}
    </Show>
  );
}
