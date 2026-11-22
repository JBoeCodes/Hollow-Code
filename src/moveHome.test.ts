import { beforeEach, describe, expect, it, vi } from 'vitest';
import { backupLine, baseName, confirmLine, groupBlockers, moveLine, moveSize, moveSummary, planSections, roughTime, rollbackState, shouldPoll, siblingsLine, startBlockedReason, statusLine, stepLabel } from './moveHome';
import { mockMoveHomeCommand, mockMoveHomePlan, mockMoveHomeRollback, mockMoveHomeStart, mockMoveHomeStatus } from './mockMoveHome';
import type { MoveHomePlan, MoveHomeStatus } from './types';

const storage = new Map<string, string>();
beforeEach(() => {
  storage.clear();
  vi.stubGlobal('localStorage', { getItem: (k: string) => storage.get(k) ?? null, setItem: (k: string, v: string) => { storage.set(k, v); }, removeItem: (k: string) => { storage.delete(k); } });
});

const status = (over: Partial<MoveHomeStatus> = {}): MoveHomeStatus =>
  ({ state: 'none', backupDir: null, steps: [], checks: [], lastResult: null, canRollBack: false, rollbackRefusal: null, ...over });

describe('Move Hollow home words (moveHome.ts)', () => {
  it('sizes and names read plainly', () => {
    expect(moveSize(0)).toBe('0 KB');
    expect(moveSize(512 * 1024)).toBe('512 KB');
    expect(moveSize(4.2 * 1024 ** 2)).toBe('4.2 MB');
    expect(moveSize(2.1 * 1024 ** 3)).toBe('2.1 GB');
    expect(moveSize(46 * 1024 ** 3)).toBe('46 GB');
    expect(baseName('/Users/mock/Documents/projects/morning-paper/')).toBe('morning-paper');
  });

  it('a move line names the kind, both paths, size, files and iCloud-only files', () => {
    const plan = mockMoveHomePlan();
    expect(moveLine(plan.moves[1])).toBe('Projects folder: /Users/mock/Documents/Old Mac/projects → /Users/mock/Hollow/Projects · 41 GB · 96,300 files, 1,200 in iCloud only (3.9 GB)');
    expect(moveLine(plan.moves[0])).toContain(' · copied out of iCloud');
    expect(moveSummary(plan)).toBe('Moves 4 folders (47 GB, 122,712 files) into /Users/mock/Hollow. 180 GB free. Same disk. 1 folder in iCloud is copied out (about 45 s), the rest renamed.');
    expect(moveSummary({ ...plan, moves: plan.moves.map(m => ({ ...m, method: 'rename' as const })) })).toContain('Same disk, so each move is a rename.');
    expect(moveSummary({ ...plan, moves: [] })).toBe('Nothing to move: the vault and Projects are already in /Users/mock/Hollow.');
  });

  it('the report covers moves, links, records, clashes, iCloud, blockers, repos, agents, login items, backup and notes', () => {
    storage.set('mockMoveHomeChatRunning', '1');
    const sections = planSections(mockMoveHomePlan());
    expect(sections.map(s => s.id)).toEqual(['moves', 'links', 'rekey', 'clashes', 'icloud', 'blockers', 'repos', 'agents', 'launch', 'backup', 'notes']);
    const text = (id: string) => sections.find(s => s.id === id)!.lines.join('\n');
    expect(text('moves')).toContain('Vault: /Users/mock/Documents/projects/hollow-vault → /Users/mock/Hollow/Vault');
    expect(text('moves')).toContain('(replaces the link at /Users/mock/Hollow/Vault)');
    expect(text('moves')).toContain('New folder: /Users/mock/Hollow/Repositories');
    expect(text('links')).toContain('/Users/mock/Documents/projects/hollow-vault links to /Users/mock/Hollow/Vault');
    expect(text('rekey')).toContain('Vault stores renamed from hollow-vault-0f3a9c2e71b4d855 to Vault-7c21e0b9a4d63f10');
    expect(text('rekey')).toContain('Conversations sessions.cwd: 412 rows');
    expect(sections.find(s => s.id === 'rekey')!.lines.filter(l => l.startsWith('Conversations '))).toHaveLength(4);
    expect(text('rekey')).toContain('Workspace local-5b1e0c7d92a4f3e6b8c1d0a9 becomes local-e04c9a1b7f3d2c6e5a8b0f1d for 3dprint');
    expect(text('clashes')).toContain('It goes to Archive.');
    expect(text('icloud')).toContain('1,200 files are in iCloud only (3.9 GB once downloaded).');
    expect(text('icloud')).toContain('1 folder in iCloud moves by copy, not rename');
    expect(text('icloud')).toContain('about 45 s for all of them');
    expect(text('icloud')).not.toContain('Download first');
    expect(text('icloud')).toContain('most in photos-archive (840), client-decks (310), 3dprint (50)');
    expect(text('icloud')).toContain('For example: photos-archive/2019/IMG_0412.HEIC');
    expect(sections.find(s => s.id === 'blockers')!.warn).toBe(true);
    expect(sections.find(s => s.id === 'blockers')!.lines[0]).toMatch(/Quarterly review draft.*\(stops the move\)$/);
    expect(text('repos')).toContain('/Users/mock/Hollow/Projects/3dprint: 2 worktrees to repair, 1 already broken (reported, left alone)');
    expect(text('repos')).toContain('skipped, no linked worktrees');
    expect(text('agents')).toContain('Claude (/Users/mock/.claude/projects): 6 changes, 1 folder merged, 1 for folders that no longer exist, reported and kept');
    expect(text('launch')).toContain('com.mock.notes-sync');
    expect(text('launch')).toContain('listed only, not changed');
    expect(text('backup')).toBe('Hollow couldn’t tell whether a backup covers the Hollow folder. tmutil gave no answer.');
  });

  it('a copy folder with files still in iCloud says what to download first and how long it takes', () => {
    const base = mockMoveHomePlan();
    const plan: MoveHomePlan = {
      ...base,
      ready: false,
      dataless: { ...base.dataless, total: 150_100, bytes: 40 * 1024 ** 3, toDownload: 150_100, toDownloadBytes: 40 * 1024 ** 3, downloadSecs: 3 * 3600 + 5 * 60, dirs: 2 },
      blockers: [{ kind: 'dataless', detail: '150,100 iCloud-only (evicted) files must be downloaded first. Run the command.', blocking: true }],
    };
    const lines = planSections(plan).find(s => s.id === 'icloud')!.lines.join('\n');
    expect(lines).toContain('Download first: 150,100 files (40 GB) in those folders, about 3 h 5 min.');
    expect(lines).toContain('2 folders are iCloud-only, so the size and time are a floor.');
    expect(startBlockedReason(plan, null)).toContain('must be downloaded first');
    expect(roughTime(40)).toBe('40 s');
    expect(roughTime(600)).toBe('10 min');
  });

  it('leaves out empty blocks', () => {
    const plan: MoveHomePlan = { ...mockMoveHomePlan(), collisions: [], repos: [], agents: [], launchAgents: [], scripts: [], notes: [], blockers: [], processes: [], dataless: { total: 0, bytes: 0, dirs: 0, toDownload: 0, toDownloadBytes: 0, downloadSecs: 0, copyFolders: [], copySecs: 0, roots: [], fileProviderDomain: null, partial: false } };
    expect(planSections(plan).map(s => s.id)).toEqual(['moves', 'links', 'rekey', 'backup']);
  });

  it('chats that block come first, then what is only told', () => {
    storage.set('mockMoveHomeChatRunning', '1');
    const { blocking, other } = groupBlockers(mockMoveHomePlan());
    expect(blocking.map(b => b.kind)).toEqual(['chat']);
    expect(other.map(b => b.kind)).toEqual(['process']);
  });

  it('Quit and move says why it is disabled, and is enabled only for a ready plan', () => {
    expect(startBlockedReason(null, null)).toBe('Check the move first.');
    expect(startBlockedReason(mockMoveHomePlan(), status())).toBe('');
    storage.set('mockMoveHomeChatRunning', '1');
    const blocked = mockMoveHomePlan();
    expect(blocked.ready).toBe(false);
    expect(startBlockedReason(blocked, status())).toBe('Wait for 1 chat to finish or stop it, then check again.');
    expect(startBlockedReason({ ...blocked, blockers: [], sameVolume: false }, null)).toMatch(/another disk/);
    expect(startBlockedReason({ ...mockMoveHomePlan(), moves: [] }, null)).toBe('Nothing to move.');
    storage.delete('mockMoveHomeChatRunning');
    expect(startBlockedReason(mockMoveHomePlan(), status({ state: 'running' }))).toBe('A move is already under way.');
  });

  it('the option names the folders beside the vault', () => {
    expect(siblingsLine(mockMoveHomePlan())).toBe('2 folders beside the vault: morning-paper, pricing-study. They join Projects.');
    expect(siblingsLine({ ...mockMoveHomePlan(), besideVault: [] })).toBe('');
    expect(confirmLine(mockMoveHomePlan())).toMatch(/^Hollow quits, moves 4 folders into \/Users\/mock\/Hollow and opens again\./);
  });

  it('backup says covered, not covered or unknown', () => {
    const plan = mockMoveHomePlan();
    expect(backupLine({ ...plan, backup: { covered: true, detail: '' } })).toBe('A Time Machine backup covers the Hollow folder.');
    expect(backupLine({ ...plan, backup: { covered: false, detail: '' } })).toMatch(/Back it up before you move/);
  });

  it('step names read as words, known or not', () => {
    expect(stepLabel('verify')).toBe('Checking the result');
    expect(stepLabel('move:vault')).toBe('Moving folders: vault');
    expect(stepLabel('rewrite_claude_json')).toBe('Rewrite claude json');
    expect(stepLabel('launchAgents')).toBe('Launch agents');
  });

  it('polls while staged or running; Roll back shows when the journal allows it', () => {
    expect(shouldPoll(null)).toBe(false);
    expect(shouldPoll(status({ state: 'staged' }))).toBe(true);
    expect(shouldPoll(status({ state: 'running' }))).toBe(true);
    expect(shouldPoll(status({ state: 'done' }))).toBe(false);
    expect(rollbackState(status({ state: 'done' }))).toEqual({ shown: false, disabledReason: '' });
    expect(rollbackState(status({ state: 'done', canRollBack: true }))).toEqual({ shown: true, disabledReason: '' });
    expect(rollbackState(status({ state: 'failed', canRollBack: true, rollbackRefusal: 'Copy new notes out first.' }))).toEqual({ shown: true, disabledReason: 'Copy new notes out first.' });
    expect(statusLine(status({ state: 'running', steps: [{ step: 'probe', state: 'done', detail: '', at: '' }, { step: 'snapshot', state: 'started', detail: '', at: '' }] }))).toBe('Moving. 1 of 2 steps done.');
    expect(statusLine(status({ state: 'done', checks: [{ name: 'a', ok: true, detail: '' }, { name: 'b', ok: false, detail: '' }] }))).toBe('Moved. 1 of 2 checks failed.');
  });

  it('writes no em or en dashes', () => {
    const plan = mockMoveHomePlan({ includeVaultSiblings: true });
    const all = [...planSections(plan).flatMap(s => [s.title, ...s.lines]), moveSummary(plan), confirmLine(plan), siblingsLine(plan), startBlockedReason(null, null)].join('\n');
    expect(all).not.toMatch(/[\u2013\u2014]/);
  });
});

