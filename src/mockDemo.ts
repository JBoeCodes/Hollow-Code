/**
 * Realistic browser-mock fixtures for design review and performance checks
 * (next-level P01/P21). `?mock=1&demo=1` seeds a populated, believable
 * workspace set; `?mock=1&demo=large` adds the scale datasets the
 * verification contract asks for (200 workspaces, 5,000 tasks, 10,000 Library
 * notes). Mock-only: main.tsx imports this lazily, and only in mock mode.
 *
 * Everything is written to the same localStorage keys the existing mocks
 * read (mockWorkspaces, mockWorkspaceNotes, …), so the real UI code paths run
 * unchanged. Nothing here describes the user's actual data.
 */
import type { AgentConversation, AgentConversationTurn, AgentSessionSummary, Task, VaultItem } from './types';
import type { EvidenceReceiptWire } from './executionReceipt';
import type { WorkWorkspace, WorkspaceReentry } from './workspaceTypes';

const H = 3_600_000;
const iso = (msAgo: number) => new Date(Date.now() - msAgo).toISOString();
// Local calendar date, like the Agenda's dateKey: toISOString() is UTC, so in
// the evening west of Greenwich "today" became tomorrow and the due-today
// fixtures fell off the Agenda (CI runs with TZ=America/New_York).
const day = (offset: number) => { const d = new Date(); d.setDate(d.getDate() + offset); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`; };

interface Seed {
  workspaces: WorkWorkspace[];
  notes: Record<string, string>;
  times: Record<string, string>;
  sessions: AgentSessionSummary[];
  conversations: Record<string, AgentConversation>;
  reentry: Record<string, WorkspaceReentry>;
  changes: Record<string, [number, number]>;
}

function session(key: string, cwd: string, title: string, agent: string, hoursAgo: number, turns = 4): AgentSessionSummary {
  return { key, title, agent, harnessSessionId: `${agent}-${key}`, cwd, createdAt: iso((hoursAgo + 3) * H), lastActivityAt: iso(hoursAgo * H), endedAt: null, turnCount: turns, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'native' } as AgentSessionSummary;
}

function turn(id: string, role: 'user' | 'assistant', text: string, agent: string, at: string, extra: Partial<AgentConversationTurn> = {}): AgentConversationTurn {
  return { id, role, text, tools: [], agent, runId: `run-${id.split('-')[0]}`, startedAt: at, status: 'success', costUsd: role === 'assistant' ? 0.18 : null, error: null, receipt: null, transcriptIncomplete: false, ...(role === 'assistant' ? { endedAt: at } : {}), ...extra };
}

const REPORT_BRIEF = `---
hollow_workspace: 1
title: "Q3 regional operations review"
status: active
created: 2026-09-22T14:00:00Z
---

# Q3 regional operations review

## Outcome

A board-ready review of Q3 performance for the Northeast and Mid-Atlantic regions, with three decisions the leadership team needs to make before October 15.

## Current context

The draft review is in outputs/q3-regional-review.md and the variance workbook is attached. Mid-Atlantic labour cost is 6.2% over plan; Northeast revenue is ahead by 3.1%. Finance confirmed the September close on October 1.

## Next step

Revise section 3 with the confirmed September close, then send the draft to Dana for review before Friday.

## Decisions and constraints

- 2026-09-24 · Use finance's closed numbers, not the operating dashboard. The dashboard lags by a week.
- 2026-09-26 · Keep the review to six pages; supporting tables go in the appendix.
`;

const REPORT_PLAN = `# Plan

- [x] Gather the regional P&L and staffing exports
- [x] Draft sections 1–2 (summary, revenue)
- [ ] Revise section 3 with the September close
- [ ] Add the overtime analysis to the appendix
- [ ] Send the draft to Dana for review
- [ ] Prepare the three decision slides
`;

const REPORT_OUTPUT = `# Q3 regional operations review

*Draft 3 · prepared for the leadership team*

## 1. Summary

Q3 closed **1.4% ahead of plan** on revenue across both regions, while labour cost ran **4.8% over plan**. The gap is concentrated in the Mid-Atlantic, where overtime doubled during the August staffing shortfall.

| Region | Revenue vs plan | Labour vs plan | Margin |
| --- | --- | --- | --- |
| Northeast | +3.1% | +2.9% | 18.4% |
| Mid-Atlantic | −0.6% | +6.2% | 14.9% |
| **Total** | **+1.4%** | **+4.8%** | **16.8%** |

## 2. Revenue

Northeast growth came from three new hospital contracts that started in July. Mid-Atlantic revenue was flat; two airport locations renegotiated rates down in August.

## 3. Labour cost

Overtime accounts for 70% of the Mid-Atlantic overrun. Hiring caught up in mid-September, and the run rate for the last two weeks is back within 1% of plan.

> Open question: does finance's September close change the overtime figure? The draft uses the operating dashboard.

## 4. Decisions needed

1. Approve two permanent float supervisors for the Mid-Atlantic.
2. Decide whether to reprice the two renegotiated airport contracts at renewal.
3. Confirm the Q4 hiring plan before October 15.
`;

const BUILD_BRIEF = `---
hollow_workspace: 1
title: "Results view for workspaces"
status: active
---

# Results view for workspaces

## Outcome

One Results view inside each workspace that joins outputs, conversation files and boards, with review state kept separate from checks.

## Current context

Branch hollow/results-view has the store and the list. The preview pane and accept flow are next.

## Next step

Finish the accept flow and run the smoke suite.
`;

function demoSeed(): Seed {
  const workspaces: WorkWorkspace[] = [];
  const notes: Record<string, string> = {};
  const times: Record<string, string> = {};
  const sessions: AgentSessionSummary[] = [];
  const conversations: Record<string, AgentConversation> = {};
  const reentry: Record<string, WorkspaceReentry> = {};
  const changes: Record<string, [number, number]> = {};

  const add = (w: WorkWorkspace, files: Record<string, [string, number]>) => {
    workspaces.push(w);
    for (const [name, [text, hoursAgo]] of Object.entries(files)) {
      notes[`${w.path}/${name}`] = text;
      times[`${w.path}/${name}`] = iso(hoursAgo * H);
    }
  };

  // ── A report workspace with a real result, a plan and sources.
  const report: WorkWorkspace = { id: 'local-q3-review', title: 'Q3 regional operations review', status: 'active', path: 'workspaces/local-q3-review', cwd: '/Users/fixture/vault/workspaces/local-q3-review', modified: iso(2 * H) };
  add(report, {
    'WORKSPACE.md': [REPORT_BRIEF, 20],
    'PLAN.md': [REPORT_PLAN, 20],
    'SOURCES.md': ['# Sources\n\n- regional-pnl-q3.csv · finance export, captured 2026-09-29\n- staffing-overtime.md · operations notes\n', 50],
    'sources/regional-pnl-q3.csv': ['region,month,revenue,plan_revenue,labour,plan_labour\nNortheast,Jul,1820000,1760000,742000,720000\nNortheast,Aug,1855000,1790000,760000,738000\nNortheast,Sep,1901000,1850000,779000,758000\nMid-Atlantic,Jul,1410000,1420000,611000,580000\nMid-Atlantic,Aug,1392000,1405000,640000,592000\nMid-Atlantic,Sep,1420000,1425000,617000,590000\n', 50],
    'sources/staffing-overtime.md': ['# Staffing and overtime notes\n\nAugust shortfall: 14 open roles in the Mid-Atlantic. Overtime hours peaked at 2,310 in week 34.\n', 52],
    'outputs/q3-regional-review.md': [REPORT_OUTPUT, 2],
    'outputs/decision-slides.md': ['# Decisions for October 15\n\n1. Float supervisors\n2. Airport repricing\n3. Q4 hiring plan\n', 26],
  });
  const r1 = 's-20260930-q3a00001', r2 = 's-20260928-q3a00002';
  sessions.push(session(r1, report.cwd, 'Revise the labour section', 'claude', 2, 6), session(r2, report.cwd, 'First draft of the review', 'codex', 50, 9));
  const at = iso(2.2 * H);
  conversations[r1] = {
    summary: sessions[0], eventsRetained: true, turns: [
      turn('r1a-user', 'user', 'Read the variance workbook and revise section 3 of the review. Keep it to two paragraphs and flag anything that depends on the September close.', 'claude', at),
      turn('r1a-assistant', 'assistant', 'I revised **section 3 · Labour cost** in `outputs/q3-regional-review.md`.\n\n- Overtime explains 70% of the Mid-Atlantic overrun (2,310 hours at the week-34 peak).\n- The last two weeks of September run within 1% of plan.\n\nOne figure still comes from the operating dashboard rather than finance\'s close, so I marked it as an open question in the draft.', 'claude', at, {
        tools: [
          { id: 't1', name: 'Read', input: { file_path: 'sources/regional-pnl-q3.csv' }, result: 'region,month,revenue…', isError: false, status: 'done' },
          { id: 't2', name: 'Edit', input: { file_path: 'outputs/q3-regional-review.md' }, result: 'Updated section 3', isError: false, status: 'done' },
        ],
        receipt: <EvidenceReceiptWire>{ id: 'rcpt-r1a', runId: 'run-r1a', source: 'chat', sourceId: 'claude-r1', startedAt: at, finishedAt: iso(2 * H), status: 'success', verificationState: 'unverified', files: [{ path: 'workspaces/local-q3-review/outputs/q3-regional-review.md', changeType: 'modify', attribution: 'confirmed', tool: 'Edit', additions: 14, deletions: 9 }], checks: [], failures: [], historyRunId: 'run-r1a', sessionKey: r1 } ,
      }),
      turn('r1b-user', 'user', 'Good. What else is left before I send it to Dana?', 'claude', iso(2.1 * H)),
      turn('r1b-assistant', 'assistant', 'Two things from the plan are open:\n\n1. **Revise section 3 with the September close** once finance publishes the closed overtime figure.\n2. **Add the overtime analysis to the appendix**, which needs the weekly hours table from the staffing notes.\n\nThe decision slides are drafted in `outputs/decision-slides.md` and match section 4.', 'claude', iso(2 * H)),
    ],
  };
  conversations[r2] = { summary: sessions[1], eventsRetained: false, turns: [
    turn('r2a-user', 'user', 'Draft the Q3 review from the attached exports.', 'codex', iso(52 * H)),
    turn('r2a-assistant', 'assistant', 'The first draft is in `outputs/q3-regional-review.md` with sections 1–4.', 'codex', iso(50 * H)),
  ] };
  reentry[report.id] = {
    asOf: iso(0), nextStep: 'Revise section 3 with the confirmed September close, then send the draft to Dana for review before Friday.',
    openItems: ['Revise section 3 with the September close', 'Add the overtime analysis to the appendix', 'Send the draft to Dana for review'], openItemCount: 4,
    lastRun: { sessionKey: r1, status: 'success', finishedAt: iso(2 * H), failedChecks: 0, checkCount: 0, unknownChecks: null, verification: 'unverified' }, gaps: [],
  };

  // ── A build workspace with a branch, a PR and a failed check.
  const build: WorkWorkspace = { id: 'work-results-view', title: 'Results view for workspaces', status: 'active', path: 'workspaces/work-results-view', cwd: '/Users/fixture/Hollow/Worktrees/hollow/results-view', modified: iso(0.5 * H), build: { repo: '/Users/fixture/Projects/hollow', worktree: '/Users/fixture/Hollow/Worktrees/hollow/results-view', branch: 'hollow/results-view', base: 'origin/main', exists: true } };
  add(build, { 'WORKSPACE.md': [BUILD_BRIEF, 5], 'PLAN.md': ['# Plan\n\n- [x] Results store\n- [x] Results list\n- [ ] Preview and accept\n- [ ] Smoke suite\n', 5], 'SOURCES.md': ['# Sources\n', 30] });
  const b1 = 's-20261001-bld00001', b2 = 's-20260930-bld00002';
  sessions.push(session(b1, build.cwd, 'Accept flow and revisions', 'claude', 0.4, 12), session(b2, build.cwd, 'Results store and list', 'codex', 20, 7));
  changes[b1] = [412, 86];
  conversations[b1] = { summary: sessions[2], eventsRetained: true, turns: [
    turn('b1a-user', 'user', 'Wire the accept action and make an edit after acceptance create a new revision.', 'claude', iso(1 * H)),
    turn('b1a-assistant', 'assistant', 'Accepting now stores the content hash; a later edit shows **Edited after acceptance** and needs a new review.\n\nThe unit tests pass. One smoke test fails: the preview pane doesn’t reload after an external edit.', 'claude', iso(0.4 * H), {
      receipt: <EvidenceReceiptWire>{ id: 'rcpt-b1a', runId: 'run-b1a', source: 'chat', sourceId: 'claude-b1', startedAt: iso(1 * H), finishedAt: iso(0.4 * H), status: 'success', verificationState: 'failed', files: [{ path: 'src/results.ts', changeType: 'modify', attribution: 'confirmed', tool: 'Edit', additions: 120, deletions: 31 }, { path: 'src/components/ResultsView.tsx', changeType: 'create', attribution: 'confirmed', tool: 'Write', additions: 292, deletions: 0 }], checks: [{ command: 'npx vitest run', kind: 'test', outcome: 'passed', sequence: 4, excerpt: '664 passed' }, { command: 'npx playwright test results.spec.ts', kind: 'test', outcome: 'failed', sequence: 6, excerpt: '1 failed' }], failures: [{ label: 'playwright results.spec.ts', sequence: 6, excerpt: 'preview stays stale after an external edit' }], historyRunId: 'run-b1a', sessionKey: b1 } as never,
    }),
  ] };
  reentry[build.id] = { asOf: iso(0), nextStep: 'Finish the accept flow and run the smoke suite.', openItems: ['Preview and accept', 'Smoke suite'], openItemCount: 2, lastRun: { sessionKey: b1, status: 'success', finishedAt: iso(0.4 * H), failedChecks: 1, checkCount: 2, unknownChecks: 0, verification: 'failed' }, gaps: [] };

  // ── Quieter workspaces, so the list has texture.
  const more: [string, string, string, number, string?][] = [
    ['local-hiring-plan', 'Q4 hiring plan', 'active', 26, 'Confirm the float supervisor budget with finance.'],
    ['notion-a1590a7e000000000000000000000000', 'Airport contract renewals', 'active', 74, 'Draft the pricing options for the two renegotiated contracts.'],
    ['local-board-pack', 'October board pack', 'planning', 98],
    ['local-vendor-review', 'Cloud vendor security review', 'blocked', 140, 'Waiting for the security questionnaire from two vendors.'],
    ['local-onboarding-guide', 'New site manager onboarding guide', 'active', 210],
    ['local-ev-charging', 'EV charging pilot', 'done', 400],
    ['local-safety-audit', 'Annual safety audit', 'active', 330, 'Schedule the walkthroughs for the three flagged garages.'],
  ];
  for (const [id, title, status, hoursAgo, next] of more) {
    const w: WorkWorkspace = { id, title, status, path: `workspaces/${id}`, cwd: `/Users/fixture/vault/workspaces/${id}`, modified: iso(hoursAgo * H) };
    add(w, { 'WORKSPACE.md': [`---\nhollow_workspace: 1\ntitle: ${JSON.stringify(title)}\nstatus: ${status}\n---\n\n# ${title}\n\n## Outcome\n\n${title}.\n\n## Next step\n\n${next ?? 'Start with the request.'}\n`, hoursAgo], 'PLAN.md': ['# Plan\n\n- [ ] Gather the inputs\n- [ ] Draft the result\n', hoursAgo] });
    if (hoursAgo < 250) {
      const key = `s-demo-${id.slice(-8)}`;
      sessions.push(session(key, w.cwd, `Working on ${title.toLowerCase()}`, hoursAgo % 2 ? 'codex' : 'claude', hoursAgo, 3));
    }
    if (next) reentry[id] = { asOf: iso(0), nextStep: next, openItems: ['Gather the inputs', 'Draft the result'], openItemCount: 2, lastRun: null, gaps: [] };
  }
  // An interrupted run, so the re-entry row has something to say.
  reentry['local-hiring-plan'].lastRun = { sessionKey: 's-demo-ing-plan', status: 'cancelled', finishedAt: iso(26 * H), failedChecks: null, checkCount: null, unknownChecks: null, verification: null };
  return { workspaces, notes, times, sessions, conversations, reentry, changes };
}

