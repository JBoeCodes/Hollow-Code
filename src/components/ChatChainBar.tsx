import { For, Show, createEffect, createRoot, on } from 'solid-js';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { chatAgentSessionKey, chatAttachments, chatBusy, chatInput, flashFooter, setChatAttachments, setChatInput, stopChatRun } from '../store/runtime';
import { conversationDrafts } from '../store/conversationDrafts';
import { pickConversation, workspaceSessions } from '../store/workspaces';
import { refreshChatInteractions } from '../store/chatRuntime';
import { chatChain, continuedIn, setChatChain, type ChainFacts } from '../store/chatChainState';
import { earlierLabel, forwardWords } from '../chatChainWords';
import type { SendAccepted } from '../types';
import '../styles/chat-chain.css';

// One conversation from the person's side (2026-11-07, Alex: "Why are
// there two Main agent chats?"). Lazy, so none of it weighs on the App
// chunk; ChatBody always mounts it, which starts the loader below.
// - A chat that continued elsewhere (a handover; any Main agent conversation
//   but the newest) shows "This conversation continued in …" with Open, in
//   place of its composer (ChatBody hides the composer).
// - The current chat folds the chats before it under "Earlier in this
//   conversation": the Main page's earlier threads, read-only from here.
// - Each shows the messages that moved between them (Rust `chat_chain`).

const reads = latestRequest();

/** Read the shown conversation's chain again. */
export async function refreshChatChain(): Promise<void> {
  const key = chatAgentSessionKey();
  const generation = reads.start();
  if (!key) { setChatChain(null); return; }
  try {
    const view = await api.conversationChain(key);
    if (!reads.isLatest(generation) || chatAgentSessionKey() !== key) return;
    const before = chatChain();
    setChatChain(view);
    carryDraft(view);
    // The earlier chats' cards follow the chain once it is known.
    const keys = (v: { current: string; earlier: readonly { key: string }[] }) => `${v.current} ${v.earlier.map(e => e.key).join(' ')}`;
    if (!before || before.key !== view.key || keys(before) !== keys(view)) void refreshChatInteractions();
  } catch {
    if (reads.isLatest(generation) && chatChain()?.key !== key) setChatChain(null);
  }
}

createRoot(() => {
  createEffect(on([chatAgentSessionKey, workspaceSessions], () => { void refreshChatChain(); }));
});

/** A handover hides this chat's composer (C-D2): what the person typed
 *  there moves to the chat it continued in instead of staying stranded. */
function carryDraft(view: ChainFacts) {
  const to = continuedIn(view);
  const text = chatInput(), atts = chatAttachments();
  if (!to || (!text.trim() && !atts.length && !conversationDrafts.stashes('', view.key).length)) return;
  if (!conversationDrafts.carry(view.key, to.key, text, atts)) {
    flashFooter(`Your unsent message is too long to add to “${to.title}”, so it stays in this chat.`, 6000);
    return;
  }
  setChatInput(''); setChatAttachments([]);
  flashFooter(`Your unsent message moved to “${to.title}”, where this conversation continues.`, 5000);
}

/** Open a conversation of the chain where it lives. */
export async function openChainConversation(key: string): Promise<boolean> {
  const target = await api.sessionTarget(key).catch(() => null);
  if (!target?.workspaceId) { flashFooter('That conversation is not in a workspace any more.', 4000); return false; }
  return pickConversation(target.workspaceId, key);
}

/** A send that went to the chain's current conversation (the backend moved
 *  it, `SendAccepted.forwardedFrom`): say so and open that conversation. */
export function followForward(accepted: SendAccepted) {
  const title = accepted.forwardedTitle?.trim();
  flashFooter(`Sent to ${title ? `“${title}”` : 'the current chat'}, where this conversation continues.`, 5000);
  void openChainConversation(accepted.sessionKey);
}

const time = (iso: string) => {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? '' : d.toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
};

export default function ChatChainBar(props: { part: 'banner' | 'earlier' }) {
  const view = () => { const v = chatChain(); return v && v.key === chatAgentSessionKey() ? v : null; };
  const out = () => (view()?.forwards ?? []).filter(f => f.from === view()!.key);
  const came = () => (view()?.forwards ?? []).filter(f => f.to === view()!.key);
  return (
    <Show when={props.part === 'banner'} fallback={
      <Show when={view() && !continuedIn(view()) && (view()!.earlier.length > 0 || came().length > 0) ? view() : null}>{v => (
        <details class="chat-earlier">
          <summary>{earlierLabel(v().earlier.length)}</summary>
          <ul class="chat-earlier-list">
            <For each={v().earlier}>{e => (
              <li>
                <button type="button" class="btn ghost sm" title={e.title ?? e.key} onClick={() => void openChainConversation(e.key)}>{e.title ?? e.key}</button>
                <span class="chat-earlier-when">{time(e.createdAt)}</span>
              </li>
            )}</For>
          </ul>
          <Show when={came().length > 0}>
            <ul class="chat-earlier-notes"><For each={came()}>{f => <li>{forwardWords(f, v().key, time)}</li>}</For></ul>
          </Show>
        </details>
      )}</Show>
    }>
      <Show when={continuedIn(view())}>{c => (
        <div class="chat-continued" role="status">
          <p class="chat-continued-text">This conversation continued in <strong>{c().title}</strong>.</p>
          <button type="button" class="btn sm" onClick={() => void openChainConversation(c().key)}>Open</button>
          {/* Its last turn may still be ending (a handover's last step): its
              Stop stays visible here, where the composer was. */}
          <Show when={chatBusy()}><button type="button" class="btn ghost sm" onClick={() => stopChatRun()}>Stop</button></Show>
          <Show when={out().length > 0}>
            <ul class="chat-continued-notes"><For each={out()}>{f => <li>{forwardWords(f, view()!.key, time)}</li>}</For></ul>
          </Show>
        </div>
      )}</Show>
    </Show>
  );
}
