import { describe, expect, it } from 'vitest';
import { canVote, replyRowButtons } from './replyActions';
import type { SubagentRow, SubagentsView } from './generated/ipc';
import type { RunQueueItem } from './types';
import {
  agoWords, cfoHeaderLine, chipAgents, latestCfo, rowWhen, chipShown, chipWords, replies, sentWords, tickFrom, countsText, durationWords, effortWords, findRow, firstWords, hasSubagents, headerMeta, olderLeft,
  liveWallMs, liveWorkingMs, modelWords, ordinal, placeWords, pollMs, rowMeta, rowTitle, statusWords, stopSubagent, subagentCall, tokenWords, waitingWords,
  workGroups, groupWords, costWords, outcomeWords, currentStep, activityWords, recentSteps, agentGlyph, finishedMark, outcomeDetail, createIdentities,
} from './subagentsModel';
import PANEL from './components/SubagentsPanel.tsx?raw';

const AT = '2026-11-10T12:00:00.000Z';
const at = (s: number) => Date.parse(AT) + s * 1000;
const CLAUDE = { models: [{ id: 'sonnet', label: 'Sonnet 5.5' }, { id: 'opus', label: 'Opus 5.5' }], newestByFamily: { sonnet: 'claude-sonnet-5-5' } };

function row(over: Partial<SubagentRow> = {}): SubagentRow {
  return {
    key: 's-1', current: 's-1', title: 'Company review', agent: 'claude', model: 'claude-sonnet-5-5', effort: 'medium', account: null,
    status: 'done', queuePlace: null, waitingFor: null, startedAt: AT, endedAt: AT, workingMs: 252_000, wallMs: 300_000,
    inputTokens: 150_000, cachedInputTokens: 90_000, outputTokens: 32_000, tokens: 182_000, costUsd: null,
    earlier: false, conversations: ['s-1'], trackerIssue: null, issue: null, freshFrom: null, ...over,
  };
}
function view(over: Partial<SubagentsView> = {}): SubagentsView {
  return { parent: 'p', parents: ['p'], active: [], done: [], doneOlder: [], olderTotal: 0, earlier: [], running: 0, queued: 0, doneCount: 0, at: AT, ...over };
}

