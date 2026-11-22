/**
 * A drag from Finder or the desktop, as the window and the message box read
 * it (2026-11-04). No imports: the window's drop listener lives in the launch
 * chunk, and everything else about a drop loads only when one lands.
 */

/** Whether a drag or drop carries files from outside the page. `types` holds
 *  "Files" in every engine, during the drag too; WebKit, the installed app's
 *  engine, may leave `items` empty until the drop (Chromium alone had been
 *  tested), so `items` and `files` only back it up. A conversation, task or
 *  link drag carries none. */
export function carriesFiles(dt: { types?: ArrayLike<string>; items?: ArrayLike<{ kind: string }>; files?: ArrayLike<unknown> } | null | undefined): boolean {
  if (!dt) return false;
  return Array.from(dt.types ?? []).includes('Files')
    || Array.from(dt.items ?? []).some(i => i.kind === 'file')
    || (dt.files?.length ?? 0) > 0;
}

/** One dropped item, read while the drop event lives (its DataTransfer
 *  empties once the handler returns): the file, and whether the entry API
 *  says it is a folder (null when it can't say). */
export interface DroppedItem { file: File | null; folder: boolean | null; name: string }

export function readDrop(dt: DataTransfer | null): DroppedItem[] {
  if (!dt) return [];
  const items = Array.from(dt.items ?? []).filter(i => i.kind === 'file');
  if (items.length > 0) return items.map(i => {
    const file = i.getAsFile();
    const entry = (i as DataTransferItem & { webkitGetAsEntry?: () => { isDirectory: boolean; name: string } | null }).webkitGetAsEntry?.() ?? null;
    return { file, folder: entry ? entry.isDirectory : null, name: entry?.name || file?.name || '' };
  });
  return Array.from(dt.files ?? []).map(file => ({ file, folder: null, name: file.name }));
}
