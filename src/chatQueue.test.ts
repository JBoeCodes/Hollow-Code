import { describe, expect, it } from 'vitest';
import {
  acceptedShowsBadge,
  addToDraft,
  adoptedClientTurnId,
  eventMatchesTurn,
  isStaleTurnEvent,
  mergeRestoredKeys,
  newClientTurnId,
  planQueuedChatRows,
  queuedBadgeText,
  turnStanding,
  queuedCancelNotice,
  queuedFailureNotice,
  queuedTurnEnd,
  restoredDraft,
  restoredKey,
  RESTORED_QUEUED_TURNS_CAP,
  type LocalQueuedTurn,
} from './chatQueue';
import CHAT_QUEUE_SOURCE from './chatQueue.ts?raw';
import type { RunQueueItem } from './types';
import { PERMISSION_REFUSED_NOTE, queuedTurnLabel, waitingText } from './runQueue';

const queued = (over: Partial<LocalQueuedTurn> = {}): LocalQueuedTurn => ({
  clientTurnId: 'ct-a', queueId: 7, started: false, ...over,
});

function row(over: Partial<RunQueueItem> = {}): RunQueueItem {
  return {
    id: 7, source: 'chat', sourceId: 's-20260925-aaaa0001', label: 'Chat', detail: 'Summarize the launch notes',
    reason: 'chat', status: 'queued', priority: 0, position: 1, attempt: 0, coalescedCount: 0,
    requestedAt: '2026-09-25T10:00:00Z', claimedAt: null, finishedAt: null, historyRunId: null, error: null,
    waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false, waitingReason: null, ...over,
  };
}

describe('queuedTurnEnd reconciles by clientTurnId', () => {
  it('runStarted before SendAccepted marks the turn started', () => {
    // queueId is still null: SendAccepted has not resolved yet.
    const local = queued({ queueId: null });
    expect(queuedTurnEnd({ type: 'runStarted', clientTurnId: 'ct-a', queueId: 7 }, local, null)).toBe('started');
    expect(acceptedShowsBadge({ ...local, started: true }, 'ct-a')).toBe(false);
    expect(acceptedShowsBadge(local, 'ct-a')).toBe(true);
  });

  it('a queued cancel restores for Edit and Stop and drops for Discard', () => {
    const cancelled = { type: 'cancelled', clientTurnId: 'ct-a', queued: true };
    expect(queuedTurnEnd(cancelled, queued(), 'edit')).toBe('restore');
    expect(queuedTurnEnd(cancelled, queued(), null)).toBe('restore');
    expect(queuedTurnEnd(cancelled, queued(), 'discard')).toBe('drop');
  });

  it('a running turn stopped is an ordinary stop', () => {
    expect(queuedTurnEnd({ type: 'cancelled', clientTurnId: 'ct-a' }, queued(), null)).toBe('none');
    expect(queuedTurnEnd({ type: 'cancelled', clientTurnId: 'ct-a', queued: true }, queued({ started: true }), 'edit')).toBe('none');
    expect(queuedTurnEnd({ type: 'done', clientTurnId: 'ct-a' }, queued({ started: true }), null)).toBe('none');
  });

  it('a recovered done restores, even if runStarted was somehow seen', () => {
    expect(queuedTurnEnd({ type: 'done', clientTurnId: 'ct-a', recovered: true }, queued(), null)).toBe('restore');
    expect(queuedTurnEnd({ type: 'done', clientTurnId: 'ct-a', recovered: true }, queued({ started: true }), null)).toBe('restore');
    expect(queuedTurnEnd({ type: 'done', clientTurnId: 'ct-a', recovered: true }, queued(), 'discard')).toBe('drop');
  });

  it('a done for a turn that never started restores it', () => {
    expect(queuedTurnEnd({ type: 'done', clientTurnId: 'ct-a' }, queued(), null)).toBe('restore');
    expect(queuedTurnEnd({ type: 'done', clientTurnId: 'ct-a' }, queued(), 'discard')).toBe('drop');
  });

  it('an event for another turn, or with no id at all, is none', () => {
    expect(queuedTurnEnd({ type: 'done', clientTurnId: 'ct-b' }, queued(), null)).toBe('none');
    expect(queuedTurnEnd({ type: 'runStarted', clientTurnId: 'ct-b', queueId: 8 }, queued(), null)).toBe('none');
    expect(queuedTurnEnd({ type: 'done' }, queued(), null)).toBe('none');
    expect(queuedTurnEnd({ type: 'done', clientTurnId: 'ct-a' }, null, null)).toBe('none');
    expect(queuedTurnEnd({ type: 'assistantText', clientTurnId: 'ct-a' }, queued(), null)).toBe('none');
  });

  it('an adopted turn matches by queue id, whatever nonce the event carries', () => {
    const adopted = queued({ clientTurnId: adoptedClientTurnId(7) });
    expect(eventMatchesTurn({ type: 'runStarted', clientTurnId: 'ct-real', queueId: 7 }, adopted)).toBe(true);
    expect(eventMatchesTurn({ type: 'runStarted', clientTurnId: 'ct-real', queueId: 8 }, adopted)).toBe(false);
    expect(eventMatchesTurn({ type: 'runStarted', queueId: 7 }, queued({ queueId: null }))).toBe(false);
  });
});