describe('subagent words', () => {
  it('groups mixed swarm states once and keeps an orchestrator with its descendants', () => {
    const swarm = { id: 'swarm-1', title: 'Review', parts: 6 };
    const rows = [row({ key: 'a', conversations: ['a'], group: swarm, status: 'running', costUsd: 1.2 }),
      row({ key: 'b', conversations: ['b'], group: swarm, costUsd: null }),
      row({ key: 'nested', conversations: ['nested'], parentKey: 'a', costUsd: null }),
      row({ key: 'lead', conversations: ['lead', 'successor'], title: 'Build' }),
      row({ key: 'child', conversations: ['child'], parentKey: 'successor', status: 'queued' })];
    const groups = workGroups(rows);
    expect(groups).toHaveLength(2);
    expect(groups[0].rows.map(r => r.key)).toEqual(['a', 'b', 'nested']);
    expect(groupWords(groups[0])).toBe('6 parts · 1 running · $1.20 recorded');
    expect(groups[1]).toMatchObject({ kind: 'team', title: 'Build', active: true });
    expect(groups[1].rows.map(r => r.key)).toEqual(['lead', 'child']);
  });

  it('does not turn an unknown cost or absent completion report into success', () => {
    expect(costWords(null)).toBe('Cost unavailable');
    expect(costWords(0)).toBe('$0.00');
    expect(outcomeWords(row())).toBe('Finished');
    const activity = { current: null, recent: [], progress: null, artifacts: [], reportedPrs: [], outcome: 'Partial: native check remains' };
    expect(outcomeWords(row({ activity }))).toBe('Partial');
    expect(outcomeWords(row({ status: 'failed', activity }))).toBe('Failed');
    expect(currentStep(row({ status: 'queued', activity }))).toBe('Waiting in line');
    expect(currentStep(row({ status: 'queued', waitingFor: 'the per-chat limit', activity }))).toBe('Waiting for the per-chat limit');
  });

  it('summarizes tool work and terminates malformed parent cycles', () => {
    const activity = { current: { name: 'functions.exec_command', target: 'npm test', status: 'running' }, recent: [], progress: null, artifacts: [], reportedPrs: [], outcome: null };
    expect(currentStep(row({ status: 'running', activity }))).toBe('Checking the changes');
    expect(workGroups([row({ key: 'a', conversations: ['a'], parentKey: 'b' }), row({ key: 'b', conversations: ['b'], parentKey: 'a' })]).flatMap(g => g.rows)).toHaveLength(2);
  });
  it('never leaks raw commands or unknown tool payloads into activity labels', () => {
    expect(activityWords({ name: 'Bash', target: 'workspaces; for f in local-secret; do cat "$f"; done' })).toBe('Working in the terminal');
    expect(activityWords({ name: 'functions.exec_command', target: 'hollow-slot --remote npm run gate' })).toBe('Running the checks');
    expect(activityWords({ name: 'Bash', target: 'cat > private.txt' })).toBe('Working in the terminal');
    expect(activityWords({ name: 'Bash', target: 'npm test && git push' })).toBe('Working in the terminal');
    expect(activityWords({ name: 'Bash', target: 'cargo fmt' })).toBe('Formatting the code');
    expect(activityWords({ name: 'private_tool', target: '/private/customer.csv' })).toBe('Working on the task');
    expect(activityWords({ name: 'Read', target: '/private/customer.csv' })).toBe('Reading project context');
    expect(activityWords({ name: 'Bash', target: 'echo "npm test"' })).toBe('Working in the terminal');
    const steps = recentSteps(row({ activity: { current: null, recent: [
      { name: 'Read', target: 'a', status: 'ok' }, { name: 'Read', target: 'b', status: 'ok' },
      { name: 'Read', target: 'c', status: 'error' }, { name: 'Edit', target: 'd', status: 'ok' },
      { name: 'Bash', target: 'secret', status: 'ok' },
    ], progress: null, outcome: null, artifacts: [], reportedPrs: [] } }));
    expect(steps).toEqual([{ label: 'Updating project files', status: 'ok' }, { label: 'Reading project context', status: 'error' }, { label: 'Reading project context', status: 'ok' }]);
    expect(agentGlyph('s-1')).toBe(agentGlyph('s-1'));
    expect(agentGlyph('s-1')).not.toBe(agentGlyph('s-2'));
    expect(groupWords(workGroups([row({ key: 'a', conversations: ['a'] }), row({ key: 'b', parentKey: 'a' })])[0])).not.toContain('unavailable');
  });
  it('durations: seconds, then minutes and seconds, then hours and minutes', () => {
    // The chat's unspaced form ("Worked for 4m 12s"), as ChatGPT's "16h ago".
    expect(durationWords(0)).toBe('0s');
    expect(durationWords(45_400)).toBe('45s');
    expect(durationWords(252_000)).toBe('4m 12s');
    expect(durationWords(3_840_000)).toBe('1h 4m');
    expect(durationWords(-5)).toBe('0s');
    expect(durationWords(Number.NaN)).toBe('0s');
  });

  it('tokens: plain under a thousand, then K and M', () => {
    expect(tokenWords(950)).toBe('950');
    expect(tokenWords(1_200)).toBe('1.2K');
    expect(tokenWords(1_000)).toBe('1K');
    expect(tokenWords(182_000)).toBe('182K');
    expect(tokenWords(999_700)).toBe('1M');
    expect(tokenWords(1_200_000)).toBe('1.2M');
    expect(tokenWords(24_000_000)).toBe('24M');
  });

  it('places and states', () => {
    expect([1, 2, 3, 4, 11, 12, 13, 21, 22, 103].map(ordinal)).toEqual(['1st', '2nd', '3rd', '4th', '11th', '12th', '13th', '21st', '22nd', '103rd']);
    expect(placeWords(2)).toBe('Queued, 2nd');
    expect(placeWords(null)).toBe('Queued');
    expect(statusWords({ status: 'queued', queuePlace: 1 })).toBe('Queued, 1st');
    expect(statusWords({ status: 'running', queuePlace: null })).toBe('Running');
    expect(statusWords({ status: 'stopped', queuePlace: null })).toBe('Stopped');
  });

  it('ends say how long ago', () => {
    expect(agoWords(AT, at(20))).toBe('just now');
    expect(agoWords(AT, at(5 * 60))).toBe('5m ago');
    expect(agoWords(AT, at(16 * 3600 + 59))).toBe('16h ago');
    expect(agoWords(AT, at(3 * 86400))).toBe('3d ago');
    expect(agoWords(null, at(0))).toBe('');
  });

  it('live elapsed grows from the read only while running', () => {
    const running = row({ status: 'running', workingMs: 10_000, wallMs: 12_000 });
    expect(liveWorkingMs(running, AT, at(5))).toBe(15_000);
    expect(liveWallMs(running, AT, at(5))).toBe(17_000);
    expect(liveWorkingMs(running, AT, at(-5))).toBe(10_000);
    expect(liveWorkingMs(row({ workingMs: 10_000 }), AT, at(60))).toBe(10_000);
    expect(liveWorkingMs(row({ status: 'queued', workingMs: 0 }), AT, at(60))).toBe(0);
  });

  it('a cached read older than 30 s does not tick until a fresh one comes', () => {
    expect(tickFrom(AT, at(30))).toBe(AT);
    expect(tickFrom(AT, at(31))).toBe(new Date(at(31)).toISOString());
    const running = row({ status: 'running', workingMs: 10_000 });
    expect(liveWorkingMs(running, tickFrom(AT, at(5)), at(5))).toBe(15_000);
    expect(liveWorkingMs(running, tickFrom(AT, at(600)), at(600))).toBe(10_000);
    expect(tickFrom('not a time', at(0))).toBe(new Date(at(0)).toISOString());
  });

  it('the chip counts and up to three distinct agents, active first', () => {
    const v = view({
      active: [row({ agent: 'codex', status: 'running' }), row({ agent: 'codex', status: 'queued' })],
      done: [row({ agent: 'claude' }), row({ agent: 'grok' }), row({ agent: 'antigravity' })],
      running: 1, queued: 1, doneCount: 5,
    });
    expect(countsText(v)).toBe('1 running, 1 queued, 5 done');
    expect(countsText(view({ doneCount: 3 }))).toBe('3 done');
    expect(countsText(view({ running: 2, doneCount: 5 }))).toBe('2 running, 5 done');
    expect(countsText(view())).toBe('');
    expect(chipAgents(v)).toEqual(['codex', 'claude', 'grok']);
    expect(hasSubagents(view())).toBe(false);
    expect(hasSubagents(view({ earlier: [row({ earlier: true })] }))).toBe(false);
    expect(hasSubagents(v)).toBe(true);
  });

  // L8 (11-10): a lead's other chats left the sidebar for Earlier chats, so
  // the chip shows for them alone too (quick chats list in the sidebar's
  // Quick chats section since 2026-11-14, not in Earlier chats).
  it('shows the chip for earlier chats alone and names them', () => {
    const earlier = [row({ key: 's-old1', earlier: true }), row({ key: 's-old2', earlier: true })];
    expect(chipShown(view())).toBe(false);
    expect(chipShown(view({ earlier }))).toBe(true);
    expect(chipWords(view({ earlier }))).toBe('2 earlier chats');
    expect(chipWords(view({ earlier: earlier.slice(0, 1) }))).toBe('1 earlier chat');
    expect(chipWords(view({ earlier, running: 1, active: [row({ status: 'running' })] }))).toBe('1 running');
    expect(chipWords(view())).toBe('');
    expect(chipWords(null)).toBe('');
    expect(findRow(view({ earlier }), 's-old2')?.key).toBe('s-old2');
  });

  it('names models from the catalog, an alias for its pinned id, else the id', () => {
    expect(modelWords('claude-sonnet-5-5', CLAUDE)).toBe('Sonnet 5.5');
    expect(modelWords('opus', CLAUDE)).toBe('Opus 5.5');
    expect(modelWords('gpt-6-astra', null)).toBe('gpt-6-astra');
    expect(modelWords(null, CLAUDE)).toBeNull();
    expect(effortWords('medium')).toBe('Medium');
    expect(effortWords('xhigh')).toBe('Extra high');
    expect(effortWords(null)).toBeNull();
  });

  it('a row: when it ended at the right, model and effort, time and tokens under the title', () => {
    // The right: when it ended, live elapsed while running, place while queued.
    expect(rowWhen(row(), AT, at(16 * 3600))).toBe('16h ago');
    expect(rowWhen(row({ status: 'running', workingMs: 1_000 }), AT, at(44))).toBe('45s');
    expect(rowWhen(row({ status: 'queued', queuePlace: 2, workingMs: 0 }), AT, at(10))).toBe('Queued, 2nd');
    expect(rowWhen(row({ status: 'failed' }), AT, at(7200))).toBe('Failed 2h ago');
    expect(rowWhen(row({ status: 'stopped', endedAt: null }), AT, at(0))).toBe('Stopped');
    // The quiet line: a running row's time is at the right, not twice.
    expect(rowMeta(row(), AT, at(16 * 3600), CLAUDE)).toEqual(['Sonnet 5.5 · Medium', '4m 12s', '182K tokens']);
    expect(rowMeta(row({ status: 'running', workingMs: 1_000, model: null, effort: null, tokens: 950 }), AT, at(44), CLAUDE)).toEqual(['Claude', '950 tokens']);
    expect(rowMeta(row({ status: 'queued', queuePlace: 2, workingMs: 0, tokens: 0 }), AT, at(10), CLAUDE)).toEqual(['Sonnet 5.5 · Medium']);
    expect(rowMeta(row({ status: 'failed', tokens: 1 }), AT, at(7200), CLAUDE)).toEqual(['Sonnet 5.5 · Medium', '4m 12s', '1 token']);
    const title = rowTitle(row({ trackerIssue: 'ENG-12' }), AT, at(0));
    expect(title).toContain('worked 4m 12s over 5m 0s');
    expect(title).toContain('150K in (90K cached) · 32K out');
    expect(title).toContain('Issue ENG-12');
  });

  it('a queued row says what holds it, in the pump\'s words (2026-11-12)', () => {
    const held = row({ status: 'queued', queuePlace: 1, workingMs: 0, tokens: 0, waitingFor: 'a free subagent slot: this chat is at its limit of 6 subagents at once (Settings → Chat → Chats at once)' });
    expect(waitingWords(held)).toBe('Waiting for a free subagent slot: this chat is at its limit of 6 subagents at once (Settings → Chat → Chats at once)');
    expect(rowMeta(held, AT, at(10), CLAUDE)).toEqual(['Waiting for a free subagent slot: this chat is at its limit of 6 subagents at once (Settings → Chat → Chats at once)', 'Sonnet 5.5 · Medium']);
    expect(headerMeta(held, AT, at(8), CLAUDE)).toBe('Sonnet 5.5 · Medium · Queued, 1st · Waiting for a free subagent slot: this chat is at its limit of 6 subagents at once (Settings → Chat → Chats at once)');
    expect(rowTitle(held, AT, at(0))).toContain('Queued, 1st · Waiting for a free subagent slot');
    // Nothing to name, or not queued: no words and no stray separator.
    expect(waitingWords(row({ status: 'queued', waitingFor: null }))).toBe('');
    expect(waitingWords(row({ status: 'running', waitingFor: 'x' }))).toBe('');
    expect(rowTitle(row({ status: 'queued', queuePlace: 2, waitingFor: null }), AT, at(0))).toContain('Queued, 2nd');
  });

  it('the header line, live while running', () => {
    expect(headerMeta(row(), AT, at(0), CLAUDE)).toBe('Sonnet 5.5 · Medium · 4m 12s · 182K tokens');
    expect(headerMeta(row({ status: 'running' }), AT, at(8), CLAUDE)).toBe('Sonnet 5.5 · Medium · 4m 20s · 182K tokens');
    expect(headerMeta(row({ status: 'queued', queuePlace: 2, workingMs: 0, tokens: 0 }), AT, at(8), CLAUDE)).toBe('Sonnet 5.5 · Medium · Queued, 2nd');
  });

  it('the CFO line under the header: agent, model, effort and the first line of its reason', () => {
    const decision = { agent: 'claude', model: 'claude-sonnet-5-5', effort: 'medium', effortSource: 'cfo', kind: 'review', reason: 'review of a small diff\nmore detail' } as const;
    expect(cfoHeaderLine(decision, CLAUDE)).toBe('CFO: Claude · Sonnet 5.5 · Medium · review of a small diff');
    expect(cfoHeaderLine({ agent: 'codex', effortSource: 'fixed', kind: 'code', reason: 'code change' }, null)).toBe('CFO: Codex · code change');
    expect(cfoHeaderLine(null, CLAUDE)).toBeNull();
    expect(latestCfo([{ cfo: decision }, { cfo: null }])).toBe(decision);
    expect(latestCfo([])).toBeNull();
  });

  it('finds a row by any of its conversations, and polls faster while active', () => {
    const r = row({ key: 's-a', current: 's-b', conversations: ['s-a', 's-b'] });
    const v = view({ done: [r] });
    expect(findRow(v, 's-b')).toBe(r);
    expect(findRow(v, 's-x')).toBeNull();
    expect(pollMs(view({ running: 1 }))).toBe(3_000);
    expect(pollMs(view({ doneCount: 4 }))).toBe(20_000);
    expect(pollMs(null)).toBe(20_000);
  });

  it('reads start_session and send_to_session calls, with the key from the result', () => {
    expect(firstWords('Review   the company\nplan for gaps in pricing and the overview', 5)).toBe('Review the company plan for…');
    expect(subagentCall('mcp__hollow_app__start_session', { workspace: 'w', prompt: 'Review the pricing page' }, '{"sessionKey":"s-20261110-a","queuePosition":0}'))
      .toEqual({ kind: 'start', label: 'Started a subagent: Review the pricing page', sessionKey: 's-20261110-a' });
    expect(subagentCall('mcp__hollow_app__start_session', { prompt: 'x' }, null)?.sessionKey).toBeNull();
    expect(subagentCall('mcp__hollow_app__run_action', { id: 'sessions.start', args: { prompt: 'Audit the docs' } }, '{"sessionKey":"s-2"}'))
      .toEqual({ kind: 'start', label: 'Started a subagent: Audit the docs', sessionKey: 's-2' });
    expect(subagentCall('mcp__hollow_app__send_to_session', { session: 's-3', prompt: 'Also check the footer' }, 'Error: busy'))
      .toEqual({ kind: 'message', label: 'Messaged a subagent: Also check the footer', sessionKey: 's-3' });
    expect(subagentCall('mcp__hollow_app__run_action', { id: 'tasks.create' }, null)).toBeNull();
    expect(subagentCall('mcp__other__start_session', {}, null)).toBeNull();
    expect(subagentCall('Agent', { description: 'x' }, null)).toBeNull();
  });

  it('the transcript as exchanges, with worked time and History only for changes', () => {
    const cfo = { agent: 'claude', effortSource: 'cfo', kind: 'code', reason: 'code change' };
    const receipt = (startedAt: string, finishedAt: string, files: number) => ({ startedAt, finishedAt, historyRunId: 'h-1', files: Array.from({ length: files }, () => ({})), repoChanges: [], ...(files ? { cfo } : {}) }) as never;
    const list = replies([
      { id: 'u1', role: 'user', text: 'Review the pricing page', error: null, receipt: null },
      { id: 'a1', role: 'assistant', text: 'Reading it.', error: null, receipt: null },
      { id: 'a2', role: 'assistant', text: 'Two gaps.', error: null, receipt: receipt('2026-11-10T12:00:00Z', '2026-11-10T12:00:32Z', 0) },
      { id: 'u2', role: 'user', text: 'Fix them', error: null, receipt: null },
      { id: 'a3', role: 'assistant', text: 'Fixed.', error: null, receipt: receipt('2026-11-10T12:01:00Z', '2026-11-10T12:01:07Z', 2) },
    ]);
    expect(list.map(r => [r.ask, r.answer, r.workedMs, r.historyRunId])).toEqual([
      ['Review the pricing page', 'Reading it.\n\nTwo gaps.', 32_000, null],
      ['Fix them', 'Fixed.', 7_000, 'h-1'],
    ]);
    expect(list.map(r => [r.finishedAt, r.cfo])).toEqual([['2026-11-10T12:00:32Z', null], ['2026-11-10T12:01:07Z', cfo]]);
    expect(replies([{ id: 'a0', role: 'assistant', text: 'Hello', error: 'boom', receipt: null }])[0]).toMatchObject({ ask: null, answer: 'Hello', error: 'boom' });
  });

  it('a reply carries the run id of the turn that answered it, for the thumbs (C10)', () => {
    const list = replies([
      { id: 'u1', role: 'user', text: 'Go', error: null, receipt: null },
      { id: 'a1', role: 'assistant', text: 'Part one.', error: null, receipt: null, runId: 'run-1' },
      { id: 'a2', role: 'assistant', text: 'Part two.', error: null, receipt: null, runId: 'run-2' },
      { id: 'u2', role: 'user', text: 'Again', error: null, receipt: null },
      { id: 'a3', role: 'assistant', text: 'Done.', error: null, receipt: null, runId: null },
    ]);
    // Each run's answer is its own reply, so a vote lands on the run voted on (11-13).
    expect(list.map(r => [r.ask, r.answer, r.runId])).toEqual([['Go', 'Part one.', 'run-1'], [null, 'Part two.', 'run-2'], ['Again', 'Done.', null]]);
    // The row's buttons for a transcript reply: thumbs only with a run id, never Retry or More.
    expect(replyRowButtons({ canVote: canVote({ role: 'assistant', text: list[0].answer, runId: list[0].runId ?? undefined }, null), canRead: true, canRetry: false, hasMore: false }))
      .toEqual(['copy', 'useful', 'notUseful', 'read']);
    expect(replyRowButtons({ canVote: canVote({ role: 'assistant', text: list[2].answer, runId: list[2].runId ?? undefined }, null), canRead: true, canRetry: false, hasMore: false }))
      .toEqual(['copy', 'read']);
  });

  it('a reply knows its turn is still running, and turns of one run stay one reply (11-13)', () => {
    const list = replies([
      { id: 'u1', role: 'user', text: 'Go', error: null, receipt: null, runId: 'run-1', status: 'success' },
      { id: 'a1', role: 'assistant', text: 'Reading.', error: null, receipt: null, runId: 'run-1', status: 'success' },
      { id: 'a1b', role: 'assistant', text: 'Done.', error: null, receipt: null, runId: 'run-1', status: 'success' },
      { id: 'u2', role: 'user', text: 'More', error: null, receipt: null, runId: 'run-2', status: 'running' },
      { id: 'a2', role: 'assistant', text: 'Half an ans', error: null, receipt: null, runId: 'run-2', status: 'running' },
    ]);
    expect(list.map(r => [r.answer, r.runId, r.running])).toEqual([['Reading.\n\nDone.', 'run-1', false], ['Half an ans', 'run-2', true]]);
  });

  it('the transcript hides Copy and Read aloud while its turn streams, and a successor chat stops speech (11-13)', () => {
    expect(PANEL).toContain('<Show when={reply.answer && !streaming(reply)}>');
    expect(PANEL).toContain('<ReplyFeedback reply={reply} settled={!streaming(reply)} />');
    expect(PANEL).toContain('return r.running || (all[all.length - 1] === r && !r.finishedAt);');
    expect(PANEL).toContain('createEffect(on(key, () => stopReading(reader), { defer: true }));');
  });

  it('an earlier answer with no run id is its own reply, so a later run never takes its vote (11-13 review)', () => {
    const list = replies([
      { id: 'u1', role: 'user', text: 'Go', error: null, receipt: null },
      { id: 'a1', role: 'assistant', text: 'Old answer.', error: null, receipt: null, runId: null },
      { id: 'a2', role: 'assistant', text: 'New answer.', error: null, receipt: null, runId: 'run-9' },
    ]);
    expect(list.map(r => [r.ask, r.answer, r.runId])).toEqual([['Go', 'Old answer.', null], [null, 'New answer.', 'run-9']]);
  });

  it('a send says whether it queued', () => {
    expect(sentWords({ heldId: 'held-1', queuePosition: 0 })).toBe('Queued: it goes when the running turn ends');
    expect(sentWords({ queuePosition: 2 })).toBe('Queued, 2nd');
    expect(sentWords({ queuePosition: 0 })).toBe('Sent');
  });

  it('Show older counts what is not listed yet and its rows can be opened', () => {
    const r = { key: 's-old1' } as SubagentRow;
    expect(olderLeft(view({}))).toBe(0);
    expect(olderLeft(view({ olderTotal: 30, doneOlder: [r] }))).toBe(29);
    expect(olderLeft(view({ olderTotal: 1, doneOlder: [r] }))).toBe(0);
    expect(olderLeft(null)).toBe(0);
    expect(findRow(view({ olderTotal: 1, doneOlder: [r] }), 's-old1')).toBe(r);
  });
});

