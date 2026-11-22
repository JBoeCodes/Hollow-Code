import { beforeEach, describe, expect, it, vi } from 'vitest';

const calls: { command: string; args: Record<string, unknown> }[] = [];
let respond: (command: string, args: Record<string, unknown>) => Promise<unknown> = () => Promise.resolve([]);
vi.mock('../api', () => ({
  invoke: (command: string, args: Record<string, unknown>) => { calls.push({ command, args }); return respond(command, args); },
}));

const { ensureReplyVote, replyVotes, resetReplyVotesForTest, saveReplyVote } = await import('./replyFeedback');

const tick = () => new Promise(r => setTimeout(r, 0));
const row = (runId: string, vote: string) => ({ runId, vote });

describe('reply votes (overnight 11-12, C2)', () => {
  beforeEach(() => { calls.length = 0; resetReplyVotesForTest(); respond = () => Promise.resolve([]); });

  it('a failed earlier save never erases a later vote, and saves go in click order', async () => {
    const pending: { resolve: (v: unknown) => void; reject: (e: unknown) => void; vote: string }[] = [];
    respond = (_c, args) => new Promise((resolve, reject) => pending.push({ resolve, reject, vote: (args.input as { vote: string }).vote }));
    const first = saveReplyVote('r1', 'notUseful');
    const second = saveReplyVote('r1', 'useful');
    expect(replyVotes().r1).toBe('useful');
    await tick();
    // One at a time: the second waits for the first.
    expect(pending.map(p => p.vote)).toEqual(['notUseful']);
    pending[0].reject('FEEDBACK_INVALID: no');
    await expect(first).rejects.toBe('FEEDBACK_INVALID: no');
    expect(replyVotes().r1).toBe('useful');
    await tick();
    expect(pending.map(p => p.vote)).toEqual(['notUseful', 'useful']);
    pending[1].resolve(row('r1', 'useful'));
    await second;
    expect(replyVotes().r1).toBe('useful');
  });

  it('a later failure puts back the vote the backend confirmed, not the one clicked before it', async () => {
    respond = (_c, args) => Promise.resolve(row('r2', (args.input as { vote: string }).vote));
    await saveReplyVote('r2', 'notUseful');
    respond = () => Promise.reject('FEEDBACK_TURN_RUNNING: busy');
    await expect(saveReplyVote('r2', 'useful')).rejects.toBeTruthy();
    expect(replyVotes().r2).toBe('notUseful');
    resetReplyVotesForTest();
    await expect(saveReplyVote('r3', 'useful')).rejects.toBeTruthy();
    expect(replyVotes().r3).toBeUndefined();
  });

  it('loads saved votes in batches of 200, asking for each run once', async () => {
    respond = (_c, args) => Promise.resolve((args.runIds as string[]).filter(id => id === 'run-250').map(id => row(id, 'useful')));
    for (let i = 0; i < 450; i++) ensureReplyVote(`run-${i}`);
    ensureReplyVote('run-0');
    for (let i = 0; i < 6; i++) await tick();
    expect(calls.map(c => (c.args.runIds as string[]).length)).toEqual([200, 200, 50]);
    expect(new Set(calls.flatMap(c => c.args.runIds as string[])).size).toBe(450);
    expect(replyVotes()['run-250']).toBe('useful');
  });

  it('keeps a vote clicked while the load was out', async () => {
    let answer: (v: unknown) => void = () => {};
    respond = (command, args) => command === 'list_feedback'
      ? new Promise(r => { answer = r; })
      : Promise.resolve(row('r4', (args.input as { vote: string }).vote));
    ensureReplyVote('r4');
    await tick();
    await saveReplyVote('r4', 'notUseful');
    answer([row('r4', 'useful')]);
    await tick();
    expect(replyVotes().r4).toBe('notUseful');
  });

  it('a load answered after a save never becomes the vote a later failure rolls back to', async () => {
    let answer: (v: unknown) => void = () => {};
    respond = (command, args) => command === 'list_feedback'
      ? new Promise(r => { answer = r; })
      : Promise.resolve(row('r5', (args.input as { vote: string }).vote));
    ensureReplyVote('r5');
    await tick();
    await saveReplyVote('r5', 'notUseful');
    answer([row('r5', 'useful')]);
    await tick();
    respond = () => Promise.reject('FEEDBACK_TURN_RUNNING: busy');
    await expect(saveReplyVote('r5', 'useful')).rejects.toBeTruthy();
    expect(replyVotes().r5).toBe('notUseful');
  });
  it('a load answered while a save is out records the saved vote, so a failed save rolls back to it', async () => {
    let answer: (v: unknown) => void = () => {};
    let settle: { resolve: (v: unknown) => void; reject: (e: unknown) => void } = { resolve: () => {}, reject: () => {} };
    respond = command => command === 'list_feedback'
      ? new Promise(r => { answer = r; })
      : new Promise((resolve, reject) => { settle = { resolve, reject }; });
    ensureReplyVote('r6');
    await tick();
    const save = saveReplyVote('r6', 'useful');
    answer([row('r6', 'notUseful')]);
    await tick();
    expect(replyVotes().r6).toBe('useful');
    settle.reject('FEEDBACK_TURN_RUNNING: busy');
    await expect(save).rejects.toBeTruthy();
    expect(replyVotes().r6).toBe('notUseful');
  });

  it('a load answered after a failed save shows the saved vote', async () => {
    let answer: (v: unknown) => void = () => {};
    respond = command => command === 'list_feedback' ? new Promise(r => { answer = r; }) : Promise.reject('FEEDBACK_TURN_RUNNING: busy');
    ensureReplyVote('r7');
    await tick();
    await expect(saveReplyVote('r7', 'useful')).rejects.toBeTruthy();
    expect(replyVotes().r7).toBeUndefined();
    answer([row('r7', 'notUseful')]);
    await tick();
    expect(replyVotes().r7).toBe('notUseful');
  });

  it('a load answered while a save is out, then the save succeeding, keeps the click and rolls back to it later', async () => {
    let answer: (v: unknown) => void = () => {};
    let settle: (v: unknown) => void = () => {};
    respond = command => command === 'list_feedback' ? new Promise(r => { answer = r; }) : new Promise(r => { settle = r; });
    ensureReplyVote('r8');
    await tick();
    const save = saveReplyVote('r8', 'useful');
    answer([row('r8', 'notUseful')]);
    await tick();
    expect(replyVotes().r8).toBe('useful');
    await tick();
    settle(row('r8', 'useful'));
    await save;
    expect(replyVotes().r8).toBe('useful');
    respond = () => Promise.reject('FEEDBACK_TURN_RUNNING: busy');
    await expect(saveReplyVote('r8', 'notUseful')).rejects.toBeTruthy();
    expect(replyVotes().r8).toBe('useful');
  });
});
