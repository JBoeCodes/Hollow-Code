import { activePage, attachChatImage, chatAgentSessionKey, chatComposerOnScreen, chatConversationVersion, flashFooter } from './store';
import { openSessionByKey } from './store/workspaces';
import { appSessionRequest } from './store/sessionRequest';
import { carriesFiles, readDrop, type DroppedItem } from './fileDrag';
import type { RailDropSnapshot } from './railFileDrop';

const snapshot = (): RailDropSnapshot => ({ key: chatAgentSessionKey(), version: chatConversationVersion(), page: activePage() });

/** Capture both the row identity and all DataTransfer bytes before imports.
 * Prevent default but let the window listener observe it and clear its veil. */
export function railRowFileTarget(key: () => string | null, unavailable = 'Open this project conversation before dropping files onto its row.') {
  const over = (e: DragEvent) => {
    if (e.defaultPrevented || !carriesFiles(e.dataTransfer)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = key() ? 'copy' : 'none';
    if (key()) (e.currentTarget as HTMLElement).classList.add('rail-row-file-target');
  };
  return {
    onDragEnter: over, onDragOver: over,
    onDragLeave(e: DragEvent) {
      const row = e.currentTarget as HTMLElement;
      if (!row.contains(e.relatedTarget as Node | null)) row.classList.remove('rail-row-file-target');
    },
    onDrop(e: DragEvent) {
      (e.currentTarget as HTMLElement).classList.remove('rail-row-file-target');
      if (e.defaultPrevented || !carriesFiles(e.dataTransfer)) return;
      e.preventDefault();
      const target = key(), origin = snapshot(), items = readDrop(e.dataTransfer);
      if (!target) { flashFooter(unavailable, 6000); return; }
      void routeRailItems(target, items, origin);
    },
  };
}

async function routeRailItems(key: string, items: DroppedItem[], origin: RailDropSnapshot) {
  try {
    const [{ splitDrop, FOLDER_REFUSED }, { attachFilesToRailChat }] = await Promise.all([import('./composerDrop'), import('./railFileDrop')]);
    const { files, folders } = splitDrop({ items: items.map(item => ({ kind: 'file', getAsFile: () => item.file, webkitGetAsEntry: () => item.folder === null ? null : { isDirectory: item.folder, name: item.name } })) });
    if (folders.length) flashFooter(FOLDER_REFUSED, 6000);
    if (!files.length) return;
    const result = await attachFilesToRailChat(key, files, origin, {
      snapshot, open: openSessionByKey, visible: chatComposerOnScreen,
      pending: key => activePage() === 'chat' && appSessionRequest()?.key === key,
      attach: (file, known) => attachChatImage(file, false, known),
      wait: () => new Promise(resolve => setTimeout(resolve, 100)),
    });
    if (result === 'changed') flashFooter('Files were not attached because the conversation changed. Drop them again.', 6000);
    else if (result === 'unavailable') flashFooter('The conversation could not open for attachments. Open it and drop the files again.', 6000);
    // A rejected file uses the composer's existing count, size and type notice.
  } catch { flashFooter('Files could not attach to this conversation. Try again.', 6000); }
}
