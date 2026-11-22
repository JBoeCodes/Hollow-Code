// Settings export/import (overnight F7): pick a file, export or preview and
// confirm, say what happened. Settings, Advanced calls these.
import { open, save } from '@tauri-apps/plugin-dialog';
import { api } from '../api';
import { isBrowserMockMode } from '../mockMode';
import { appliedText, exportFileName, exportedText, nothingToImportText, previewBody } from '../settingsTransfer';
import { errMsg } from '../utils';
import { flashFooter, showConfirm, trackTask } from './runtime';

const FILTERS = [{ name: 'Hollow settings', extensions: ['json'] }];

/** Null when the person cancels. In mock mode a fixed path stands in. */
async function pickTarget(): Promise<string | null> {
  if (isBrowserMockMode()) return `/mock/exports/${exportFileName(new Date())}`;
  const picked = await save({ title: 'Export settings', defaultPath: exportFileName(new Date()), filters: FILTERS });
  return typeof picked === 'string' ? picked : null;
}

async function pickSource(): Promise<string | null> {
  if (isBrowserMockMode()) return '/mock/exports/hollow-settings.json';
  const picked = await open({ title: 'Import settings', multiple: false, directory: false, filters: FILTERS });
  return typeof picked === 'string' ? picked : null;
}

export async function exportSettings(): Promise<void> {
  const path = await pickTarget();
  if (!path) return;
  try {
    const r = await trackTask('Export settings', () => api.settingsExport(path), { detail: path });
    flashFooter(exportedText(r), 8000);
  } catch (e) {
    flashFooter(`× Couldn’t export settings: ${errMsg(e)}`, 7000);
  }
}

/** Preview, then apply only what the person confirmed (the hash binds it). */
export async function importSettings(): Promise<void> {
  const path = await pickSource();
  if (!path) return;
  try {
    const preview = await trackTask('Read settings file', () => api.settingsImportPreview(path), { detail: path });
    if (!preview.changes.length) {
      flashFooter(nothingToImportText(preview), 6000);
      return;
    }
    const ok = await showConfirm({
      title: 'Import settings?',
      body: previewBody(preview),
      confirmLabel: `Apply ${preview.changes.length} change${preview.changes.length === 1 ? '' : 's'}`,
      focusCancel: true,
    });
    if (!ok) return;
    const applied = await trackTask('Import settings', () => api.settingsImportApply(path, preview.hash), { detail: path });
    flashFooter(appliedText(applied), 8000);
  } catch (e) {
    flashFooter(`× Couldn’t import settings: ${errMsg(e)}`, 7000);
  }
}
