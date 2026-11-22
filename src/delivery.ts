/**
 * A build workspace's delivery record (next-level P20). Pure, unit-tested.
 *
 * Each step needs its own evidence and none implies the next: a commit isn't
 * published, a merged PR isn't built, a build isn't installed, and a build
 * waiting in the Update button's "Update when chats finish" (row U10) is
 * "pending" until the restarted app reports a receipt whose commit matches. Steps after the first missing one
 * are not "done" by inference.
 */
import type { AppBuildStatus, BuildStatus, SelfInstallReceipt } from './workspaceTypes';
import type { DeliveryStep } from './workState';

export type StepState = 'done' | 'current' | 'pending' | 'failed' | 'unknown';
export interface DeliveryRow { step: DeliveryStep; state: StepState; evidence: string }

const short = (c: string | null | undefined) => (c ?? '').slice(0, 7);
const sameCommit = (a: string | null | undefined, b: string | null | undefined) => !!a && !!b && (a.startsWith(b) || b.startsWith(a));

/** `receiptError`: the install receipt exists but couldn't be read (P1-46),
 *  which is "unknown", never "no install recorded". `waitingInUpdate`: this
 *  build is the Local build in the Update button and its wait is armed. */
export function deliveryRows(input: { status: BuildStatus | null; build: AppBuildStatus | null; receipt: SelfInstallReceipt | null; receiptError?: string | null; waitingInUpdate?: boolean }): DeliveryRow[] {
  const s = input.status, b = input.build, r = input.receipt;
  if (!s) return [];
  const rows: DeliveryRow[] = [];
  const dirty = s.files.filter(f => f.uncommitted).length;
  const merged = !!s.pr && /merged/i.test(s.pr.state);
  rows.push(dirty ? { step: 'local', state: 'current', evidence: `${dirty} uncommitted change${dirty === 1 ? '' : 's'} in the worktree` } : { step: 'local', state: 'done', evidence: 'Worktree clean' });
  // A merged branch is committed even with nothing left ahead of the base.
  rows.push(s.ahead > 0 ? { step: 'committed', state: 'done', evidence: `${s.ahead} commit${s.ahead === 1 ? '' : 's'} on ${s.branch}, ahead of ${s.base}` }
    : merged ? { step: 'committed', state: 'done', evidence: `Merged into ${s.base} with PR #${s.pr!.number}` }
    : { step: 'committed', state: 'pending', evidence: 'No commits ahead of the base' });
  const pr = s.pr;
  if (pr && (pr.state === 'OPEN' || pr.state === 'open')) rows.push({ step: 'published', state: 'done', evidence: `PR #${pr.number} open${s.unpushed ? ` · ${s.unpushed} commit${s.unpushed === 1 ? '' : 's'} not pushed yet` : ''}` });
  else if (pr && /merged/i.test(pr.state)) rows.push({ step: 'published', state: 'done', evidence: `PR #${pr.number}` });
  else if (pr && /closed/i.test(pr.state)) rows.push({ step: 'published', state: 'failed', evidence: `PR #${pr.number} closed without merging` });
  else rows.push({ step: 'published', state: 'pending', evidence: s.upstream && !s.unpushed && s.ahead ? 'Pushed; no pull request' : 'Not pushed' });
  rows.push(pr && /merged/i.test(pr.state) ? { step: 'merged', state: 'done', evidence: `PR #${pr.number} merged` } : { step: 'merged', state: 'pending', evidence: 'Not merged' });
  if (b?.isHollow) {
    if (b.state === 'building') rows.push({ step: 'built', state: 'current', evidence: 'Building now' });
    else if (b.state === 'failed') rows.push({ step: 'built', state: 'failed', evidence: b.error ?? 'The last build failed' });
    else if (b.state === 'built' && b.commit) rows.push({ step: 'built', state: sameCommit(b.commit, s.head) ? 'done' : 'pending', evidence: sameCommit(b.commit, s.head) ? `Built from ${short(b.commit)}` : `Last build is from ${short(b.commit)}, not ${short(s.head)}` });
    else rows.push({ step: 'built', state: 'pending', evidence: 'No app built from this branch' });
    if (b.state === 'built' && input.waitingInUpdate) rows.push({ step: 'scheduled', state: 'current', evidence: 'Waiting in the Update button until chats finish; pending until the restarted app confirms the install' });
    const receiptCommit = r?.result.commit;
    const unread = "The install record couldn't be read";
    if (input.receiptError) rows.push({ step: 'installed', state: 'unknown', evidence: input.receiptError.startsWith(unread) ? input.receiptError : `${unread}: ${input.receiptError}` });
    else if (r && r.result.ok && sameCommit(receiptCommit, s.head) && r.matchesRunning) rows.push({ step: 'installed', state: 'done', evidence: `Install ${r.result.id}: ${short(receiptCommit)} installed, and the running app reports it` });
    // Only a branch with work of its own: a fresh branch's HEAD is the base tip, which any later install contains.
    else if (r && r.result.ok && s.installedContainsHead && (s.ahead > 0 || merged) && r.matchesRunning) rows.push({ step: 'installed', state: 'done', evidence: `Install ${r.result.id}: ${short(receiptCommit)} installed, contains this branch, and the running app reports it` });
    else if (r && !r.result.ok && sameCommit(receiptCommit, s.head)) rows.push({ step: 'installed', state: 'failed', evidence: r.state === 'failed' ? `The install of ${short(receiptCommit)} failed; the previous app was kept` : `The install of ${short(receiptCommit)} failed; which app is installed now is unknown` });
    else if (r && r.result.ok && sameCommit(receiptCommit, s.head) && !r.matchesRunning) rows.push({ step: 'installed', state: 'pending', evidence: 'The installer reported success, but the running app is a different commit' });
    else rows.push({ step: 'installed', state: 'pending', evidence: r ? `The installed app is ${short(r.runningCommit)}, not this branch` : 'No install recorded for this branch' });
  }
  return rows;
}
