import type { NoteRef } from './types';
import { naturalCompare } from './collate';

/** One Library row: what the note is called and a line about it. */
export interface LibraryEntry {
  path: string;
  basename: string;
  title: string;
  /** Date (for dated file names) and summary; empty when neither helps. The path is the row's tooltip. */
  detail: string;
}

/** Notes from one folder, named for people rather than for paths. */
export interface LibraryGroup {
  key: string;
  label: string;
  entries: LibraryEntry[];
  /** Citation evidence, raw copies and archives start folded. */
  folded: boolean;
}

const DATED = /^\d{4}-\d{2}-\d{2}/;
const FOLDED_SEGMENT = /^(evidence|raw|readable|archive.*|attachments)$/i;

export function libraryTitle(n: Pick<NoteRef, 'basename' | 'title'>): string {
  return n.title?.trim() || n.basename.replace(/\.(md|html?)$/i, '');
}

/** `archive-v1` → `Archive v1`, `quarter-2026-09-09` → `Quarter 2026-09-09`; dates keep their dashes. */
function humanize(segment: string): string {
  const s = segment.replace(/[-_]/g, (c, i: number) => /\d/.test(segment[i - 1] ?? '') && /\d/.test(segment[i + 1] ?? '') ? c : ' ');
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function folderOf(path: string): string {
  const i = path.lastIndexOf('/');
  return i < 0 ? '' : path.slice(0, i);
}

/** Folder label. A folder note (`atlas/Atlas.md`) names its folder; `library/` is implied. */
function folderLabel(dir: string, byPath: Map<string, NoteRef>): string {
  if (!dir) return 'Vault';
  const parts = dir.split('/');
  const shown = parts[0] === 'library' && parts.length > 1 ? 1 : 0;
  return parts.slice(shown).map((seg, i) => {
    const note = byPath.get(`${parts.slice(0, shown + i + 1).join('/')}/${seg}.md`.toLowerCase());
    return note ? libraryTitle(note) : humanize(seg);
  }).join(' › ');
}

/** A summary that only repeats the title (or a heading cut short with `…`) adds nothing. */
function echoes(summary: string, title: string): boolean {
  const a = summary.toLowerCase(), b = title.replace(/…$/, '').toLowerCase();
  return b.startsWith(a) || (title.endsWith('…') && a.startsWith(b));
}

function entries(notes: NoteRef[]): LibraryEntry[] {
  // A sentence every note in the folder opens with ("Untrusted email
  // evidence.") is boilerplate, not a description.
  const seen = new Map<string, number>();
  for (const n of notes) if (n.summary) seen.set(n.summary, (seen.get(n.summary) ?? 0) + 1);
  const dated = notes.filter(n => DATED.test(n.basename)).length * 2 >= notes.length;
  return notes.map(n => {
    const title = libraryTitle(n);
    const summary = n.summary && (seen.get(n.summary) ?? 0) < 3 && !echoes(n.summary, title) ? n.summary : '';
    // Dated notes keep their date in view once the title replaces the file name.
    const date = n.basename.match(DATED)?.[0];
    const when = date && !title.includes(date) ? date : '';
    return { path: n.path, basename: n.basename, title, detail: [when, summary].filter(Boolean).join(' · ') };
  }).sort((a, b) => dated
    ? b.basename.localeCompare(a.basename)
    : naturalCompare(a.title, b.title));
}

/**
 * The Library grouped by folder: `library/` folders first, then the rest of
 * the vault, root files last; folded groups after open ones at each level.
 * Folders are only merged, never split.
 * Dated folders (meetings, daily plans) list newest first.
 */
export function groupLibrary(notes: NoteRef[]): LibraryGroup[] {
  const byPath = new Map(notes.map(n => [n.path.toLowerCase(), n]));
  const byDir = new Map<string, NoteRef[]>();
  for (const n of notes) {
    const dir = folderOf(n.path);
    const list = byDir.get(dir);
    if (list) list.push(n); else byDir.set(dir, [n]);
  }
  // A folder holding one note (an index, a lone email scan) joins the
  // nearest listed ancestor in the same top-level folder, so the page isn't
  // a column of one-row groups. Folded and open folders never mix.
  const isFolded = (dir: string) => dir.split('/').some(seg => FOLDED_SEGMENT.test(seg));
  for (const dir of [...byDir.keys()].sort((a, b) => b.split('/').length - a.split('/').length)) {
    const list = byDir.get(dir)!;
    // A folder note (`atlas/Atlas.md`) is its folder's hub and stays put.
    if (list.length !== 1 || list[0].basename.toLowerCase() === dir.split('/').pop()!.toLowerCase()) continue;
    for (let up = folderOf(dir); up; up = folderOf(up)) {
      const into = byDir.get(up);
      if (!into) continue;
      if (isFolded(up) === isFolded(dir)) { into.push(...list); byDir.delete(dir); }
      break;
    }
  }
  const rank = (key: string) => key === '' ? 2 : key === 'library' || key.startsWith('library/') ? 0 : 1;
  return [...byDir].map(([key, list]): LibraryGroup => ({
    key,
    label: folderLabel(key, byPath),
    entries: entries(list),
    folded: isFolded(key),
  })).sort((a, b) => rank(a.key) - rank(b.key) || Number(a.folded) - Number(b.folded) || a.key.localeCompare(b.key));
}

/** What a `[[wikilink]]` clicked in Library does (P0-26): open the note it
 *  names, by path, file name or title (case and a trailing `.md` ignored),
 *  or offer to create it when no note matches. */
export type WikilinkStep = { kind: 'open'; path: string } | { kind: 'create'; title: string };
export function wikilinkStep(name: string, notes: readonly NoteRef[]): WikilinkStep {
  const want = name.trim().replace(/\.md$/i, '').toLowerCase();
  const base = want.split('/').pop() ?? '';
  const hit = want ? notes.find(n => n.path.replace(/\.md$/i, '').toLowerCase() === want)
    ?? notes.find(n => n.basename.replace(/\.md$/i, '').toLowerCase() === base)
    ?? notes.find(n => n.title?.trim().toLowerCase() === want) : undefined;
  return hit ? { kind: 'open', path: hit.path } : { kind: 'create', title: name.trim().replace(/\.md$/i, '').split('/').pop() ?? '' };
}

/** Name filter over title, file name and path. */
export function matchesLibraryQuery(n: NoteRef, query: string): boolean {
  const q = query.trim().toLowerCase();
  return !q || `${n.title ?? ''} ${n.summary ?? ''} ${n.basename} ${n.path}`.toLowerCase().includes(q);
}
