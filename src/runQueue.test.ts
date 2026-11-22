import { describe, expect, it } from 'vitest';
import type { RunQueueItem } from './types';
import {
  PERMISSION_REFUSED_NOTE, PERMISSION_UNAVAILABLE_NOTE, STALE_HOLDER,
  cancelLabel, dockVisible, heldText, liveItemForSlug, queueChipText, queuedCount, readsWorking, runningAutomations, sourceLabel,
  waitingText,
} from './runQueue';
import { permissionHoldText } from './workflowPermissions';

function row(overrides: Partial<RunQueueItem> = {}): RunQueueItem {
  return {
    id: 1, source: 'cron', sourceId: 'morning-brief', label: 'Morning brief', detail: null,
    reason: 'scheduled', status: 'queued', priority: 3, position: 1, attempt: 0, coalescedCount: 0,
    requestedAt: '2026-09-25T13:00:00.000Z', claimedAt: null, finishedAt: null, historyRunId: null,
    error: null, waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false, waitingReason: null,
    ...overrides,
  };
}

describe('run queue helpers', () => {
  it('keeps continued reports visible and stoppable while the paused predecessor is terminal', () => {
    const paused = row({ id: 1, status: 'paused' });
    const next = row({ id: 2, source: 'restrictedResume', status: 'queued' });
    expect(liveItemForSlug([paused, next], 'morning-brief')?.id).toBe(2);
    expect(cancelLabel(next)).toBe('Cancel');
    expect(cancelLabel({ ...next, status: 'running' })).toBe('Stop');
    expect(cancelLabel(paused)).toBeNull();
    expect(waitingText(paused)).toBe('Paused for requested work');
    expect(sourceLabel(next.source)).toBe('Continue after requested work');
    expect(queuedCount([paused, next])).toBe(1);
  });
  it('counts queued and claimed rows only', () => {
    expect(queuedCount([
      row({ id: 1, status: 'queued' }),
      row({ id: 2, status: 'claimed' }),
      row({ id: 3, status: 'running' }),
      row({ id: 4, status: 'done' }),
      row({ id: 5, status: 'cancelled' }),
    ])).toBe(2);
    expect(queuedCount([])).toBe(0);
  });

  it('a chat row nothing truly holds back reads Working, never Queued (2026-11-16)', () => {
    const chat = (over: Partial<RunQueueItem> = {}) => row({ source: 'chat', sourceId: 's-1', label: 'Chat', ...over });
    // Claimed, running and "queued but claimable" are all working.
    expect(readsWorking(chat({ status: 'claimed' }))).toBe(true);
    expect(readsWorking(chat({ status: 'running' }))).toBe(true);
    expect(readsWorking(chat({ status: 'queued', working: true }))).toBe(true);
    // A real wait, or a workflow row, is not.
    expect(readsWorking(chat({ status: 'queued', waitingReason: 'memory' }))).toBe(false);
    expect(readsWorking(row({ status: 'queued', working: true }))).toBe(false);
    expect(readsWorking(row({ status: 'claimed' }))).toBe(false);
    // None of them counts as Queued, and a lone chat never mounts the dock.
    expect(queuedCount([chat({ status: 'claimed' }), chat({ id: 2, status: 'queued', working: true })])).toBe(0);
    expect(dockVisible([chat({ status: 'claimed' }), chat({ id: 2, status: 'queued', working: true })], 0)).toBe(false);
    expect(queuedCount([chat({ status: 'queued', waitingReason: 'memory' })])).toBe(1);
    // The row's words: Preparing, with the reason only when it truly waits.
    expect(waitingText(chat({ status: 'claimed' }))).toBe('Preparing');
    expect(waitingText(chat({ status: 'queued', working: true }))).toBe('Preparing');
    expect(waitingText(chat({ status: 'queued', waitingReason: 'memory' }))).toBe('Waiting for memory');
    // A tracker sync hold is not named on the row (the backend leaves it out).
    expect(waitingText(chat({ status: 'queued', working: true, waitingHolder: null }))).toBe('Preparing');
  });

  it('mounts the dock for a running automation alone, never for a running chat turn alone', () => {
    const running = row({ id: 1, status: 'running' });
    const chat = row({ id: 2, source: 'chat', sourceId: 's-1', label: 'Chat turn', status: 'running' });
    expect(runningAutomations([running, chat, row({ id: 3 })]).map(item => item.id)).toEqual([1]);
    // Paperclip PR 15b: before the fix the dock hid here, and with it the
    // running row's quiet time and Stop.
    expect(dockVisible([running], 0)).toBe(true);
    expect(dockVisible([chat], 0)).toBe(false);
    expect(dockVisible([row({ status: 'done' })], 0)).toBe(false);
    expect(dockVisible([], 0)).toBe(false);
    expect(dockVisible([], 1)).toBe(true);
    expect(dockVisible([row({ status: 'claimed' })], 0)).toBe(true);
  });

  it('says where a row stands', () => {
    expect(waitingText(row({ position: 3 }))).toBe('Queued #3');
    expect(waitingText(row({ position: null }))).toBe('Queued');
    expect(waitingText(row({ waitingHolder: 'a chat turn', waitingSince: '2026-09-25T13:01:00.000Z' })))
      .toBe('Waiting for a chat turn');
    expect(waitingText(row({ waitingHolder: 'workflow `close-my-day`' }))).toBe('Waiting for workflow `close-my-day`');
    // Overnight A2: what else holds a queued row back, after a lease wait.
    expect(waitingText(row({ position: 2, waitingReason: 'a free subagent slot: this chat is at its limit of 6 subagents at once (Settings → Chat → Chats at once)' })))
      .toBe('Waiting for a free subagent slot: this chat is at its limit of 6 subagents at once (Settings → Chat → Chats at once)');
    expect(waitingText(row({ position: 2, waitingReason: 'the 10-chat limit' })))
      .toBe('Waiting for the 10-chat limit');
    expect(waitingText(row({ position: 2, waitingReason: "this workspace's 5-chat limit" })))
      .toBe("Waiting for this workspace's 5-chat limit");
    expect(waitingText(row({ position: 1, waitingReason: 'memory' }))).toBe('Waiting for memory');
    expect(waitingText(row({ waitingHolder: 'a chat turn', waitingReason: 'the vault (X is running)' }))).toBe('Waiting for a chat turn');
    expect(waitingText(row({ status: 'running' }))).toBe('Running');
    expect(waitingText(row({ status: 'claimed' }))).toBe('Preparing');
    expect(waitingText(row({ status: 'claimed', cancelRequested: true }))).toBe('Stopping');
    expect(waitingText(row({ cancelRequested: true }))).toBe('Skipping this occurrence');
    expect(waitingText(row({ status: 'done' }))).toBe('done');
  });

  it('adds the restart hint only behind a stale lease', () => {
    const stale = waitingText(row({ waitingHolder: STALE_HOLDER }));
    expect(stale).toContain(STALE_HOLDER);
    expect(stale).toContain('restart Hollow');
    expect(waitingText(row({ waitingHolder: 'Notion sync' }))).not.toContain('restart');
  });

  it('words a permission hold as the workflow’s own paused text, for both hold notes', () => {
    const review = permissionHoldText('review')!;
    const unavailable = permissionHoldText('unavailable')!;
    expect(heldText(PERMISSION_REFUSED_NOTE)).toBe(review);
    expect(heldText(PERMISSION_UNAVAILABLE_NOTE)).toBe(unavailable);
    expect(heldText(null)).toBeNull();
    expect(heldText('something else')).toBe('paused: something else');
    // A hold outranks a stale wait or a queue position.
    const held = row({ heldReason: PERMISSION_REFUSED_NOTE, waitingHolder: 'a chat turn', position: 1 });
    expect(waitingText(held)).toBe(review[0].toUpperCase() + review.slice(1));
    expect(waitingText(row({ heldReason: PERMISSION_UNAVAILABLE_NOTE })).toLowerCase()).toBe(unavailable.toLowerCase());
  });

  it('finds a workflow’s live row, running first, and ignores other sources and finished rows', () => {
    const items = [
      row({ id: 1, sourceId: 'brief', status: 'queued' }),
      row({ id: 2, sourceId: 'brief', status: 'running', source: 'manual' }),
      row({ id: 3, sourceId: 'brief', status: 'done' }),
      row({ id: 4, sourceId: 'other', status: 'queued' }),
      row({ id: 5, sourceId: 'brief', status: 'queued', source: 'chat' }),
    ];
    expect(liveItemForSlug(items, 'brief')?.id).toBe(2);
    expect(liveItemForSlug(items.filter(item => item.id !== 2), 'brief')?.id).toBe(1);
    expect(liveItemForSlug(items, 'missing')).toBeNull();
    expect(queueChipText(row({ position: 2 }))).toBe('Queued #2');
    expect(queueChipText(row({ waitingHolder: 'a chat turn' }))).toBe('Queued · Waiting for a chat turn');
    expect(queueChipText(row({ status: 'running' }))).toBe('Running');
  });

  it('labels sources and offers the right action', () => {
    expect(sourceLabel('cron')).toBe('Scheduled');
    expect(sourceLabel('manual')).toBe('Run now');
    expect(sourceLabel('trigger')).toBe('Triggered');
    expect(sourceLabel('heartbeat')).toBe('Heartbeat');
    expect(cancelLabel(row())).toBe('Skip');
    expect(cancelLabel(row({ source: 'manual' }))).toBe('Cancel');
    expect(cancelLabel(row({ status: 'claimed' }))).toBe('Stop');
    expect(cancelLabel(row({ status: 'running', source: 'trigger' }))).toBe('Stop');
    expect(cancelLabel(row({ status: 'running', source: 'heartbeat' }))).toBeNull();
    expect(cancelLabel(row({ cancelRequested: true }))).toBeNull();
    expect(cancelLabel(row({ status: 'done' }))).toBeNull();
  });
});
