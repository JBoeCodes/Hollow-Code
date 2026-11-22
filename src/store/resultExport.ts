// Export a Result as Markdown (overnight F6): pick the file, write it, say
// where it went. The Result's "More" menu calls this.
import { save } from '@tauri-apps/plugin-dialog';
import { api } from '../api';
import { isBrowserMockMode } from '../mockMode';
import { exportFileName, exportedText } from '../resultExport';
import { errMsg } from '../utils';
import { flashFooter, trackTask } from './runtime';

export async function exportResultMarkdown(workspaceId: string, path: string, title: string): Promise<void> {
  const name = exportFileName(title);
  const target = isBrowserMockMode()
    ? `/mock/exports/${name}`
    : await save({ title: 'Export as Markdown', defaultPath: name, filters: [{ name: 'Markdown', extensions: ['md'] }] });
  if (typeof target !== 'string' || !target) return;
  try {
    const r = await trackTask('Export as Markdown', () => api.resultExportMarkdown(workspaceId, path, target), { detail: target });
    flashFooter(exportedText(r.target, r.sources), 8000);
  } catch (e) {
    flashFooter(`× Couldn’t export it: ${errMsg(e)}`, 7000);
  }
}
