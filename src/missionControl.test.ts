import { describe, expect, it } from 'vitest';
import { FEED_KEYS, FEED_LINES, LINE_CHARS, applyFeedEvent, canResume, elapsed, hashAngle, missionCards, money, orbitLayout, shownLines, stopCard, textLines, toolLine, workflowFeedEvent, workflowFeedKey, type ChatPlace, type Feed } from './missionControl';
import type { RunQueueItem } from './types';
import { applyActivityEvent, type ActivityMap } from './runActivity';
import { NO_ROW_NOTICE, rowStopNote } from './runQueue';
import MISSION from './components/MissionControl.tsx?raw';

const NOW = Date.parse('2026-10-11T03:00:00Z');
const at = (min: number) => new Date(NOW - min * 60_000).toISOString();

function row(id: number, over: Partial<RunQueueItem>): RunQueueItem {
  return {
    id, source: 'chat', sourceId: null, label: `row ${id}`, detail: null, reason: 'chat', status: 'queued', priority: 0, position: null,
    attempt: 1, coalescedCount: 0, requestedAt: at(10), claimedAt: null, finishedAt: null, historyRunId: null, error: null,
    waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false, waitingReason: null, ...over,
  };
}

const places: Record<string, ChatPlace> = {
  'k-a': { workspaceId: 'w-1', workspaceTitle: 'Results view', title: 'Accept flow', agent: 'claude' },
  'k-b': { workspaceId: 'w-2', workspaceTitle: 'Q3 review', title: 'Labour section', agent: 'codex' },
};
const place = (key: string) => places[key] ?? null;

describe('applyFeedEvent', () => {
  it('ignores an event with no conversation key', () => {
    const feed: Feed = {};
    expect(applyFeedEvent(feed, { type: 'assistantDelta', text: 'hi' }, NOW)).toBe(feed);
  });

  it('starts fresh on runStarted, streams deltas, and keeps the last lines', () => {
    let feed: Feed = applyFeedEvent({}, { type: 'runStarted', conversationKey: 'k-a', asked: ' do it ' }, NOW);
    expect(feed['k-a']).toMatchObject({ running: true, startedAt: NOW, asked: 'do it', lines: [] });
    feed = applyFeedEvent(feed, { type: 'assistantDelta', conversationKey: 'k-a', text: 'one\ntwo\nthr' }, NOW);
    feed = applyFeedEvent(feed, { type: 'assistantDelta', conversationKey: 'k-a', text: 'ee\nfour' }, NOW);
    expect(shownLines(feed['k-a'])).toEqual(['two', 'three', 'four'].slice(-FEED_LINES));
    feed = applyFeedEvent(feed, { type: 'turnBoundary', conversationKey: 'k-a' }, NOW);
    expect(feed['k-a'].partial).toBe('');
    expect(shownLines(feed['k-a'])).toEqual(['two', 'three', 'four']);
  });

  it('shows the tool in use until its result, and the cost and model at the end', () => {
    let feed = applyFeedEvent({}, { type: 'toolUse', conversationKey: 'k-a', name: 'Read', input: { file_path: '/repo/src/Rail.tsx' } }, NOW);
    expect(toolLine(feed['k-a'].tool)).toContain('Rail.tsx');
    feed = applyFeedEvent(feed, { type: 'toolResult', conversationKey: 'k-a' }, NOW);
    expect(feed['k-a'].tool).toBeNull();
    feed = applyFeedEvent(feed, { type: 'usage', sessionKey: 'k-a', usage: { costUsd: 0.42, model: 'claude-opus-5-5' } }, NOW);
    feed = applyFeedEvent(feed, { type: 'done', conversationKey: 'k-a' }, NOW + 5);
    expect(feed['k-a']).toMatchObject({ running: false, costUsd: 0.42, model: 'claude-opus-5-5', finishedAt: NOW + 5 });
  });

  it('remembers at most FEED_KEYS conversations, forgetting the least recently updated', () => {
    let feed: Feed = {};
    for (let i = 0; i <= FEED_KEYS; i++) feed = applyFeedEvent(feed, { type: 'runStarted', conversationKey: `k${i}` }, NOW + i);
    expect(Object.keys(feed)).toHaveLength(FEED_KEYS);
    expect(feed.k0).toBeUndefined();
    expect(feed[`k${FEED_KEYS}`]).toBeDefined();
  });

  it('keeps only a compact copy of a tool input (never a whole file)', () => {
    const feed = applyFeedEvent({}, { type: 'toolUse', conversationKey: 'k-a', name: 'Write', input: { file_path: '/a.md', content: 'x'.repeat(10_000), n: 3 } }, NOW);
    expect(feed['k-a'].tool?.input).toEqual({ file_path: '/a.md', content: 'x'.repeat(300) });
  });

  it('cuts long lines', () => {
    expect(textLines('x'.repeat(400))[0]).toHaveLength(LINE_CHARS);
  });
});

