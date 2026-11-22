// Library → All (overnight 11-12 L3): one searchable list of what Library's
// readers already load. Notes come from the vault's classified index
// (`vault_items`, with dates and kinds), outputs and canvases from the
// Results model (`libraryResultsModel.ts`), recordings from Plaud's list.
// Pure: no reads, no signals. `components/LibraryList.tsx` renders it.
// Nothing in the vault moves: the filters are rules over paths and kinds.
import type { VaultItem } from './types';
import type { MixedResult } from './libraryResultsModel';
import type { RecordingList, RecordingRow } from './recordingTypes';
import { isProjectRecord } from './sourceCatalog';

export type LibraryListKind = 'note' | 'meeting' | 'person' | 'output' | 'canvas' | 'recording';
export type LibraryListFilter = 'recent' | 'meetings' | 'people' | 'outputs';
export type LibraryListSource = 'notes' | 'results' | 'canvases' | 'recordings';

export interface LibraryListRow {
  key: string;
  kind: LibraryListKind;
  title: string;
  /** Where it lives, short: a workspace's title, a folder, or Plaud. */
  where: string;
  /** ISO date or null when unknown (sorted last). */
  modified: string | null;
  /** Vault-relative path for a file; null for a recording or a canvas. */
  path: string | null;
  workspaceId?: string;
  canvasId?: string;
  recordingId?: string;
  source: LibraryListSource;
  /** A result file that is also a meeting note or a person note (the note
   *  at its path): it shows in Meetings or People as well as Outputs. */
  also?: 'meeting' | 'person';
}

export const LIBRARY_LIST_FILTERS: readonly { value: LibraryListFilter; label: string }[] = [
  { value: 'recent', label: 'Recent' }, { value: 'meetings', label: 'Meetings' },
  { value: 'people', label: 'People' }, { value: 'outputs', label: 'Outputs' },
];

/** What each filter shows, for its empty state and its description. */
export const FILTER_SHOWS: Readonly<Record<LibraryListFilter, string>> = {
  recent: 'Everything in your Library from the last 30 days, newest first.',
  meetings: 'Recordings in Plaud, kept transcripts and meeting notes (calendar/, meetings/, recordings/ and library/records/meetings/).',
  people: 'People notes (people/, library/people/, library/work/people/ and library/records/people/).',
  outputs: 'Result files from workspaces and saved canvases.',
};

/** Recent's window; Show older lifts it. */
export const RECENT_DAYS = 30;

// Case-insensitive: a `People/` folder made in Finder is still people.
const MEETING_PATH = /^(calendar|meetings|recordings)\/|^library\/records\/meetings\//i;
const PEOPLE_PATH = /^(people|library\/people|library\/work\/people|library\/records\/people)\//i;
const OUTPUT_PATH = /^(briefings|outputs)\/|^workspaces\/[^/]+\/outputs\//i;
/** Folders the list leaves to their own pages: workspaces' working files
 *  (their outputs come from Results), conversations, automations, the
 *  operator profile and task files (Tasks). */
const SKIPPED_PATH = /^(workspaces|sessions|workflows|me|tasks)\//i;

export const isMeetingNote = (path: string, kind?: string) => MEETING_PATH.test(path) || kind === 'meeting' || kind === 'recording';
export const isPeopleNote = (path: string, kind?: string) => PEOPLE_PATH.test(path) || kind === 'person';

/** A note's kind in the list: meeting and people rules first, then outputs. */
export function noteKind(path: string, kind?: string): LibraryListKind {
  if (isMeetingNote(path, kind)) return 'meeting';
  if (isPeopleNote(path, kind)) return 'person';
  if (OUTPUT_PATH.test(path)) return 'output';
  return 'note';
}

/** Whether a classified vault file belongs in the list: hidden mirrors stay
 *  out except meeting notes and kept transcripts; workspace files only as
 *  their project record. */