/** Ten canvases of the kinds agents draw most, for `npm run perf` (P1). */
function perfCanvas(n: number): string {
  const head = (kind: string) => `<hc-canvas id="perf-canvas-${n + 1}" title="Perf canvas ${n + 1}: ${kind}">`;
  const days = Array.from({ length: 14 }, (_, d) => `Day ${d + 1}: ${10 + ((d * 7 + n * 3) % 23)}`).join('\n');
  const bodies = [
    () => `${head('stats and bars')}\n<hc-stats><hc-stat label="Merged" value="12" delta="+4" tone="success"></hc-stat><hc-stat label="Open" value="3"></hc-stat></hc-stats>\n<hc-chart type="bar" label="Turns per day">\n${days}\n</hc-chart>`,
    () => `${head('table')}\n<hc-table label="Lanes">\nLane | Items | Done\nA | 11 | 4\nB | 9 | 2\nV | 9 | 2\n</hc-table>`,
    () => `${head('flow')}\n<hc-flow label="Merge path">\nb: Branch\ng: Gate {success}\nr: Review\nm: Merge {end}\nb -> g -> r -> m\n</hc-flow>`,
    () => `${head('line')}\n<hc-chart type="line" label="Free disk" unit="GB">\n${days}\n</hc-chart>`,
    () => `${head('steps')}\n<hc-steps current="2"><div label="Plan" status="done"></div><div label="Build">Running</div><div label="Ship"></div></hc-steps>`,
    () => `${head('compare')}\n<hc-compare label="Hosts"><div label="Fly" recommended>Closest.</div><div label="Render">Simplest.</div></hc-compare>`,
    () => `${head('donut')}\n<hc-chart type="donut" label="Spend">\nClaude: 60\nCodex: 40\n</hc-chart>`,
    () => `${head('area')}\n<hc-chart type="area" label="Tokens" series="In, Out">\n${Array.from({ length: 10 }, (_, d) => `Day ${d + 1}: ${20 + d}, ${10 + d}`).join('\n')}\n</hc-chart>`,
    () => `${head('callout and progress')}\n<hc-progress label="Backlog" value="64"></hc-progress>\n<hc-callout tone="info" title="Next">The lanes stop at 07:55.</hc-callout>`,
    () => `${head('bars')}\n<hc-bars label="Minutes per item" unit="min">\nA1: 30\nA2: 31\nB1: 64\nB2: 22\n</hc-bars>`,
  ];
  return `${bodies[n % bodies.length]()}\n</hc-canvas>`;
}

