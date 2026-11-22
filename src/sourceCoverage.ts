/**
 * Connected-source coverage summary (audit H03, next-level P07). The scan
 * reports structured coverage (directory_connections.rs `Coverage`) alongside
 * its warning strings. This turns both into one headline plus the actionable
 * details, without ever presenting incomplete or unknown coverage as complete.
 * Since P07 the inventory is complete up to a per-project share and a safety
 * bound; files past either are counted (`omitted`), and listed files Hollow
 * can't read are counted as too large or needing a reader, never as empty.
 */
import type { DirectoryConnection, SourceCoverage } from './api';
import type { ConnectedIndexStatus } from './types';

export type CoverageSummary = {
  state: 'complete' | 'partial' | 'unknown';
  headline: string;
  /** Capped projects, most omitted first: where leaving folders out helps. */
  capped: SourceCoverage['projects'];
  /** Warnings that are not about limits (read errors, a stale inventory). */
  other: string[];
  /** Listed files whose contents Hollow doesn't read (needsReader: images). */
  unread: { tooLarge: number; needsReader: number };
  /** Listed PDF and Office files whose text Hollow reads itself (R14a). */
  extracted: number;
  /** Listed / seen / seen-but-not-listed. */
  counts: { indexed: number; available: number; omitted: number };
};

const LIMIT_WARNING = /: only the first \d+ files are indexed|^Scan limit reached|: [\d,]+ files past the first [\d,]+ are not listed|^Inventory limit reached/;
const n = (x: number) => x.toLocaleString('en-US');
const files = (x: number) => `${n(x)} ${x === 1 ? 'file' : 'files'}`;
const sum = (xs: number[]) => xs.reduce((a, b) => a + b, 0);

/** Listed files in a connection's inventory (0 before the first scan). */
export function indexedCount(c: Pick<DirectoryConnection, 'coverage'> | undefined): number {
  const cov = c?.coverage;
  return cov ? cov.indexed ?? sum(cov.projects.map(p => p.indexed)) : 0;
}

export function summarizeCoverage(c: Pick<DirectoryConnection, 'projects' | 'warnings' | 'coverage'> | undefined): CoverageSummary {
  const none = { unread: { tooLarge: 0, needsReader: 0 }, extracted: 0, counts: { indexed: 0, available: 0, omitted: 0 } };
  if (!c) return { state: 'unknown', headline: 'Coverage not checked yet.', capped: [], other: [], ...none };
  const other = c.warnings.filter(w => !LIMIT_WARNING.test(w));
  const cov = c.coverage;
  if (!cov) {
    // A manifest from before structured coverage: keep every warning visible.
    const limited = c.warnings.some(w => LIMIT_WARNING.test(w));
    return { state: limited ? 'partial' : 'unknown', headline: limited ? 'Coverage incomplete. Refresh projects for details.' : `${n(c.projects.length)} ${c.projects.length === 1 ? 'project' : 'projects'}. Refresh projects to check coverage.`, capped: [], other: c.warnings, ...none };
  }
  // A manifest from before P07 has no totals; derive them from the projects.
  const legacy = cov.indexed === undefined;
  const indexed = cov.indexed ?? sum(cov.projects.map(p => p.indexed));
  const available = cov.available ?? Math.max(indexed, sum(cov.projects.map(p => p.found)));
  const omitted = cov.omitted ?? Math.max(0, available - indexed);
  const unread = { tooLarge: cov.tooLarge ?? 0, needsReader: cov.needsReader ?? 0 };
  const counts = { indexed, available, omitted };
  const extracted = cov.extracted ?? 0;
  const projectOmitted = (p: SourceCoverage['projects'][number]) => p.omitted ?? Math.max(0, p.found - p.indexed);
  const capped = cov.projects.filter(p => p.capped).sort((a, b) => projectOmitted(b) - projectOmitted(a) || b.found - a.found);
  const projects = `${n(cov.projects.length)} ${cov.projects.length === 1 ? 'project' : 'projects'}`;
  const parts: string[] = [];
  if (legacy) {
    if (capped.length) parts.push(`${capped.length} of ${projects} ${capped.length === 1 ? 'has' : 'have'} more than ${n(cov.maxProjectFiles)} files, so only the first ${n(cov.maxProjectFiles)} of each are indexed`);
    if (cov.scanLimited) parts.push(`the scan stopped at ${n(cov.maxFiles)} files, so projects later in the folder may be missing`);
  } else {
    if (capped.length) parts.push(`${capped.length} of ${projects} ${capped.length === 1 ? 'has' : 'have'} more than ${n(cov.maxProjectFiles)} files, so ${files(omitted)} ${omitted === 1 ? 'isn’t' : 'aren’t'} listed (search still finds them by name)`);
    else if (omitted) parts.push(`${files(omitted)} ${omitted === 1 ? 'isn’t' : 'aren’t'} listed`);
    if (cov.inventoryLimited) parts.push(`the list reached its ${n(cov.maxFiles)}-file limit`);
    if (cov.scanLimited) parts.push('the scan stopped early, so projects later in the folder may be missing');
  }
  if (parts.length) return { state: 'partial', headline: `Coverage incomplete: ${parts.join('; ')}.`, capped, other, unread, extracted, counts };
  const excluded = cov.excluded.length ? ` ${cov.excluded.length} ${cov.excluded.length === 1 ? 'folder is' : 'folders are'} left out by you.` : '';
  const notRead = [
    unread.tooLarge ? `${n(unread.tooLarge)} ${unread.tooLarge === 1 ? 'is' : 'are'} too large to read` : '',
    unread.needsReader ? `${n(unread.needsReader)} ${unread.needsReader === 1 ? 'needs' : 'need'} a reader` : '',
  ].filter(Boolean).join(' and ');
  const readByHollow = extracted ? ` ${n(extracted)} PDF and Office ${extracted === 1 ? 'file is' : 'files are'} read by Hollow.` : '';
  return { state: other.length ? 'partial' : 'complete', headline: `${files(indexed)} across ${projects}, all indexed.${readByHollow}${notRead ? ` ${notRead[0].toUpperCase()}${notRead.slice(1)}.` : ''}${excluded}`, capped, other, unread, extracted, counts };
}

