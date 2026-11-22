// Browser-mock task tracker (overhaul A2; backend
// `src-tauri/src/commands/tracker_api.rs`). It plays the finished tracker
// (engine on, set up, a small synced team) so Settings' Task tracker panel
// and the rebuilt Tasks page can be built and tested before the sync engine
// lands. localStorage `mockTracker` (JSON) may seed: `provider` (null = off),
// `key` ('missing' | 'saved' | 'unreadable'),
// `engine` (false = this build's real answers: empty view with a note,
// writes refused), `teamId`, `teamKey`, `pollSeconds`, `trackAgentWork`
// (X1's switch), `workRuns` ({ [issueId]: WorkRun[] }, the runs
// `tracker_issue_work` returns; none by default), `workAudit`
// ({ [issueId]: AuditEntry[] }, its X2 audit rows; none by default), `error` (the last
// pass's error), `viewer` (false = whom the key belongs to isn't read yet),
// `historyFails` (true = `tracker_issue_history` refuses, so the detail shows
// its synthetic feed), `historyMore` (true = the history says older entries
// were left out), `personalTeamId` / `personalTeamKey` (the personal team, Personal
// HOL: set, the view gains its issues PER-1..3 and project P-PER-1 and
// `teamKeys` becomes [AI, HOL]; unset by default, so one team). ENG-31 has a change history (`HISTORY`); the rest
// have none. `conflicts` ({ [issueId]: TrackerConflict[] }, open conflicts;
// a save of that field clears its entry, as Rust's acknowledgement does).
// Writes change the in-memory fixture for the page's life.
// Refuses what Rust refuses, with its words.

import type {
  AuditEntry,
  PortfolioUpdate,
  ProjectPortfolio,
  TrackerHistory,
  TrackerHistoryEvent,
  TrackerIssue,
  TrackerIssueDetail,
  TrackerConflict,
  TrackerIssueWork,
  TrackerProjectRow,
  TrackerState,
  TrackerStatus,
  TrackerTeamMeta,
  TrackerUser,
  TrackerView,
  TrackerViewQuery,
  TrackerViewRow,
  TrackerWriteOutcome,
  WorkRun,
} from './generated/ipc';

const KEY = 'mockTracker';
// The Engineering team's id: the one team Hollow may use (trackerSettings.ts ALLOWED_TEAMS).
const TEAM_ID = '11111111-1111-4111-8111-111111111111';
// Personal, the personal team (trackerSettings.ts PERSONAL_TEAMS).
const HOL_TEAM_ID = '22222222-2222-4222-8222-222222222222';
const NOT_YET = "The task tracker's sync engine isn't part of this build yet, so nothing was read or sent.";

interface MockSettings {
  provider: string | null;
  key: 'missing' | 'saved' | 'unreadable';
  engine: boolean;
  teamId: string | null;
  teamKey: string | null;
  personalTeamId?: string | null;
  personalTeamKey?: string | null;
  pollSeconds: number;
  trackAgentWork?: boolean;
  fridayStatus?: boolean;
  postStatusUpdates?: boolean;
  workRuns?: Record<string, WorkRun[]>;
  workAudit?: Record<string, AuditEntry[]>;
  error: string | null;
  viewer?: boolean;
  historyFails?: boolean;
  historyMore?: boolean;
  conflicts?: Record<string, TrackerConflict[]>;
}

function settings(): MockSettings {
  let seeded: Partial<MockSettings> = {};
  try { seeded = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Partial<MockSettings>; } catch { /* default */ }
  return {
    provider: 'linear',
    key: 'saved',
    engine: true,
    teamId: TEAM_ID,
    teamKey: 'ENG',
    pollSeconds: 120,
    error: null,
    ...seeded,
  };
}

function save(next: Partial<MockSettings>): void {
  localStorage.setItem(KEY, JSON.stringify({ ...settings(), ...next }));
}

const STATES: TrackerState[] = [
  { id: 'st-backlog', name: 'Backlog', kind: 'backlog', position: 0, color: '#bec2c8' },
  { id: 'st-todo', name: 'Todo', kind: 'unstarted', position: 1, color: '#e2e2e2' },
  { id: 'st-progress', name: 'In Progress', kind: 'started', position: 2, color: '#f2c94c' },
  { id: 'st-review', name: 'In Review', kind: 'started', position: 3, color: '#0f783c' },
  { id: 'st-done', name: 'Done', kind: 'completed', position: 4, color: '#5e6ad2' },
  { id: 'st-canceled', name: 'Canceled', kind: 'canceled', position: 5, color: '#95a2b3' },
];

