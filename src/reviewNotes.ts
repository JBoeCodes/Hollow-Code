/** Line-anchored review notes on a file in the Preview pane (overnight 11-17,
 *  row OC9): a person reads a generated file or a plan once, leaves a note on
 *  each line that needs work, and the notes go to the chat as ONE message for
 *  one revision turn. Pure model and storage helpers, unit-tested;
 *  `components/PreviewReviewNotes.tsx` draws them.
 *
 *  A note remembers its line's trimmed text and the trimmed lines around it, so
 *  after the agent edits the file `reanchor` can find it again. Text is plain
 *  text everywhere: it is shown as text nodes and sent as words, never HTML. */

export const REVIEW_NOTES_KEY = 'hollow.reviewNotes.v1';
/** Notes kept per file (the oldest are dropped past this) and files kept. */
export const MAX_NOTES_PER_FILE = 200;
export const MAX_NOTE_FILES = 40;
/** A note's own text, and the quoted line in a sent message. */
export const MAX_NOTE_CHARS = 2000;
export const QUOTE_CHARS = 120;
/** The composer has no cap of its own, so a batch keeps to a size a person
 *  can still read in the box and an agent reads as one request. */
export const BATCH_MAX_CHARS = 12_000;
/** Lines the notes view draws, so a 1.5 MB file never makes a DOM node per line. */
export const MAX_REVIEW_LINES = 1500;

export interface ReviewNote {
  id: string;
  path: string;
  /** 1-based line the note is on now; the last known one while `lost`. */
  line: number;
  /** That line's trimmed text when the note was made (or last found). */
  anchorText: string;
  /** The trimmed lines just before and after it, to tell repeats apart. */
  before: string;
  after: string;
  text: string;
  resolved: boolean;
  /** The line could not be found again in the current file. */
  lost: boolean;
  createdAt: number;
  /** When the note last went into the chat draft (informational). */
  sentAt?: number;
}

/** A file's lines (a trailing newline is not a line). */
export function splitLines(text: string): string[] {
  const lines = text.split(/\r?\n/);
  if (lines.length > 1 && lines[lines.length - 1] === '') lines.pop();
  return lines;
}

const trimmedAt = (trimmed: readonly string[], line: number): string => trimmed[line - 1] ?? '';

/** A new note on `line` (1-based) of `lines`, with the line's words and its neighbours. */
export function makeNote(path: string, lines: readonly string[], line: number, text: string, now: number, id: string): ReviewNote {
  const trimmed = lines.map(l => l.trim());
  return {
    id, path, line,
    anchorText: trimmedAt(trimmed, line),
    before: trimmedAt(trimmed, line - 1),
    after: trimmedAt(trimmed, line + 1),
    text: text.trim().slice(0, MAX_NOTE_CHARS),
    resolved: false, lost: false, createdAt: now,
  };
}

/**
 * Each note on its line in `newText`. A note stays where it is while that line
 * still reads the same; otherwise it moves to the nearest line with the same
 * words, preferring the one whose neighbours also match; a line nobody can
 * find is marked `lost` and keeps its old number (the note still goes out with
 * its quoted words). A note that cannot move stays the same object, so a
 * caller can tell whether anything changed.
 */
export function reanchor(notes: readonly ReviewNote[], newText: string): ReviewNote[] {
  const trimmed = splitLines(newText).map(l => l.trim());
  const byText = new Map<string, number[]>();
  trimmed.forEach((t, i) => { const at = byText.get(t); if (at) at.push(i + 1); else byText.set(t, [i + 1]); });
  return notes.map(note => {
    const candidates = byText.get(note.anchorText) ?? [];
    if (!candidates.length) return note.lost ? note : { ...note, lost: true };
    const fit = (line: number) => (trimmedAt(trimmed, line - 1) === note.before ? 1 : 0) + (trimmedAt(trimmed, line + 1) === note.after ? 1 : 0);
    const here = candidates.includes(note.line) ? note.line : 0;
    // The same line stays unless its neighbours both changed while another
    // identical line has both of its own intact (the line moved, a twin took its place).
    let line = here && (fit(here) > 0 || !candidates.some(c => c !== here && fit(c) === 2)) ? here : 0;
    if (!line) {
      line = candidates.reduce((best, c) => {
        const better = fit(c) - fit(best) || Math.abs(best - note.line) - Math.abs(c - note.line);
        return better > 0 ? c : best;
      });
    }
    // A blank line is too common to follow on its own: it needs a neighbour.
    if (note.anchorText === '' && fit(line) === 0) return note.lost ? note : { ...note, lost: true };
    const before = trimmedAt(trimmed, line - 1);
    const after = trimmedAt(trimmed, line + 1);
    if (line === note.line && !note.lost && before === note.before && after === note.after) return note;
    return { ...note, line, before, after, lost: false };
  });
}

