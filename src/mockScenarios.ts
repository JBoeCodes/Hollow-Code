/**
 * Named whole-app scenarios for the browser mock (plan completion L5, review
 * V07 / roadmap B, matrix V7a). `?mock=1&demo=1&scenario=interrupted` boots
 * the populated demo in that state; several combine with commas
 * (`scenario=disconnected,stale`). Each scenario only sets the mock's
 * existing knobs (localStorage keys and `window.__mockScenario`), so the real
 * UI code paths run unchanged, and a reload re-applies the state because the
 * URL keeps it (one-time notices the app marks seen, such as a restored
 * queued message, don't show twice). Mock-only: main.tsx imports this lazily, and only in mock mode.
 *
 * The contract (which state, which URL, where it shows, which spec proves
 * it) is docs/FIXTURES.md; `tests/smoke/fixture-states.spec.ts` asserts one
 * visible state per scenario and `fixture-parity.spec.ts` mounts every
 * Settings section in each without a page error.
 */
import type { AgentConversation } from './types';

/** Per-command behaviour a scenario adds to `mockInvoke`. */
export interface MockScenarioHooks {
  /** Answer these commands only after this many milliseconds (the loading state). */
  delayMs: Record<string, number>;
  /** Reject these commands with this message, as Tauri rejects (a plain string). */
  unavailable: Record<string, string>;
}

declare global {
  interface Window { __mockScenario?: MockScenarioHooks & { names: string[] } }
}

export interface MockScenario {
  /** What the person sees, in a sentence. */
  summary: string;
  /** The states from the review's matrix this scenario covers. */
  states: readonly ScenarioState[];
  apply(hooks: MockScenarioHooks): void;
}

export type ScenarioState =
  | 'empty' | 'populated' | 'large' | 'duplicate-name' | 'unavailable' | 'permission-required'
  | 'loading' | 'stale' | 'partial-success' | 'conflict' | 'disconnected' | 'interrupted';

const H = 3_600_000;
const iso = (msAgo: number) => new Date(Date.now() - msAgo).toISOString();
const set = (key: string, value: unknown) => localStorage.setItem(key, typeof value === 'string' ? value : JSON.stringify(value));
/** Add needs-you rows to whatever another scenario already seeded. */
function addNeedsYou(rows: Record<string, unknown>[]): void {
  const have = JSON.parse(localStorage.getItem('mockNeedsYouExtra') ?? '[]') as { id: string }[];
  set('mockNeedsYouExtra', [...have.filter(h => !rows.some(r => r.id === h.id)), ...rows]);
}
function addUnreadable(sources: string[]): void {
  const have = JSON.parse(localStorage.getItem('mockNeedsYouUnavailable') ?? '[]') as string[];
  set('mockNeedsYouUnavailable', [...new Set([...have, ...sources])]);
}
const needsYou = (id: string, kind: string, title: string, extra: Record<string, unknown> = {}) =>
  ({ id, kind, sourceId: id.split(':')[1] ?? id, title, detail: null, state: 'pending', at: iso(H), workspaceId: null, sessionKey: null, path: null, category: null, link: null, ...extra });

/** The demo's report workspace and its newest chat (src/mockDemo.ts). */
const REPORT = 'local-q3-review', REPORT_CHAT = 's-20260930-q3a00001';

