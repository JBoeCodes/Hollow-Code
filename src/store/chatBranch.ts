// CH1 (2026-11-18): Branch from here on a reply (`branch_chat`, user-only),
// a branch's record for its notice (`chat_branch_info`) and Back to the
// original with a written handoff (`chat_branch_handoff`). Lazy: the reply's
// More menu and the branch notice load it. Backend: `chat_branch.rs`.
import { invoke } from '../api';
import { flashFooter } from './snackbars';
import { showConfirm } from './overlays';
import { errMsg } from '../utils';
import { addToDraft } from '../chatQueue';
import { chatAgentSessionKey, chatInput, setChatInput } from './chatRuntime';
import { pickConversation, refreshWorkspaceSessions, workspaceId } from './workspaces';
import { branchConfirm, branchedNote, type Branched, type BranchInfo } from './chatBranchWords';

/** Branch from here: asks, branches, opens the branch. */
export async function branchChat(key: string, runId: string, title: string, isolated: boolean | null): Promise<void> {
  if (!await showConfirm({ ...branchConfirm(title, isolated), confirmLabel: 'Branch' })) return;
  let made: Branched;
  try {
    made = await invoke<Branched>('branch_chat', { key, runId });
  } catch (e) {
    flashFooter(`× ${errMsg(e)}`);
    return;
  }
  const id = workspaceId();
  if (!id) return;
  await refreshWorkspaceSessions();
  if (!await pickConversation(id, made.key)) { flashFooter('× The branch didn’t open. It is in the sidebar.'); return; }
  flashFooter(branchedNote(made), 7000);
}

/** A branch's record; `null` for any other chat or a failed read. */
export async function loadBranchInfo(key: string): Promise<BranchInfo | null> {
  try { return await invoke<BranchInfo | null>('chat_branch_info', { key }); } catch { return null; }
}

/** Back to the original: opens it and, when the branch has turns of its
 *  own, adds their written handoff to the original's draft (never sent). */
let leaving: string | null = null;
export async function backToOriginal(info: BranchInfo): Promise<void> {
  // A second click while the first is opening adds nothing twice.
  if (leaving === info.key) return;
  leaving = info.key;
  try { await leave(info); } finally { leaving = null; }
}
async function leave(info: BranchInfo): Promise<void> {
  let text: string | null = null;
  if (info.ownTurns > 0) {
    try { text = await invoke<string>('chat_branch_handoff', { key: info.key }); } catch (e) { flashFooter(`× ${errMsg(e)}`); return; }
  }
  const id = workspaceId();
  if (!id) return;
  if (!await pickConversation(id, info.fromKey)) { flashFooter('× The original chat didn’t open.'); return; }
  // Only into the original's draft: another chat may have opened since.
  if (text && chatAgentSessionKey() === info.fromKey) setChatInput(addToDraft(chatInput(), text));
}