describe('isStaleTurnEvent keeps a late end from closing a newer send', () => {
  // Edit on turn A answered at once; the user re-sent as turn B before A's
  // own cancelled + done arrived.
  const a = { clientTurnId: 'ct-a', queueId: 7 };
  const b = queued({ clientTurnId: 'ct-b', queueId: 8 });

  it("an ended turn's done is stale, even with no local turn", () => {
    expect(isStaleTurnEvent({ type: 'done', clientTurnId: 'ct-a', queueId: 7 }, b, [a])).toBe(true);
    expect(isStaleTurnEvent({ type: 'done', clientTurnId: 'ct-a', queueId: 7 }, null, [a])).toBe(true);
    // B still awaiting SendAccepted: A's end is still not B's.
    expect(isStaleTurnEvent({ type: 'done', clientTurnId: 'ct-a', queueId: 7 }, { ...b, queueId: null }, [a])).toBe(true);
  });

  it("any other tagged event is stale while a different turn is local", () => {
    expect(isStaleTurnEvent({ type: 'done', clientTurnId: 'ct-x', queueId: 3 }, b, [])).toBe(true);
    expect(isStaleTurnEvent({ type: 'done', clientTurnId: 'ct-x', queueId: 3 }, null, [])).toBe(false);
  });

  it("the local turn's own events and untagged events are never stale", () => {
    expect(isStaleTurnEvent({ type: 'done', clientTurnId: 'ct-b', queueId: 8 }, b, [a])).toBe(false);
    // Adopted after a reload: its nonce is a placeholder, the queue id matches.
    expect(isStaleTurnEvent({ type: 'done', clientTurnId: 'ct-real', queueId: 8 }, { ...b, clientTurnId: adoptedClientTurnId(8) }, [])).toBe(false);
    expect(isStaleTurnEvent({ type: 'done' }, b, [a])).toBe(false);
  });
});

