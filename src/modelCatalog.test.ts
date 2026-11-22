import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('./api', () => ({ api: { agentModels: vi.fn() } }));
import { api } from './api';
import { chatOnlyModel, checkedLabel, forgetCatalog, loadCatalog, newerModel, withoutChatOnly } from './modelCatalog';

const newest = { newestByFamily: { sonnet: 'claude-sonnet-5-5', haiku: 'claude-haiku-5-5' } };
beforeEach(() => {
  vi.mocked(api.agentModels).mockReset();
  for (const agent of ['claude', 'codex', 'grok']) forgetCatalog(agent);
});

describe('model catalog', () => {
  it('keeps a refreshed request shared when an older request rejects', async () => {
    forgetCatalog('codex');
    const call = vi.mocked(api.agentModels);
    call.mockClear();
    call.mockResolvedValue({ models: [], source: 'unexpected third request', warning: null, checkedAt: null, newestByFamily: {} });
    let rejectOld!: (error: Error) => void;
    let resolveNew!: (value: Awaited<ReturnType<typeof loadCatalog>>) => void;
    call.mockImplementationOnce(() => new Promise((_, reject) => { rejectOld = reject; }));
    call.mockImplementationOnce(() => new Promise(resolve => { resolveNew = resolve; }));
    const old = loadCatalog('codex');
    const refreshed = loadCatalog('codex', true);
    rejectOld(new Error('old catalog failed'));
    expect((await old).warning).toContain('old catalog failed');
    expect(loadCatalog('codex')).toBe(refreshed);
    resolveNew({ models: [], source: 'fresh', warning: null, checkedAt: null, newestByFamily: {} });
    expect((await refreshed).source).toBe('fresh');
    expect(call).toHaveBeenCalledTimes(2);
    call.mockClear();
  });

  it('flags a pinned Claude model whose family has a newer one', () => {
    expect(newerModel('claude-sonnet-5', newest)).toBe('claude-sonnet-5-5');
    expect(newerModel('claude-haiku-4-5-20251001', newest)).toBe('claude-haiku-5-5');
    expect(newerModel('claude-sonnet-5-5', newest)).toBeNull();
    expect(newerModel('claude-sonnet-5-5[1m]', newest)).toBeNull();
    // Aliases already follow the newest; other agents' IDs have no family.
    expect(newerModel('sonnet', newest)).toBeNull();
    expect(newerModel('gpt-6-sol', newest)).toBeNull();
    expect(newerModel('claude-opus-4-8', newest)).toBeNull();
    expect(newerModel('claude-sonnet-5', null)).toBeNull();
    // Never a downgrade, and a dated snapshot of the newest isn't older.
    expect(newerModel('claude-haiku-6', newest)).toBeNull();
    expect(newerModel('claude-haiku-5-5-20261007', newest)).toBeNull();
  });

  it('says when the list was checked', () => {
    const now = new Date('2026-10-13T16:00:00');
    expect(checkedLabel('2026-10-13T12:40:00', now)).toMatch(/^checked \d/);
    expect(checkedLabel('2026-10-12T12:40:00', now)).toMatch(/^checked [A-Z][a-z]{2} 12$/);
    expect(checkedLabel(null, now)).toBe('');
    expect(checkedLabel('nonsense', now)).toBe('');
  });

  it('shares one request per agent until Refresh asks again', async () => {
    const call = vi.mocked(api.agentModels);
    call.mockResolvedValue({ models: [], source: 's', warning: null, checkedAt: null, newestByFamily: {} });
    await loadCatalog('grok');
    await loadCatalog('grok');
    expect(call).toHaveBeenCalledTimes(1);
    await loadCatalog('grok', true);
    expect(call).toHaveBeenCalledTimes(2);
    expect(call).toHaveBeenLastCalledWith('grok', true);
  });

  it('keeps self-hosted models to the chat menu (self-hosted models Q5)', () => {
    const models = [{ id: 'opus', label: 'Opus 5.5' }, { id: 'self-hosted:qwen', label: 'Qwen3.8 27B · self-hosted' }];
    expect(chatOnlyModel('self-hosted:qwen')).toBe(true);
    expect(chatOnlyModel(' self-hosted:qwen')).toBe(true);
    expect(chatOnlyModel('opus')).toBe(false);
    expect(chatOnlyModel('qwen3.8-27b')).toBe(false);
    expect(withoutChatOnly(models)).toEqual([{ id: 'opus', label: 'Opus 5.5' }]);
  });
});
