import { describe, expect, it } from 'vitest';
import {
  QUIET_AFTER_MS, activityFor, activityLine, applyActivityEvent, canStopRunning, liveRunIds,
  nextChatActivity, progressLabel, pruneActivity, quietLabel, type ActivityMap,
} from './runActivity';
import type { RunQueueItem } from './types';

const NOW = Date.parse('2026-09-26T12:00:00.000Z');
const ago = (ms: number) => new Date(NOW - ms).toISOString();

function row(over: Partial<RunQueueItem>): RunQueueItem {
  return {
    id: 1, source: 'cron', sourceId: 'w', label: 'W', detail: null, reason: 'scheduled',
    status: 'running', priority: 3, position: null, attempt: 0, coalescedCount: 0,
    requestedAt: ago(60_000), claimedAt: ago(50_000), finishedAt: null, historyRunId: 'r1',
    error: null, waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false,
    ...over,
  } as RunQueueItem;
}

describe('silent-run indicator (Paperclip PR 15b)', () => {
  it('says quiet only after two minutes, in minutes then hours', () => {
    expect(quietLabel(null, NOW)).toBeNull();
    expect(quietLabel('not a time', NOW)).toBeNull();
    expect(quietLabel(ago(QUIET_AFTER_MS - 1), NOW)).toBeNull();
    expect(quietLabel(ago(6 * 60_000 + 30_000), NOW)).toBe('quiet 6m');
    expect(quietLabel(ago(65 * 60_000), NOW)).toBe('quiet 1h 5m');
  });

  it('labels all four progress phases and nothing else', () => {
    expect(progressLabel('researching')).toBe('Researching');
    expect(progressLabel('usingTools')).toBe('Using tools');
    expect(progressLabel('workingWithFiles')).toBe('Working with files');
    expect(progressLabel('coordinating')).toBe('Coordinating');
    expect(progressLabel(null)).toBeNull();
    expect(progressLabel('somethingNew')).toBeNull();
  });

  it('upserts an unknown run on activity and drops it on its done', () => {
    let map: ActivityMap = new Map();
    map = applyActivityEvent(map, { type: 'activity', runId: 'r1', lastEventAt: ago(1000), progress: 'researching' });
    expect(activityFor(map, 'r1')).toEqual({ lastEventAt: ago(1000), progress: 'researching' });
    map = applyActivityEvent(map, { type: 'activity', runId: 'r1', lastEventAt: ago(10), progress: null });
    expect(activityFor(map, 'r1')?.progress).toBeNull();
    const same = applyActivityEvent(map, { type: 'toolUse' });
    expect(same).toBe(map);
    expect(applyActivityEvent(map, { type: 'done', runId: 'other' })).toBe(map);
    map = applyActivityEvent(map, { type: 'done', runId: 'r1' });
    expect(activityFor(map, 'r1')).toBeNull();
    expect(activityFor(map, null)).toBeNull();
  });

  it('prunes runs with no live queue row and matches rows by historyRunId', () => {
    let map: ActivityMap = new Map([
      ['r1', { lastEventAt: ago(1000), progress: null }],
      ['gone', { lastEventAt: ago(1000), progress: null }],
    ]);
    const items = [row({}), row({ id: 2, status: 'queued', historyRunId: null }), row({ id: 3, status: 'done', historyRunId: 'gone' })];
    expect(liveRunIds(items)).toEqual(['r1']);
    map = pruneActivity(map, liveRunIds(items));
    expect([...map.keys()]).toEqual(['r1']);
    expect(pruneActivity(map, ['r1'])).toBe(map);
    expect(activityFor(map, items[0].historyRunId)).not.toBeNull();
  });

  it('keeps only the chat turn in flight', () => {
    const evt = { type: 'activity', runId: 'run-a', lastEventAt: ago(7 * 60_000), progress: 'usingTools' as const };
    expect(nextChatActivity(null, null, evt)).toBeNull();
    expect(nextChatActivity(null, 'run-b', evt)).toBeNull();
    const current = nextChatActivity(null, 'run-a', evt);
    expect(current).toEqual({ runId: 'run-a', lastEventAt: ago(7 * 60_000), progress: 'usingTools' });
    expect(nextChatActivity(current, 'run-a', { type: 'done', runId: 'run-a' })).toBe(current);
  });

  it('offers Stop on running workflow and chat rows, never a heartbeat', () => {
    expect(canStopRunning(row({}))).toBe(true);
    expect(canStopRunning(row({ source: 'chat' }))).toBe(true);
    expect(canStopRunning(row({ source: 'heartbeat' }))).toBe(false);
    expect(canStopRunning(row({ status: 'queued' }))).toBe(false);
    expect(activityLine({ lastEventAt: ago(7 * 60_000), progress: 'coordinating' }, NOW)).toBe('Coordinating · quiet 7m');
    expect(activityLine({ lastEventAt: ago(7 * 60_000), progress: null }, NOW)).toBe('Running · quiet 7m');
    expect(activityLine({ lastEventAt: ago(1000), progress: 'researching' }, NOW)).toBe('Researching');
    expect(activityLine({ lastEventAt: ago(1000), progress: null }, NOW)).toBeNull();
    expect(activityLine(null, NOW)).toBeNull();
  });
});
