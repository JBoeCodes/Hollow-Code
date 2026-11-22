/**
 * Files attached to a chat message (C5, 2026-11-04). Any file attaches by
 * the + menu, a drop or a paste: an image keeps its thumbnail and its
 * `<attached-images>` line, and any other file is saved the same safe way
 * (`chat_save_attachment` with its name, into the chat folder's hidden
 * `.hollow-attachments/`) and reaches the agent as one line of an
 * `<attached-files>` block: its original name, then the saved copy's
 * absolute path. Pure helpers only; ChatBody does the reading and saving.
 */

import { errMsg } from './utils';
import { ATTACH_MAX_COUNT } from './turnFiles';

// The cap and the sent message's chips live in `turnFiles.ts`, which the
// launch chunk's store imports without pulling this module in (C5 bundle fix).
export { ATTACH_MAX_COUNT, attachmentFiles, turnFilesField, type TurnFile } from './turnFiles';

/** The largest file that attaches, as `save_attachment` caps it. */
export const ATTACH_MAX_BYTES = 20 * 1024 * 1024;
/** What one message carries at most (review follow-up): 10 attachments
 *  (`ATTACH_MAX_COUNT`) and 50 MB in all. */
export const ATTACH_MAX_TOTAL_BYTES = 50 * 1024 * 1024;

/** Long clipboard text uses the same capped attachment save as a picked file. */
export const LARGE_PASTE_CHARS = 12_000;
export const attachmentSavingLabel = (percent?: number) => percent == null ? 'Reading…' : percent < 100 ? `Reading ${percent}%` : 'Saving…';

/** The image types that show as a thumbnail and go to the agent as an
 *  image. Any other type, image or not (HEIC, AVIF, TIFF, SVG, PSD…),
 *  attaches as a file chip and keeps its own extension. */
const THUMBNAIL_TYPES = new Set(['image/png', 'image/jpeg', 'image/jpg', 'image/gif', 'image/webp']);
export const isImageFile = (f: { type: string }) => THUMBNAIL_TYPES.has(f.type.toLowerCase());

/** A folder in a drop or paste, when nothing better says: no type and no
 *  bytes. */
export const isFolderLike = (f: { type: string; size: number }) => f.type === '' && f.size === 0;

/** "0 B", "512 B", "12.4 KB", "3.2 MB": what a file chip shows. */
export function fileSizeLabel(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 1024) return `${Math.max(0, Math.round(bytes || 0))} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${(Math.round(kb * 10) / 10).toLocaleString('en-US')} KB`;
  return `${(Math.round((kb / 1024) * 10) / 10).toLocaleString('en-US')} MB`;
}

/** Characters a name must not carry onto the screen or into the prompt:
 *  C0 and C1 controls, DEL, the line and paragraph separators, and the
 *  bidi controls (an RLO would make "exe.pdf" read as a PDF). */
// eslint-disable-next-line no-control-regex
const UNSAFE_NAME_CHARS = /[\u0000-\u001f\u007f-\u009f\u2028\u2029\u202a-\u202e\u2066-\u2069]/g;

/** A file's name as its chip shows it: every unsafe character becomes `_`. */
export function displayFileName(name: string): string {
  return name.replace(UNSAFE_NAME_CHARS, '_').trim();
}

/** Why a dragged, pasted or picked file doesn't attach, or null when it
 *  does. A folder reads as no type and no bytes unless the caller knows it
 *  is a file (`knownFile`, a drop whose entry said so); a known file with
 *  no bytes is empty; anything over 20 MB is refused before it is read. */
export function attachRefusal(f: { name: string; type: string; size: number }, knownFile = false): string | null {
  const name = displayFileName(f.name) || 'That file';
  if (!knownFile && isFolderLike(f)) return `${name} looks like a folder. Folders don't attach to a message: attach the files inside it.`;
  if (f.size === 0) return `${name} is empty, so it doesn't attach.`;
  if (f.size > ATTACH_MAX_BYTES) return `${name} is over 20 MB, so it doesn't attach.`;
  return null;
}

/** Why one more file would take a message past its 10 attachments or
 *  50 MB in all, or null when it fits beside `current`. */
export function messageCapRefusal(current: readonly { size?: number }[], f: { name: string; size: number }): string | null {
  const name = displayFileName(f.name) || 'That file';
  if (current.length >= ATTACH_MAX_COUNT) return `A message holds up to ${ATTACH_MAX_COUNT} attachments, so ${name} wasn't added.`;
  const total = current.reduce((sum, a) => sum + (a.size ?? 0), 0);
  if (total + f.size > ATTACH_MAX_TOTAL_BYTES) return `A message holds up to 50 MB of attachments, so ${name} wasn't added.`;
  return null;
}

/** + → Add to Sources… in words (C5 review): what the footer says after
 *  `pick` (the Sources file picker), or null when it was cancelled. The
 *  pick throws by design (a file over the Sources limit, one it can't
 *  read), so the person reads why instead of nothing happening. */
export async function addToSourcesNotice(pick: () => Promise<boolean>): Promise<{ text: string; ms?: number } | null> {
  try {
    return (await pick()) ? { text: 'Added to Sources.' } : null;
  } catch (e) {
    return { text: `× ${errMsg(e)}`, ms: 5000 };
  }
}

/** A chip for `file` before its read starts (C5 review): Saving…, which
 *  blocks Send, with its size counted toward the message's 50 MB. `file`
 *  marks one that isn't a thumbnail image. */
export function pendingAttachment(id: number, file: { name: string; type: string; size: number }, asFile: boolean) {
  return {
    id,
    name: displayFileName(file.name) || (asFile ? 'pasted file' : 'pasted image'),
    mime: file.type,
    dataUrl: '',
    saving: true,
    size: file.size,
    ...(asFile ? { file: true } : {}),
  };
}

/** Whether the composer's attachments hold Send back: one still saving or
 *  one that failed (remove it to send). */
export const attachmentsHoldSend = (atts: readonly { saving: boolean; error?: boolean }[]) => atts.some(a => a.saving || !!a.error);

/** A file's name as the agent reads it: no line breaks, controls, bidi
 *  controls, slashes or angle brackets, so a name can't add a line to the
 *  block, close it, read backwards, or look like part of the path after it. */
export function promptFileName(name: string): string {
  const clean = displayFileName(name).replace(/[<>/\\]/g, '_').trim();
  return clean || 'file';
}

/** The `<attached-files>` block for the agent, or '' with no files. */
export function attachedFilesBlock(files: readonly { name: string; path?: string }[]): string {
  const saved = files.filter(f => f.path);
  if (saved.length === 0) return '';
  const n = saved.length;
  const lines = saved.map((f, i) => `${i + 1}. ${promptFileName(f.name)} at ${f.path}`);
  return `<attached-files count="${n}">\nThe user attached ${n} file${n > 1 ? 's' : ''} to this message. Each line gives the file's original name, then the absolute path of its saved copy. Open ${n > 1 ? 'them' : 'it'} with your file-reading tools:\n${lines.join('\n')}\n</attached-files>`;
}
