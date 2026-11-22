// The PR tab's words (parity 5.4): state chips, the checks summary, review
// and merge wording, and the draft a comment turns into. PrPanel.tsx renders
// them; prPanelModel.test.ts holds them. Local and provider-neutral.

export interface PrCheck { name: string; state: string; url: string | null }
export interface PrComment { author: string; kind: string; state: string | null; body: string; createdAt: string; url: string | null }
export interface PrDetail {
  number: number; title: string; state: string; url: string; draft: boolean;
  reviewDecision: string | null; mergeState: string | null; head: string; base: string;
  checks: PrCheck[]; moreChecks: number; comments: PrComment[]; moreComments: number;
}
export interface PrView { detail: PrDetail | null; note: string | null }

/** "Open", "Draft", "Merged" or "Closed". */
export function prStateWords(pr: Pick<PrDetail, 'state' | 'draft'>): string {
  if (pr.state === 'open') return pr.draft ? 'Draft' : 'Open';
  return pr.state === 'merged' ? 'Merged' : pr.state === 'closed' ? 'Closed' : pr.state;
}

/** "3 of 5 checks passed, 1 failing, 1 running" and the worst state, for the chip. */
export function checksSummary(checks: readonly PrCheck[]): { words: string; worst: 'success' | 'failure' | 'pending' | 'none' } {
  if (!checks.length) return { words: 'No checks', worst: 'none' };
  const n = (state: string) => checks.filter(c => c.state === state).length;
  const failing = n('failure'), running = n('pending'), ok = n('success'), skipped = n('skipped');
  const parts = [`${ok} of ${checks.length} passed`];
  if (failing) parts.push(`${failing} failing`);
  if (running) parts.push(`${running} running`);
  if (skipped) parts.push(`${skipped} skipped`);
  return { words: parts.join(', '), worst: failing ? 'failure' : running ? 'pending' : 'success' };
}

/** The review decision in words; null when there is none to show. */
export function reviewWords(decision: string | null): string | null {
  switch (decision) {
    case 'APPROVED': return 'Approved';
    case 'CHANGES_REQUESTED': return 'Changes requested';
    case 'REVIEW_REQUIRED': return 'Review required';
    default: return null;
  }
}

/** GitHub's merge state in words; null for the unremarkable ones. */
export function mergeWords(state: string | null): string | null {
  switch (state) {
    case 'CLEAN': return 'Ready to merge';
    case 'BLOCKED': return 'Blocked by required checks or reviews';
    case 'BEHIND': return 'Behind the base branch';
    case 'DIRTY': return 'Has merge conflicts';
    case 'UNSTABLE': return 'Some checks are failing';
    case 'DRAFT': return 'Draft';
    default: return null;
  }
}

/** A review row's state word: "Approved", "Requested changes", "Reviewed" (a comment), "Dismissed". */
export function reviewStateWords(state: string | null): string {
  switch (state) {
    case 'APPROVED': return 'Approved';
    case 'CHANGES_REQUESTED': return 'Requested changes';
    case 'DISMISSED': return 'Dismissed';
    default: return 'Reviewed';
  }
}

/** The composer draft for "Ask about this" on a comment: the quote and who wrote it. Nothing is sent. */
export function commentDraft(pr: Pick<PrDetail, 'number' | 'title'>, c: Pick<PrComment, 'author' | 'body' | 'kind' | 'state'>): string {
  const who = c.kind === 'review' ? `${c.author}'s review (${reviewStateWords(c.state).toLowerCase()})` : `${c.author}'s comment`;
  const quote = c.body.trim().split('\n').map(l => `> ${l}`).join('\n');
  return `About ${who} on PR #${pr.number} (${pr.title}):\n\n${quote}\n\n`;
}

/** Only an https GitHub-style address opens from the tab. */
export function safeLink(url: string | null | undefined): string | null {
  return url && /^https:\/\//i.test(url) ? url : null;
}
