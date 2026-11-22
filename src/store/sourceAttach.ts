import { open } from '@tauri-apps/plugin-dialog';
import { api } from '../api';
import { refreshWorkspace, workspaceId } from './workspaces';

/** Pick a local file and attach it to workspace `id` as a source snapshot:
 *  the workspace's "Choose local file" and the chat composer's "Add to
 *  Sources…" (P0-07, a dropped file carries no path). `busy` is on from the
 *  pick until the attach settles. True when it was attached and `id` is
 *  still the open workspace; false when the picker was cancelled or the
 *  person moved on. Errors are thrown. */
export async function chooseAndAttachSource(id: string, busy?: (on: boolean) => void): Promise<boolean> {
  const path = await open({ multiple: false, directory: false, title: 'Attach a source snapshot' });
  if (typeof path !== 'string') return false;
  busy?.(true);
  try {
    await api.attachWorkspaceSource(id, path);
    if (workspaceId() !== id) return false;
    await refreshWorkspace();
    return workspaceId() === id;
  } finally { busy?.(false); }
}
