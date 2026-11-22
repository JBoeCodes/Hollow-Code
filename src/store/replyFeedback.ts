// Useful and Not useful under a reply (overnight 11-12, lane C2;
// `replyActions.ts` says where the votes go). One shared map from run id to
// the vote shown, so Conversation history and the side panel agree. A plain
// signal, never a resource, so a load can't suspend a reply. Saved votes are
// asked for once per run per session, at most 200 runs per request.
//
// Saves on one run go to the backend one at a time, in click order. A failed
// save puts back the last vote the backend confirmed, and only when no later
// click on that run is waiting or under way: a late failure never erases a
// later vote. A loaded vote is dropped only when a save on that run succeeded
// after the load was sent (the save is newer); otherwise it is what the
// backend holds, so a failed save rolls back to it, and it shows once no save
// on that run is still out.
import { createSignal } from 'solid-js';
import { invoke } from '../api';
import type { FeedbackVote, TurnFeedback } from '../types';
import { voteInput } from '../replyActions';

const RUN_IDS_PER_REQUEST = 200;

const [votes, setVotes] = createSignal<Readonly<Record<string, FeedbackVote>>>({});
export const replyVotes = votes;

/** What the backend last confirmed for each run (a load or a save). */
const confirmed: Record<string, FeedbackVote> = {};
/** The newest click on each run, by number, and each run's save chain. */
const latest: Record<string, number> = {};
const chains: Record<string, Promise<unknown>> = {};
/** Saves on each run that succeeded, and clicks whose save hasn't settled. */
const saved: Record<string, number> = {};
const unsettled: Record<string, number> = {};
let clicks = 0;

const asked = new Set<string>();
let waiting: string[] = [];
let flushing = false;

function show(runId: string, vote: FeedbackVote | undefined) {
  setVotes(prev => {
    if (prev[runId] === vote) return prev;
    const next = { ...prev };
    if (vote) next[runId] = vote; else delete next[runId];
    return next;
  });
}

/** Load the saved vote for `runId`, batched with the other replies on screen. */
export function ensureReplyVote(runId: string | undefined): void {
  if (!runId || asked.has(runId)) return;
  asked.add(runId);
  waiting.push(runId);
  if (flushing) return;
  flushing = true;
  queueMicrotask(flush);
}

/** One request per 200 runs, until nothing waits. */
function flush() {
  const runIds = waiting.slice(0, RUN_IDS_PER_REQUEST);
  waiting = waiting.slice(RUN_IDS_PER_REQUEST);
  if (!runIds.length) { flushing = false; return; }
  const savedBefore = new Map(runIds.map(id => [id, saved[id] ?? 0]));
  invoke<TurnFeedback[]>('list_feedback', { sessionKey: null, runIds, limit: 1000 }).then(rows => {
    for (const row of rows) {
      // A save that succeeded since the ask was sent is newer: an older loaded
      // vote must not replace it (a later failed save rolls back to it).
      if ((saved[row.runId] ?? 0) !== savedBefore.get(row.runId)) continue;
      // Otherwise it is what the backend holds; a click still saving keeps
      // showing until its save settles (and rolls back to this on failure).
      confirmed[row.runId] = row.vote;
      if (!unsettled[row.runId]) show(row.runId, row.vote);
    }
  }, () => { for (const id of runIds) asked.delete(id); }).finally(flush);
}

/** Save a vote on a reply. It shows at once; a refusal rethrows for the
 *  caller's message after putting back what the backend holds (unless a
 *  later click on the same reply has taken over). */
export function saveReplyVote(runId: string, vote: FeedbackVote, reason?: string | null): Promise<void> {
  const mine = ++clicks;
  latest[runId] = mine;
  unsettled[runId] = (unsettled[runId] ?? 0) + 1;
  show(runId, vote);
  const run = (chains[runId] ?? Promise.resolve()).catch(() => undefined).then(async () => {
    try {
      const row = await invoke<TurnFeedback>('record_turn_feedback', { input: voteInput(runId, vote, reason) });
      confirmed[runId] = row?.vote ?? vote;
      saved[runId] = (saved[runId] ?? 0) + 1;
    } catch (e) {
      if (latest[runId] === mine) show(runId, confirmed[runId]);
      throw e;
    } finally {
      unsettled[runId]--;
    }
  });
  chains[runId] = run;
  return run;
}

/** Test hook: forget every vote and request. */
export function resetReplyVotesForTest(): void {
  setVotes({});
  for (const k of Object.keys(confirmed)) delete confirmed[k];
  for (const k of Object.keys(latest)) delete latest[k];
  for (const k of Object.keys(chains)) delete chains[k];
  for (const k of Object.keys(saved)) delete saved[k];
  for (const k of Object.keys(unsettled)) delete unsettled[k];
  asked.clear();
  waiting = [];
  flushing = false;
}