const PEOPLE: TrackerUser[] = [
  { id: 'u-1', name: 'Alex Rivera', email: null },
  { id: 'u-2', name: 'Sam Lee', email: null },
];

const PROJECTS: { teamKey: string; id: string; key: string | null; name: string; statusName: string; statusKind: string; lead: string | null; targetDate: string | null; progress: number | null; workspaceId: string | null }[] = [
  { teamKey: 'ENG', id: 'p-1', key: 'P-ENG-1', name: 'Receipts rebuild', statusName: 'In Progress', statusKind: 'started', lead: 'Alex Rivera', targetDate: day(21), progress: 0.4, workspaceId: 'ws-receipts' },
  { teamKey: 'ENG', id: 'p-2', key: 'P-ENG-2', name: 'Warehouse enrichment', statusName: 'Planned', statusKind: 'planned', lead: 'Alex Rivera', targetDate: day(45), progress: 0.1, workspaceId: null },
  // A project with no issues yet (B4's Projects view).
  { teamKey: 'ENG', id: 'p-3', key: 'P-ENG-3', name: 'Claims intake', statusName: 'Backlog', statusKind: 'backlog', lead: null, targetDate: null, progress: null, workspaceId: null },
  // The personal team's project; served only while `personalTeamKey` is set.
  { teamKey: 'PER', id: 'p-per-1', key: 'P-PER-1', name: 'Personal ops', statusName: 'In Progress', statusKind: 'started', lead: 'Alex Rivera', targetDate: null, progress: 0.5, workspaceId: null },
];

// Project records (B3): PROJECT.md by workspace. `ws-receipts` is p-1's
// tracked record; `ws-fleet` is a vault-only project for Track in.
const emptyText = () => Object.fromEntries(['department', 'type', 'hours_saved_per_year', 'value_per_year', 'owners_note', 'github_url', 'live_url'].map(k => [k, '']));
const emptyLists = () => Object.fromEntries(['requested_by', 'stakeholders', 'domains', 'meetings', 'captures', 'war_room', 'docs'].map(k => [k, [] as string[]]));
let portfolios: Map<string, ProjectPortfolio> | null = null;
let revisions = 0;
function records(): Map<string, ProjectPortfolio> {
  portfolios ??= new Map([
    ['ws-receipts', { workspaceId: 'ws-receipts', path: 'workspaces/ws-receipts/PROJECT.md', exists: true, title: 'Receipts rebuild', status: 'In Progress',
      targetDate: day(21), description: 'Match every **deposit** to its receipt.', tracked: { provider: 'linear', projectId: 'p-1', url: 'https://tracker.example/project/P-ENG-1' },
      text: { ...emptyText(), department: 'AI', type: 'Automation', hours_saved_per_year: '1,200', github_url: 'https://example.com/receipts' },
      lists: { ...emptyLists(), domains: ['library/records/domains/accounting.md'], meetings: ['library/records/meetings/2026/2026-11-04-receipts-kickoff.md'] }, revision: 'r-0' }],
    ['ws-fleet', { workspaceId: 'ws-fleet', path: 'workspaces/ws-fleet/PROJECT.md', exists: true, title: 'Fleet dashboard', status: 'Discovery',
      targetDate: null, description: 'A dashboard of the fleet.', tracked: null, text: { ...emptyText(), department: 'AI' }, lists: emptyLists(), revision: 'r-0' }],
  ]);
  return portfolios;
}
function portfolio(workspaceId: string): ProjectPortfolio {
  const found = records().get(workspaceId);
  if (found) return structuredClone(found);
  return { workspaceId, path: `workspaces/${workspaceId}/PROJECT.md`, exists: false, title: workspaceId, status: null, targetDate: null,
    description: '', tracked: null, text: emptyText(), lists: emptyLists(), revision: '' };
}