/** Settings, Advanced, Search & indexes's progress for Search inside project files
 *  (repo features R14b): `12,340 of 15,002 files · 41 image-only PDFs`,
 *  plus failures and files left out for privacy when there are any. */
export function contentIndexProgress(s: ConnectedIndexStatus | null | undefined): string {
  if (!s) return 'Checking…';
  if (!s.enabled) return 'Off: agents find project files by name and date only. No index is kept.';
  const leftOut = `${n(s.skipped)} left out (restricted folders, secret names)`;
  if (!s.total) return s.skipped ? `Nothing to index · ${leftOut}` : 'Nothing to index yet.';
  const parts = [`${n(s.indexed)} of ${files(s.total)}`];
  if (s.imageOnly) parts.push(`${n(s.imageOnly)} image-only ${s.imageOnly === 1 ? 'PDF' : 'PDFs'}`);
  if (s.failed) parts.push(`${n(s.failed)} couldn’t be read`);
  if (s.skipped) parts.push(leftOut);
  return parts.join(' · ');
}

/** Whether the index is still filling in (Settings polls only then):
 *  on, and fewer files read (indexed, image-only or failed) than listed. */
export function contentIndexFilling(s: ConnectedIndexStatus | null | undefined): boolean {
  return !!s?.enabled && s.indexed + s.imageOnly + s.failed < s.total;
}

/** Library → Connected projects' coverage line: `Contents indexed: N of M`;
 *  empty while the index is off or unknown. */
export function contentsIndexedLine(s: ConnectedIndexStatus | null | undefined): string {
  return s?.enabled ? `Contents indexed: ${n(s.indexed)} of ${n(s.total)}` : '';
}

/** "Refreshed 4 min ago": when the last successful refresh finished. */
export function refreshedLabel(refreshedAt: string | undefined, now = Date.now()): string {
  const at = refreshedAt ? Date.parse(refreshedAt) : NaN;
  if (!Number.isFinite(at)) return 'Not refreshed yet';
  const min = Math.floor(Math.max(0, now - at) / 60_000);
  if (min < 1) return 'Refreshed just now';
  if (min < 60) return `Refreshed ${min} min ago`;
  if (min < 24 * 60) return `Refreshed ${Math.floor(min / 60)} h ago`;
  return `Refreshed ${new Date(at).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}`;
}

/** What leaving a folder out does, stated before it happens. */
export function exclusionConsequence(relative: string, files: number): string {
  return `Hollow stops indexing “${relative}” (${n(files)} indexed ${files === 1 ? 'file' : 'files'}). Library won’t list it and agents won’t receive excerpts from it. Nothing inside the folder changes, and you can include it again here. Hollow rescans your projects afterwards.`;
}
