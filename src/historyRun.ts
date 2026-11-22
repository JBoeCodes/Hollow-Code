/**
 * Human presentation of an automated-write run (audit H12): lead with the
 * workspace, time, outcome and changed files; keep raw ids, hashes and byte
 * sizes for the details. Pure, unit-tested; recovery behaviour is unchanged.
 */
import type { HistoryChange, HistoryCoverage, HistoryRun } from './types';

const SOURCE: Record<string, string> = { chat: 'Conversation', workflow: 'Automation', heartbeat: 'Heartbeat', notion: 'Notion sync', ingest: 'Import', teammate: 'Teammate', ask: 'Ask Hollow', workspaceArchive: 'Archive older entries' };

export function runOutcome(status: string): { label: string; tone: 'ok' | 'warn' | 'error' } {
  const s = status.toLowerCase();
  if (s === 'success') return { label: 'Completed', tone: 'ok' };
  if (s === 'running') return { label: 'Running', tone: 'warn' };
  if (s === 'cancelled' || s === 'canceled') return { label: 'Stopped', tone: 'warn' };
  // Cut off when Hollow quit, reconciled at the next start: not a failure.
  if (s === 'interrupted') return { label: 'Interrupted', tone: 'warn' };
  return { label: 'Failed', tone: 'error' };
}

/** The workspace a run wrote to: its scope, or the one workspace folder all
 *  its changes share. Null when it spans the vault. */