function day(offset: number): string {
  const d = new Date();
  d.setDate(d.getDate() + offset);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

function issue(n: number, title: string, state: string, priority: number, extra: Partial<TrackerIssue> = {}, team = 'ENG'): TrackerIssue {
  const key = `${team}-${n}`;
  return {
    id: `i-${n}`,
    key,
    url: `https://tracker.example/issue/${key}`,
    title,
    description: '',
    state: STATES.find(s => s.id === state)!,
    assignee: PEOPLE[0],
    priority,
    due: null,
    projectId: 'p-1',
    labels: [],
    parentId: null,
    teamId: team === 'PER' ? HOL_TEAM_ID : TEAM_ID,
    updatedAt: '2026-11-08T05:00:00Z',
    archivedAt: null,
    trashed: false,
    estimate: null,
    cycle: null,
    milestone: null,
    creator: PEOPLE[0],
    createdAt: '2026-11-07T21:27:00Z',
    children: [],
    relations: [],
    links: [],
    attachments: [],
    comments: [],
    ...extra,
  };
}

// ENG-31's change history (`tracker_issue_history`), hours before now so the
// feed reads "2h ago".
function history(id: string): TrackerHistoryEvent[] {
  if (id !== 'i-31' && id !== 'ENG-31') return [];
  const at = (hours: number) => new Date(Date.now() - hours * 3_600_000).toISOString();
  const change = (n: number, hours: number, actor: string | null, over: Partial<TrackerHistoryEvent>): TrackerHistoryEvent => ({
    id: `h-${n}:${over.kind}`, at: at(hours), actor, kind: 'state', from: null, to: null, fromKind: null, toKind: null, added: [], removed: [], ...over,
  });
  return [
    change(1, 30, 'Sam Lee', { kind: 'priority', from: '0', to: '2' }),
    change(2, 29, 'Sam Lee', { kind: 'labels', added: ['Feature', 'Accounting'] }),
    change(3, 26, 'Sam Lee', { kind: 'assignee', to: 'Alex Rivera' }),
    change(4, 2, 'Alex Rivera', { kind: 'state', from: 'Todo', to: 'In Progress', fromKind: 'unstarted', toKind: 'started' }),
  ];
}

let issues: TrackerIssue[] | null = null;
const commentKeys = new Map<string, string>();
let nextNumber = 40;

function all(): TrackerIssue[] {
  issues ??= [
    issue(31, 'Match bank deposits to receipts', 'st-progress', 2, { due: day(3), labels: ['Feature', 'Accounting'], estimate: 3,
      description: 'Match each **bank deposit** to its receipt, so month end closes without a spreadsheet.',
      comments: [{ id: 'c-1', author: 'Sam Lee', body: 'Bank file format attached.', createdAt: '2026-11-07T22:02:00Z', editedAt: null, parentId: null }],
      relations: [{ kind: 'blockedBy', key: 'ENG-33' }, { kind: 'related', key: 'ENG-32' }], children: ['ENG-36'],
      cycle: { id: 'cy-4', number: 4, name: 'Cycle 4', startsAt: day(-3), endsAt: day(11) },
      milestone: { id: 'm-1', name: 'Receipts live', targetDate: day(21), projectId: 'p-1' },
      links: [{ id: 'ln-1', title: 'Pull request #12', url: 'https://example.com/pr/12', source: 'GitHub', createdAt: '2026-11-08T01:00:00Z' }],
      attachments: [{ id: 'at-1', title: 'Bank file format', url: 'https://example.com/bank-format.pdf', source: null, createdAt: '2026-11-07T22:02:00Z' }],
      creator: PEOPLE[1] }),
    issue(32, 'Export the month-end report', 'st-todo', 3, { due: day(9), labels: ['Feature'] }),
    issue(33, 'Read the bank file format', 'st-review', 1, { due: day(-1), assignee: PEOPLE[1], labels: ['Accounting'] }),
    issue(34, 'Pick a warehouse schema', 'st-backlog', 0, { projectId: 'p-2', assignee: null }),
    issue(35, 'Retire the old receipts sheet', 'st-done', 4, { labels: ['Chore'] }),
    issue(36, 'Parse deposit lines', 'st-todo', 3, { parentId: 'i-31', due: day(5) }),
    issue(37, 'Old import idea', 'st-canceled', 0, { projectId: 'p-2' }),
    // The personal team's issues (PER), served only while it is set up.
    issue(1, 'Plan the week with Hollow', 'st-progress', 2, { projectId: 'p-per-1', id: 'i-per-1' }, 'PER'),
    issue(2, 'Tidy the vault inbox', 'st-todo', 3, { projectId: 'p-per-1', id: 'i-per-2', due: day(2) }, 'PER'),
    issue(3, 'Try a new morning brief', 'st-backlog', 0, { projectId: null, id: 'i-per-3', assignee: null }, 'PER'),
  ];
  return issues;
}

function status(): TrackerStatus {
  const s = settings();
  const on = s.provider === 'linear';
  const missing = !on ? 'Choose a task tracker.' : !s.teamId ? 'Choose the team Hollow may use.'
    : s.key === 'missing' ? 'Save an API key.' : s.key === 'unreadable' ? "Hollow can't read its saved credentials (Keychain access)." : null;
  // Nothing blocks the mock tracker (the Notion two-way guard went with Notion sync, 11-10).
  const blockedBy: string | null = null;
  const state = !on ? 'off' : missing ? 'needsSetup' : blockedBy ? 'blocked' : s.error ? 'error' : 'ready';
  const synced = s.engine && state !== 'off' && !missing;
  return {
    state,
    provider: on ? 'linear' : null,
    displayName: on ? 'Linear' : null,
    providers: [{ id: 'linear', displayName: 'Linear', keychainItem: true, transports: ['graphql', 'mcp'] }],
    teamId: s.teamId,
    teamKey: s.teamKey,
    personalTeamId: s.personalTeamKey ? s.personalTeamId ?? HOL_TEAM_ID : null,
    personalTeamKey: s.personalTeamKey ?? null,
    transport: 'graphql',
    pollSeconds: s.pollSeconds,
    trackAgentWork: s.trackAgentWork ?? false,
    fridayStatus: s.fridayStatus ?? false,
    postStatusUpdates: s.postStatusUpdates ?? false,
    key: s.key,
    missing,
    blockedBy,
    lastSync: synced ? new Date(Date.now() - 60_000).toISOString() : null,
    nextSync: synced ? new Date(Date.now() + 60_000).toISOString() : null,
    lastError: s.error,
    rate: synced ? { requestsRemaining: 2431, requestsLimit: 2500, complexityRemaining: 2_950_000, complexityLimit: 3_000_000, resetAt: null } : null,
    outbox: 0,
    problems: [],
    restartNeeded: false,
    engine: s.engine,
  };
}

/** What Rust says when a read or write can't happen; null when it can. */
function unavailable(): string | null {
  const s = status();
  if (s.blockedBy) return s.blockedBy;
  if (s.missing) return `The task tracker isn't set up: ${s.missing}`;
  return s.engine ? null : NOT_YET;
}

function projectName(id: string | null): string | null {
  return PROJECTS.find(p => p.id === id)?.name ?? null;
}

function row(i: TrackerIssue): TrackerViewRow {
  return {
    id: i.id, key: i.key, title: i.title, statusName: i.state.name, statusKind: i.state.kind, priority: i.priority,
    labels: i.labels, project: projectName(i.projectId), projectId: i.projectId, due: i.due,
    assignee: i.assignee?.name ?? null, assigneeId: i.assignee?.id ?? null, estimate: i.estimate,
    cycle: i.cycle?.name ?? null, milestone: i.milestone?.name ?? null, updatedAt: i.updatedAt, url: i.url,
    path: `tasks/${i.key.toLowerCase()}.md`, lock: { kind: 'linked', fields: [] }, pending: false, conflict: conflictsOf(i.id).length > 0,
    archived: i.archivedAt !== null, teamKey: teamOf(i),
  };
}

/** The key of the team an issue belongs to. */
function teamOf(i: Pick<TrackerIssue, 'teamId'>): string {
  return i.teamId === HOL_TEAM_ID ? 'PER' : 'ENG';
}

/** The team keys the mirror holds: the shared one, then the personal one once set. */
function teamKeys(): string[] {
  const s = settings();
  return [s.teamKey ?? 'ENG', ...(s.personalTeamKey ? [s.personalTeamKey] : [])];
}

function projectRows(team: string | null = null): TrackerProjectRow[] {
  return PROJECTS.filter(p => teamKeys().includes(p.teamKey) && (!team || p.teamKey === team)).map(p => {
    const mine = all().filter(i => i.projectId === p.id && !i.archivedAt);
    return { ...p, url: `https://tracker.example/project/${p.key}`, issueCount: mine.length,
      openCount: mine.filter(i => i.state.kind !== 'completed' && i.state.kind !== 'canceled').length };
  });
}

function view(query: Partial<TrackerViewQuery> | null): TrackerView {
  const s = status();
  const why = unavailable();
  const base = { provider: s.provider, displayName: s.displayName, teamKey: s.teamKey, teamKeys: teamKeys(), lastSync: s.lastSync, syncError: s.lastError };
  if (why) return { ...base, states: [], rows: [], projects: [], total: 0, note: why };
  const q = query ?? {};
  const words = q.search?.trim().toLowerCase();
  const matching = all().filter(i =>
    teamKeys().includes(teamOf(i)) && (!q.teamKey || teamOf(i) === q.teamKey)
    && (q.includeArchived || !i.archivedAt)
    && (q.stateKinds ? q.stateKinds.includes(i.state.kind) : q.includeClosed || (i.state.kind !== 'completed' && i.state.kind !== 'canceled'))
    && (!q.assigneeId || i.assignee?.id === q.assigneeId)
    && (!q.projectId || i.projectId === q.projectId)
    && (!q.label || i.labels.includes(q.label))
    && (!q.cycleId || i.cycle?.id === q.cycleId)
    && (!words || i.key.toLowerCase().includes(words) || i.title.toLowerCase().includes(words)));
  const rows = matching.slice(0, q.limit ?? undefined).map(row);
  return { ...base, states: STATES, rows, projects: projectRows(q.teamKey ?? null), total: matching.length, note: null };
}

function detail(id: string): TrackerIssueDetail {
  const found = all().find(i => i.id === id);
  if (!found) throw new Error('The task tracker has no such item.');
  const project = projectRows().find(p => p.id === found.projectId) ?? null;
  return {
    issue: found, project, path: `tasks/${found.key.toLowerCase()}.md`, workspaceId: null,
    lock: { kind: 'linked', fields: [] },
    writableFields: ['title', 'description', 'status', 'priority', 'assignee', 'labels', 'project', 'due', 'parent', 'estimate', 'cycle', 'milestone'],
    conflicts: conflictsOf(found.id), pending: [], vaultNotes: null,
  };
}

function meta(): TrackerTeamMeta {
  return {
    team: { id: TEAM_ID, key: settings().teamKey ?? 'ENG', name: 'Engineering' },
    states: STATES,
    labels: ['Feature', 'Bug', 'Accounting', 'Chore', 'Blocked'].map((name, n) => ({ id: `l-${n}`, name, color: null, group: null })),
    members: PEOPLE,
    cycles: [{ id: 'cy-4', number: 4, name: 'Cycle 4', startsAt: day(-3), endsAt: day(11) }],
    milestones: [{ id: 'm-1', name: 'Receipts live', targetDate: day(21), projectId: 'p-1' }],
    projects: PROJECTS.map(p => ({ id: p.id, name: p.name })),
    // Whom the key belongs to; seed `viewer: false` for a pass that hasn't read it.
    viewer: settings().viewer === false ? null : PEOPLE[0],
  };
}

const canvasCopies = new Set<string>();

function wrote(found: TrackerIssue, note: string): TrackerWriteOutcome {
  return { id: found.id, key: found.key, path: `tasks/${found.key.toLowerCase()}.md`, queued: false, note };
}

export function isMockTrackerCommand(command: string): boolean {
  return command.startsWith('tracker_') || command.endsWith('_tracker_item') || command.endsWith('_tracker_task')
    || command.endsWith('_tracker_project') || command.endsWith('_tracker_key') || command === 'set_tracker_settings'
    || command === 'project_portfolio' || command === 'set_project_portfolio' || command === 'comment_tracker_issue'
    || command === 'write_friday_status' || command === 'attach_canvas_to_issue';
}

export function mockTrackerCommand(command: string, args: Record<string, unknown> = {}): unknown {
  switch (command) {
    case 'tracker_status': return status();
    case 'tracker_link_setup': return { value: 'hollow://issue?prompt={{prompt}}', steps: 'In Linear, open Settings, then the coding tools, add a Custom link and paste this value as its URL.' };
    // The vault's own record: no tracker needed (B3).
    case 'project_portfolio': return portfolio(String(args.workspaceId ?? ''));
    case 'set_project_portfolio': {
      const u = args.update as PortfolioUpdate;
      const current = portfolio(u.workspaceId);
      if (current.revision !== u.expectedRevision) throw new Error('PORTFOLIO_CHANGED: PROJECT.md changed since it was read; nothing was saved. Read it again.');
      for (const [k, v] of Object.entries(u.values)) {
        if (Array.isArray(v)) current.lists[k] = v; else current.text[k] = v;
        if (!Array.isArray(v) && /_per_year$/.test(k) && v && !Number.isFinite(Number(v.replace(/[,$\s]/g, '')))) throw new Error(`${k} must be a number.`);
      }
      const next = { ...current, exists: true, revision: `r-${++revisions}` };
      records().set(u.workspaceId, next);
      return structuredClone(next);
    }
    case 'tracker_view': return view((args.query ?? null) as Partial<TrackerViewQuery> | null);
    case 'set_tracker_key': {
      const key = String(args.key ?? '').trim();
      if (!key) throw new Error('Paste an API key first.');
      if (/\s/.test(key)) throw new Error("That doesn't look like an API key (one line, no spaces).");
      save({ key: 'saved' });
      return status();
    }
    case 'clear_tracker_key': save({ key: 'missing' }); return status();
    case 'use_keychain_tracker_key': save({ key: 'saved' }); return status();
    case 'set_tracker_settings': {
      const u = (args.update ?? {}) as { provider?: string | null; pollSeconds?: number | null; teamId?: string | null; teamKey?: string | null; personalTeamId?: string; personalTeamKey?: string; trackAgentWork?: boolean | null; fridayStatus?: boolean | null; postStatusUpdates?: boolean | null };
      if (u.pollSeconds != null && (u.pollSeconds < 60 || u.pollSeconds > 3600)) throw new Error('The sync interval must be between 1 and 60 minutes.');
      save({
        ...(u.provider != null ? { provider: u.provider.trim() === '' ? null : u.provider } : {}),
        ...(u.pollSeconds != null ? { pollSeconds: u.pollSeconds } : {}),
        ...(u.teamId != null ? { teamId: u.teamId || null } : {}),
        ...(u.teamKey != null ? { teamKey: u.teamKey || null } : {}),
        ...(u.personalTeamId != null ? { personalTeamId: u.personalTeamId || null } : {}),
        ...(u.personalTeamKey != null ? { personalTeamKey: u.personalTeamKey || null } : {}),
        ...(u.trackAgentWork != null ? { trackAgentWork: u.trackAgentWork } : {}),
        ...(u.fridayStatus != null ? { fridayStatus: u.fridayStatus } : {}),
        ...(u.postStatusUpdates != null ? { postStatusUpdates: u.postStatusUpdates } : {}),
      });
      return status();
    }
  }
  const why = unavailable();
  if (why) throw new Error(why);
  switch (command) {
    case 'tracker_issue': return detail(String(args.id));
    case 'tracker_issue_history': {
      const id = String(args.id);
      if (!all().some(i => i.id === id || i.key === id)) throw new Error("The tracker issue isn't mirrored yet. Sync first.");
      if (settings().historyFails) throw new Error("Couldn't reach the task tracker: offline.");
      return { events: history(id), more: !!settings().historyMore } satisfies TrackerHistory;
    }
    // X1: the runs tied to an issue; X2: its audit rows (none by default).
    case 'tracker_issue_work': return { runs: structuredClone(settings().workRuns?.[String(args.issueId)] ?? []), audit: structuredClone(settings().workAudit?.[String(args.issueId)] ?? []) } satisfies TrackerIssueWork;
    case 'tracker_team_meta': return meta();
    case 'tracker_sync': return { pulled: 0, pushed: 0, conflicts: 0, droppedOutOfScope: 0, errors: [], finishedAt: new Date().toISOString() };
    case 'edit_tracker_item': {
      const edit = args.edit as { id: string; values: Record<string, unknown> };
      const found = all().find(i => i.id === edit.id);
      if (!found) throw new Error('The task tracker has no such item.');
      const v = edit.values;
      if (typeof v.title === 'string') found.title = v.title;
      if (typeof v.description === 'string') found.description = v.description;
      if (typeof v.status === 'string') found.state = STATES.find(s => s.id === v.status || s.name === v.status) ?? found.state;
      if (typeof v.priority === 'number') found.priority = v.priority;
      if ('due' in v) found.due = (v.due as string | null) || null;
      if ('assignee' in v) found.assignee = PEOPLE.find(p => p.id === v.assignee || p.name === v.assignee) ?? null;
      if (Array.isArray(v.labels)) found.labels = v.labels.map(String);
      if ('project' in v) found.projectId = (v.project as string | null) || null;
      if ('estimate' in v) found.estimate = (v.estimate as number | null) ?? null;
      if ('cycle' in v) found.cycle = meta().cycles.find(c => c.id === v.cycle) ?? null;
      if ('milestone' in v) found.milestone = meta().milestones.find(m => m.id === v.milestone) ?? null;
      if (typeof v.priority === 'string' && /^\d$/.test(v.priority)) found.priority = Number(v.priority);
      acknowledgeConflicts(found.id, Object.keys(v));
      found.updatedAt = new Date().toISOString();
      return wrote(found, 'Sent to Linear.');
    }
    case 'create_tracker_task': {
      const task = args.task as { teamKey?: string; title?: string; stateId?: string | null; priority?: number | null; projectId?: string | null; due?: string | null; assigneeId?: string | null; labels?: string[] };
      if (!task?.title?.trim()) throw new Error('A task needs a title.');
      const team = checkedTeam(task.teamKey);
      const created = issue(nextNumber++, task.title.trim(), task.stateId ?? 'st-todo', task.priority ?? 0, {
        projectId: task.projectId ?? null, due: task.due ?? null, labels: task.labels ?? [],
        assignee: PEOPLE.find(p => p.id === task.assigneeId) ?? null, updatedAt: new Date().toISOString(),
      }, team);
      all().push(created);
      return wrote(created, 'Created in Linear.');
    }
    case 'create_tracker_project': {
      const project = args.project as { teamKey?: string; name?: string; targetDate?: string | null; workspaceId?: string | null };
      if (!project?.name?.trim()) throw new Error('A project needs a name.');
      const team = checkedTeam(project.teamKey);
      const id = `p-${PROJECTS.length + 1}`;
      const key = `P-${team}-${PROJECTS.length + 1}`;
      if (project.workspaceId) {
        const record = portfolio(project.workspaceId);
        if (record.tracked) throw new Error('This project is already tracked; open it from Tasks, Projects.');
        records().set(project.workspaceId, { ...record, exists: true, status: 'Backlog', tracked: { provider: 'linear', projectId: id, url: `https://tracker.example/project/${key}` }, revision: `r-${++revisions}` });
      }
      PROJECTS.push({ teamKey: team, id, key, name: project.name.trim(), statusName: 'Backlog', statusKind: 'backlog', lead: null, targetDate: project.targetDate ?? null, progress: null, workspaceId: project.workspaceId ?? null });
      return { id, key, path: project.workspaceId ? `workspaces/${project.workspaceId}/PROJECT.md` : null, queued: false, note: 'Created in Linear.' };
    }
    case 'comment_tracker_issue': {
      const found = all().find(i => i.id === args.id);
      if (!found) throw new Error('Sync the tracker before commenting on that issue.');
      const body = String(args.body ?? '').trim();
      if (!body) throw new Error('Write a comment first.');
      if (new TextEncoder().encode(body).length > 2000) throw new Error('A comment holds at most 2000 bytes (accented letters and emoji take more than one).');
      const key = String(args.requestKey ?? '');
      const done = commentKeys.get(key);
      if (done) return { id: done, key: found.key, path: `tasks/${found.key.toLowerCase()}.md`, queued: false, note: 'Comment posted.' };
      const id = `c-${found.comments.length + 100}`;
      commentKeys.set(key, id);
      found.comments.push({ id, author: PEOPLE[0].name, body, createdAt: new Date().toISOString(), editedAt: null, parentId: null });
      return { id, key: found.key, path: `tasks/${found.key.toLowerCase()}.md`, queued: false, note: 'Comment posted.' };
    }
    case 'archive_tracker_task': {
      const found = all().find(i => i.id === args.id);
      if (!found) throw new Error('The task tracker has no such item.');
      found.archivedAt = new Date().toISOString();
      return wrote(found, 'Archived in Linear.');
    }
    // A18: the Friday status notes of the tracked workspaces (here ws-receipts).
    case 'write_friday_status': {
      const only = (args.workspaceId as string | null) ?? null;
      const tracked = PROJECTS.filter(p => p.workspaceId && (!only || p.workspaceId === only));
      if (!tracked.length) throw new Error('That workspace isn’t a tracked project (its PROJECT.md names no tracker project).');
      const date = new Date().toISOString().slice(0, 10);
      const projects = tracked.map(p => {
        const issues = all().filter(i => i.projectId === p.id && !i.archivedAt);
        const count = (kind: string) => issues.filter(i => i.state.kind === kind).length;
        return { workspaceId: p.workspaceId!, title: p.name, projectId: p.id, path: `workspaces/${p.workspaceId}/outputs/status/${date}-friday-status.md`,
          done: count('completed'), inProgress: count('started'), blocked: 0, next: count('unstarted'), health: 'onTrack' as const };
      });
      const sum = (f: (n: (typeof projects)[number]) => number) => projects.reduce((a, n) => a + f(n), 0);
      const heraldLine = only
        ? `Friday status for ${projects[0].title}: ${projects[0].done} done this week, ${projects[0].inProgress} in progress, 0 blocked or overdue.`
        : `Friday status ${date}: ${projects.length} project${projects.length === 1 ? '' : 's'}, ${sum(n => n.done)} done this week, ${sum(n => n.inProgress)} in progress, 0 blocked or overdue.`;
      const copies = settings().postStatusUpdates ? projects.map(p => ({ workspaceId: p.workspaceId, projectId: p.projectId, posted: true, note: `Status for ${p.title} posted to the tracker.` })) : [];
      return { date, digestPath: only ? null : `workspaces/main/outputs/status/${date}-friday-status.md`, heraldLine, projects, copies };
    }
    // A19: a copy of a saved canvas on an issue; the same version twice adds nothing.
    case 'attach_canvas_to_issue': {
      const found = all().find(i => i.id === args.issueId);
      if (!found) throw new Error('Sync the tracker before attaching to that issue.');
      const key = `${String(args.workspaceId)}/${String(args.canvasId)}/${String(args.revision)}/${found.id}`;
      const again = canvasCopies.has(key);
      canvasCopies.add(key);
      return { id: `att-${canvasCopies.size}`, key: found.key, path: `tasks/${found.key.toLowerCase()}.md`, queued: false,
        note: again ? `v${String(args.revision)} of this canvas is already on ${found.key}.` : `v${String(args.revision)} of the canvas is on ${found.key} as a copy; the canvas stays in Hollow.` };
    }
  }
  throw new Error(`mockTracker: no fixture for '${command}'.`);
}

/** Rust's team rule for a create: ENG by default, only ENG or PER, PER only once it is set up. */
function checkedTeam(key: string | undefined): string {
  const team = key ?? 'ENG';
  if (team !== 'ENG' && team !== 'PER') throw new Error(`Team ${team} isn't one Hollow may write to (ENG or PER only).`);
  if (!teamKeys().includes(team)) throw new Error(`Team ${team} isn't set up in Settings, Task tracker.`);
  return team;
}

/** An issue's open conflicts, seeded through `mockTracker.conflicts`. */
function conflictsOf(id: string): TrackerConflict[] {
  return structuredClone(settings().conflicts?.[id] ?? []);
}

/** Saving a field settles its conflict (Rust acknowledges the note). */
function acknowledgeConflicts(id: string, fields: string[]): void {
  const seeded = settings().conflicts;
  if (!seeded?.[id]) return;
  const left = seeded[id].filter(c => !fields.includes(c.field));
  const next = { ...seeded };
  if (left.length) next[id] = left; else delete next[id];
  save({ conflicts: next });
}
