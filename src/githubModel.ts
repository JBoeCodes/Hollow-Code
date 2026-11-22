// The Pull requests view's words and rules (parity 7.3): the list's rows and
// filters, which triage moves a state offers, the confirm text every move
// shows before it runs, the quoted drafts "Ask about this" makes, and the
// Linear links. GithubPages.tsx renders them; githubModel.test.ts holds them.
// Local and provider-neutral: GitHub through the person's own `gh`, Linear
// stays the tracker (a link, never a second task list).
import type { PrComment, PrDetail } from './prPanelModel';
import { commentDraft, safeLink } from './prPanelModel';

export type GhKind = 'pr' | 'issue';
export type GhStateFilter = 'open' | 'closed' | 'merged' | 'all';
export type TriageAction = 'comment' | 'label' | 'unlabel' | 'assign' | 'unassign' | 'close' | 'reopen';

export interface GhLabel { name: string; color: string }
export interface LinearRef { key: string; title: string | null; url: string | null; tracked: boolean }
export interface GhRow {
  number: number; title: string; state: string; url: string; draft: boolean; author: string;
  labels: GhLabel[]; assignees: string[]; updatedAt: string; comments: number;
  reviewDecision: string | null; checks: 'success' | 'failure' | 'pending' | null; head: string; linear: LinearRef[];
}
export interface GhList { kind: GhKind; repo: string | null; rows: GhRow[]; more: boolean; note: string | null }
export interface GhDetail {
  kind: GhKind; repo: string | null; item: PrDetail; body: string; author: string;
  labels: GhLabel[]; assignees: string[]; createdAt: string; updatedAt: string; linear: LinearRef[];
}
export interface GhDetailView { detail: GhDetail | null; note: string | null }
export interface GhQuery {
  kind: GhKind; state?: GhStateFilter; label?: string; assignee?: string; author?: string; search?: string; reviewRequested?: boolean;
}
export interface Triage { kind: GhKind; number: number; action: TriageAction; value?: string }
export interface TriageDone { message: string; repo: string | null }

export const noun = (kind: GhKind) => (kind === 'pr' ? 'pull request' : 'issue');
export const nouns = (kind: GhKind) => (kind === 'pr' ? 'pull requests' : 'issues');

/** The state filter's choices: issues have no merged state. */
export function stateChoices(kind: GhKind): { value: GhStateFilter; label: string }[] {
  const all: { value: GhStateFilter; label: string }[] = [
    { value: 'open', label: 'Open' }, { value: 'closed', label: 'Closed' },
    ...(kind === 'pr' ? [{ value: 'merged' as const, label: 'Merged' }] : []),
    { value: 'all', label: 'All' },
  ];
  return all;
}

/** A filter the other kind can't take falls back to Open when the kind changes. */
export function stateFor(kind: GhKind, state: GhStateFilter): GhStateFilter {
  return stateChoices(kind).some(c => c.value === state) ? state : 'open';
}

/** What the filters say, for the query: empty text is left out. */
export function buildQuery(kind: GhKind, f: { state: GhStateFilter; label: string; assignee: string; author: string; search: string; reviewRequested: boolean }): GhQuery {
  const q: GhQuery = { kind, state: stateFor(kind, f.state) };
  if (f.label.trim()) q.label = f.label.trim();
  if (f.assignee.trim()) q.assignee = f.assignee.trim();
  if (f.author.trim()) q.author = f.author.trim();
  if (f.search.trim()) q.search = f.search.trim();
  if (kind === 'pr' && f.reviewRequested) q.reviewRequested = true;
  return q;
}

/** Whether any filter beyond Open narrows the list (the empty state's words and Clear). */
export function filtered(q: GhQuery): boolean {
  return Boolean((q.state && q.state !== 'open') || q.label || q.assignee || q.author || q.search || q.reviewRequested);
}

/** "No open pull requests." or "No pull requests match these filters." */
export function emptyWords(kind: GhKind, q: GhQuery): string {
  if (filtered(q)) return `No ${nouns(kind)} match these filters.`;
  return `No open ${nouns(kind)}.`;
}

/** "Open", "Draft", "Merged" or "Closed". */
export function rowStateWords(row: Pick<GhRow, 'state' | 'draft'>): string {
  if (row.state === 'open') return row.draft ? 'Draft' : 'Open';
  return row.state === 'merged' ? 'Merged' : row.state === 'closed' ? 'Closed' : row.state;
}

/** The checks chip's words for a row. */
export function rowChecksWords(checks: GhRow['checks']): string | null {
  return checks === 'success' ? 'Checks passed' : checks === 'failure' ? 'Checks failing' : checks === 'pending' ? 'Checks running' : null;
}

/** "ann, 3 comments" under a row's title. */
export function rowByline(row: Pick<GhRow, 'author' | 'comments' | 'assignees'>): string {
  const parts = [row.author];
  if (row.assignees.length) parts.push(`assigned to ${row.assignees.join(', ')}`);
  if (row.comments) parts.push(`${row.comments} ${row.comments === 1 ? 'comment' : 'comments'}`);
  return parts.join(' · ');
}