export function runWorkspaceId(run: Pick<HistoryRun, 'scope' | 'changes'>): string | null {
  const scoped = /^workspaces\/([^/]+)/.exec(run.scope ?? '')?.[1];
  if (scoped) return scoped;
  const ids = new Set(run.changes.map(c => /^workspaces\/([^/]+)\//.exec(c.vaultPath)?.[1] ?? '*'));
  return ids.size === 1 && !ids.has('*') ? [...ids][0] : null;
}

export function runTitle(run: Pick<HistoryRun, 'source' | 'sourceId' | 'scope' | 'changes'>, workspaceTitle: (id: string) => string | undefined): string {
  const what = run.source === 'workflow' && run.sourceId ? `Automation “${run.sourceId}”` : SOURCE[run.source] ?? (run.source.charAt(0).toUpperCase() + run.source.slice(1));
  const id = runWorkspaceId(run);
  const where = id ? workspaceTitle(id) ?? id : null;
  return where ? `${what} in ${where}` : what;
}

/** "README.md, plan.md +3 more" — file names only; paths are in the rows. */
export function runFilesLine(run: Pick<HistoryRun, 'changes'>): string {
  const names = run.changes.map(c => c.vaultPath.split('/').pop() || c.vaultPath);
  if (!names.length) return 'No file changes';
  return names.length <= 2 ? names.join(', ') : `${names.slice(0, 2).join(', ')} +${names.length - 2} more`;
}

type CoverageFields = Partial<Pick<HistoryChange,
  'beforeHash' | 'afterHash' | 'beforeUncaptured' | 'afterUncaptured' | 'beforeCoverage' | 'afterCoverage'>>;

/** What History knows about one side of a change (merged plan slice 4). A
 *  record without stored coverage (before slice 4) derives it like Rust's
 *  `side_coverage`: a hash is captured, the slice 1 flag is uncaptured,
 *  neither is absent, and a record that contradicts itself is unknown. */
export function sideCoverage(change: CoverageFields, side: 'before' | 'after'): HistoryCoverage {
  const stored = side === 'before' ? change.beforeCoverage : change.afterCoverage;
  const hashed = (side === 'before' ? change.beforeHash : change.afterHash) != null;
  const flagged = !!(side === 'before' ? change.beforeUncaptured : change.afterUncaptured);
  if (stored && (stored.state === 'uncaptured' || stored.state === 'unknown')) return stored;
  if (!flagged && hashed && (!stored || stored.state === 'captured')) return { state: 'captured' };
  if (!flagged && !hashed && (!stored || stored.state === 'absent')) return { state: 'absent' };
  if (!stored && !hashed && flagged) return { state: 'uncaptured', reason: null };
  return { state: 'unknown' };
}

function restorable(coverage: HistoryCoverage): boolean {
  return coverage.state === 'absent' || coverage.state === 'captured';
}

/** History can restore this change: it holds every version that existed. */
export function changeRecoverable(change: CoverageFields): boolean {
  return restorable(sideCoverage(change, 'before')) && restorable(sideCoverage(change, 'after'));
}

/** Decision 5: a run with any uncaptured or unknown file is never fully
 *  recoverable. */
export function runRecoverable(run: Pick<HistoryRun, 'changes'>): boolean {
  return run.changes.every(changeRecoverable);
}

/** A short label for a side History can't restore ("over 2 MiB"). */
export function coverageLabel(coverage: HistoryCoverage): string {
  if (coverage.state === 'unknown') return 'unknown';
  if (coverage.state !== 'uncaptured') return '';
  switch (coverage.reason) {
    case 'tooLarge': return 'over 2 MiB';
    case 'unreadable': return 'unreadable';
    case 'excluded': return 'link or special file';
    default: return 'not captured';
  }
}

function why(coverage: HistoryCoverage): string {
  if (coverage.state !== 'uncaptured') return '';
  switch (coverage.reason) {
    case 'tooLarge': return 'over 2 MiB';
    case 'unreadable': return 'it couldn’t be read';
    case 'excluded': return 'History doesn’t store links or special files';
    default: return 'over 2 MiB or unreadable';
  }
}

/** Why a change can't be previewed or restored: History holds no copy of one
 *  side, or can't tell whether it existed. Null when it can. */
export function uncapturedReason(change: CoverageFields): string | null {
  for (const side of ['before', 'after'] as const) {
    const coverage = sideCoverage(change, side);
    if (restorable(coverage)) continue;
    const consequence = side === 'before' ? 'so it can’t restore it' : 'so it won’t replace it';
    if (coverage.state === 'unknown') {
      return `History can’t tell whether this file existed ${side} the run (a folder above it couldn’t be read), ${consequence}.`;
    }
    return `History didn’t capture this file ${side} the run (${why(coverage)}), ${consequence}.`;
  }
  return null;
}

/** The restore preview (merged plan slice 4): every file in these changes a
 *  restore can't return, with why, in path order. Shown before any restore;
 *  a whole-run restore refuses while it is non-empty. */
export function restorePreview(changes: ReadonlyArray<CoverageFields & Pick<HistoryChange, 'vaultPath'>>): { path: string; reason: string }[] {
  return changes
    .flatMap(change => {
      const reason = uncapturedReason(change);
      return reason ? [{ path: change.vaultPath, reason }] : [];
    })
    .sort((a, b) => a.path.localeCompare(b.path));
}

/** Parallel chats in one workspace (2026-10-07): the files in this run that
 *  another conversation, running at the same time in the same workspace, also
 *  changed. Restore refuses them unless forced. */
export function sharedChanges<C extends Pick<HistoryChange, 'vaultPath' | 'alsoChangedBy'>>(run: { changes: readonly C[] }): C[] {
  return run.changes.filter(change => (change.alsoChangedBy ?? []).length > 0);
}

/** Rust `SHARED_RESTORE_CONFLICT`: a restore refused because another
 *  conversation also changed the file. */
export function isSharedRestoreConflict(message: string): boolean {
  return message.startsWith('history restore conflict (shared)');
}

/** Rust `unverified_refusal` (Q-R11): a restore refused because the run
 *  ended with Hollow's process, so its change list is unknown. */
export function isChangesUnknown(message: string): boolean {
  return message.includes('changes unknown');
}

/** "Ran at the same time as Conversation in Hollow" (one peer) or "… as 2
 *  other runs in this workspace". Empty when it ran alone. */
export function overlapLine(run: Pick<HistoryRun, 'overlaps'>, title: (runId: string) => string | undefined): string {
  const peers = run.overlaps ?? [];
  if (peers.length === 0) return '';
  if (peers.length === 1) return `Ran at the same time as ${title(peers[0]) ?? 'another run'} in this workspace.`;
  return `Ran at the same time as ${peers.length} other runs in this workspace.`;
}