/** True when two lists hold the very same note objects, in order. */
export function sameNotes(a: readonly ReviewNote[], b: readonly ReviewNote[]): boolean {
  return a.length === b.length && a.every((n, i) => n === b[i]);
}

/** The notes that would be sent: not resolved, in file order. */
export function sendableNotes(notes: readonly ReviewNote[]): ReviewNote[] {
  return notes.filter(n => !n.resolved).sort((a, b) => a.line - b.line || a.createdAt - b.createdAt);
}

/** The notes the next send would carry: not resolved, and not already in a draft. */
export function pendingNotes(notes: readonly ReviewNote[]): ReviewNote[] {
  return sendableNotes(notes).filter(n => n.sentAt === undefined);
}

/** Resolve or reopen one note. A reopened note is open work again, so it goes in the next send. */
export function setResolved(notes: readonly ReviewNote[], id: string, resolved: boolean): ReviewNote[] {
  return notes.map(n => {
    if (n.id !== id) return n;
    const { sentAt: _sent, ...rest } = n;
    return { ...rest, resolved };
  });
}

/** Note that these notes went into the chat draft (the draft is not sent). */
export function markSent(notes: readonly ReviewNote[], ids: ReadonlySet<string>, now: number): ReviewNote[] {
  return notes.map(n => (ids.has(n.id) ? { ...n, sentAt: now } : n));
}

/** Put a note that already went into a draft back into the next send. */
export function sendAgain(notes: readonly ReviewNote[], id: string): ReviewNote[] {
  return notes.map(n => {
    if (n.id !== id) return n;
    const { sentAt: _sent, ...rest } = n;
    return rest;
  });
}

const oneLine = (s: string) => s.replace(/\s+/g, ' ').trim();
const quote = (s: string) => { const t = oneLine(s); return t.length > QUOTE_CHARS ? `${t.slice(0, QUOTE_CHARS)}…` : t; };

/** One note as it reads in the message: its line and quoted words, then the note. */
function noteBlock(n: ReviewNote): string {
  const where = n.lost ? `Line ${n.line} (it may have moved or changed since)` : `Line ${n.line}`;
  const body = n.text.trim().split('\n').map(l => `  ${l}`).join('\n');
  return `${where}: "${quote(n.anchorText)}"\n${body}`;
}

export interface ReviewBatch {
  /** The message; '' when no note can go. */
  text: string;
  /** Notes in it, and notes left out because the message would pass `max`. */
  included: ReviewNote[];
  omitted: number;
}

/**
 * ONE message for every unresolved note on `path`: a header naming the file, a
 * block per note (the line, its quoted words, the note) and a closing sentence
 * asking for a revised version. Resolved notes never go. Past `max`
 * characters the later notes are left out and counted in `omitted`.
 */
export function composeBatch(path: string, notes: readonly ReviewNote[], max = BATCH_MAX_CHARS): ReviewBatch {
  const queue = sendableNotes(notes).filter(n => n.text.trim());
  const closing = 'Please make the changes and send back a revised version of the file.';
  const head = (count: number) => `I reviewed ${path} and left ${count === 1 ? 'one note' : `${count} notes`} on it. Each one names the line and what it said.`;
  const included: ReviewNote[] = [];
  const blocks: string[] = [];
  let used = head(queue.length).length + closing.length + 8;
  for (const note of queue) {
    const block = noteBlock(note);
    if (included.length && used + block.length + 2 > max) break;
    included.push(note);
    blocks.push(block);
    used += block.length + 2;
  }
  if (!included.length) return { text: '', included, omitted: 0 };
  const text = `${head(included.length)}\n\n${blocks.join('\n\n')}\n\n${closing}`;
  return { text: text.length > max ? `${text.slice(0, max - 1)}…` : text, included, omitted: queue.length - included.length };
}

// ---- storage -------------------------------------------------------------

interface FileNotes { path: string; updatedAt: number; notes: ReviewNote[] }
export interface ReviewNotesStore { files: FileNotes[] }