describe('missionCards', () => {
  const queue = [
    row(3, { source: 'chat', sourceId: 'k-b', status: 'queued', position: 1, waitingReason: 'a free slot' }),
    row(1, { source: 'chat', sourceId: 'k-a', status: 'running', claimedAt: at(6) }),
    row(2, { source: 'cron', sourceId: 'morning', label: 'Morning brief', status: 'running', claimedAt: at(1) }),
    row(4, { source: 'chat', sourceId: 'k-b', status: 'done', claimedAt: at(70), finishedAt: at(50) }),
    row(5, { source: 'cron', sourceId: 'old', status: 'failed', claimedAt: at(60 * 30), finishedAt: at(60 * 29) }),
  ];
  const feed: Feed = applyFeedEvent(applyFeedEvent({}, { type: 'runStarted', conversationKey: 'k-a' }, NOW), { type: 'assistantText', conversationKey: 'k-a', text: 'Working on it' }, NOW);

  it('puts running turns first, then queued, each where it lives', () => {
    const { live } = missionCards(queue, feed, place, NOW);
    expect(live.map(c => [c.queueId, c.status, c.system])).toEqual([[1, 'running', 'w-1'], [2, 'running', 'automations'], [3, 'queued', 'w-2']]);
    expect(live[0]).toMatchObject({ title: 'Accept flow', agent: 'claude', lines: ['Working on it'], startedAt: NOW - 6 * 60_000, costUsd: null });
    expect(live[2].waiting).toMatch(/free slot/i);
  });

  it('a claimed chat, or a queued one nothing holds back, is Preparing, not Queued (2026-11-16)', () => {
    const rows = [
      row(1, { source: 'chat', sourceId: 'k-a', status: 'claimed', claimedAt: at(0) }),
      row(2, { source: 'chat', sourceId: 'k-b', status: 'queued', working: true }),
      row(3, { source: 'chat', sourceId: 'k-c', status: 'queued', waitingReason: 'memory' }),
    ];
    const { live } = missionCards(rows, {}, place, NOW);
    expect(live.map(c => [c.queueId, c.status, c.waiting])).toEqual([
      [1, 'starting', 'Preparing'], [2, 'starting', 'Preparing'], [3, 'queued', 'Waiting for memory'],
    ]);
  });

  it('lists only today’s finished turns, newest first', () => {
    const { finished } = missionCards(queue, feed, place, NOW);
    expect(finished.map(c => c.queueId)).toEqual([4]);
    expect(finished[0].status).toBe('done');
  });

  it('shows a running conversation the queue no longer lists', () => {
    const only = applyFeedEvent({}, { type: 'runStarted', conversationKey: 'k-z', asked: 'Summarise' }, NOW);
    const { live } = missionCards([], only, place, NOW);
    expect(live).toHaveLength(1);
    expect(live[0]).toMatchObject({ key: 'k-z', status: 'running', system: 'chats', title: 'Summarise' });
  });
});