describe('Working, not Queued (2026-11-16)', () => {
  it('a turn the queue accepted with no real wait is preparing, never queued', () => {
    expect(turnStanding({ queueId: null, started: false, waitingFor: null })).toBe('sending');
    expect(turnStanding({ queueId: 7, started: false, waitingFor: null })).toBe('preparing');
    expect(turnStanding({ queueId: 7, started: true, waitingFor: null })).toBe('started');
  });

  it('only a named limit, memory or another run puts it in Queued', () => {
    expect(turnStanding({ queueId: 7, started: false, waitingFor: 'memory' })).toBe('queued');
    expect(turnStanding({ queueId: 7, started: false, waitingFor: 'a free chat slot: Hollow is at its limit of 10 chats at once (Settings → Chat → Chats at once)' })).toBe('queued');
    expect(turnStanding({ queueId: 7, started: false, waitingFor: "a free chat slot: this workspace is at its limit of 5 chats at once" })).toBe('queued');
    expect(turnStanding({ queueId: 7, started: false, waitingFor: 'a free subagent slot: this chat is at its limit of 6 subagents at once' })).toBe('queued');
    // Once it runs the reason no longer applies.
    expect(turnStanding({ queueId: 7, started: true, waitingFor: 'memory' })).toBe('started');
  });

  it('a claimed row with no wait reason keeps reading Preparing through the queue refresh', () => {
    const idle = { local: null, busy: false, composerEmpty: true, restoredKeys: [], foregroundKey: row().sourceId, backgroundKeys: [] as string[] };
    const claimed = row({ status: 'claimed', working: true });
    const [action] = planQueuedChatRows([claimed], { ...idle, local: { ...queued(), adopted: false }, busy: true });
    expect(action).toMatchObject({ kind: 'update', waitingFor: null, started: false });
    // A queued row held by the 10-chat limit still carries its reason.
    const held = row({ waitingReason: 'a free chat slot: Hollow is at its limit of 10 chats at once' });
    const [update] = planQueuedChatRows([held], { ...idle, local: { ...queued(), adopted: false }, busy: true });
    expect(update).toMatchObject({ kind: 'update', waitingFor: 'a free chat slot: Hollow is at its limit of 10 chats at once' });
  });
});

describe('queued-turn copy and drafts', () => {
  it('the badge names the holder; a place in line alone is not a reason', () => {
    expect(queuedBadgeText({ waitingFor: 'the Morning brief workflow' })).toBe('Queued · waiting for the Morning brief workflow');
    expect(queuedBadgeText({ waitingFor: null })).toBe('Queued');
  });

  it('the badge names the limit or memory a turn waits for (2026-10-20)', () => {
    expect(queuedBadgeText({ waitingFor: 'the 10-chat limit' })).toBe('Queued · waiting for the 10-chat limit');
    expect(queuedBadgeText({ waitingFor: "this workspace's 5-chat limit" })).toBe("Queued · waiting for this workspace's 5-chat limit");
    expect(queuedBadgeText({ waitingFor: 'memory' })).toBe('Queued · waiting for memory');
  });

  it('notices never read like an error', () => {
    expect(queuedCancelNotice('edit')).toBeNull();
    for (const intent of ['discard', null] as const) {
      expect(queuedCancelNotice(intent)).not.toMatch(/error|fail/i);
    }
  });

  it('a failure before start says it was not sent, once, without a doubled stop', () => {
    expect(queuedFailureNotice('Hollow is busy: a stale run holds the vault; restart Hollow.'))
      .toBe('Your queued message was not sent: Hollow is busy: a stale run holds the vault; restart Hollow. It is back in the composer.');
    expect(queuedFailureNotice('  ')).toBe('Your queued message was not sent. It is back in the composer.');
    // A box that already held other text keeps it, and the notice says no more (2026-10-15).
    expect(queuedFailureNotice('busy', false)).toBe('Your queued message was not sent: busy.');
    expect(queuedFailureNotice('', false)).toBe('Your queued message was not sent.');
  });

  it('restoring keeps anything typed meanwhile', () => {
    expect(restoredDraft('Queued text', '')).toBe('Queued text');
    expect(restoredDraft('Queued text', '  ')).toBe('Queued text');
    expect(restoredDraft('Queued text', 'new thought')).toBe('Queued text\n\nnew thought');
    expect(restoredDraft('Queued text', 'Queued text\n\nnew thought')).toBe('Queued text\n\nnew thought');
    expect(restoredDraft('', 'new thought')).toBe('new thought');
  });

  it('client turn ids are unique', () => {
    const ids = new Set(Array.from({ length: 50 }, () => newClientTurnId()));
    expect(ids.size).toBe(50);
    for (const id of ids) expect(id).toMatch(/^ct-/);
  });
});