const num = (v: unknown, fallback: number) => (typeof v === 'number' && Number.isFinite(v) ? v : fallback);
const str = (v: unknown, max: number) => (typeof v === 'string' ? v.slice(0, max) : null);

function parseNote(value: unknown, path: string): ReviewNote | null {
  if (!value || typeof value !== 'object') return null;
  const o = value as Record<string, unknown>;
  const id = str(o.id, 80);
  const text = str(o.text, MAX_NOTE_CHARS);
  const line = Math.floor(num(o.line, 0));
  if (!id || !text || !text.trim() || line < 1 || line > 10_000_000) return null;
  const note: ReviewNote = {
    id, path, line, text,
    anchorText: str(o.anchorText, 400) ?? '',
    before: str(o.before, 400) ?? '',
    after: str(o.after, 400) ?? '',
    resolved: o.resolved === true,
    lost: o.lost === true,
    createdAt: num(o.createdAt, 0),
  };
  if (typeof o.sentAt === 'number' && Number.isFinite(o.sentAt)) note.sentAt = o.sentAt;
  return note;
}

/** Keep the newest files, and each file's newest notes; drop empty files. */
export function pruneStore(store: ReviewNotesStore): ReviewNotesStore {
  const files = store.files
    .map(f => ({ ...f, notes: f.notes.length > MAX_NOTES_PER_FILE ? [...f.notes].sort((a, b) => b.createdAt - a.createdAt).slice(0, MAX_NOTES_PER_FILE) : f.notes }))
    .filter(f => f.notes.length)
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .slice(0, MAX_NOTE_FILES);
  return { files };
}

/** Whatever stored text says, as a bounded store: bad JSON, a wrong shape and
 *  single bad notes are dropped rather than thrown. */
export function parseReviewNotes(raw: string | null): ReviewNotesStore {
  try {
    const value: unknown = JSON.parse(raw ?? '');
    const list = value && typeof value === 'object' ? (value as { files?: unknown }).files : null;
    if (!Array.isArray(list)) return { files: [] };
    const files: FileNotes[] = [];
    for (const entry of list) {
      if (!entry || typeof entry !== 'object') continue;
      const e = entry as Record<string, unknown>;
      const path = str(e.path, 2000);
      if (!path || !Array.isArray(e.notes) || files.some(f => f.path === path)) continue;
      const notes = e.notes.map(n => parseNote(n, path)).filter((n): n is ReviewNote => !!n);
      files.push({ path, updatedAt: num(e.updatedAt, 0), notes });
    }
    return pruneStore({ files });
  } catch { return { files: [] }; }
}

/** `path`'s notes in `store`. */
export const notesFor = (store: ReviewNotesStore, path: string): ReviewNote[] => store.files.find(f => f.path === path)?.notes ?? [];

/** `store` with `path`'s notes replaced (none removes the file), bounded. */
export function withNotes(store: ReviewNotesStore, path: string, notes: readonly ReviewNote[], now: number): ReviewNotesStore {
  const others = store.files.filter(f => f.path !== path);
  return pruneStore({ files: notes.length ? [...others, { path, updatedAt: now, notes: [...notes] }] : others });
}

type Storage = Pick<globalThis.Storage, 'getItem' | 'setItem'>;
const defaultStorage = (): Storage | null => { try { return globalThis.localStorage ?? null; } catch { return null; } };

/** `path`'s saved notes; none when storage is missing or unreadable. */
export function loadReviewNotes(path: string, storage: Storage | null = defaultStorage()): ReviewNote[] {
  try { return notesFor(parseReviewNotes(storage?.getItem(REVIEW_NOTES_KEY) ?? null), path); } catch { return []; }
}

/** Save `path`'s notes; a full or blocked store loses nothing already on screen. */
export function saveReviewNotes(path: string, notes: readonly ReviewNote[], now = Date.now(), storage: Storage | null = defaultStorage()): void {
  try {
    if (!storage) return;
    const store = parseReviewNotes(storage.getItem(REVIEW_NOTES_KEY));
    storage.setItem(REVIEW_NOTES_KEY, JSON.stringify(withNotes(store, path, notes, now)));
  } catch { /* the notes stay in memory for this session */ }
}

/** A note id: unique within this window, not a secret. */
export function newNoteId(): string {
  const uuid = (globalThis as { crypto?: { randomUUID?: () => string } }).crypto?.randomUUID?.();
  return uuid ? `rn-${uuid}` : `rn-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}