describe('paused chats (P1a PR 2)', () => {
  const stopped = (id: number, key: string) => row(id, { source: 'chat', sourceId: key, status: 'cancelled', claimedAt: at(30), finishedAt: at(20 - id) });

  it('a paused chat’s newest cancelled row reads paused with Resume; an older stop and other chats stay stopped', () => {
    const queue = [stopped(1, 'k-a'), stopped(2, 'k-a'), stopped(3, 'k-b')];
    const { finished } = missionCards(queue, {}, place, NOW, new Map(), new Set(['k-a']));
    const by = new Map(finished.map(c => [c.queueId, c]));
    expect(by.get(2)?.status).toBe('paused');
    expect(by.get(1)?.status).toBe('stopped');
    expect(by.get(3)?.status).toBe('stopped');
    expect(canResume(by.get(2)!)).toBe(true);
    expect(canResume(by.get(1)!)).toBe(false);
    // Without the paused set (older callers) nothing changes.
    expect(missionCards(queue, {}, place, NOW).finished.every(c => c.status === 'stopped')).toBe(true);
  });

  it('a newer row of the chat (a send while paused) wins over the pause', () => {
    const queue = [stopped(1, 'k-a'), row(2, { source: 'chat', sourceId: 'k-a', status: 'running', claimedAt: at(1) })];
    const { live, finished } = missionCards(queue, {}, place, NOW, new Map(), new Set(['k-a']));
    expect(finished[0].status).toBe('stopped');
    expect(live[0].status).toBe('running');
  });

  it('the card and the finished row both offer Resume', () => {
    expect(MISSION).toContain('<Show when={canResume(c())}>');
    expect(MISSION).toContain('<Show when={canResume(c)}>');
  });
});

describe('Running exact-run activity', () => {
  const automation = (id: number, runId: string, status: RunQueueItem['status'] = 'running') =>
    row(id, { source: 'cron', sourceId: 'morning', status, historyRunId: runId, claimedAt: at(10), finishedAt: status === 'done' ? at(1) : null });

  it('restores activity after a reload without a started event, on its exact running row only', () => {
    const activity = applyActivityEvent(new Map(), { type: 'activity', runId: 'run-two', progress: 'researching', lastEventAt: at(7) });
    const feed = applyFeedEvent({}, workflowFeedEvent({ type: 'activity', slug: 'morning', progress: 'researching', lastEventAt: at(7) })!, NOW);
    const cards = missionCards([
      automation(2, 'run-two'), automation(3, 'run-three'),
      automation(4, 'run-two', 'queued'), automation(5, 'run-two', 'done'),
    ], feed, place, NOW, activity);
    expect(cards.live).toHaveLength(3);
    expect(cards.live.find(c => c.queueId === 2)).toMatchObject({ progress: 'researching', lastEventAt: at(7) });
    for (const c of [...cards.live.filter(c => c.queueId !== 2), ...cards.finished]) {
      expect(c).toMatchObject({ progress: null, lastEventAt: null });
    }
  });

  it('ignores another run of the same slug and clears done even before the queue refreshes', () => {
    let activity: ActivityMap = applyActivityEvent(new Map(), { type: 'activity', runId: 'run-two', progress: 'researching', lastEventAt: at(7) });
    const feed = applyFeedEvent(
      applyFeedEvent({}, { type: 'runStarted', conversationKey: workflowFeedKey('morning'), queueId: 2 }, NOW),
      { type: 'activity', conversationKey: workflowFeedKey('morning'), progress: 'researching', lastEventAt: at(7) }, NOW,
    );
    activity = applyActivityEvent(activity, { type: 'activity', runId: 'run-other', progress: 'coordinating', lastEventAt: at(8) });
    const read = () => missionCards([automation(2, 'run-two')], feed, place, NOW, activity).live[0];
    expect(read()).toMatchObject({ progress: 'researching', lastEventAt: at(7) });
    activity = applyActivityEvent(activity, { type: 'done', runId: 'run-other' });
    expect(read().progress).toBe('researching');
    activity = applyActivityEvent(activity, { type: 'done', runId: 'run-two' });
    expect(read()).toMatchObject({ progress: null, lastEventAt: null });
  });

  it.each(['done', 'cancelled'])('%s clears the chat feed activity while preserving its text', type => {
    let feed = applyFeedEvent({}, { type: 'runStarted', conversationKey: 'k-a', queueId: 7 }, NOW);
    feed = applyFeedEvent(feed, { type: 'assistantText', conversationKey: 'k-a', text: 'Keep this result' }, NOW);
    feed = applyFeedEvent(feed, { type: 'activity', conversationKey: 'k-a', progress: 'researching', lastEventAt: at(7) }, NOW);
    feed = applyFeedEvent(feed, { type, conversationKey: 'k-a' }, NOW);
    expect(feed['k-a']).toMatchObject({ running: false, progress: null, lastEventAt: null, lines: ['Keep this result'] });
    expect(missionCards([row(7, { sourceId: 'k-a', status: 'running' })], feed, place, NOW).live[0])
      .toMatchObject({ progress: null, lastEventAt: null });
  });
});