export function listsNote(item: Pick<VaultItem, 'path' | 'kind' | 'hidden'>): boolean {
  if (!/\.md$/i.test(item.path) || item.path.startsWith('.')) return false;
  if (isProjectRecord(item.path)) return true;
  if (SKIPPED_PATH.test(item.path)) return false;
  return !item.hidden || isMeetingNote(item.path, item.kind);
}

/** A note's folder, short: a leading `library/` dropped when more follows,
 *  the last two folders kept. */
export function shortFolder(path: string): string {
  const parts = path.split('/').slice(0, -1);
  if (parts[0] === 'library' && parts.length > 1) parts.shift();
  if (!parts.length) return 'Vault root';
  return parts.length > 2 ? `…/${parts.slice(-2).join('/')}` : parts.join('/');
}

const knownDate = (value: string | null | undefined): string | null => value && Number.isFinite(Date.parse(value)) ? value : null;

export function noteRows(items: readonly VaultItem[]): LibraryListRow[] {
  return items.filter(listsNote).map(n => {
    const ws = isProjectRecord(n.path) ? n.path.split('/')[1] : undefined;
    return {
      key: JSON.stringify(['note', n.path]), kind: noteKind(n.path, n.kind),
      title: n.title || (n.path.split('/').pop() ?? n.path).replace(/\.md$/i, ''),
      where: ws ? 'Project record' : shortFolder(n.path), modified: knownDate(n.mtime), path: n.path,
      workspaceId: ws, source: 'notes' as const,
    };
  });
}

export function resultRows(results: readonly MixedResult[]): LibraryListRow[] {
  return results.map(r => ({
    key: r.key, kind: r.canvas ? 'canvas' as const : 'output' as const, title: r.title, where: r.workspaceTitle,
    modified: knownDate(r.modified), path: r.canvas ? null : r.card?.path ?? null, workspaceId: r.workspaceId,
    canvasId: r.canvas?.id, source: r.canvas ? 'canvases' as const : 'results' as const,
  }));
}

export function recordingRows(recordings: readonly RecordingRow[]): LibraryListRow[] {
  return recordings.map(r => ({
    key: JSON.stringify(['recording', r.id]), kind: 'recording' as const, title: r.name || 'Untitled recording',
    where: 'Plaud', modified: knownDate(r.date), path: null, recordingId: r.id, source: 'recordings' as const,
  }));
}

/** One list, newest first (unknown dates last, then by title). A note at a
 *  result file's path is the same file: the result row, which carries the
 *  workspace and review, wins. `items` are the vault's classified files
 *  before {@link listsNote}: results live under `workspaces/<id>/outputs/`,
 *  which the list leaves out as notes, so the index's meeting or person kind
 *  for a result is read from them (lane S re-review of #1319). */
export function buildLibraryList(parts: {
  notes?: readonly LibraryListRow[]; results?: readonly LibraryListRow[]; recordings?: readonly LibraryListRow[];
  items?: readonly Pick<VaultItem, 'path' | 'kind'>[];
}): LibraryListRow[] {
  const results = parts.results ?? [];
  const fold = (p: string) => p.toLowerCase();
  const resultPaths = new Set(results.map(r => r.path).filter((p): p is string => !!p).map(fold));
  // The note's meeting or person kind survives on the result row.
  const noteKindAt = new Map<string, 'meeting' | 'person'>();
  for (const n of parts.items ?? []) {
    const kind = noteKind(n.path, n.kind);
    if (kind === 'meeting' || kind === 'person') noteKindAt.set(fold(n.path), kind);
  }
  for (const n of parts.notes ?? []) if (n.path && (n.kind === 'meeting' || n.kind === 'person')) noteKindAt.set(fold(n.path), n.kind);
  const kept = results.map(r => { const also = r.path ? noteKindAt.get(fold(r.path)) : undefined; return also ? { ...r, also } : r; });
  const rows = [...(parts.notes ?? []).filter(n => !n.path || !resultPaths.has(fold(n.path))), ...kept, ...(parts.recordings ?? [])];
  const time = (value: string | null) => { const t = value === null ? NaN : Date.parse(value); return Number.isFinite(t) ? t : null; };
  return rows.map(row => ({ row, at: time(row.modified) }))
    .sort((a, b) => a.at === null ? b.at === null ? a.row.title.localeCompare(b.row.title) : 1 : b.at === null ? -1 : b.at - a.at || a.row.title.localeCompare(b.row.title))
    .map(({ row }) => row);
}