describe('planQueuedChatRows adopts, updates and restores', () => {
  const idle = { local: null, busy: false, composerEmpty: true, restoredKeys: [], foregroundKey: row().sourceId, backgroundKeys: [] as string[] };

  it('adopts a live chat row when no turn is in flight', () => {
    expect(planQueuedChatRows([row({ source: 'cron', id: 3 }), row()], idle)).toEqual([{ kind: 'adopt', row: row() }]);
    expect(planQueuedChatRows([row()], { ...idle, busy: true })).toEqual([]);
  });

  it('does not adopt a wake that is waiting on an open question card', () => {
    const held = row({ waitingReason: 'an open question card' });
    expect(planQueuedChatRows([held], idle)).toEqual([]);
    const running = row({ status: 'running', waitingReason: null });
    expect(planQueuedChatRows([running], idle)).toEqual([{ kind: 'adopt', row: running }]);
    const own = row({ id: 9 });
    expect(planQueuedChatRows([held, own], idle)).toEqual([{ kind: 'adopt', row: own }]);
  });

  it('never adopts another conversation into the open workspace or a fresh draft', () => {
    const foreign = row({ id: 8, sourceId: 's-other', status: 'running' });
    expect(planQueuedChatRows([foreign], idle)).toEqual([]);
    expect(planQueuedChatRows([foreign, row()], idle)).toEqual([{ kind: 'adopt', row: row() }]);
    expect(planQueuedChatRows([row()], { ...idle, foregroundKey: null })).toEqual([]);
    expect(planQueuedChatRows([row()], { ...idle, backgroundKeys: [row().sourceId!] })).toEqual([]);
    expect(planQueuedChatRows([row({ sourceId: null })], { ...idle, foregroundKey: null })).toEqual([]);
  });

  it('restores interrupted messages only to their own conversation', () => {
    const own = row({ status: 'interrupted' });
    const foreign = row({ id: 8, sourceId: 's-other', status: 'interrupted' });
    expect(planQueuedChatRows([foreign], idle)).toEqual([]);
    expect(planQueuedChatRows([foreign, own], idle)).toEqual([{ kind: 'restore', row: own, seen: [restoredKey(own)] }]);
    expect(planQueuedChatRows([own], { ...idle, foregroundKey: null })).toEqual([]);
  });

  it('refreshes the local turn and un-adopts one whose row ended', () => {
    const local = { ...queued(), adopted: true };
    expect(planQueuedChatRows([row({ position: 2, waitingHolder: 'a workflow' })], { ...idle, local, busy: true }))
      .toEqual([{ kind: 'update', queuePosition: 2, waitingFor: 'a workflow', heldReason: null, started: false }]);
    expect(planQueuedChatRows([row({ status: 'running' })], { ...idle, local, busy: true }))
      .toEqual([{ kind: 'update', queuePosition: 1, waitingFor: null, heldReason: null, started: true }]);
    // Overnight A2: the badge names what else holds it back.
    expect(planQueuedChatRows([row({ position: 1, waitingReason: "this conversation's previous turn to finish" })], { ...idle, local, busy: true }))
      .toEqual([{ kind: 'update', queuePosition: 1, waitingFor: "this conversation's previous turn to finish", heldReason: null, started: false }]);
    // P1-46: a held build worktree names its conversation, for Open it.
    const held = planQueuedChatRows([row({ position: 1, waitingReason: '“Fix the panel” to finish (it has used this worktree for 12 min)', waitingSession: 's-holder' })], { ...idle, local, busy: true });
    expect(held).toEqual([{ kind: 'update', queuePosition: 1, waitingFor: '“Fix the panel” to finish (it has used this worktree for 12 min)', waitingSession: 's-holder', heldReason: null, started: false }]);
    expect(planQueuedChatRows([row({ status: 'done', position: null })], { ...idle, local, busy: true })).toEqual([{ kind: 'unadopt' }]);
    expect(planQueuedChatRows([], { ...idle, local, busy: true })).toEqual([{ kind: 'unadopt' }]);
    // A turn sent from this window ends through its own events, never here.
    expect(planQueuedChatRows([], { ...idle, local: { ...local, adopted: false }, busy: true })).toEqual([]);
  });

  it('restores the newest never-started interrupted turn once, into an empty composer', () => {
    const older = row({ id: 4, status: 'interrupted', position: null, detail: 'Older text' });
    const newer = row({ id: 6, status: 'interrupted', position: null, detail: 'Newer text' });
    const ranAndCleared = row({ id: 5, status: 'interrupted', position: null, detail: null });
    expect(planQueuedChatRows([older, ranAndCleared, newer], idle))
      .toEqual([{ kind: 'restore', row: newer, seen: [restoredKey(newer), restoredKey(older)] }]);
    expect(planQueuedChatRows([older, newer], { ...idle, restoredKeys: [restoredKey(newer), restoredKey(older)] })).toEqual([]);
    expect(planQueuedChatRows([newer], { ...idle, composerEmpty: false })).toEqual([]);
    expect(planQueuedChatRows([newer], { ...idle, busy: true })).toEqual([]);
  });

  it('keeps the restored markers capped, newest first', () => {
    const existing = Array.from({ length: RESTORED_QUEUED_TURNS_CAP }, (_, i) => `k${i}`);
    const merged = mergeRestoredKeys(existing, ['new', 'k3']);
    expect(merged).toHaveLength(RESTORED_QUEUED_TURNS_CAP);
    expect(merged.slice(0, 3)).toEqual(['new', 'k3', 'k0']);
    expect(restoredKey(row())).toBe('7@2026-09-25T10:00:00Z');
  });
});

