/**
 * Unified capture (next-level P12): what an "Add" input most likely is, and
 * where it would go. Pure, unit-tested. The guess is only a default: the Add
 * sheet shows the kind and destination before anything is saved, and the
 * user can switch it. Import a source (the palette, Library's ⋯, ⌘⌥⇧I and
 * the tray) opens the same sheet set to a Library source (S5, 2026-11-03).
 */
import type { IngestSource } from './types';

/** `source` is Import a source: a link, text or file read into source notes
 *  in Library. It took over the old `link` kind (S5), which did the same for
 *  a web address. `file` is a workspace snapshot. */
export type CaptureKind = 'task' | 'note' | 'source' | 'file' | 'recording';
export interface CaptureGuess { kind: CaptureKind; reason: string }

const URL_RE = /^https?:\/\/\S+$/i;
const PLAUD = /(^|\s)(plaud:|https?:\/\/(?:web\.|app\.)?plaud\.ai\/)/i;
const TASK_HINT = /\b(today|tomorrow|tonight|next (week|month|monday|tuesday|wednesday|thursday|friday)|by (mon|tue|wed|thu|fri|sat|sun)\w*|every (day|week|weekday|month)|p[1-4]|due)\b|(^|\s)[#@]\w|![1-4]/i;
const VERB = /^(call|email|send|draft|write|review|book|schedule|fix|follow up|follow-up|ask|check|confirm|update|prepare|finish|buy|pay|plan|share|read|reply|set up|move|cancel)\b/i;

export function classifyText(raw: string): CaptureGuess {
  const text = raw.trim();
  if (!text) return { kind: 'note', reason: 'Nothing typed yet' };
  if (PLAUD.test(text) && text.split(/\s+/).length <= 4) return { kind: 'recording', reason: 'A Plaud recording link or id' };
  if (URL_RE.test(text)) return { kind: 'source', reason: 'A single web address: Library keeps source notes from its page text' };
  const lines = text.split('\n').filter(l => l.trim());
  if (lines.length === 1 && text.length <= 140 && (TASK_HINT.test(text) || VERB.test(text))) return { kind: 'task', reason: TASK_HINT.test(text) ? 'A short line with a date, priority or tag' : 'A short action' };
  if (lines.length === 1 && text.length <= 80) return { kind: 'task', reason: 'A short single line' };
  return { kind: 'note', reason: lines.length > 1 ? 'Several lines of text' : 'A longer passage' };
}

/** A picked or dropped file becomes a workspace source snapshot by default
 *  (the original is never changed); Library source is the other choice.
 *  Conversation exports did too once From other apps was removed (run 15 P4). */
export function classifyFile(_name: string): CaptureGuess {
  return { kind: 'file', reason: 'A file: attached as a snapshot; the original stays where it is' };
}

export const CAPTURE_LABEL: Record<CaptureKind, string> = {
  task: 'Task', note: 'Note', source: 'Library source', file: 'Workspace file', recording: 'Recording link',
};

/** One web address and nothing else. */
export function isWebAddress(text: string): boolean { return URL_RE.test(text.trim()); }

/** What Import a source hands `ingest_run`: the picked file, else a single
 *  web address, else the text itself titled by its first line. Null when
 *  there is nothing to import. */
export function ingestSourceFor(text: string, file: string | null): IngestSource | null {
  if (file) return { kind: 'file', path: file, displayName: file.split('/').pop() || file };
  const t = text.trim();
  if (!t) return null;
  if (URL_RE.test(t)) return { kind: 'url', url: t };
  return { kind: 'text', content: t, title: t.split('\n')[0].trim().slice(0, 60) };
}

/** A note's file name from its first line (no slashes or line breaks). */
export function noteName(text: string): string {
  const first = text.trim().split('\n')[0].replace(/^#+\s*/, '').replace(/[\\/:*?"<>|]+/g, ' ').trim();
  return (first.slice(0, 80) || 'Untitled note').replace(/\s+/g, ' ');
}
