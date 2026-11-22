/**
 * The small part of attaching files (C5, 2026-11-04) that the store and the
 * shell need: a message's attachment cap and a sent message's file chips.
 * Kept apart from `composerFiles.ts` (C5 bundle fix): `store/chatRuntime.ts`
 * is in the launch chunk, and importing from `composerFiles.ts` there put
 * every composer helper into it (the App chunk went past its 340 KB
 * budget). `composerFiles.ts` re-exports these, so the composer imports one
 * module. Pure.
 */

/** What one message carries at most (review follow-up): 10 attachments (and
 *  50 MB in all, `composerFiles.ts` `ATTACH_MAX_TOTAL_BYTES`). */
export const ATTACH_MAX_COUNT = 10;

/** A file attached to a sent message, as its bubble shows it. */
export interface TurnFile { name: string; path: string; size?: number }

/** The files among a message's attachments, for its bubble's chips. */
export function attachmentFiles(atts: readonly { file?: boolean; name: string; path?: string; size?: number }[]): TurnFile[] {
  return atts.filter(a => a.file && a.path).map(a => ({ name: a.name, path: a.path!, ...(a.size != null ? { size: a.size } : {}) }));
}

/** `{ files }` for a user turn that carries any, else nothing, so a turn
 *  without files keeps its old shape. */
export function turnFilesField(atts: readonly { file?: boolean; name: string; path?: string; size?: number }[]): { files?: TurnFile[] } {
  const files = attachmentFiles(atts);
  return files.length ? { files } : {};
}

// A saved user turn's `<attached-files>` block (B1a, ENG-104), exactly as
// `composerFiles.ts` `attachedFilesBlock` writes it, at the very end of the
// text only: words typed to look like one stay words.
const SAVED_FILES = /(?:^|\n\n)<attached-files count="(\d+)">\nThe user attached \d+ files? to this message\. Each line gives the file's original name, then the absolute path of its saved copy\. Open (?:them|it) with your file-reading tools:\n((?:\d+\. [^\n]*\n)+)<\/attached-files>\s*$/;
// One line: a cleaned name (`promptFileName`: no slashes) and a saved copy
// in a chat folder's `.hollow-attachments/`, never a path that climbs out.
const SAVED_LINE = /^\d+\. ([^/\n]+) at (\/(?:[^/\n]+\/)*\.hollow-attachments\/[^/\n]+)$/;

/** A saved user turn's words without its trailing `<attached-files>` block,
 *  and that block's files as chips (size unknown), so a chat reloaded from
 *  the store shows the chips it was sent with. Text without a block, or
 *  with one whose count doesn't match its lines, comes back unchanged. */
export function savedTurnFiles(text: string): { text: string; files?: TurnFile[] } {
  const m = SAVED_FILES.exec(text);
  const lines = m ? m[2].trimEnd().split('\n') : [];
  if (!m || lines.length !== Number(m[1])) return { text };
  const files = lines.flatMap(line => {
    const hit = SAVED_LINE.exec(line);
    return hit && !hit[2].split('/').includes('..') ? [{ name: hit[1], path: hit[2] }] : [];
  });
  return { text: text.slice(0, m.index).trimEnd(), ...(files.length ? { files } : {}) };
}
