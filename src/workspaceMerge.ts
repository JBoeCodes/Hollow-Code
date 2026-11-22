// Merge into… (master plan #18): which workspaces can receive a merge, and
// the confirm dialog's text listing every file and conversation that moves.
// The plan itself comes from Rust (`preview_workspace_merge`); its hash binds
// the confirm, so what this text shows is exactly what moves.
import type { MergeOutcome, MergePlan, UnmergeOutcome } from './generated/ipc';
import { workspaceChoices, type WorkspaceChoice } from './sessionMove';
import type { WorkWorkspace } from './workspaceTypes';

/** Where workspace `from` can be merged: every other workspace that isn't
 *  built in or archived, rail order first. */
export function mergeTargets(all: readonly WorkWorkspace[], railOrder: readonly WorkWorkspace[], from: string): WorkspaceChoice[] {
  const archived = new Set(all.filter(w => w.status.trim().toLowerCase() === 'archived').map(w => w.id));
  return workspaceChoices(all, railOrder, from).filter(c => !c.quick && !archived.has(c.id));
}

/** Whether workspace `w` can be merged into another (the menu's entry). */
export function canMergeFrom(w: WorkWorkspace): boolean {
  return w.status.trim().toLowerCase() !== 'archived' && !w.mergedInto;
}

const rel = (path: string, id: string) => path.startsWith(`workspaces/${id}/`) ? path.slice(`workspaces/${id}/`.length) : path;

const count = (n: number, one: string) => `${n || 'no'} ${one}${n === 1 ? '' : 's'}`;
/** The confirm's one-line description (P1-38): what a screen reader reads
 *  with the title, before the long list. */
export function mergeConfirmSummary(plan: MergePlan): string {
  return `Moves ${count(plan.files.length, 'file')} and ${count(plan.sessions.length, 'conversation')} into “${plan.intoTitle}”, then archives “${plan.fromTitle}”.`;
}

/** The confirm dialog's body: what happens, then every file and
 *  conversation that moves, one per line. */
export function mergeConfirmBody(plan: MergePlan): string {
  const lines = [
    `Everything below moves into “${plan.intoTitle}”, and its brief, plan and sources list are added there under a dated heading. “${plan.fromTitle}” is then archived. Nothing is deleted, and Undo merge (its menu) puts everything back.`,
  ];
  if (plan.sessions.length) lines.push(`The conversations’ next turns run in the folder of “${plan.intoTitle}”, with its brief and sources; each conversation so far comes along as written context.`);
  if (plan.keepsWorktree) lines.push('Its git worktree stays; remove it separately if you no longer need it.');
  lines.push('', plan.files.length ? `Files (${plan.files.length}):` : 'No files in sources/ or outputs/.');
  for (const f of plan.files) {
    const from = rel(f.from, plan.from);
    lines.push(f.renamed ? `• ${from} → ${rel(f.to, plan.into)} (name taken)` : `• ${from}`);
  }
  lines.push('', plan.sessions.length ? `Conversations (${plan.sessions.length}):` : 'No conversations.');
  for (const s of plan.sessions) lines.push(`• ${s.title || s.key}${s.archived ? ' (archived)' : ''}`);
  if (plan.left.length) lines.push('', `Left in “${plan.fromTitle}” (symbolic links never move):`, ...plan.left.map(p => `• ${p}`));
  return lines.join('\n');
}

/** The footer line after a merge. */
export function mergeDoneMessage(out: MergeOutcome): string {
  const { plan } = out;
  const parts = [`${plan.files.length} file${plan.files.length === 1 ? '' : 's'}`, `${plan.sessions.length} conversation${plan.sessions.length === 1 ? '' : 's'}`];
  return `Merged ${plan.fromTitle} into ${plan.intoTitle} (${parts.join(', ')})`;
}

/** The footer line after an undo. */
export function unmergeDoneMessage(out: UnmergeOutcome, fromTitle: string): string {
  return `Undid the merge of ${fromTitle} (${out.files} file${out.files === 1 ? '' : 's'}, ${out.sessions} conversation${out.sessions === 1 ? '' : 's'} back)`;
}