describe('chatQueue module', () => {
  it('imports only types, so the node suite can load it', () => {
    const imports = CHAT_QUEUE_SOURCE.match(/^\s*import\s.*$/gm) ?? [];
    for (const line of imports) expect(line).toMatch(/^\s*import type\s/);
  });
});

describe('addToDraft (morning M5)', () => {
  it('adds after a draft, never replacing it, and never twice', () => {
    expect(addToDraft('', 'Work on the task')).toBe('Work on the task');
    expect(addToDraft('  \n', 'Work on the task')).toBe('Work on the task');
    expect(addToDraft('my half-written note\n', 'Work on the task')).toBe('my half-written note\n\nWork on the task');
    expect(addToDraft('note\n\nWork on the task', 'Work on the task')).toBe('note\n\nWork on the task');
    expect(addToDraft('note', '  ')).toBe('note');
  });
});

describe('a permission-held chat row (2026-11-17, a #1499 leftover)', () => {
  const idle = { local: null, busy: false, composerEmpty: true, restoredKeys: [], foregroundKey: row().sourceId, backgroundKeys: [] as string[] };

  it('reads paused in the composer, in the dock\'s own words, never Preparing', () => {
    const held = row({ heldReason: PERMISSION_REFUSED_NOTE });
    const [update] = planQueuedChatRows([held], { ...idle, local: { ...queued(), adopted: false }, busy: true });
    expect(update).toMatchObject({ kind: 'update', waitingFor: null, heldReason: PERMISSION_REFUSED_NOTE });
    const turn = { queueId: 7, started: false, waitingFor: null, heldReason: PERMISSION_REFUSED_NOTE };
    expect(turnStanding(turn)).toBe('queued');
    expect(queuedTurnLabel(turn)).toBe(waitingText(held));
    expect(queuedTurnLabel(turn)).toBe('Paused until its settings change is approved');
    // Any other hold note reads the same as the dock's line too.
    const other = { ...turn, heldReason: 'usage for a hard-stop budget could not be read' };
    expect(queuedTurnLabel(other)).toBe(waitingText(row({ heldReason: other.heldReason })));
    // A real wait without a hold keeps Queued; once it runs, the hold is gone.
    expect(queuedTurnLabel({ waitingFor: 'memory', heldReason: null })).toBe('Queued · waiting for memory');
    expect(turnStanding({ ...turn, started: true })).toBe('started');
  });
});
