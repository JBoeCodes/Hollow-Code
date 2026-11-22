// CH1 (2026-11-18): the words of Branch from here and the branch notice
// (`chatBranch.ts`, `ChatBranchNotice.tsx`). Pure, for tests.

/** `chat_branch::Branched`. */
export interface Branched { key: string; title: string | null; through: number; turns: number; checkpoint: string | null; refName: string | null; changedSince: string[] }
/** `chat_branch::BranchInfo`. */
export interface BranchInfo { key: string; fromKey: string; fromTitle: string | null; through: number; checkpoint: string | null; ownTurns: number }

/** The confirm's words. `isolated`: the chat owns its worktree (CP1), which
 *  two chats in one folder no longer do; `null` while that isn't known. */
export function branchConfirm(title: string, isolated: boolean | null): { title: string; body: string } {
  const files = isolated === true
    ? ' It works in the same worktree, so neither chat records turn checkpoints from then on; the files as they were at this turn are kept in a git ref.'
    : isolated === false
      ? ' It works in the same folder, with the files as they are now.'
      : ' It works in the same folder, with the files as they are now. If this chat owns its worktree, neither chat records turn checkpoints from then on.';
  return {
    title: 'Branch from this reply?',
    body: `A new chat starts with “${title}” up to this reply, so you can try another way from here. The original stays as it is. Its next turn starts a fresh session with these turns as its history.${files}`,
  };
}

/** The note after a branch: where the files stand. */
export function branchedNote(b: Branched): string {
  const n = b.changedSince.length;
  if (!b.checkpoint) return `Branched after turn ${b.through}.`;
  if (!n) return `Branched after turn ${b.through}. The files are as they were then.`;
  const files = n === 1 ? '1 file has' : `${n} files have`;
  return `Branched after turn ${b.through}. ${files} changed since; the files as they were are in ${b.refName ?? 'the branch checkpoint'}.`;
}

/** The branch notice's sentence. */
export function branchNotice(info: BranchInfo): string {
  const from = info.fromTitle?.trim() ? `“${info.fromTitle.trim()}”` : 'another chat';
  const own = info.ownTurns === 0 ? 'No turns of its own yet.' : info.ownTurns === 1 ? '1 turn of its own.' : `${info.ownTurns} turns of its own.`;
  return `A branch of ${from} after turn ${info.through}. ${own}`;
}
