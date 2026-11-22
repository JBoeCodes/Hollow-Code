// CH1 (2026-11-18): the notice on a branched chat: where it branched from,
// and Back to the original (with a written handoff of the branch's own
// turns in the original's draft, never sent). Lazy, like ChatMoveNotice,
// whose look it shares. Backend: `chat_branch.rs`.
import { Show, createResource } from 'solid-js';
import { backToOriginal, loadBranchInfo } from '../store/chatBranch';
import { branchNotice } from '../store/chatBranchWords';

export default function ChatBranchNotice(props: { chatKey: string; turnsNow: number }) {
  // Read again when the chat or its turn count changes (own turns).
  const [info] = createResource(() => [props.chatKey, props.turnsNow] as const, ([key]) => loadBranchInfo(key));
  return (
    <Show when={info()}>{i => (
      <div class="chat-move-notice" data-kind="branch" role="status" aria-label="Where this chat branched from">
        <p class="chat-move-notice-text">{branchNotice(i())}</p>
        <button type="button" class="btn ghost sm" title={i().ownTurns > 0 ? 'Open the original with a written handoff of this branch in its draft' : 'Open the original chat'} onClick={() => void backToOriginal(i())}>Back to the original</button>
      </div>
    )}</Show>
  );
}