describe('review fixes', () => {
  it('never shows another turn’s lines or cost on a row', () => {
    let feed = applyFeedEvent({}, { type: 'runStarted', conversationKey: 'k-a', queueId: 7 }, NOW);
    feed = applyFeedEvent(feed, { type: 'assistantText', conversationKey: 'k-a', text: 'turn seven' }, NOW);
    feed = applyFeedEvent(feed, { type: 'usage', conversationKey: 'k-a', usage: { costUsd: 2 } }, NOW);
    feed = applyFeedEvent(feed, { type: 'done', conversationKey: 'k-a' }, NOW);
    const { live, finished } = missionCards([
      row(8, { source: 'chat', sourceId: 'k-a', status: 'queued' }),
      row(6, { source: 'chat', sourceId: 'k-a', status: 'done', finishedAt: at(30) }),
      row(7, { source: 'chat', sourceId: 'k-a', status: 'done', finishedAt: at(5) }),
    ], feed, place, NOW);
    expect(live[0].lines).toEqual([]);
    expect(finished.map(c => [c.queueId, c.costUsd])).toEqual([[7, 2], [6, null]]);
  });

  it('offers only the stop the queue allows', () => {
    const { live } = missionCards([
      row(1, { source: 'chat', sourceId: 'k-a', status: 'running' }),
      row(2, { source: 'cron', sourceId: 'm', status: 'queued' }),
      row(3, { source: 'chat', sourceId: 'k-b', status: 'queued', position: 2 }),
      row(4, { source: 'ingest', sourceId: 'i', status: 'running' }),
      row(5, { source: 'chat', sourceId: 'k-b', status: 'running', cancelRequested: true }),
    ], {}, place, NOW);
    const stops = Object.fromEntries(live.map(c => [c.queueId, c.stop]));
    expect(stops).toMatchObject({ 1: 'Stop', 2: 'Skip', 3: 'Cancel', 4: null, 5: null });
  });

  it('sorts queued turns by their place in the queue and names skipped and interrupted rows', () => {
    const { live, finished } = missionCards([
      row(9, { source: 'chat', sourceId: 'k-a', status: 'queued', position: 2 }),
      row(10, { source: 'chat', sourceId: 'k-b', status: 'queued', position: 1 }),
      row(11, { source: 'cron', sourceId: 'x', status: 'skipped', finishedAt: at(3) }),
      row(12, { source: 'chat', sourceId: 'k-a', status: 'interrupted', finishedAt: at(4) }),
    ], {}, place, NOW);
    expect(live.map(c => c.queueId)).toEqual([10, 9]);
    expect(finished.map(c => c.status)).toEqual(['skipped', 'interrupted']);
  });

  it('keeps text still streaming when a whole block arrives', () => {
    let feed = applyFeedEvent({}, { type: 'assistantDelta', conversationKey: 'k-a', text: 'partial thought' }, NOW);
    feed = applyFeedEvent(feed, { type: 'assistantText', conversationKey: 'k-a', text: 'API error' }, NOW);
    expect(shownLines(feed['k-a'])).toEqual(['partial thought', 'API error']);
  });
});

describe('orbitLayout', () => {
  it('centres one system, and keeps automations last with queued turns still on the outer ring', () => {
    const { live } = missionCards([
      row(1, { source: 'chat', sourceId: 'k-a', status: 'running' }),
      row(2, { source: 'cron', sourceId: 'm', status: 'running' }),
      row(3, { source: 'chat', sourceId: 'k-a', status: 'queued' }),
    ], {}, place, NOW);
    const systems = orbitLayout(live);
    expect(systems.map(s => s.id)).toEqual(['w-1', 'automations']);
    const ws = systems[0];
    expect(ws.satellites.map(s => [s.ring, s.period > 0])).toEqual([[1, true], [2, false]]);
    expect(orbitLayout(live.slice(0, 1))[0]).toMatchObject({ x: 0.5, y: 0.5 });
  });

  it('gives a system the same start angle every time', () => {
    expect(hashAngle('w-1')).toBe(hashAngle('w-1'));
    expect(hashAngle('w-1')).toBeGreaterThanOrEqual(0);
    expect(hashAngle('w-1')).toBeLessThan(360);
  });
});