describe('row marks and tooltips', () => {
  it('gives Done and Finished a check and Partial and Blocked the half circle', () => {
    expect(finishedMark(row())).toBe('✓');
    expect(finishedMark(row({ activity: { outcome: 'Done: all' } as SubagentRow['activity'] }))).toBe('✓');
    expect(finishedMark(row({ activity: { outcome: 'Partial: some' } as SubagentRow['activity'] }))).toBe('◐');
    expect(finishedMark(row({ activity: { outcome: 'Blocked: x' } as SubagentRow['activity'] }))).toBe('◐');
    expect(finishedMark(row({ status: 'failed' }))).toBe('!');
  });
  it('carries what is missing in the tooltip, not the label', () => {
    const r = row({ activity: { outcome: 'Partial:  tests not run ' } as SubagentRow['activity'] });
    expect(outcomeWords(r)).toBe('Partial');
    expect(outcomeDetail(r)).toBe('tests not run');
    expect(rowTitle(r, AT, at(0))).toContain('\ntests not run');
    expect(outcomeDetail(row())).toBe('');
  });
  it('keeps a child with its own swarm group in its root team', () => {
    const swarm = { id: 'swarm-9', title: 'Other', parts: 2 };
    const groups = workGroups([row({ key: 'lead', conversations: ['lead'], title: 'Build' }),
      row({ key: 'kid', conversations: ['kid'], parentKey: 'lead', group: swarm })]);
    expect(groups).toHaveLength(1);
    expect(groups[0]).toMatchObject({ kind: 'team', teamKey: 'lead' });
    expect(groups[0].rows.map(r => r.key)).toEqual(['lead', 'kid']);
  });
  it('restarts row numbers when the chat changes', () => {
    const n = createIdentities();
    expect([n('c1', 'a'), n('c1', 'b'), n('c1', 'a')]).toEqual([1, 2, 1]);
    expect(n('c2', 'b')).toBe(1);
  });
  it('names the outcome in the tooltip', () => {
    const t = rowTitle(row({ activity: { outcome: 'Partial: some' } as SubagentRow['activity'] }), AT, at(0));
    expect(t).toContain('Partial · worked');
    expect(t).not.toContain('Done');
  });
  it('says the waiting words once and drops worked for a queued row', () => {
    const q = row({ status: 'queued', queuePlace: 2, waitingFor: 'a run slot', workingMs: 0, wallMs: 0, tokens: 0 });
    const t = rowTitle(q, AT, at(0));
    expect(t).not.toContain('worked');
    expect(t.match(/Waiting for a run slot/g)).toHaveLength(1);
    expect(PANEL).toContain('.filter(m => m !== waitingWords(row()))');
  });
});

