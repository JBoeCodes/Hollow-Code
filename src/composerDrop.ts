/**
 * The chat composer's drop target (P0-07; C5, 2026-11-04). Any dropped file
 * attaches to the message: an image as a thumbnail, any other file as a
 * chip (composerFiles.ts). A folder is refused in words, and in a workspace
 * chat the notice offers Add to Sources… (an HTML5 drop carries no file
 * path, so the folder itself can't be copied in). Every drop that carries
 * files is the composer's, so the webview never runs its own default for it.
 */
import { isFolderLike } from './composerFiles';
import { carriesFiles } from './fileDrag';

export { isFolderLike, carriesFiles };

export type DropLabel = 'Drop files to attach';

/** What the composer says when a dropped item is a folder. */
export const FOLDER_REFUSED = "Folders don't attach to a message. Attach the files inside it, or add the folder in Sources.";

/** The target's label while `items` are dragged over it; null when no file
 *  is dragged (text or a link), which the composer leaves alone. */
export function dragLabel(items: ArrayLike<{ kind: string; type: string }>): DropLabel | null {
  return Array.from(items).some(i => i.kind === 'file') ? 'Drop files to attach' : null;
}


/** A drop's item as the webview gives it. `webkitGetAsEntry` (WebKit and
 *  Chromium) says whether an item the OS dropped is a folder; it answers
 *  null for a file a page made itself. */
export interface DropItemLike {
  kind: string;
  getAsFile(): File | null;
  webkitGetAsEntry?: () => { isDirectory: boolean; name?: string } | null;
}

/** A drop's files and folders. Where the entry API answers it decides, so
 *  an empty file with no extension is still a file (`known`); without it a
 *  folder is guessed as no type and no bytes. Read while the event is live:
 *  the DataTransfer empties once the handler returns. */
export function splitDrop(dt: { files?: ArrayLike<File>; items?: ArrayLike<DropItemLike> } | null | undefined): { files: { file: File; known: boolean }[]; folders: string[] } {
  const files: { file: File; known: boolean }[] = [];
  const folders: string[] = [];
  const items = Array.from(dt?.items ?? []).filter(i => i.kind === 'file');
  if (items.length > 0) {
    for (const item of items) {
      const file = item.getAsFile();
      const entry = item.webkitGetAsEntry?.() ?? null;
      if (entry ? entry.isDirectory : !!file && isFolderLike(file)) folders.push(file?.name || entry?.name || 'A folder');
      else if (file) files.push({ file, known: !!entry });
    }
    return { files, folders };
  }
  for (const file of Array.from(dt?.files ?? [])) {
    if (isFolderLike(file)) folders.push(file.name);
    else files.push({ file, known: false });
  }
  return { files, folders };
}

export interface DropHandlers {
  /** Called for each dropped file that isn't a folder; `known` when the
   *  entry API said it is a file. */
  attach: (file: File, known: boolean) => void;
  /** Called once with the names of the folders. */
  refuse: (names: string[]) => void;
}

/** Take a drop on the composer: cancel its default, attach every file and
 *  refuse the folders. False (and nothing done) when it carries no files. */
export function takeComposerDrop(e: { dataTransfer: { types?: ArrayLike<string>; files?: ArrayLike<File>; items?: ArrayLike<DropItemLike> } | null; preventDefault(): void }, h: DropHandlers): boolean {
  const dt = e.dataTransfer;
  if (!carriesFiles(dt)) return false;
  e.preventDefault();
  const { files, folders } = splitDrop(dt);
  for (const f of files) h.attach(f.file, f.known);
  if (folders.length > 0) h.refuse(folders);
  return true;
}
