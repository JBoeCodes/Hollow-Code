// Import conversations' words and selection rules (H6 with H5a; design
// `sources-and-import-2026-11-13` section 4). Pure, unit-tested in
// importConversations.test.ts. The component is
// `components/ImportConversations.tsx` (lazy: the first-run sheet's Find
// what's already here step, and Library later); calls are `api.*SessionImport`.
import type { ImportCommitResult, ImportEntry, ImportJob, ImportManifest, ImportSource, ImportStatusId } from './api';

/** A source card is shown: a folder source only when its folder exists; an
 *  export source (no default folder) whenever the backend lists it. */
export function visibleSources(sources: readonly ImportSource[]): ImportSource[] {
  return sources.filter(s => isExportSource(s) || s.exists);
}

/** Whether a source is read from an export file the person picks (ChatGPT). */
export const isExportSource = (s: Pick<ImportSource, 'export' | 'defaultRoot'>): boolean => s.export || s.defaultRoot === null;

/** The line on a source card before Look: where it reads, and that it asks first. */
export function sourceLine(s: ImportSource): string {
  if (isExportSource(s)) return `Choose the ${s.label} export file. Hollow reads it only after you pick it.`;
  return `Reads ${s.defaultRoot}. Nothing is read until you click Look.`;
}

/** Statuses a person can pick: the rest are listed but never imported. */
const PICKABLE: ReadonlySet<ImportStatusId> = new Set(['new', 'updated', 'newVersion', 'alreadyImported']);
/** Statuses picked by default: everything not imported yet. */
const DEFAULT_PICK: ReadonlySet<ImportStatusId> = new Set(['new', 'updated', 'newVersion']);

export const pickable = (e: ImportEntry): boolean => PICKABLE.has(e.status);

/** The default selection: everything not yet imported (and not Hollow's own). */
export function defaultSelection(manifest: Pick<ImportManifest, 'conversations'>): Set<string> {
  return new Set(manifest.conversations.filter(e => DEFAULT_PICK.has(e.status)).map(e => e.id));
}

/** Select all: every pickable entry. */
export function selectAll(manifest: Pick<ImportManifest, 'conversations'>): Set<string> {
  return new Set(manifest.conversations.filter(pickable).map(e => e.id));
}

export const STATUS_LABELS: Record<ImportStatusId, string> = {
  new: 'New',
  alreadyImported: 'Already imported',
  updated: 'Updated since import',
  newVersion: 'New version',
  hollowOwn: 'Hollow’s own chat',
  subagent: 'Subagent',
  unsupported: 'Not supported',
};

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** An inspect pass's progress line. */
export function progressText(job: Pick<ImportJob, 'state' | 'done' | 'total' | 'error'> | null | undefined): string {
  if (!job) return 'Starting';
  if (job.error) return `Could not read it: ${job.error}`;
  if (job.state === 'cancelled') return 'Cancelled';
  if (job.state === 'failed') return 'Could not read it.';
  if (job.total > 0) return `Reading ${job.done} of ${plural(job.total, 'file')}`;
  return 'Reading';
}

/** Whether an inspect pass is over: `inspecting` while it reads; then
 *  `ready`, `cancelled` or `failed` (Rust `session_import::JobStatus`). */
export const jobFinished = (job: Pick<ImportJob, 'state'> | null | undefined): boolean => !!job && job.state !== 'inspecting';

/** Whether a finished pass left a list to read. */
export const jobListable = (job: Pick<ImportJob, 'state'> | null | undefined): boolean => !!job && (job.state === 'ready' || job.state === 'committed');

/** The list's heading line: "38 conversations: 30 new, 6 already imported, 2 updated". */
export function countsText(manifest: Pick<ImportManifest, 'conversations' | 'counts'>): string {
  const c = manifest.counts;
  const parts: string[] = [];
  if (c.new) parts.push(`${c.new} new`);
  if (c.updated) parts.push(`${c.updated} updated`);
  if (c.newVersion) parts.push(plural(c.newVersion, 'new version'));
  if (c.alreadyImported) parts.push(`${c.alreadyImported} already imported`);
  if (c.hollowOwn) parts.push(`${c.hollowOwn} Hollow’s own`);
  const head = plural(manifest.conversations.length, 'conversation');
  return parts.length ? `${head}: ${parts.join(', ')}` : head;
}

/** Partial discovery is visible even when the backend returned usable rows. */
export function scanNotice(m: Pick<ImportManifest, 'capped' | 'stopped' | 'unreadable' | 'symlinksSkipped'>): string | null {
  const notes: string[] = [];
  if (m.capped || m.stopped) notes.push('Scan limit reached. This list is incomplete.');
  if (m.stopped) notes.push(m.stopped);
  if (m.unreadable) notes.push(`${plural(m.unreadable, 'file')} could not be read.`);
  if (m.symlinksSkipped) notes.push(`${plural(m.symlinksSkipped, 'symbolic link')} skipped.`);
  return notes.length ? notes.join(' ') : null;
}

/** The Import button's words for `n` picked. */
export const importLabel = (n: number): string => (n ? `Import ${plural(n, 'conversation')}` : 'Import');

/** A commit's result line: imported, updated, already there, skipped, failed. */
export function resultText(r: Pick<ImportCommitResult, 'imported' | 'updated' | 'newVersions' | 'alreadyImported' | 'skipped' | 'failed'>): string {
  const parts = [`Imported ${r.imported}`];
  if (r.updated) parts.push(`updated ${r.updated}`);
  if (r.newVersions) parts.push(plural(r.newVersions, 'new version'));
  if (r.alreadyImported) parts.push(`${r.alreadyImported} already there`);
  if (r.skipped) parts.push(`${r.skipped} skipped`);
  if (r.failed.length) parts.push(`${r.failed.length} failed`);
  return `${parts.join(', ')}.`;
}

/** How many conversations a commit brought in (new, updated or a new version). */
export const broughtIn = (r: Pick<ImportCommitResult, 'imported' | 'updated' | 'newVersions'>): number => r.imported + r.updated + r.newVersions;

/** One entry's second line: dates, messages and project folder. */
export function entryMeta(e: Pick<ImportEntry, 'lastAt' | 'createdAt' | 'messageCount' | 'projectFolder'>): string {
  const when = (e.lastAt ?? e.createdAt ?? '').slice(0, 10);
  const parts = [when, plural(e.messageCount, 'message')].filter(Boolean);
  if (e.projectFolder) parts.push(e.projectFolder);
  return parts.join(' · ');
}