/** A label's colour as a CSS value, or null when GitHub sent none that is plain hex. */
export function labelColor(color: string): string | null {
  return /^[0-9a-fA-F]{6}$/.test(color) ? `#${color.toLowerCase()}` : null;
}

/** Which triage moves a state offers: close while open, reopen while closed
 *  (a merged pull request is state `merged`, so it offers neither); comment,
 *  label and assign always. */
export function movesFor(state: string): { close: boolean; reopen: boolean } {
  return { close: state === 'open', reopen: state === 'closed' };
}

/** What the confirm dialog shows before a move runs. It always names the
 *  repository, the number and the change; a comment shows its text. Nothing
 *  runs until the person confirms it. */
export function confirmCopy(repo: string | null, kind: GhKind, t: Triage, title: string): { title: string; body: string; confirmLabel: string; danger: boolean } {
  const where = repo ? `${repo} on GitHub` : 'GitHub';
  const what = `${noun(kind)} #${t.number} (${title})`;
  const v = (t.value ?? '').trim();
  switch (t.action) {
    case 'comment': return { title: 'Post this comment?', body: `Posts to ${what} in ${where}, as you. Comments are public to anyone who can see the repository.\n\n${v}`, confirmLabel: 'Post comment', danger: false };
    case 'label': return { title: 'Add a label', body: `Adds a label to ${what} in ${where}.`, confirmLabel: 'Add label', danger: false };
    case 'unlabel': return { title: 'Remove this label?', body: `Removes the label “${v}” from ${what} in ${where}.`, confirmLabel: 'Remove label', danger: false };
    case 'assign': return { title: 'Assign someone', body: `Assigns ${what} in ${where}.`, confirmLabel: 'Assign', danger: false };
    case 'unassign': return { title: 'Unassign?', body: `Removes ${v} from ${what} in ${where}.`, confirmLabel: 'Unassign', danger: false };
    case 'close': return { title: `Close this ${noun(kind)}?`, body: `Closes ${what} in ${where}. You can reopen it later${kind === 'pr' ? ' unless it was merged' : ''}.`, confirmLabel: 'Close', danger: true };
    case 'reopen': return { title: `Reopen this ${noun(kind)}?`, body: `Reopens ${what} in ${where}.`, confirmLabel: 'Reopen', danger: false };
  }
}

/** The composer draft for "Ask about this" on the whole item. Nothing is sent. */
export function itemDraft(d: Pick<GhDetail, 'kind' | 'repo' | 'item' | 'body' | 'labels' | 'author'>): string {
  const head = `About ${noun(d.kind)} #${d.item.number} in ${d.repo ?? 'this repository'}: ${d.item.title}`;
  const facts = [`State: ${d.item.state}`, `By ${d.author}`, d.labels.length ? `Labels: ${d.labels.map(l => l.name).join(', ')}` : ''].filter(Boolean).join('. ');
  const body = d.body.trim() ? `\n\n${d.body.trim().split('\n').map(l => `> ${l}`).join('\n')}` : '';
  return `${head}\n${facts}.${body}\n\n`;
}

/** The draft for one comment: the PR tab's own wording. */
export function discussionDraft(d: Pick<GhDetail, 'item'>, c: Pick<PrComment, 'author' | 'body' | 'kind' | 'state'>): string {
  return commentDraft(d.item, c);
}

/** A Linear issue's words: "ENG-12 Fix the lot map" or "ENG-12" alone. */
export function linearWords(ref: Pick<LinearRef, 'key' | 'title'>): string {
  return ref.title ? `${ref.key} ${ref.title}` : ref.key;
}

/** The note under a Linear key Hollow's tracker mirror doesn't have. */
export const LINEAR_UNTRACKED = 'Not in Hollow’s tracker mirror, so there is no link to open.';

/** The tracker address a Linear link opens (https only), else null. */
export function linearLink(ref: Pick<LinearRef, 'url'>): string | null {
  return safeLink(ref.url);
}

/** A project the page can list: a real folder, not Quick chats or the COO. */
export interface RepoChoice { id: string; title: string; cwd: string }

export function repoChoices(workspaces: readonly { id: string; title: string; cwd: string; kind?: string }[]): RepoChoice[] {
  return workspaces
    .filter(w => w.cwd && w.kind !== 'quick' && w.kind !== 'main')
    .map(w => ({ id: w.id, title: w.title, cwd: w.cwd }));
}

/** The project to show first: the open one, else the last chosen, else the first. */
export function firstChoice(choices: readonly RepoChoice[], opts: { open: string | null; saved: string | null }): RepoChoice | null {
  return choices.find(c => c.id === opts.open) ?? choices.find(c => c.id === opts.saved) ?? choices[0] ?? null;
}
