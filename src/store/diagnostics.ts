// "Save diagnostics…" (overnight C6): pick a folder, write the redacted zip,
// say where it went. Shared by Settings, Advanced and the command palette.
import { open } from '@tauri-apps/plugin-dialog';
import { api } from '../api';
import { isBrowserMockMode } from '../mockMode';
import { savedDiagnosticsText } from '../diagnosticsBundle';
import { flashFooter, flashRetry, trackTask } from './runtime';

/** Null when the person cancels. In mock mode, localStorage
 *  `mockDiagnosticsDir` (default `/mock/exports`) stands in for the picker. */
async function chooseFolder(): Promise<string | null> {
  if (isBrowserMockMode()) return localStorage.getItem('mockDiagnosticsDir') ?? '/mock/exports';
  const picked = await open({ directory: true, multiple: false, title: 'Save diagnostics to…' });
  return typeof picked === 'string' ? picked : null;
}

export async function saveDiagnosticsBundle(): Promise<void> {
  const dir = await chooseFolder();
  if (dir) await saveTo(dir);
}

/** Retry (overnight G2) writes to the same folder without asking again. */
async function saveTo(dir: string): Promise<void> {
  try {
    const saved = await trackTask('Save diagnostics', () => api.saveDiagnostics(dir), { detail: dir });
    flashFooter(savedDiagnosticsText(saved), 8000);
  } catch (e: any) {
    flashRetry(`× Couldn’t save diagnostics: ${e?.message || e}`, () => saveTo(dir));
  }
}
