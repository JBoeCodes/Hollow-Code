import { describe, expect, it, vi } from 'vitest';
import { LAUNCH_IPC_WINDOW_MS, judgeLaunchIpc, launchIpcReport, noteIpc } from './launchIpc';

// Overnight Y7: the launch IPC tally and its repeat budget.
describe('launch IPC budget', () => {
  it('flags only the same command with the same details past the budget, most repeated first', () => {
    const counts = new Map([
      ['list_workspaces\u0000', 3],
      ['workspace_sessions\u0000', 2],
      ['read_note\u0000{"path":"a.md"}', 1],
      ['read_note\u0000{"path":"b.md"}', 1],
      ['list_run_queue\u0000', 5],
    ]);
    expect(judgeLaunchIpc(counts)).toEqual([
      { command: 'list_run_queue', args: '', count: 5 },
      { command: 'list_workspaces', args: '', count: 3 },
    ]);
    expect(judgeLaunchIpc(counts, 5)).toEqual([]);
  });

  it('counts calls inside the window, stops after it and warns once when over', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const t0 = performance.now();
    for (let i = 0; i < 3; i++) noteIpc('list_workspaces', undefined, t0);
    noteIpc('read_note', { path: 'a.md' }, t0);
    noteIpc('read_note', { path: 'b.md' }, t0);
    const during = launchIpcReport(t0);
    expect(during.settled).toBe(false);
    expect(during.total).toBeGreaterThanOrEqual(5);
    expect(during.repeated).toContainEqual({ command: 'list_workspaces', args: '', count: 3 });
    expect(during.repeated.some(r => r.command === 'read_note')).toBe(false);
    const after = t0 + LAUNCH_IPC_WINDOW_MS + 1;
    noteIpc('list_workspaces', undefined, after);
    noteIpc('list_workspaces', undefined, after);
    expect(launchIpcReport(after).total).toBe(during.total);
    expect(launchIpcReport(after).settled).toBe(true);
    expect(warn).toHaveBeenCalledTimes(1);
    warn.mockRestore();
  });

  it('never retains credential values, including short nested values and collections', () => {
    const t0 = performance.now();
    const args = { id: 'fake-connector', config: {
      clientSecret: 'short-secret', access_token: 'short-token', password: 'short-pass',
      apiKey: 'short-key', key: 'bare-key', env: { CUSTOM: 'env-value' },
      headers: { 'X-Custom': 'header-value' }, nested: [{ refreshToken: 'rotating-value' }],
    } };
    for (let i = 0; i < 3; i++) noteIpc('test_save_connector', args, t0);
    const row = launchIpcReport(t0).repeated.find(r => r.command === 'test_save_connector');
    expect(row?.count).toBe(3);
    expect(row?.args).toContain('fake-connector');
    for (const secret of ['short-secret', 'short-token', 'short-pass', 'short-key', 'bare-key', 'env-value', 'header-value', 'rotating-value']) {
      expect(row?.args).not.toContain(secret);
    }
    expect(args.config.clientSecret).toBe('short-secret'); // tallying never alters the real IPC payload
  });

  it('keeps documented session identity keys distinct while masking nested keys', () => {
    const now = performance.now();
    for (const key of ['s-test-a', 's-test-b', 's-test-c']) noteIpc('read_agent_conversation', { key }, now);
    expect(launchIpcReport(now).repeated.some(r => r.command === 'read_agent_conversation')).toBe(false);
    for (let i = 0; i < 3; i++) noteIpc('read_agent_session', { key: 's-test-safe', extra: { key: 'nested-secret' } }, now);
    const row = launchIpcReport(now).repeated.find(r => r.command === 'read_agent_session');
    expect(row?.args).toContain('s-test-safe');
    expect(row?.args).not.toContain('nested-secret');
  });

  it('cuts long arguments but keeps calls that differ past the cut apart', () => {
    const t0 = performance.now();
    const big = (end: string) => 'x'.repeat(200_000) + end;
    for (const end of ['a', 'b', 'c']) noteIpc('chat_save_attachment', { data: big(end) }, t0);
    const report = launchIpcReport(t0);
    expect(report.repeated.some(r => r.command === 'chat_save_attachment')).toBe(false);
    for (let i = 0; i < 3; i++) noteIpc('chat_save_attachment', { data: big('z') }, t0);
    const row = launchIpcReport(t0).repeated.find(r => r.command === 'chat_save_attachment');
    expect(row?.count).toBe(3);
    expect(row!.args.length).toBeLessThan(120);
  });
});