export const SCENARIOS: Record<string, MockScenario> = {
  disconnected: {
    summary: 'Every outside service is unreachable or signed out: Microsoft 365 (calendar), the companion backend, a claude.ai connector, Linear sync and web capture.',
    states: ['disconnected', 'unavailable', 'permission-required'],
    apply() {
      set('mockCalendarOff', '1');
      localStorage.removeItem('mockM365');
      set('mockBuiltins', '1');
      set('mockConnectors', [{ id: 'microsoft_365', name: 'Microsoft 365', description: '', transport: 'claudeAi', command: '', args: [], url: 'https://microsoft365.mcp.claude.com/mcp', serverId: '', iconUrl: '', envKeys: [], headerKeys: [], agents: ['claude'], chat: true }]);
      set('mockCompanionBackend', {
        url: 'https://hollow-companion.example.ts.net', connected: false, lastContactAt: iso(26 * H),
        // The bridge's own words for no answer (companion_bridge.rs `unreachable_words`).
        lastError: "Hollow couldn't reach hollow-companion.example.ts.net. Check the address, that the backend is running and that this Mac can reach it. If the backend is on your tailnet, check that Tailscale is running and connected on this Mac.",
        mirrorError: null, mirroredTasks: 12, lastPushAt: iso(26 * H), mirroredDocs: null, docsError: null, docsSupported: null, scope: null, scopeError: null,
      });
      addUnreadable(['Linear sync']);
      set('mockIngestFail', '1');
    },
  },
  interrupted: {
    summary: 'Hollow quit while work ran: a chat to resume, its unsent follow-up, an interrupted turn with Continue, and an interrupted automation in the run queue.',
    states: ['interrupted'],
    apply() {
      const askedAt = iso(0.5 * H);
      set('mockInterruptedChats', [{ sessionKey: REPORT_CHAT, title: 'Revise the labour section', agent: 'claude', workspaceId: REPORT, askedAt, model: null, permissionMode: 'acceptEdits', agentStarted: false }]);
      addNeedsYou([needsYou(`interrupted:${askedAt}`, 'interrupted', 'Resume 1 interrupted chat', { detail: 'Hollow quit while they ran: Revise the labour section', at: askedAt })]);
      const row = (id: number, source: string, sourceId: string, label: string, extra: Record<string, unknown> = {}) => ({
        id, source, sourceId, label, detail: null, reason: source === 'chat' ? 'chat' : 'schedule', status: 'interrupted', priority: 0, position: null, attempt: 1, coalescedCount: 0,
        requestedAt: iso(0.6 * H), claimedAt: iso(0.55 * H), finishedAt: iso(0.5 * H), historyRunId: null, error: 'Hollow quit before this finished', waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false, waitingReason: null, ...extra,
      });
      set('mockRunQueue', [
        row(51, 'cron', 'morning-brief', 'Morning brief'),
        row(52, 'chat', REPORT_CHAT, 'Revise the labour section', { detail: 'Also add the overtime table to the appendix.', attempt: 0, claimedAt: null }),
      ]);
      // The chat's last turn ended mid-run (needs the demo's conversations).
      const all = JSON.parse(localStorage.getItem('mockWorkspaceConversations') ?? '{}') as Record<string, AgentConversation>;
      const convo = all[REPORT_CHAT];
      if (convo?.turns.length) {
        const last = convo.turns[convo.turns.length - 1];
        convo.turns[convo.turns.length - 1] = { ...last, status: 'interrupted', text: `${last.text}\n\nReading the September close…`, costUsd: null };
        // A conversation that ran records its permission mode, so it resumes
        // its native session and Continue is offered (permModes.ts
        // conversationPermission; without one it continues by handoff).
        convo.permissionMode = 'acceptEdits';
        set('mockWorkspaceConversations', all);
      }
    },
  },
  'permission-required': {
    summary: 'Work waits on the person: an automation held for a permission review, a tool approval, and a connector that needs signing in again.',
    states: ['permission-required'],
    apply() {
      set('mockRefusedWorkflows', ['morning-brief']);
      addNeedsYou([needsYou('approval:ap-1', 'approval', 'Allow “git push” in Results view for workspaces?', { detail: 'The agent asked to push the branch feature/results-view.', sessionKey: 's-20261001-bld00001', category: 'approval' })]);
      set('mockConnectors', [{ id: 'microsoft_365', name: 'Microsoft 365', description: '', transport: 'claudeAi', command: '', args: [], url: 'https://microsoft365.mcp.claude.com/mcp', serverId: '', iconUrl: '', envKeys: [], headerKeys: [], agents: ['claude'], chat: true }]);
    },
  },
  stale: {
    summary: 'Things are out of date: a workspace brief unchanged while chats ran, a question the agent can no longer use, and Home cards kept from the last good read.',
    states: ['stale'],
    apply() {
      const reentry = JSON.parse(localStorage.getItem('mockWorkspaceReentry') ?? '{}') as Record<string, Record<string, unknown>>;
      if (reentry[REPORT]) { reentry[REPORT] = { ...reentry[REPORT], briefStale: { days: 12, conversations: 5 } }; set('mockWorkspaceReentry', reentry); }
      addNeedsYou([needsYou('question:q-stale', 'question', 'Keep the old heading?', { detail: 'The file changed', state: 'stale', sessionKey: REPORT_CHAT, category: 'question' })]);
    },
  },
  'partial-success': {
    summary: 'Reads that half worked: a calendar whose second page failed and a review list that couldn’t read two of its sources.',
    states: ['partial-success'],
    apply() {
      set('mockCalendarPartial', '1');
      addUnreadable(['questions', 'Linear sync']);
    },
  },
  conflict: {
    summary: 'Two writers disagreed: a task changed in two places, and conflict copies iCloud made in the vault.',
    states: ['conflict'],
    apply() {
      addNeedsYou([
        needsYou('conflict:task-1', 'conflict', 'Send the draft to Dana changed in Linear and here', { detail: 'Due date: Friday here, Monday in Linear', category: 'task', path: 'tasks/send-the-draft-to-dana.md' }),
        needsYou('copies:vault', 'copies', '2 conflict copies in the vault', { detail: 'notes/Ideas (1).md (same as the original); notes/Plan 2.md', at: null }),
      ]);
    },
  },
  loading: {
    summary: 'A slow Mac: the workspace list, Home’s cards, the review list and the task list each take 4 seconds to answer.',
    states: ['loading'],
    apply(hooks) {
      for (const c of ['list_workspaces', 'list_needs_you', 'tasks', 'list_all_results', 'list_all_results_peek']) hooks.delayMs[c] = 4_000;
    },
  },
  'library-scale': {
    summary: 'The real library’s size (the review saw 213 workspaces): exactly 213 workspaces, the same name in a project folder, a Hollow workspace, a Notion mirror and a build, twice in one place, titles longer than a row, a conversation in most of them, and a busy review list. Use on `demo=large` (and `missions=1` for running work).',
    states: ['large', 'duplicate-name'],
    apply() {
      const at = (h: number) => iso(h * H);
      const vault = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
      const ws = (id: string, title: string, cwd: string, hoursAgo: number, extra: Record<string, unknown> = {}) =>
        ({ id, title, status: 'active', path: `workspaces/${id}`, cwd, modified: at(hoursAgo), ...extra });
      const long = 'Quarterly regional operations review for the Northeast, Mid-Atlantic and Southeast parking portfolios, with the lease renewals';
      const have = JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { id: string; title: string; cwd: string }[];
      const added = [
        ws('local-q3-folder', 'Q3 review', '/Users/fixture/Projects/q3-review', 0.8),
        ws('q3-hollow', 'Q3 review', vault('q3-hollow'), 0.7),
        ws('notion-0123456789abcdef0123456789abcdef', 'Q3 review', vault('notion-q3'), 0.6),
        ws('q3-build', 'Q3 review', '/Users/fixture/Hollow/Worktrees/q3/q3-review', 0.5, { build: { repo: '/Users/fixture/Projects/q3', worktree: '/Users/fixture/Hollow/Worktrees/q3/q3-review', branch: 'feature/q3-review', base: 'origin/main', exists: true } }),
        ws('budget-east', 'Budget', vault('budget-east'), 1),
        ws('budget-west', 'Budget', vault('budget-west'), 1.1),
        ws('long-hollow', long, vault('long-hollow'), 0.4),
        ws('notion-fedcba9876543210fedcba9876543210', long, vault('notion-long'), 0.3),
      ];
      // The same-named set first, so trimming to 213 never drops it.
      const all = [...added, ...have.filter(w => !added.some(a => a.id === w.id))];
      for (let i = 0; all.length < 213; i++) all.push(ws(`site-${String(i).padStart(3, '0')}`, `Site ${200 + i} staffing plan`, vault(`site-${i}`), 500 + i));
      all.length = 213;
      set('mockWorkspaces', all);
      // A conversation in most workspaces (the native library had a large imported backlog).
      const sessions = JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]') as { key: string }[];
      for (const [i, w] of all.entries()) {
        if (i % 4 === 3) continue;
        const key = `s-20261013-scl${String(i).padStart(5, '0')}`;
        if (sessions.some(x => x.key === key)) continue;
        sessions.push({ key, title: `Notes from ${w.title.slice(0, 40)}`, agent: i % 3 ? 'claude' : 'codex', harnessSessionId: null, cwd: w.cwd, createdAt: at(24 + i), lastActivityAt: at(2 + i), endedAt: null, turnCount: 2, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' } as never);
      }
      set('mockWorkspaceSessions', sessions);
      // Simultaneous events: a long review list.
      addNeedsYou(Array.from({ length: 30 }, (_, i) => needsYou(`question:scale-${i}`, 'question', `Question ${i + 1} from a running chat`, { sessionKey: `s-20261013-scl${String(i * 4).padStart(5, '0')}`, category: 'question', at: iso((i + 1) * 60_000) })));
    },
  },
  subagents: {
    summary: 'The demo report chat started four subagents, all done: the side panel card says "4 done", its Subagents tab lists them, and one opens its transcript with the model in the header.',
    states: ['populated'],
    apply() {
      const at = iso(0);
      const ago = (h: number) => iso(h * H);
      const row = (key: string, title: string, agent: string, model: string | null) => ({
        key, current: key, title, agent, model, effort: model ? 'high' : null, account: null, status: 'done', queuePlace: null, waitingFor: null,
        startedAt: ago(73), endedAt: ago(72), workingMs: 70_000, wallMs: 90_000, inputTokens: 120_000, cachedInputTokens: 80_000, outputTokens: 9_000, tokens: 129_000,
        costUsd: null, earlier: false, conversations: [key], trackerIssue: null, issue: null, freshFrom: null,
      });
      const done = [
        row('s-20261108-sub00011', 'Map review', 'codex', 'gpt-6-astra'),
        row('s-20261108-sub00012', 'Hollow context', 'claude', 'claude-sonnet-5-5'),
        row('s-20261108-sub00013', 'Delta docs', 'claude', null),
        row('s-20261108-sub00014', 'Delta source', 'grok', null),
      ];
      const view = (parent: string) => ({ parent, parents: [parent], active: [], done, doneOlder: [], olderTotal: 0, earlier: [], running: 0, queued: 0, doneCount: 4, at });
      set('mockSubagents', Object.fromEntries([REPORT_CHAT, 's-20260928-q3a00002'].map(k => [k, view(k)])));
      const key = 's-20261108-sub00012';
      const receipt = { id: 'rcpt-sub-12', runId: 'run-sub-12', source: 'chat', sourceId: 'claude-sub-12', startedAt: ago(73), finishedAt: ago(72), status: 'success',
        verificationState: 'unverified', checks: [], failures: [], historyRunId: 'run-sub-12', sessionKey: key,
        cfo: { agent: 'claude', model: 'claude-sonnet-5-5', effort: 'high', effortSource: 'cfo', kind: 'research', reason: 'reads the repo for context' }, files: [] };
      const conversation = {
        summary: { key, title: 'Hollow context', agent: 'claude', harnessSessionId: 'claude-sub-12', cwd: '/Users/fixture/vault/workspaces/local-q3-review',
          createdAt: ago(73), lastActivityAt: ago(72), endedAt: ago(72), turnCount: 2, parentKey: REPORT_CHAT, forkKind: 'subagent', adopted: false, stints: [], resumable: 'native' },
        eventsRetained: true, permissionMode: 'acceptEdits',
        turns: [
          { id: 'sub12-u1', role: 'user', text: 'Read the regional figures and say which sections of the review they support.', tools: [], agent: 'claude', runId: 'run-sub-12', startedAt: ago(73), status: null, costUsd: null, error: null, receipt: null, transcriptIncomplete: false },
          { id: 'sub12-a1', role: 'assistant', text: 'The figures support **sections 2 and 3**. Section 4 cites a September number the close later revised, so it needs a check before the review goes out.', tools: [], agent: 'claude', runId: 'run-sub-12', startedAt: ago(73), status: 'success', costUsd: null, error: null, receipt, transcriptIncomplete: false },
        ],
      };
      const have = JSON.parse(localStorage.getItem('mockAgentConversations') ?? '{}') as Record<string, unknown>;
      set('mockAgentConversations', { ...have, [key]: conversation });
    },
  },
  unavailable: {
    summary: 'Stores Hollow couldn’t read: the Results review state was damaged and moved aside, the session store is unreachable, and the tasks folder can’t be read.',
    states: ['unavailable'],
    apply(hooks) {
      set('mockResultReviewsDamaged', '1');
      set('mockArtifactSessionsError', '1');
      hooks.unavailable.tasks = 'Couldn’t read the tasks folder: the vault isn’t readable right now (it may be offloaded by iCloud).';
    },
  },
};

/** Apply the scenarios named in `names` (comma-separated). Unknown names are
 *  reported in the console and ignored, so a typo shows up rather than
 *  silently testing the plain demo. */
export function applyScenarios(names: string | null): void {
  if (!names) return;
  const hooks: MockScenarioHooks = { delayMs: {}, unavailable: {} };
  const applied: string[] = [];
  for (const name of names.split(',').map(n => n.trim()).filter(Boolean)) {
    const s = SCENARIOS[name];
    if (!s) { console.warn(`mock scenario "${name}" doesn't exist; known: ${Object.keys(SCENARIOS).join(', ')}`); continue; }
    s.apply(hooks);
    applied.push(name);
  }
  window.__mockScenario = { names: applied, ...hooks };
}

/** What a scenario adds to one mock command: a delay, or a rejection. */
export async function scenarioHook(command: string): Promise<void> {
  const s = typeof window === 'undefined' ? undefined : window.__mockScenario;
  if (!s) return;
  const delay = s.delayMs[command];
  if (delay) await new Promise(resolve => setTimeout(resolve, delay));
  const message = s.unavailable[command];
  if (message) throw message;
}
