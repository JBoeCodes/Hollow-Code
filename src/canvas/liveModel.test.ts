import { describe, expect, it } from 'vitest';
import { decodePins, pinKey, refreshLoop, sharedReads, taskValues, usageValues } from './liveModel';

describe('live canvas data and pins', () => {
  const pin = { vault: '/vault', workspaceId: 'test', sessionKey: null, id: 'status', revision: 1, sha256: 'a'.repeat(64), title: 'Status', destination: 'home' as const };
  it('keeps valid exact-revision pins and refuses malformed or duplicate identities', () => {
    expect(decodePins(JSON.stringify([pin, pin, { ...pin, revision: -1 }, { ...pin, sha256: '' }]))).toEqual([pin]);
    expect(decodePins('bad')).toEqual([]);
    expect(pinKey(pin)).not.toBe(pinKey({ ...pin, vault: '/other' }));
    expect(pinKey(pin)).not.toBe(pinKey({ ...pin, destination: 'workspace' }));
  });
  it('counts exact project matches and completed buckets, never infers matching names', () => {
    const rows = [{ projectUrl: 'vault:projects/a.md', bucket: 'done' }, { projectUrl: 'vault:projects/a.md', bucket: 'today' }, { projectUrl: 'vault:projects/b.md', bucket: 'overdue' }];
    expect(taskValues(rows, 'vault:projects/a.md')).toEqual({ 'tasks.open': 1, 'tasks.done': 1, 'tasks.total': 2 });
    expect(taskValues(rows, '')['tasks.open']).toBe(2);
  });
  it('does not turn missing or incomplete cost into zero or a complete total', () => {
    expect(usageValues({ reportedCostUsd: null, runs: 0, unknown: 0, partial: 0, inProgress: 0 })['usage.reportedUsd']).toBe(null);
    expect(usageValues({ reportedCostUsd: 1.2345, runs: 2, unknown: 1, partial: 0, inProgress: 0 })['usage.reportedUsd']).toBe(null);
    expect(usageValues({ reportedCostUsd: 1.2345, runs: 2, unknown: 0, partial: 0, inProgress: 0 })['usage.reportedUsd']).toBe(1.2345);
  });
  it('shares only concurrent reads with the same vault and resource key', async () => {
    const read = sharedReads(); let calls = 0;
    const action = async () => ++calls;
    const a = read('/vault/tasks', action), b = read('/vault/tasks', action);
    expect(a).toBe(b); expect(await a).toBe(1);
    expect(await read('/vault/tasks', action)).toBe(2);
    expect(await read('/other/tasks', action)).toBe(3);
  });
});

describe('visible refresh lifecycle', () => {
  it('coalesces events during a read and drops late results after disposal', async () => {
    const resolves: Array<(n: number) => void> = [], values: number[] = [];
    const loop = refreshLoop(() => new Promise<number>(r => resolves.push(r)), n => values.push(n), () => {});
    loop.request(); expect(resolves).toHaveLength(0);
    loop.visible(true); loop.request(); loop.request(); expect(resolves).toHaveLength(1);
    resolves[0](1); await Promise.resolve(); await Promise.resolve();
    expect(resolves).toHaveLength(2); expect(values).toEqual([1]);
    loop.dispose(); resolves[1](2); await Promise.resolve(); await Promise.resolve();
    expect(values).toEqual([1]);
  });
  it('does no reads while hidden and refreshes when visible again', async () => {
    let reads = 0; const values: number[] = [];
    const loop = refreshLoop(async () => ++reads, n => values.push(n), () => {});
    loop.visible(true); await Promise.resolve(); await Promise.resolve();
    loop.visible(false); loop.request(); loop.request(); expect(reads).toBe(1);
    loop.visible(true); await Promise.resolve(); await Promise.resolve();
    expect(values).toEqual([1, 2]); loop.dispose();
  });
  it('reports failures without erasing the last success and permits retry', async () => {
    let fail = false; const values: number[] = [], errors: unknown[] = [];
    const loop = refreshLoop(async () => { if (fail) throw Error('read'); return 5; }, n => values.push(n), e => errors.push(e));
    loop.visible(true); await Promise.resolve(); await Promise.resolve();
    fail = true; loop.request(); await Promise.resolve(); await Promise.resolve();
    expect(values).toEqual([5]); expect(errors).toHaveLength(1);
    fail = false; loop.request(); await Promise.resolve(); await Promise.resolve();
    expect(values).toEqual([5, 5]); loop.dispose();
  });
});