export function demoTasks(count: number): Task[] {
  const base = (i: number, over: Partial<Task>): Task => ({
    url: `vault://tasks/demo-${i}.md`, title: `Task ${i}`, status: 'todo', priority: null, due: null, effort: null, description: '',
    projectUrl: null, projectName: null, daysUntilDue: null, bucket: 'later', vaultPath: `tasks/demo-${i}.md`, labels: [], section: null,
    dueTime: null, durationMin: null, recurrence: null, subtasks: [], contentHash: `demo-${i}`, fingerprint: null, origin: null,
    doneWhen: null, activity: null, blockedBy: [], blocks: [], ready: true, unresolvedBlockers: [], blockerWarnings: [], ...over,
  } as Task);
  const named: Partial<Task>[] = [
    { title: 'Revise section 3 with the September close', priority: 'P1', due: day(0), dueTime: '10:00', durationMin: 60, daysUntilDue: 0, bucket: 'today', projectName: 'Q3 regional operations review', labels: ['finance'], origin: 'plan:workspaces/local-q3-review/PLAN.md' },
    { title: 'Send the draft to Dana for review', priority: 'P1', due: day(2), daysUntilDue: 2, bucket: 'thisweek', projectName: 'Q3 regional operations review' },
    { title: 'Confirm float supervisor budget', priority: 'P2', due: day(1), dueTime: '14:30', durationMin: 30, daysUntilDue: 1, bucket: 'thisweek', projectName: 'Q4 hiring plan' },
    { title: 'Weekly safety walkthrough notes', priority: 'P3', due: day(0), daysUntilDue: 0, bucket: 'today', recurrence: 'every Monday', projectName: 'Annual safety audit' },
    { title: 'Collect vendor security questionnaires', priority: 'P2', due: day(-2), daysUntilDue: -2, bucket: 'overdue', status: 'waiting', projectName: 'Cloud vendor security review' },
    { title: 'Draft airport pricing options', priority: 'P2', due: day(4), daysUntilDue: 4, bucket: 'thisweek', projectName: 'Airport contract renewals', subtasks: [{ text: 'Current rates', done: true }, { text: 'Two options', done: false }] },
    { title: 'Book the October board room', priority: null, due: null, bucket: 'later', projectName: 'October board pack' },
    { title: 'Read the EV pilot utilisation report', priority: 'P3', status: 'done', due: day(-5), daysUntilDue: -5, bucket: 'overdue', projectName: 'EV charging pilot' },
  ];
  const out = named.map((n, i) => base(i + 1, n));
  for (let i = out.length + 1; i <= count; i++) {
    const offset = (i * 7) % 30 - 10;
    out.push(base(i, { title: `Follow up on location ${100 + (i % 400)} ${['invoice', 'staffing', 'signage', 'gate repair', 'audit'][i % 5]}`, priority: (['P1', 'P2', 'P3', null] as const)[i % 4], due: i % 3 ? day(offset) : null, daysUntilDue: i % 3 ? offset : null, bucket: i % 3 ? (offset < 0 ? 'overdue' : offset === 0 ? 'today' : offset < 7 ? 'thisweek' : 'later') : 'undated', status: i % 11 === 0 ? 'done' : i % 13 === 0 ? 'waiting' : 'todo', recurrence: i % 50 === 0 ? 'every week' : null, blockedBy: i % 97 === 0 ? [`vault://tasks/demo-${i - 1}.md`] : [] }));
  }
  return out;
}

