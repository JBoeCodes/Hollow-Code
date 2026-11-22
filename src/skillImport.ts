// Skill import (Skills page): turning a picked path or a drop into the
// `preview_skill_import` source. Rust checks everything that matters
// (src-tauri/src/skill_import.rs); the limit here only stops the page from
// reading a huge drop into memory before Rust can refuse it. Folders come
// through the picker only (see FOLDER_DROP).
import type { SkillImportSource } from './types';

/** Mirrors `skill_import::MAX_TOTAL_BYTES`. */
export const MAX_IMPORT_BYTES = 50 * 1024 * 1024;
/** File types the picker offers. */
export const SKILL_IMPORT_EXTENSIONS = ['skill', 'zip', 'md'];

const TOO_BIG = 'This skill is larger than 50 MB, the most Hollow imports.';

/** Base64 without a data: prefix, in chunks (a spread of 50 MB overflows the stack). */
export function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

/** `12 B`, `3.4 KB`, `1.2 MB`. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** Whether a drag carries files (and so may be a skill). */
export function isFileDrag(dt: DataTransfer | null | undefined): boolean {
  return !!dt && Array.from(dt.types ?? []).includes('Files');
}

/** The parts of a dropped `File` this module reads. */
export interface FileLike { name: string; size: number; arrayBuffer(): Promise<ArrayBuffer> }

/** The part of WebKit's FileSystemEntry this module reads. */
export interface EntryLike { isDirectory: boolean }

/** A dropped folder is refused: WebKit reads through links inside it, so it
 *  can't be checked the way Rust checks a folder chosen in the picker. */
export const FOLDER_DROP = 'To import a folder, use Choose folder…: a dropped folder can’t be checked for links.';

/** What a drop holds, captured while the event is live (the DataTransfer
 *  empties once the handler returns, so nothing here may await first). */
export type DropCapture = { file: FileLike } | { error: string };

export function captureDrop(dt: DataTransfer | null | undefined): DropCapture | null {
  if (!dt) return null;
  const items = Array.from(dt.items ?? []).filter(i => i.kind === 'file');
  if (items.length === 0) return null;
  if (items.length > 1) return { error: 'Drop one skill at a time: a .skill or .zip file or a SKILL.md.' };
  const item = items[0] as DataTransferItem & { webkitGetAsEntry?: () => EntryLike | null };
  if (item.webkitGetAsEntry?.()?.isDirectory) return { error: FOLDER_DROP };
  const file = item.getAsFile();
  return file ? { file } : { error: 'Couldn’t read what was dropped.' };
}

/** One file's import source. */
export async function fileSource(file: FileLike): Promise<SkillImportSource> {
  if (file.size > MAX_IMPORT_BYTES) throw new Error(TOO_BIG);
  return { kind: 'bytes', name: file.name, data: bytesToBase64(new Uint8Array(await file.arrayBuffer())) };
}


/** SD2: the legal name a refused import offers (`Import it as "x" instead`),
 *  or null when the refusal has none. Rust writes the message; this reads the
 *  one quoted name back out so the page can offer it as a button. */
export function suggestedRename(error: string | null | undefined): string | null {
  const m = /Import it as "([A-Za-z0-9_-]{1,64})" instead/.exec(error ?? '');
  return m ? m[1] : null;
}