describe('formatting', () => {
  it('says elapsed time and money plainly', () => {
    expect(elapsed(42_000)).toBe('42 s');
    expect(elapsed(3 * 60_000)).toBe('3 min');
    expect(elapsed(72 * 60_000)).toBe('1 h 12 min');
    expect(money(0.004)).toBe('<$0.01');
    expect(money(1.5)).toBe('$1.50');
    expect(money(null)).toBeNull();
  });
});


it('does not attach an unbound conversation cost to a finished queue row', () => {
  let f = applyFeedEvent({}, { type: 'runStarted', conversationKey: 'k-a' }, NOW);
  f = applyFeedEvent(f, { type: 'usage', sessionKey: 'k-a', usage: { costUsd: 12 } }, NOW);
  f = applyFeedEvent(f, { type: 'done', conversationKey: 'k-a' }, NOW);
  const { finished } = missionCards([row(77, { sourceId: 'k-a', status: 'done', finishedAt: at(1) })], f, place, NOW);
  expect(finished[0].costUsd).toBeNull();
});

describe('automations stream into Mission Control (overnight 10-11/12 A7)', () => {
  const fold = (events: Parameters<typeof workflowFeedEvent>[0][]) => events.reduce<Feed>((feed, e) => {
    const evt = workflowFeedEvent(e);
    return evt ? applyFeedEvent(feed, evt, NOW) : feed;
  }, {});

  it('turns workflow events into feed events keyed by the automation', () => {
    expect(workflowFeedEvent({ type: 'started', slug: 'morning', queueId: 7 })).toEqual({ type: 'runStarted', conversationKey: workflowFeedKey('morning'), queueId: 7 });
    expect(workflowFeedEvent({ type: 'assistantText', slug: 'morning', text: 'Reading the calendar' })).toMatchObject({ type: 'assistantText', text: 'Reading the calendar' });
    expect(workflowFeedEvent({ type: 'toolUse', slug: 'morning', name: 'Read', input: { file_path: 'a.md' } })).toMatchObject({ type: 'toolUse', name: 'Read' });
    expect(workflowFeedEvent({ type: 'activity', slug: 'morning', progress: 'tool', lastEventAt: at(1) })).toMatchObject({ type: 'activity', progress: 'tool' });
    expect(workflowFeedEvent({ type: 'done', slug: 'morning' })).toMatchObject({ type: 'done' });
    // No slug, or a kind the feed doesn't read: nothing.
    expect(workflowFeedEvent({ type: 'assistantText', text: 'x' })).toBeNull();
    expect(workflowFeedEvent({ type: 'unknown', slug: 'morning' })).toBeNull();
  });

  it('an automation’s card shows its own streamed lines and tool', () => {
    const feed = fold([
      { type: 'started', slug: 'morning', queueId: 2 },
      { type: 'assistantText', slug: 'morning', text: 'Reading the calendar' },
      { type: 'toolUse', slug: 'morning', name: 'Read', input: { file_path: 'notes/today.md' } },
    ]);
    const { live } = missionCards([row(2, { source: 'cron', sourceId: 'morning', label: 'Morning brief', status: 'running', claimedAt: at(1) })], feed, place, NOW);
    expect(live).toHaveLength(1);
    expect(live[0]).toMatchObject({ queueId: 2, system: 'automations', lines: ['Reading the calendar'] });
    expect(live[0].tool).toBeTruthy();
    // The tool's result clears the tool line, as a chat's does.
    const after = applyFeedEvent(feed, workflowFeedEvent({ type: 'toolResult', slug: 'morning' })!, NOW);
    expect(missionCards([row(2, { source: 'cron', sourceId: 'morning', status: 'running', claimedAt: at(1) })], after, place, NOW).live[0].tool).toBeNull();
  });

  it('a queued run of the same automation never shows the running run’s tool', () => {
    // A webview reload mid-run: the feed's entry has no queue id yet.
    const feed = fold([{ type: 'toolUse', slug: 'morning', name: 'Read', input: { file_path: 'a.md' } }]);
    const { live } = missionCards([row(3, { source: 'cron', sourceId: 'morning', status: 'queued', position: 1 })], feed, place, NOW);
    expect(live[0].tool).toBeNull();
  });

  it('never shows another run’s lines, and a running automation is not a stray chat card', () => {
    const feed = fold([{ type: 'started', slug: 'morning', queueId: 9 }, { type: 'assistantText', slug: 'morning', text: 'A later run' }]);
    const { live } = missionCards([row(2, { source: 'manual', sourceId: 'morning', status: 'running', claimedAt: at(1) })], feed, place, NOW);
    expect(live.map(c => c.queueId)).toEqual([2]);
    expect(live[0].lines).toEqual([]);
    // A heartbeat row with the same id never reads an automation's lines.
    const beat = missionCards([row(9, { source: 'heartbeat', sourceId: 'morning', status: 'running', claimedAt: at(1) })], feed, place, NOW).live;
    expect(beat[0].lines).toEqual([]);
    // With no queue row at all, the running automation's entry makes no card of its own.
    expect(missionCards([], feed, place, NOW).live).toEqual([]);
  });
});

