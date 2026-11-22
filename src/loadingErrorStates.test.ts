import { beforeEach, describe, expect, it, vi } from 'vitest';
import workspacesSource from './store/workspaces.ts?raw';
import snackbarsSource from './store/snackbars.ts?raw';
import { retryNotice } from './plainErrors';

const api = vi.hoisted(() => ({ saveDiagnostics: vi.fn() }));
const runtime = vi.hoisted(() => ({
  flashFooter: vi.fn(), flashRetry: vi.fn(),
  trackTask: vi.fn((_name: string, fn: () => Promise<unknown>) => fn()),
}));
const dialog = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock('./api', () => ({ api }));
vi.mock('./store/runtime', () => runtime);
vi.mock('./store/workspaces', () => ({ workspaceId: () => null, workspaceDetail: () => null }));
vi.mock('@tauri-apps/plugin-dialog', () => dialog);
vi.mock('./mockMode', () => ({ isBrowserMockMode: () => false }));

/** The retry function the last flashRetry call was given. */
const lastRetry = () => runtime.flashRetry.mock.calls.at(-1)![1] as () => Promise<void>;

describe('retryNotice (overnight G2)', () => {
  it('puts a raw error in plain language, keeps it under Details and offers Retry', () => {
    const retry = vi.fn();
    const n = retryNotice('× Couldn’t save diagnostics: No space left on device (os error 28)', retry);
    expect(n.message).toBe('Couldn’t save diagnostics: The disk is full. Free some space, then try again.');
    expect(n.detail).toBe('No space left on device (os error 28)');
    expect(n.actionLabel).toBe('Retry');
    expect(n.undo).toBe(retry);
    expect(n.kind).toBe('error');
    expect(n.ttlMs).toBeGreaterThanOrEqual(10000);
  });

  it('passes a message already in plain language through, with no Details', () => {
    const n = retryNotice('× Couldn’t preview the merge: the workspace is busy', () => {});
    expect(n.message).toBe('Couldn’t preview the merge: the workspace is busy');
    expect('detail' in n).toBe(false);
  });
});

describe('Retry on tonight’s flows (overnight G2)', () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it('Save diagnostics retries into the same folder without asking again', async () => {
    dialog.open.mockResolvedValue('/Users/me/Desktop');
    api.saveDiagnostics.mockRejectedValueOnce(new Error('Permission denied (os error 13)')).mockResolvedValue({ path: '/Users/me/Desktop/x.zip', bytes: 1, entries: 1 });
    const { saveDiagnosticsBundle } = await import('./store/diagnostics');
    await saveDiagnosticsBundle();
    expect(runtime.flashRetry.mock.calls[0][0]).toMatch(/^× Couldn’t save diagnostics: /);
    await lastRetry()();
    expect(dialog.open).toHaveBeenCalledTimes(1);
    expect(api.saveDiagnostics.mock.calls.map(c => c[0])).toEqual(['/Users/me/Desktop', '/Users/me/Desktop']);
    expect(runtime.flashFooter.mock.calls.at(-1)![0]).not.toContain('undefined');
  });

  it('the merge preview fails with Retry, not the Work page’s error line', () => {
    const merge = workspacesSource.slice(workspacesSource.indexOf('export async function mergeWorkspaceInto'));
    expect(merge).toContain("trackTask('Preview merge'");
    expect(merge).toMatch(/flashRetry\(`× Couldn’t preview the merge: [^\n]*\(\) => mergeWorkspaceInto\(from, into, onChanged\)\)/);
    expect(merge.match(/onChanged\?\.\(\)/g)?.length).toBe(3);
  });

  it('flashRetry is a plain-language snackbar whose button says Retry', () => {
    const fn = snackbarsSource.slice(snackbarsSource.indexOf('export function flashRetry'));
    expect(fn.slice(0, 300)).toContain('retryNotice(msg, retry)');
    expect(fn.slice(0, 300)).toContain('flashWithUndo(notice)');
    expect(snackbarsSource).toContain("(bar.actionLabel ?? 'Undo').toLowerCase()");
  });
});