function demoLibrary(count: number): VaultItem[] {
  const items: VaultItem[] = [];
  const folders = ['notes/meetings', 'notes/research', 'sources/finance', 'projects', 'reference/policies'];
  for (let i = 0; i < count; i++) {
    const folder = folders[i % folders.length];
    items.push({ path: `${folder}/note-${String(i).padStart(5, '0')}.md`, title: i === 2600 ? 'Garage 14 lease renewal terms' : `${['Weekly ops sync', 'Vendor call', 'Site visit', 'Policy draft', 'Research note'][i % 5]} ${i}`, kind: 'note', surface: 'vault', folder, editable: true, hidden: false, trust: 'authored', mtime: iso(i * 0.5 * H), size: 800 + (i % 50) * 20, excerpt: i === 2600 ? 'Renewal option at 3% escalation; landlord asked for a five-year term.' : 'Notes from the week.', drift: null } as VaultItem);
  }
  return items;
}

/** Apply a demo set. Idempotent: rewrites the same keys each load. */
export function seedDemo(kind: string | null): void {
  if (!kind) return;
  const seed = demoSeed();
  if (kind === 'large') {
    // A 1,000-turn conversation (verification contract: long threads).
    const long = 's-demo-longthread', cwd = seed.workspaces.find(w => w.id === 'work-results-view')!.cwd;
    const longSummary = { ...session(long, cwd, 'Long design thread', 'claude', 0.2, 500) };
    seed.sessions.unshift(longSummary);
    const turns: AgentConversationTurn[] = [];
    for (let i = 0; i < 500; i++) {
      const at = iso((500 - i) * 60_000);
      turns.push(turn(`l${i}-user`, 'user', i === 3 ? 'Where did we put the needle-17 decision?' : `Step ${i}: review the next part of the layout.`, 'claude', at));
      turns.push(turn(`l${i}-assistant`, 'assistant', `Reviewed part ${i}. ${i % 7 === 0 ? 'Changed the spacing tokens and the list density.' : 'No change needed.'}`, 'claude', at));
    }
    seed.conversations[long] = { summary: longSummary, eventsRetained: true, turns };
    // Overnight P1: a 60-turn thread with 10 canvases (one of each common
    // kind), for scroll frame time and memory in `npm run perf`.
    const canvasThread = 's-demo-canvasthread';
    const canvasSummary = { ...session(canvasThread, cwd, 'Canvas review thread', 'claude', 0.4, 30) };
    // After the long thread: the workspace still opens that one first
    // (`openWorkspace` takes its first session; next-level-conversation.spec).
    seed.sessions.splice(1, 0, canvasSummary);
    const canvasTurns: AgentConversationTurn[] = [];
    for (let i = 0; i < 30; i++) {
      const at = iso((30 - i) * 120_000);
      canvasTurns.push(turn(`c${i}-user`, 'user', `Round ${i + 1}: where do the lanes stand?`, 'claude', at));
      const prose = Array.from({ length: 3 }, (_, p) => `Paragraph ${p + 1} of round ${i + 1}: the lanes moved, the gate held, and the next step is written down.`).join('\n\n');
      const canvas = i % 3 === 0 && i / 3 < 10 ? `\n\n\`\`\`hollow-canvas\n${perfCanvas(i / 3)}\n\`\`\`\n\n` : '\n\n';
      canvasTurns.push(turn(`c${i}-assistant`, 'assistant', `${prose}${canvas}That's round ${i + 1}.`, 'claude', at));
    }
    seed.conversations[canvasThread] = { summary: canvasSummary, eventsRetained: true, turns: canvasTurns };
    for (let i = 0; i < 190; i++) {
      const id = `local-scale-${String(i).padStart(3, '0')}`;
      seed.workspaces.push({ id, title: `Location ${100 + i} operating plan`, status: i % 7 === 0 ? 'done' : 'active', path: `workspaces/${id}`, cwd: `/Users/fixture/vault/workspaces/${id}`, modified: iso((i + 300) * H) });
    }
  }
  localStorage.setItem('mockWorkspaces', JSON.stringify(seed.workspaces));
  localStorage.setItem('mockWorkspaceNotes', JSON.stringify(seed.notes));
  localStorage.setItem('mockWorkspaceFileTimes', JSON.stringify(seed.times));
  localStorage.setItem('mockWorkspaceSessions', JSON.stringify(seed.sessions));
  localStorage.setItem('mockWorkspaceConversations', JSON.stringify(seed.conversations));
  localStorage.setItem('mockWorkspaceReentry', JSON.stringify(seed.reentry));
  localStorage.setItem('mockSessionChanges', JSON.stringify(seed.changes));
  localStorage.setItem('mockRailPr', JSON.stringify({ 'work-results-view': 'open' }));
  // The scale sets are far over localStorage's ~5 MB quota: keep them in
  // memory for this page (mockApi reads the globals first).
  const w = window as unknown as { __mockDemoTasks?: Task[]; __mockDemoLibrary?: VaultItem[] };
  w.__mockDemoTasks = demoTasks(kind === 'large' ? 5000 : 8);
  w.__mockDemoLibrary = demoLibrary(kind === 'large' ? 10000 : 40);
  localStorage.removeItem('mockTasks');
  localStorage.removeItem('mockVaultItemsDemo');
  localStorage.setItem('hollow.lastConversation', '');
  if (new URLSearchParams(window.location.search).get('missions') === '1') seedMissions();
  if (new URLSearchParams(window.location.search).get('main') === '1') seedMainAgent();
}

