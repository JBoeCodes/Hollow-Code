// Side chat pane (t3code parity 7.6, docs/SIDE_CHAT.md): a second chat beside the main chat's
// transcript, for a quick question about it that must not disturb its run. Mounted by Work.tsx
// in the desk's second column while open. It is the one place the side chat is drawn: its
// state is store/sideChat.ts (its own turns, read from the events tagged with its own
// conversation key), never chatRuntime.ts, so the main chat's transcript, busy flag and message
// box are not touched. Nothing goes back to the main chat unless the person presses "Add to main
// chat", which puts words in its message box as a draft.
import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from 'solid-js';
import type { AgentConversationTurn } from '../types';
import { parseSegments } from '../chatSegments';
import { chatAgentSessionKey, chatBusy, flashFooter, pendingApprovals, approvalsResponding, respondToApproval, returnToConversation } from '../store';
import { openExternal } from '../utils';
import { Icon } from '../icons';
import { Button, IconButton } from './ui';
import { CodeBlock } from './CodeBlock';
import { MarkdownView } from './MarkdownView';
import {
  bindSideChat, closeSideChat, newSideChat, openEarlierSideChat, resultForMain, sendSideChat, setSideChatSameAccess, sideChat,
  sideChatSameAccess, stopSideChat,
} from '../store/sideChat';
import '../styles/side-chat.css';

