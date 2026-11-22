// Browser-mock Move Hollow home (2026-11-12): `move_home_plan`,
// `move_home_start`, `move_home_status` and `move_home_rollback`, answered
// from fixtures under /Users/mock. Nothing on disk is touched and the app
// never quits.
//
// Knobs (localStorage):
// - `mockMoveHomeChatRunning=1`: one other chat is running, so the plan is
//   not ready and Quit and move is disabled (the backend refuses the same).
// - `mockMoveHomeState`: the journal's state (`none` by default). Start sets
//   `staged`; each `move_home_status` call then moves it on, staged to
//   running, then running to done on the second running poll
//   (`mockMoveHomePolls`). A test can set `done` before load to see Roll back.
// - `mockMoveHomeRefuse=1`: a rollback is refused (new writes since the
//   verify), shown under Roll back.
// Starts and rollbacks are logged in `mockMoveHomeCalls` ({ command, hash }).
import type { MoveHomeOptions, MoveHomePlan, MoveHomeRollback, MoveHomeStarted, MoveHomeStatus, MoveHomeStep, PlannedMove } from './types';

const GB = 1024 ** 3;
const MB = 1024 ** 2;
const HOME = '/Users/mock';
const H = `${HOME}/Hollow`;
const VAULT = `${HOME}/Documents/projects/hollow-vault`;
const PROJECTS = `${HOME}/Documents/Old Mac/projects`;
const BESIDE = [`${HOME}/Documents/projects/morning-paper`, `${HOME}/Documents/projects/pricing-study`];
const BACKUP = `${H}/Backups/move-home-20261112-161500`;
const OLD_STEM = 'hollow-vault-0f3a9c2e71b4d855';
const NEW_STEM = 'Vault-7c21e0b9a4d63f10';

const STEPS = ['probe', 'snapshot', 'mkdirs', 'move:vault', 'move:projects', 'symlinks', 'stores', 'sessions', 'ids', 'repos', 'agents', 'verify'];

const get = (k: string) => localStorage.getItem(k);
const chatRunning = () => get('mockMoveHomeChatRunning') === '1';
type State = MoveHomeStatus['state'];
const state = (): State => (get('mockMoveHomeState') as State | null) ?? 'none';

function log(command: string, hash: unknown): void {
  const calls = JSON.parse(get('mockMoveHomeCalls') ?? '[]') as unknown[];
  localStorage.setItem('mockMoveHomeCalls', JSON.stringify([...calls, { command, hash }]));
}

const move = (from: string, to: string, kind: PlannedMove['kind'], bytes: number, files: number, dataless: number, why: string, replacesSymlink: string | null = null, method: PlannedMove['method'] = 'rename'): PlannedMove =>
  ({ from, to, kind, bytes, files, dataless, datalessBytes: dataless * 3_500_000, method, replacesSymlink, compatSymlink: true, why });

/** The dry run. With `includeVaultSiblings` the two folders beside the vault
 *  join Projects and pricing-study's older copy goes to Archive. */