/** The Main agent with today's conversation (overnight 10-11 B4's drawer):
 *  `&main=1`. */
function seedMainAgent(): void {
  const cwd = '/Users/fixture/vault/workspaces/main';
  const workspaces = JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as WorkWorkspace[];
  workspaces.push({ id: 'main', title: 'COO', status: 'active', path: 'workspaces/main', cwd, modified: iso(0.2 * H) });
  localStorage.setItem('mockWorkspaces', JSON.stringify(workspaces));
  const key = 's-demo-main-today';
  const summary = { ...session(key, cwd, 'Today', 'claude', 0.1, 4), createdAt: new Date(new Date().setHours(7, 30, 0, 0)).toISOString() };
  const sessions = JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]') as AgentSessionSummary[];
  sessions.unshift(summary);
  localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
  const at = iso(0.5 * H);
  const conversations = JSON.parse(localStorage.getItem('mockWorkspaceConversations') ?? '{}') as Record<string, unknown>;
  conversations[key] = { summary, eventsRetained: true, turns: [
    turn('m1-user', 'user', 'Morning check-in', 'claude', at),
    turn('m1-assistant', 'assistant', 'Good morning. **Three things today:**\n\n1. The Q3 review is due Friday: section 3 still needs the September close.\n2. Two chats I started overnight finished; the Results view PR is ready for your review.\n3. The vendor security questionnaire is 2 days overdue.', 'claude', at),
    turn('m2-user', 'user', 'Draft a reply plan for the vendor questionnaire', 'claude', at),
    turn('m2-assistant', 'assistant', 'Started a chat in **Cloud vendor security review** to draft it from last week\'s notes. It will show up in Mission Control.', 'claude', at),
  ] };
  localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
}

