/**
 * A chat with subagents for the browser mock (COO rebuild L3): two active
 * (one running, one queued second) and three done, under the demo's Q3
 * conversations, plus the running one's transcript. Seeded through
 * localStorage `mockSubagents` and `mockAgentConversations`, which the demo
 * seed leaves alone. `at` is the read's time: pin the page clock to it for
 * exact live elapsed.
 */
import type { Page } from './fixtures/test';
import type { SubagentRow, SubagentsView } from '../../src/generated/ipc';

/** The demo's Q3 conversations (src/mockDemo.ts): whichever opens first. */
export const Q3_CHATS = ['s-20260930-q3a00001', 's-20260928-q3a00002'];
export const SUBAGENTS_AT = '2026-11-10T15:00:00.000Z';
export const RUNNING = 's-20261110-sub00001';

const ago = (hours: number) => new Date(Date.parse(SUBAGENTS_AT) - hours * 3_600_000).toISOString();

function row(over: Partial<SubagentRow> & Pick<SubagentRow, 'key' | 'title' | 'agent' | 'status'>): SubagentRow {
  return {
    current: over.key, model: null, effort: null, account: null, queuePlace: null, startedAt: ago(17), endedAt: ago(16),
    workingMs: 32_000, wallMs: 40_000, inputTokens: 700, cachedInputTokens: 300, outputTokens: 250, tokens: 950, costUsd: null,
    earlier: false, conversations: [over.key], trackerIssue: null, issue: null, ...over,
  };
}

export function subagentsView(parent: string, older = 0): SubagentsView {
  return {
    parent, parents: [parent], at: SUBAGENTS_AT, running: 1, queued: 1, doneCount: 3,
    active: [
      row({ key: RUNNING, title: 'Check the labour figures', agent: 'claude', model: 'claude-sonnet-5-5', effort: 'medium', status: 'running', startedAt: ago(0.1), endedAt: null,
        workingMs: 252_000, wallMs: 300_000, inputTokens: 150_000, cachedInputTokens: 90_000, outputTokens: 32_000, tokens: 182_000, trackerIssue: 'ENG-301', queueId: 71 }),
      row({ key: 's-20261110-sub00002', title: 'Draft the appendix table', agent: 'codex', model: 'gpt-6-astra', effort: 'high', status: 'queued', queuePlace: 2, endedAt: null,
        workingMs: 0, wallMs: 0, inputTokens: 0, cachedInputTokens: 0, outputTokens: 0, tokens: 0 }),
    ],
    done: [
      row({ key: 's-20261109-sub00003', title: 'Company review', agent: 'claude', model: 'opus', effort: 'high', status: 'done' }),
      row({ key: 's-20261109-sub00004', title: 'Included usage research', agent: 'codex', status: 'done', tokens: 1_200_000, inputTokens: 1_000_000, outputTokens: 200_000 }),
      row({ key: 's-20261109-sub00005', title: 'Company pilot fixture', agent: 'grok', status: 'failed', endedAt: ago(17) }),
    ],
    // The chain's older finished ones: every one is kept here; the mock lists as many as Show older asked for.
    doneOlder: Array.from({ length: older }, (_, i) => row({ key: `s-20261001-old${String(i).padStart(5, '0')}`, title: `Older subagent ${i + 1}`, agent: 'claude', status: 'done', endedAt: ago(24 * 20 + i) })),
    olderTotal: older, earlier: [],
  };
}

/** The running subagent's conversation: one finished exchange that changed a file. */
export function runningConversation() {
  const summary = { key: RUNNING, title: 'Check the labour figures', agent: 'claude', harnessSessionId: 'claude-sub-1', cwd: '/Users/fixture/vault/workspaces/local-q3-review',
    createdAt: ago(0.1), lastActivityAt: ago(0.05), endedAt: null, turnCount: 2, parentKey: Q3_CHATS[0], forkKind: 'subagent', adopted: false, stints: [], resumable: 'native' };
  const receipt = { id: 'rcpt-sub-1', runId: 'run-sub-1', source: 'chat', sourceId: 'claude-sub-1', startedAt: ago(0.1), finishedAt: new Date(Date.parse(ago(0.1)) + 32_000).toISOString(),
    status: 'success', verificationState: 'unverified', checks: [], failures: [], historyRunId: 'run-sub-1', sessionKey: RUNNING,
    cfo: { agent: 'claude', model: 'claude-sonnet-5-5', effort: 'medium', effortSource: 'cfo', kind: 'review', reason: 'checks figures against a source' },
    files: [{ path: 'workspaces/local-q3-review/outputs/q3-regional-review.md', changeType: 'modify', attribution: 'confirmed', tool: 'Edit', additions: 3, deletions: 1 }] };
  return {
    summary, eventsRetained: true, permissionMode: 'acceptEdits',
    turns: [
      { id: 'sub-u1', role: 'user', text: 'Check the labour figures in section 3 against the September close.', tools: [], agent: 'claude', runId: 'run-sub-1', startedAt: ago(0.1), status: null, costUsd: null, error: null, receipt: null, transcriptIncomplete: false },
      { id: 'sub-a1', role: 'assistant', text: 'Section 3 matches the close except **one overtime figure**, which I corrected.', tools: [], agent: 'claude', runId: 'run-sub-1', startedAt: ago(0.1), status: 'success', costUsd: null, error: null, receipt, transcriptIncomplete: false },
    ],
  };
}

/** Seed before the app loads: every Q3 conversation has these subagents. */
export async function seedSubagents(page: Page, older = 0) {
  const views = Object.fromEntries(Q3_CHATS.map(k => [k, subagentsView(k, older)]));
  await page.addInitScript(([v, c]) => {
    localStorage.setItem('mockSubagents', v);
    localStorage.setItem('mockAgentConversations', c);
  }, [JSON.stringify(views), JSON.stringify({ [RUNNING]: runningConversation() })] as const);
}