export function mockMoveHomePlan(options: MoveHomeOptions = {}): MoveHomePlan {
  const siblings = !!options.includeVaultSiblings;
  const moves: PlannedMove[] = [
    move(VAULT, `${H}/Vault`, 'vault', 2.1 * GB, 18_400, 0, 'The vault', `${H}/Vault`, 'copy'),
    move(PROJECTS, `${H}/Projects`, 'projectsRoot', 41 * GB, 96_300, 1_200, 'The Projects folder, as a whole', `${H}/Projects`),
    move(`${H}/Projects/personal`, `${H}/Projects/Personal`, 'personal', 3.4 * GB, 7_800, 0, 'A child named personal'),
    move(`${H}/Projects/hollow-vault`, `${H}/Archive/hollow-vault`, 'archive', 6.7 * MB, 212, 0, 'A second vault (workspaces/ and AGENTS.md): never into Projects'),
    ...(siblings ? [
      move(BESIDE[0], `${H}/Projects/morning-paper`, 'project', 180 * MB, 640, 0, 'Beside the vault'),
      move(BESIDE[1], `${H}/Archive/pricing-study`, 'archive', 95 * MB, 310, 0, 'Same name in Projects, older content'),
    ] : []),
  ];
  const blockers = [
    ...(chatRunning() ? [{ kind: 'chat', detail: 'The chat “Quarterly review draft” is running (Claude, started 3 min ago).', blocking: true }] : []),
    { kind: 'process', detail: 'Obsidian has the vault open. It is pointed at the new path; quit it before the move.', blocking: false },
  ];
  return {
    version: 2,
    created: '2026-11-12T16:14:00Z',
    hash: `mock-move-home-${siblings ? 'siblings' : 'base'}`,
    ready: !chatRunning(),
    partial: false,
    hollowHome: H,
    vaultFrom: VAULT,
    vaultTo: `${H}/Vault`,
    oldStem: OLD_STEM,
    newStem: NEW_STEM,
    moves,
    mkdirs: [`${H}/Repositories`, `${H}/Worktrees`, `${H}/Archive`, `${H}/Backups`],
    mappings: moves.map(m => [m.from, m.to] as [string, string]),
    besideVault: BESIDE,
    stores: [
      { from: `${OLD_STEM}.sqlite`, to: `${NEW_STEM}.sqlite`, what: 'Conversations' },
      { from: `${OLD_STEM}.search.sqlite`, to: `${NEW_STEM}.search.sqlite`, what: 'Search index' },
      { from: `${OLD_STEM}-directories.json`, to: `${NEW_STEM}-directories.json`, what: 'Connected projects' },
    ],
    columns: [
      { table: 'sessions', column: 'cwd', rows: 412 },
      { table: 'sessions', column: 'workspace', rows: 57 },
      { table: 'external_sessions', column: 'cwd', rows: 88 },
      { table: 'interactions', column: 'target_path', rows: 37 },
    ],
    files: [
      { path: `${HOME}/Library/Application Support/Hollow/config.json`, changes: 3, what: 'vault, projects and Hollow folder paths' },
      { path: `${H}/Vault/workspaces/local-5b1e0c7d92a4f3e6b8c1d0a9/WORKSPACE.md`, changes: 1, what: 'project path' },
    ],
    ids: [
      { old: 'local-5b1e0c7d92a4f3e6b8c1d0a9', new: 'local-e04c9a1b7f3d2c6e5a8b0f1d', projectFrom: `${PROJECTS}/3dprint`, projectTo: `${H}/Projects/3dprint`, hasWorkspace: true },
      { old: 'local-9a2f4c6e8b0d1f3a5c7e9b2d', new: 'local-1c3e5a7b9d0f2a4c6e8b1d3f', projectFrom: `${PROJECTS}/personal/garden-plan`, projectTo: `${H}/Projects/Personal/garden-plan`, hasWorkspace: false },
    ],
    orphans: [],
    collisions: [
      { kind: 'vaultCopy', path: `${PROJECTS}/hollow-vault`, detail: 'Old Mac/projects/hollow-vault is a second vault (6.7 MB, its own workspaces). It goes to Archive.', winner: null },
      ...(siblings ? [{ kind: 'sameName', path: 'pricing-study', detail: 'pricing-study is in both Old Mac/projects (newest file 2026-11-10) and beside the vault (newest file 2026-10-02).', winner: `${PROJECTS}/pricing-study` }] : []),
    ],
    dataless: {
      total: 1_200,
      bytes: 4_200_000_000,
      dirs: 0,
      toDownload: 0,
      toDownloadBytes: 0,
      downloadSecs: 0,
      copyFolders: [VAULT],
      copySecs: 45,
      roots: [{
        root: PROJECTS, count: 1_200, bytes: 4_200_000_000,
        top: [['photos-archive', 840], ['client-decks', 310], ['3dprint', 50]],
        sample: ['photos-archive/2019/IMG_0412.HEIC', 'client-decks/q3-pitch.key', '3dprint/bracket-v2.stl'],
      }],
      fileProviderDomain: 'iCloud Drive',
      partial: false,
    },
    sameVolume: true,
    freeBytes: 180 * GB,
    totalBytes: moves.reduce((n, m) => n + m.bytes, 0),
    blockers,
    processes: ['Obsidian (pid 812) has /Users/mock/Documents/projects/hollow-vault open'],
    launchAgents: [{ label: 'com.mock.notes-sync', path: `${HOME}/Library/LaunchAgents/com.mock.notes-sync.plist`, mentions: [VAULT], onPlan: false }],
    scripts: [{ label: 'notes-backup', path: `${HOME}/.local/bin/notes-backup`, mentions: [VAULT], onPlan: false }],
    repos: [
      { repoFrom: `${PROJECTS}/3dprint`, repoTo: `${H}/Projects/3dprint`, worktrees: 2, broken: 1, skipped: null },
      { repoFrom: `${PROJECTS}/site`, repoTo: `${H}/Projects/site`, worktrees: 0, broken: 0, skipped: 'no linked worktrees' },
    ],
    agents: [
      { agent: 'Claude', path: `${HOME}/.claude/projects`, changes: 6, dead: 1, merges: 1, ambiguous: [] },
      { agent: 'Codex', path: `${HOME}/.codex/config.toml`, changes: 2, dead: 0, merges: 0, ambiguous: [] },
    ],
    backup: { covered: null, detail: 'tmutil gave no answer.' },
    notes: siblings ? [] : ['The 2 folders beside the vault stay where they are unless you include them.'],
    text: [],
  };
}