// Overnight 11-13: Stop is confirmed in a dialog. While it is open the
// clicked turn can end and the same chat's next turn start (a held message
// drains, an agent's send_to_session); the stop must never reach that one.
describe('stopCard', () => {
  const rows: Record<number, RunQueueItem> = {};
  const deps = (during: () => void) => {
    const calls = { rows: [] as number[], keys: [] as string[] };
    return {
      calls,
      confirm: async () => { during(); return true; },
      cancelRow: async (id: number) => { calls.rows.push(id); return rows[id]; },
    };
  };

  it('stops by the clicked row, so a turn that ended meanwhile stops nothing', async () => {
    let feed: Feed = applyFeedEvent({}, { type: 'runStarted', conversationKey: 'k-a', queueId: 1 }, NOW);
    rows[1] = row(1, { sourceId: 'k-a', status: 'running', claimedAt: at(2) });
    const [c] = missionCards([rows[1]], feed, place, NOW).live;
    const d = deps(() => {
      // T1 ends; T2 starts on the same key before the person confirms.
      rows[1] = row(1, { sourceId: 'k-a', status: 'done', finishedAt: at(0) });
      rows[2] = row(2, { sourceId: 'k-a', status: 'running' });
      feed = applyFeedEvent(feed, { type: 'runStarted', conversationKey: 'k-a', queueId: 2 }, NOW + 1);
    });
    expect(await stopCard(c, () => feed, d)).toBe('ended');
    expect(d.calls).toEqual({ rows: [1], keys: [] });
  });

  it('stops the clicked row when it is still running', async () => {
    rows[3] = row(3, { sourceId: 'k-b', status: 'running', claimedAt: at(2) });
    const [c] = missionCards([rows[3]], {}, place, NOW).live;
    const d = deps(() => {});
    expect(await stopCard(c, () => ({}), d)).toBe('stopped');
    expect(d.calls).toEqual({ rows: [3], keys: [] });
    expect(await stopCard(c, () => ({}), { ...d, confirm: async () => false })).toBe('declined');
    expect(d.calls.rows).toEqual([3]);
  });

  it('a feed-only card stops by the row its turn started on, and a card with no row is refused, never stopped by key', async () => {
    let feed: Feed = applyFeedEvent({}, { type: 'runStarted', conversationKey: 'k-a', queueId: 4 }, NOW);
    rows[4] = row(4, { sourceId: 'k-a', status: 'done', finishedAt: at(0) });
    const [byRow] = missionCards([], feed, place, NOW).live;
    expect(byRow.queueId).toBeNull();
    const d = deps(() => {});
    expect(await stopCard(byRow, () => feed, d)).toBe('ended');
    expect(d.calls).toEqual({ rows: [4], keys: [] });

    feed = applyFeedEvent({}, { type: 'runStarted', conversationKey: 'k-b' }, NOW);
    const [noRow] = missionCards([], feed, place, NOW).live;
    let asked = false;
    const none = deps(() => { asked = true; });
    expect(await stopCard(noRow, () => feed, none)).toBe('noRow');
    expect(asked).toBe(false);
    expect(none.calls).toEqual({ rows: [], keys: [] });
    expect(MISSION).toContain('setNote({ text: rowStopNote(done, c.title) ?? (');
    expect(MISSION).not.toContain('chatCancel');
    expect(NO_ROW_NOTICE).toBe('That turn has no queue row any more, so nothing was stopped. Open its chat to stop it.');
    expect(rowStopNote('noRow', 'A')).toBe(NO_ROW_NOTICE);
    expect(rowStopNote('ended', 'A')).toBe('“A” had already ended, so nothing was stopped.');
    expect(rowStopNote('stopped', 'A')).toBeNull();
  });
});
