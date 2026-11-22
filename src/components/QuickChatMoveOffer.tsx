// Quick chats (2026-11-14, Alex's card 2): under a quick chat's latest
// reply, when the exchange is clearly one project's work
// (`quickChatMoveTarget`), a one-click "Move to <project>". The chat moves
// with its history, lists under that project lead's Subagents, and the lead
// gets a note (Rust `chat_chain::record_arrival`). "Not now" hides it for
// this reply. The same move stays in the chat's menu (Move to project…).
// Lazy, so none of it weighs on the App chunk.
import { Show, createMemo, createSignal } from 'solid-js';
import { moveSessionTo, projectWorkspaces } from '../store/workspaces';
import { quickChatMoveTarget } from '../quickChatMove';

/** Replies whose offer was put aside this session (by session key and reply). */
const [dismissed, setDismissed] = createSignal<ReadonlySet<string>>(new Set());

export default function QuickChatMoveOffer(props: { sessionKey: string; replyId: string; prompt: string; reply: string }) {
  const id = () => `${props.sessionKey}:${props.replyId}`;
  const target = createMemo(() => quickChatMoveTarget(props.prompt, props.reply, projectWorkspaces()));
  const [moving, setMoving] = createSignal(false);
  const move = async () => {
    const p = target();
    if (!p || moving()) return;
    setMoving(true);
    try { await moveSessionTo(props.sessionKey, p.id, { open: true }); } finally { setMoving(false); }
  };
  return <Show when={!dismissed().has(id()) ? target() : null}>{p =>
    <div class="chat-move-offer" role="group" aria-label="This looks like project work">
      <span class="chat-move-offer-text">This looks like {p().title} work.</span>
      <button type="button" class="btn sm" disabled={moving()} onClick={() => void move()}>Move to {p().title}</button>
      <button type="button" class="btn ghost sm" onClick={() => setDismissed(prev => new Set([...prev, id()]))}>Not now</button>
    </div>
  }</Show>;
}