/** Mission Control (overnight 10-11 B2) fixtures: `&missions=1` adds running,
 *  queued and finished turns to the run queue, and a live feed for them (the
 *  mock has no event stream). */
function seedMissions(): void {
  const now = Date.now();
  const at = (minutesAgo: number) => new Date(now - minutesAgo * 60_000).toISOString();
  const row = (id: number, source: string, sourceId: string, label: string, status: string, claimedMin: number | null, finishedMin: number | null, extra: Record<string, unknown> = {}) => ({
    id, source, sourceId, label, detail: null, reason: source === 'chat' ? 'chat' : 'schedule', status, priority: 0, position: null, attempt: 1, coalescedCount: 0,
    requestedAt: at((claimedMin ?? 1) + 1), claimedAt: claimedMin === null ? null : at(claimedMin), finishedAt: finishedMin === null ? null : at(finishedMin),
    historyRunId: null, error: null, waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false, waitingReason: null, ...extra,
  });
  localStorage.setItem('mockRunQueue', JSON.stringify([
    row(41, 'chat', 's-20261001-bld00001', 'Accept flow and revisions', 'running', 6, null),
    row(42, 'chat', 's-20260930-q3a00001', 'Revise the labour section', 'running', 2, null),
    row(43, 'cron', 'morning-brief', 'Morning brief', 'running', 1, null),
    row(44, 'chat', 's-20260930-bld00002', 'Results store and list', 'queued', null, null, { position: 1, waitingReason: 'a free slot (2 of 2 in use)' }),
    row(38, 'chat', 's-20260928-q3a00002', 'First draft of the review', 'done', 70, 52),
    row(36, 'cron', 'inbox-sweep', 'Inbox sweep', 'failed', 140, 138, { error: 'The agent stopped: no network' }),
  ]));
  const entry = (queueId: number, key: string, minutes: number, lines: string[], tool: { name: string; input: Record<string, string> } | null, progress: string, model: string) => ({
    key, queueId, running: true, startedAt: now - minutes * 60_000, finishedAt: null, updatedAt: now, asked: null, lines, partial: '', tool, progress,
    lastEventAt: new Date(now - 4_000).toISOString(), costUsd: null, model, error: null,
  });
  localStorage.setItem('mockMissionFeed', JSON.stringify({
    's-20261001-bld00001': entry(41, 's-20261001-bld00001', 6, ['The accept action now stores the content hash.', 'Writing the revision rule: an edit after acceptance opens a new review.', 'Running the Results tests…'], { name: 'Bash', input: { command: 'npm test -- results' } }, 'usingTools', 'claude-opus-5-5'),
    's-20260930-q3a00001': entry(42, 's-20260930-q3a00001', 2, ['Reading the September close from regional-pnl-q3.csv.', 'Labour ran 3.4 % over plan in the Mid-Atlantic.'], { name: 'Read', input: { file_path: '/Users/fixture/vault/workspaces/q3/sources/regional-pnl-q3.csv' } }, 'researching', 'claude-sonnet-5-5'),
    's-20260928-q3a00002': { ...entry(38, 's-20260928-q3a00002', 70, ['Drafted sections 1–4.'], null, 'researching', 'gpt-5.5'), running: false, finishedAt: now - 52 * 60_000, costUsd: 0.84 },
  }));
}