describe('browser-mock Move Hollow home (mockMoveHome.ts)', () => {
  it('the plan has a vault move, a Projects move with Personal, an Archive move, 3 stores, 4 columns, 2 id changes and 1,200 iCloud-only files', () => {
    const plan = mockMoveHomePlan();
    expect(plan.moves.map(m => m.kind)).toEqual(['vault', 'projectsRoot', 'personal', 'archive']);
    expect(plan.besideVault).toHaveLength(2);
    expect(plan.stores).toHaveLength(3);
    expect(plan.columns).toHaveLength(4);
    expect(plan.ids).toHaveLength(2);
    expect(plan.dataless.total).toBe(1200);
    expect(plan.backup.covered).toBeNull();
    expect(plan.ready).toBe(true);
    expect(plan.blockers.some(b => b.blocking)).toBe(false);
  });

  it('including the folders beside the vault adds them, a clash with a winner, and a new hash', () => {
    const base = mockMoveHomePlan();
    const plan = mockMoveHomeCommand('move_home_plan', { options: { includeVaultSiblings: true } }) as MoveHomePlan;
    expect(plan.hash).not.toBe(base.hash);
    expect(plan.moves.map(m => m.to)).toContain('/Users/mock/Hollow/Projects/morning-paper');
    expect(plan.collisions.find(c => c.winner)?.winner).toBe('/Users/mock/Documents/Old Mac/projects/pricing-study');
  });

  it('start refuses a stale hash or a running chat and moves nothing', () => {
    expect(() => mockMoveHomeStart({ hash: 'old', options: {} })).toThrow(/plan changed/);
    storage.set('mockMoveHomeChatRunning', '1');
    expect(() => mockMoveHomeStart({ hash: mockMoveHomePlan().hash, options: {} })).toThrow(/Another chat is running/);
    expect(mockMoveHomeStatus().state).toBe('none');
    expect(storage.get('mockMoveHomeCalls')).toBeUndefined();
  });

  it('a started move goes staged, running, done, and then can be rolled back', () => {
    const started = mockMoveHomeStart({ hash: mockMoveHomePlan().hash, options: {} });
    expect(started.planFile).toBe(`${started.backupDir}/plan.json`);
    expect(JSON.parse(storage.get('mockMoveHomeCalls')!)).toEqual([{ command: 'move_home_start', hash: 'mock-move-home-base' }]);
    const seen = [mockMoveHomeStatus(), mockMoveHomeStatus(), mockMoveHomeStatus(), mockMoveHomeStatus()];
    expect(seen.map(s => s.state)).toEqual(['staged', 'running', 'running', 'done']);
    expect(seen[0].canRollBack).toBe(false);
    const done = seen[3];
    expect(done.canRollBack).toBe(true);
    expect(done.checks.every(c => c.ok)).toBe(true);
    expect(rollbackState(done).shown).toBe(true);
    const r = mockMoveHomeRollback({ backupDir: done.backupDir });
    expect(r.staged).toBe(true);
    const after = mockMoveHomeStatus();
    expect(after.state).toBe('rolledBack');
    expect(rollbackState(after).shown).toBe(false);
  });

  it('a refused rollback says why and changes nothing', () => {
    storage.set('mockMoveHomeState', 'done');
    storage.set('mockMoveHomeRefuse', '1');
    const s = mockMoveHomeStatus();
    expect(rollbackState(s)).toEqual({ shown: true, disabledReason: s.rollbackRefusal });
    expect(() => mockMoveHomeRollback({})).toThrow(/Copy them out/);
    expect(mockMoveHomeStatus().state).toBe('done');
  });
});
