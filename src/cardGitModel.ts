// The chat card's git rows, as pure functions (parity 5.2; `git_card.rs`
// holds the facts, SidePanelCardGit.tsx renders them, cardGitModel.test.ts
// holds the words). Provider-neutral: nothing here names an agent.

export interface CardGitFile { path: string; status: string; additions: number | null; deletions: number | null }

export interface CardGitStatus {
  repo: boolean;
  root: string;
  branch: string;
  head: string;
  upstream: string | null;
  ahead: number;
  behind: number;
  linkedWorktree: boolean;
  files: CardGitFile[];
  moreFiles: number;
  additions: number;
  deletions: number;
  blocked: string | null;
}

export interface CardGitBranches { current: string; branches: string[] }

/** The branch chip's words: the branch (or "Detached HEAD"), then what is
 *  unpushed (↑) and what the upstream has that this lacks (↓). */
export function branchWords(s: Pick<CardGitStatus, 'branch' | 'head' | 'ahead' | 'behind'>): string {
  const { name, marks } = branchParts(s);
  return marks ? `${name} ${marks}` : name;
}

/** The chip in two pieces, so a long branch name can shrink with an ellipsis
 *  while the push and behind marks stay whole. */
export function branchParts(s: Pick<CardGitStatus, 'branch' | 'head' | 'ahead' | 'behind'>): { name: string; marks: string } {
  const name = s.branch || (s.head ? `Detached at ${s.head}` : 'No commits yet');
  return { name, marks: [s.ahead > 0 ? `↑${s.ahead}` : '', s.behind > 0 ? `↓${s.behind}` : ''].filter(Boolean).join(' ') };
}

/** The chip's accessible name, with the same facts in words. */
export function branchLabel(s: Pick<CardGitStatus, 'branch' | 'head' | 'ahead' | 'behind' | 'linkedWorktree'>): string {
  const parts = [`Branch ${s.branch || 'none'}`];
  if (s.linkedWorktree) parts.push('in a worktree');
  if (s.ahead > 0) parts.push(`${s.ahead} commit${s.ahead === 1 ? '' : 's'} to push`);
  if (s.behind > 0) parts.push(`${s.behind} behind its upstream`);
  return parts.join(', ');
}

/** "3 files", "1 file", "None": the Changes row's count. */
export function changesCount(s: Pick<CardGitStatus, 'files' | 'moreFiles'>): string {
  const n = s.files.length + s.moreFiles;
  return n === 0 ? 'None' : `${n} file${n === 1 ? '' : 's'}`;
}

export type GitPrimary = { label: string; kind: 'commit-push' | 'push' | 'none'; why: string | null };

/** What the card's primary button does now: Commit and push with changes,
 *  Push with only unpushed commits, nothing otherwise (with the reason). */
export function primaryAction(s: Pick<CardGitStatus, 'files' | 'moreFiles' | 'ahead' | 'branch' | 'blocked'>, chatBusy: boolean): GitPrimary {
  const dirty = s.files.length + s.moreFiles > 0;
  if (!s.branch && (dirty || s.ahead > 0)) return { label: 'Commit and push', kind: 'none', why: 'Switch to a branch first: this folder is on a detached HEAD.' };
  const wait = s.blocked ?? (chatBusy ? 'Wait for this chat’s turn to finish: git would race it.' : null);
  if (dirty) return { label: 'Commit and push', kind: wait ? 'none' : 'commit-push', why: wait };
  if (s.ahead > 0) return { label: 'Push', kind: wait ? 'none' : 'push', why: wait };
  return { label: 'Commit and push', kind: 'none', why: 'Nothing to commit or push.' };
}

/** The generated message split for the one-line prompt: the subject goes in
 *  the field, the file list rides along unless the person rewrites the subject. */
export function splitMessage(message: string): { subject: string; body: string } {
  const i = message.indexOf('\n');
  if (i < 0) return { subject: message.trim(), body: '' };
  return { subject: message.slice(0, i).trim(), body: message.slice(i).trim() };
}

/** The commit message to send: the edited subject, with the generated file
 *  list added only when the subject is still the generated one. */
export function finalMessage(edited: string, generated: { subject: string; body: string }): string {
  const subject = edited.trim();
  return subject === generated.subject && generated.body ? `${subject}\n\n${generated.body}` : subject;
}