const step = (name: string, s: MoveHomeStep['state']): MoveHomeStep => ({ step: name, state: s, detail: '', at: '2026-11-12T16:15:00Z' });

function statusFor(s: State): MoveHomeStatus {
  const steps = s === 'running' ? [...STEPS.slice(0, 5).map(n => step(n, 'done')), step(STEPS[5], 'started')]
    : s === 'done' ? STEPS.map(n => step(n, 'done'))
      : s === 'rolledBack' ? STEPS.map(n => step(n, 'undone'))
        : [];
  const checks = s === 'done' ? [
    { name: 'File counts', ok: true, detail: '122,712 files before and after' },
    { name: 'Conversations', ok: true, detail: '412 sessions before and after' },
    { name: 'Git worktrees', ok: true, detail: '2 worktrees, none prunable' },
  ] : [];
  const refuse = get('mockMoveHomeRefuse') === '1';
  return {
    state: s,
    backupDir: s === 'none' ? null : BACKUP,
    steps,
    checks,
    lastResult: s === 'done' ? `Moved into ${H}. Hollow opened on ${H}/Vault.` : s === 'rolledBack' ? 'Every folder and record is back where it was.' : null,
    canRollBack: s === 'done' || s === 'failed',
    rollbackRefusal: (s === 'done' || s === 'failed') && refuse
      ? `New notes landed in ${H}/Vault after the move was checked. Copy them out of ${H}/Vault first, then roll back.`
      : null,
  };
}

/** The journal; a staged or running move takes a step forward per call. */
export function mockMoveHomeStatus(): MoveHomeStatus {
  const s = state();
  if (s === 'staged') localStorage.setItem('mockMoveHomeState', 'running');
  if (s === 'running') {
    const polls = Number(get('mockMoveHomePolls') ?? 0) + 1;
    localStorage.setItem('mockMoveHomePolls', String(polls));
    if (polls >= 2) localStorage.setItem('mockMoveHomeState', 'done');
  }
  return statusFor(s);
}

export function mockMoveHomeStart(args?: Record<string, unknown>): MoveHomeStarted {
  const options = (args?.options ?? {}) as MoveHomeOptions;
  const plan = mockMoveHomePlan(options);
  if (args?.hash !== plan.hash) throw new Error('The plan changed since you checked it. Check the move again. Nothing moved.');
  if (!plan.ready) throw new Error('Another chat is running. Wait for it to finish, then check again. Nothing moved.');
  if (state() === 'staged' || state() === 'running') throw new Error('A move is already under way. Nothing new was started.');
  log('move_home_start', args?.hash);
  localStorage.setItem('mockMoveHomeState', 'staged');
  localStorage.setItem('mockMoveHomePolls', '0');
  return { backupDir: BACKUP, planFile: `${BACKUP}/plan.json`, helperPid: 4242, note: `Hollow quits in a moment, moves your folders into ${H} and opens again.` };
}

export function mockMoveHomeRollback(args?: Record<string, unknown>): MoveHomeRollback {
  const status = statusFor(state());
  if (!status.canRollBack) throw new Error('There is no move to roll back.');
  if (status.rollbackRefusal) throw new Error(status.rollbackRefusal);
  log('move_home_rollback', args?.backupDir ?? null);
  localStorage.setItem('mockMoveHomeState', 'rolledBack');
  return { staged: true, backupDir: BACKUP, note: 'Hollow quits in a moment, puts every folder and record back and opens again.' };
}

const COMMANDS = new Set(['move_home_plan', 'move_home_start', 'move_home_status', 'move_home_rollback']);
export const isMockMoveHomeCommand = (command: string) => COMMANDS.has(command);
export function mockMoveHomeCommand(command: string, args: Record<string, unknown>): unknown {
  if (command === 'move_home_plan') return mockMoveHomePlan((args.options ?? {}) as MoveHomeOptions);
  if (command === 'move_home_start') return mockMoveHomeStart(args);
  if (command === 'move_home_status') return mockMoveHomeStatus();
  return mockMoveHomeRollback(args);
}