export function inFilter(row: Pick<LibraryListRow, 'kind' | 'also'>, filter: LibraryListFilter): boolean {
  if (filter === 'meetings') return row.kind === 'meeting' || row.kind === 'recording' || row.also === 'meeting';
  if (filter === 'people') return row.kind === 'person' || row.also === 'person';
  if (filter === 'outputs') return row.kind === 'output' || row.kind === 'canvas';
  return true;
}

/** Every word of `query` in the title, where it lives or the path. */
export function matchesListQuery(row: Pick<LibraryListRow, 'title' | 'where' | 'path'>, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = `${row.title}\n${row.where}\n${row.path ?? ''}`.toLowerCase();
  return words.every(w => hay.includes(w));
}

export interface ListView { rows: LibraryListRow[]; /** Rows Recent's window hides (Show older). */ older: number }

/** The rows on screen. Recent keeps the last 30 days until Show older; a
 *  search looks through everything, as do the other filters. */
export function filterLibraryList(rows: readonly LibraryListRow[], opts: { filter: LibraryListFilter; query: string; showOlder: boolean; now: number }): ListView {
  const matched = rows.filter(r => inFilter(r, opts.filter) && matchesListQuery(r, opts.query));
  if (opts.filter !== 'recent' || opts.showOlder || opts.query.trim()) return { rows: matched, older: 0 };
  const since = opts.now - RECENT_DAYS * 86_400_000;
  const recent = matched.filter(r => r.modified !== null && Date.parse(r.modified) >= since); // NaN never passes
  return { rows: recent, older: matched.length - recent.length };
}

/** By meaning: the index's hits in its order, as rows. A hit the list has
 *  keeps its row; one it skips (a workspace file) still shows, as a note. */
export function meaningRows(rows: readonly LibraryListRow[], hits: readonly { path: string; title?: string | null; basename?: string }[], filter: LibraryListFilter): LibraryListRow[] {
  const byPath = new Map(rows.filter(r => r.path).map(r => [r.path!, r]));
  const out: LibraryListRow[] = [];
  const seen = new Set<string>();
  for (const h of hits) {
    if (seen.has(h.path)) continue;
    seen.add(h.path);
    const row = byPath.get(h.path) ?? {
      key: JSON.stringify(['note', h.path]), kind: noteKind(h.path), source: 'notes' as const, path: h.path, modified: null,
      title: h.title || h.basename || (h.path.split('/').pop() ?? h.path).replace(/\.md$/i, ''), where: shortFolder(h.path),
    };
    if (inFilter(row, filter)) out.push(row);
  }
  return out;
}

export type SourceState = { status: 'loading' } | { status: 'ready' } | { status: 'failed'; error: string } | { status: 'off'; reason: string };

/** A source's state from a read that never suspends (lane S re-review of
 *  #1319): its error first, then no value yet is still loading. Each source
 *  answers on its own, so a slow one (Plaud can take 90 s) holds back only
 *  its own rows. */
export function readState(error: string | null, value: unknown): SourceState {
  if (error) return { status: 'failed', error };
  return value === undefined ? { status: 'loading' } : { status: 'ready' };
}

/** Recordings: {@link readState}, then off when Plaud isn't connected and
 *  failed when it didn't answer. */
export function recordingsState(error: string | null, list: Pick<RecordingList, 'state' | 'error'> | undefined): SourceState {
  const read = readState(error, list);
  if (read.status !== 'ready' || !list) return read;
  if (list.state === 'notConnected') return { status: 'off', reason: 'Plaud isn’t connected, so recordings aren’t listed.' };
  if (list.state === 'unavailable') return { status: 'failed', error: list.error ?? 'Plaud didn’t answer.' };
  return read;
}