// Overnight 11-13: Stop is confirmed in a dialog. While it is open the
// subagent's turn can end and a send_to_session start the next one on the
// same key; the stop goes by the clicked row and never reaches that one.
describe('stopSubagent', () => {
  const item = (id: number, status: RunQueueItem['status']) => ({ id, status }) as RunQueueItem;
  const deps = (during: () => void, rows: Record<number, RunQueueItem>) => {
    const calls = { rows: [] as number[], keys: [] as string[] };
    return {
      calls,
      confirm: async () => { during(); return true; },
      cancelRow: async (id: number) => { calls.rows.push(id); return rows[id]; },
    };
  };

  it('stops by the queue row read when clicked, so a turn that ended meanwhile stops nothing', async () => {
    const rows: Record<number, RunQueueItem> = { 7: item(7, 'running') };
    let current: SubagentRow | null = row({ status: 'running', endedAt: null, queueId: 7 });
    const d = deps(() => {
      rows[7] = item(7, 'done');
      rows[8] = item(8, 'running');
      current = row({ status: 'running', endedAt: null, queueId: 8 });
    }, rows);
    expect(await stopSubagent(() => current, d)).toBe('ended');
    expect(d.calls).toEqual({ rows: [7], keys: [] });
  });

  it('stops the clicked row while it runs, and does nothing when declined', async () => {
    const rows = { 7: item(7, 'running') };
    const r = row({ status: 'running', endedAt: null, queueId: 7 });
    const d = deps(() => {}, rows);
    expect(await stopSubagent(() => r, d)).toBe('stopped');
    expect(await stopSubagent(() => r, { ...d, confirm: async () => false })).toBe('declined');
    expect(d.calls).toEqual({ rows: [7], keys: [] });
  });

  it('a row with no queue row is refused with the notice, never stopped by key', async () => {
    let asked = false;
    const d = deps(() => { asked = true; }, {});
    expect(await stopSubagent(() => row({ status: 'running', endedAt: null }), d)).toBe('noRow');
    expect(asked).toBe(false);
    expect(d.calls).toEqual({ rows: [], keys: [] });
    expect(PANEL).toContain("flashFooter(done === 'noRow' ? NO_ROW_NOTICE");
    expect(PANEL).not.toContain('chatCancel');
  });
});