/** A reply's text: prose, code, and a placeholder for each canvas (a canvas is drawn in the main chat). */
function SideText(props: { text: string }) {
  const segments = createMemo(() => parseSegments(props.text));
  const link = (e: MouseEvent | KeyboardEvent) => {
    const a = (e.target as Element | null)?.closest?.('a');
    if (!a) return;
    const href = a.getAttribute('href') ?? '';
    if (/^https?:\/\//i.test(href)) { e.preventDefault(); openExternal(href); }
  };
  return (
    <For each={segments()}>
      {seg => seg.kind === 'md'
        ? <MarkdownView text={seg.text} class="turn-md assistant" forbidMedia onLinkClick={link} />
        : seg.kind === 'canvas'
          ? <div class="side-chat-canvas" role="note"><Icon name="presentation" size={14} /><span>A canvas: ask for it in the main chat.</span></div>
          : <CodeBlock code={seg.code} lang={seg.lang} />}
    </For>
  );
}

const toolLine = (names: readonly string[]): string => {
  const unique = [...new Set(names)];
  return names.length === 0 ? '' : names.length === 1 ? `Used ${unique[0]}` : `${names.length} tools: ${unique.join(', ')}`;
};

export default function SideChatPane(props: { closing?: boolean }) {
  const s = sideChat;
  const parent = () => chatAgentSessionKey();
  const [draft, setDraft] = createSignal('');
  const [sending, setSending] = createSignal(false);
  const [answering, setAnswering] = createSignal<string | null>(null);
  let box: HTMLTextAreaElement | undefined;
  let scroller: HTMLDivElement | undefined;
  let pinned = true;

  // Follow the main chat on screen: another chat opens its own side chat. The side chat's own
  // events keep arriving for the old one, and are ignored once the key changes.
  createEffect(on(parent, p => { void bindSideChat(p ?? null); }));
  // A chat's first turn creates its record a moment after its key shows up: read again once
  // the main chat settles if it wasn't ready.
  createEffect(on(chatBusy, () => { if (!s().key && parent() && !s().loading) void bindSideChat(parent()); }, { defer: true }));

  const mine = createMemo(() => pendingApprovals().filter(a => !!s().key && a.sessionKey === s().key));
  const atBottom = () => !scroller || scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 48;
  createEffect(on(() => [s().turns.length, s().live?.text.length, s().pending, mine().length], () => {
    if (pinned && scroller) queueMicrotask(() => { if (scroller) scroller.scrollTop = scroller.scrollHeight; });
  }));
  onMount(() => {
    box?.focus();
    // Escape closes the pane while focus is in it (a visible Close button does the same).
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || document.querySelector('.ctx-menu, [role="dialog"][aria-modal="true"]')) return;
      const at = document.activeElement;
      if (!at || !(at as Element).closest?.('.side-chat')) return;
      e.preventDefault();
      closeSideChat();
    };
    document.addEventListener('keydown', key);
    onCleanup(() => document.removeEventListener('keydown', key));
  });

  const ready = () => !!s().key && !s().loading;
  async function send() {
    const text = draft().trim();
    if (!text || sending() || s().busy || !ready()) return;
    setSending(true);
    pinned = true;
    const sent = await sendSideChat(text);
    if (sent) setDraft('');
    setSending(false);
  }

  /** The visible button: words into the main chat's message box as a draft. Nothing is sent. */
  function addToMain(text: string) {
    const p = s().parent;
    if (!p || !text.trim()) return;
    returnToConversation(p, resultForMain(text), [], null);
    flashFooter('Added to the main chat’s message box. Edit it, then send it there.', 5000);
  }

  async function answer(id: string, approved: boolean) {
    setAnswering(id);
    try { await respondToApproval(id, approved); } finally { setAnswering(null); }
  }

  const sub = () => sideChatSameAccess()
    ? 'Reads this chat. Can use the same access as it, never more.'
    : 'Reads this chat and answers. It can’t change files.';
  const earlierOptions = () => s().earlier;

  return (
    <aside class="side-chat" aria-label="Side chat" data-closing={props.closing ? '' : undefined}>
      <header class="side-chat-head">
        <h2 class="side-chat-title">Side chat</h2>
        <p class="side-chat-sub">{sub()}</p>
        <span class="side-chat-tools">
          <Show when={earlierOptions().length > 0}>
            <select class="side-chat-earlier" aria-label="Earlier side chats" value={s().key ?? ''} disabled={s().busy}
              onChange={e => void openEarlierSideChat(e.currentTarget.value)}>
              <Show when={s().key && !earlierOptions().some(x => x.key === s().key)}><option value={s().key!}>This side chat</option></Show>
              <For each={earlierOptions()}>{x => <option value={x.key}>{x.title?.trim() || 'Side chat'}</option>}</For>
            </select>
          </Show>
          <IconButton icon="plus" size={14} label="New side chat" disabled={!s().parent || s().busy || s().loading || (s().turns.length === 0 && !s().pending)} onClick={() => void newSideChat()} />
          <IconButton icon="x" size={15} label="Close side chat" onClick={closeSideChat} />
        </span>
      </header>
      <Show when={s().error}>{why => <p class="side-chat-banner" role="status">{why()}</p>}</Show>
      <Show when={s().asked}>
        <p class="side-chat-banner" role="status">The side chat asked a question. Answer it from Waiting on you.</p>
      </Show>
      <div class="side-chat-scroll" ref={el => (scroller = el)} tabindex="-1" onScroll={() => { pinned = atBottom(); }}>
        <div class="side-chat-turns">
          <Show when={s().loading}><p class="side-chat-empty" role="status">Opening the side chat…</p></Show>
          <Show when={ready() && s().turns.length === 0 && !s().pending}>
            <p class="side-chat-empty">Ask a quick question about this chat. The main chat keeps working, and nothing here goes into it unless you add it.</p>
          </Show>
          <For each={s().turns}>{(t: AgentConversationTurn) => t.role === 'user'
            ? <div class="turn user side-chat-turn"><div class="side-chat-ask"><span class="sr-only">You</span>{t.text}</div></div>
            : (
              <div class="turn assistant side-chat-turn">
                <Show when={t.tools.length > 0}><p class="side-chat-note">{toolLine(t.tools.map(x => x.name))}</p></Show>
                <SideText text={t.text} />
                <Show when={t.error}>{e => <p class="side-chat-note">{e()}</p>}</Show>
                <Show when={t.text.trim()}>
                  <div class="side-chat-actions">
                    <Button variant="ghost" size="sm" icon="arrowUp" onClick={() => addToMain(t.text)} title="Put this reply in the main chat’s message box as a draft. Nothing is sent.">Add to main chat</Button>
                  </div>
                </Show>
              </div>
            )}
          </For>
          <Show when={s().pending}>{q => <div class="turn user side-chat-turn"><div class="side-chat-ask"><span class="sr-only">You</span>{q()}</div></div>}</Show>
          <Show when={s().live}>
            {live => (
              <div class="turn assistant partial side-chat-turn" aria-live="polite" aria-busy="true">
                <Show when={live().tools.length > 0}><p class="side-chat-note">{toolLine(live().tools)}</p></Show>
                <Show when={live().text} fallback={<p class="side-chat-note">Working…</p>}><SideText text={live().text} /></Show>
              </div>
            )}
          </Show>
          <For each={mine()}>{a => (
            <div class="side-chat-approval" role="group" aria-label={`Approval required: ${a.title}`}>
              <p><strong>Approval required</strong> ({a.title})<br />{a.action}</p>
              <div class="side-chat-actions">
                <Button size="sm" variant="primary" disabled={answering() === a.id || approvalsResponding().includes(a.id)} onClick={() => void answer(a.id, true)}>Allow</Button>
                <Button size="sm" disabled={answering() === a.id || approvalsResponding().includes(a.id)} onClick={() => void answer(a.id, false)}>Deny</Button>
              </div>
            </div>
          )}</For>
        </div>
      </div>
      <form class="side-chat-composer" aria-label="Ask the side chat" onSubmit={e => { e.preventDefault(); void send(); }}>
        <textarea
          ref={el => (box = el)}
          aria-label="Side chat message"
          placeholder={ready() ? 'Ask about this chat' : (s().error ?? 'Opening…')}
          rows={2}
          value={draft()}
          disabled={!ready()}
          onInput={e => setDraft(e.currentTarget.value)}
          onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); void send(); } }}
        />
        <div class="side-chat-foot">
          <label class="side-chat-access" title="Off: the side chat can read and answer, and can’t change files. On: it may use the main chat’s own access, never more.">
            <input type="checkbox" checked={sideChatSameAccess()} onChange={e => setSideChatSameAccess(e.currentTarget.checked)} />
            <span>Same access as the main chat</span>
          </label>
          <Show when={s().busy}>
            <Button variant="ghost" size="sm" icon="stop" onClick={() => void stopSideChat()} aria-label="Stop the side chat">Stop</Button>
          </Show>
          <button type="submit" class="btn primary side-chat-send" aria-label="Send to side chat" title="Send (Enter)" disabled={!ready() || sending() || s().busy || !draft().trim()}><Icon name="arrowUp" size={16} /></button>
        </div>
      </form>
    </aside>
  );
}