/** How many rows to paint so the row at `index` is among them: whole pages
 *  of `page`, never fewer than `current`. Show older reveals rows past the
 *  recent ones, and the first of them takes focus (lane S re-review). */
export function limitShowing(index: number, page: number, current = page): number {
  return Math.max(current, page, Math.ceil((index + 1) / page) * page);
}

/** The sources a filter still waits on, for "Still reading …". */
export function stillReading(sources: readonly LibraryListSource[], state: (source: LibraryListSource) => SourceState): LibraryListSource[] {
  return sources.filter(s => state(s).status === 'loading');
}

export const SOURCE_LABEL: Readonly<Record<LibraryListSource, string>> = {
  notes: 'Notes', results: 'Result files', canvases: 'Canvases', recordings: 'Recordings',
};

/** Which sources a filter reads: a failure elsewhere doesn't touch it.
 *  Meetings and People read result files too: a result the index calls a
 *  meeting or a person shows there (`also`), so a slow or failed results
 *  read says so there instead of an empty list (lane S re-review). */
export const FILTER_SOURCES: Readonly<Record<LibraryListFilter, readonly LibraryListSource[]>> = {
  recent: ['notes', 'results', 'canvases', 'recordings'],
  meetings: ['notes', 'results', 'recordings'],
  people: ['notes', 'results'],
  outputs: ['notes', 'results', 'canvases'],
};

/** The empty state for what is on screen: the list's, the search's or the filter's. */
export function listEmpty(opts: { total: number; filter: LibraryListFilter; query: string; older: number; failed: boolean; loading: boolean }):
  { kind: 'loading' | 'empty' | 'search' | 'filter' | 'failed'; title: string; detail: string } {
  if (opts.query.trim()) return { kind: 'search', title: 'Nothing matches your search', detail: `Nothing in ${opts.filter === 'recent' ? 'your Library' : LIBRARY_LIST_FILTERS.find(f => f.value === opts.filter)!.label} matches “${opts.query.trim()}”.` };
  if (opts.loading && !opts.total) return { kind: 'loading', title: 'Loading your Library', detail: '' };
  if (!opts.total) return opts.failed
    ? { kind: 'failed', title: 'Your Library couldn’t be read', detail: 'Retry the sources above.' }
    : { kind: 'empty', title: 'Nothing in your Library yet', detail: 'Notes, meeting notes, recordings and the results your workspaces make show up here.' };
  if (opts.filter === 'recent' && opts.older) return { kind: 'filter', title: `Nothing from the last ${RECENT_DAYS} days`, detail: FILTER_SHOWS.recent };
  return { kind: 'filter', title: `No ${LIBRARY_LIST_FILTERS.find(f => f.value === opts.filter)!.label.toLowerCase()} yet`, detail: FILTER_SHOWS[opts.filter] };
}

/** The most result files Library asks for (the backend clamps at 2,000). */
export const LIBRARY_RESULTS_CAP = 1000;

/** What Library asks the backend for: one past the cap, so a listing of exactly the cap is not mistaken for a cut one. */
export const LIBRARY_RESULTS_ASK = LIBRARY_RESULTS_CAP + 1;

/** True when the listing came back longer than the cap, so more files exist than are shown. */
export const resultsCapped = (listed: number | undefined): boolean => (listed ?? 0) > LIBRARY_RESULTS_CAP;

/** True for the backend's message for a review store it found damaged and left in place. */
export const isDamagedResultsStore = (error: string | null | undefined): boolean => !!error && /Results review state (could not be read|was damaged)/.test(error);

/** The count label's partial note: a source still reading or failed, or a listing cut at the cap. */
export const partialNote = (opts: { loading: boolean; failed: boolean; capped: boolean }): string => opts.loading || opts.failed || opts.capped ? ' · partial' : '';
