import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AuditEntry, TrackerHistory, TrackerIssueWork, TrackerStatus, TrackerView, WorkRun } from './generated/ipc';
import { isMockTrackerCommand, mockTrackerCommand } from './mockTracker';

const store = new Map<string, string>();

beforeEach(() => {
  store.clear();
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
  });
});

const seed = (value: Record<string, unknown>) => store.set('mockTracker', JSON.stringify(value));

describe('mock task tracker', () => {
  it('claims every tracker command and nothing else', () => {
    for (const command of [
      'tracker_status', 'tracker_view', 'tracker_issue', 'tracker_team_meta', 'tracker_sync', 'edit_tracker_item',
      'create_tracker_task', 'create_tracker_project', 'archive_tracker_task', 'set_tracker_key', 'clear_tracker_key',
      'use_keychain_tracker_key', 'set_tracker_settings', 'tracker_issue_work', 'tracker_issue_history',
    ]) expect(isMockTrackerCommand(command), command).toBe(true);
    for (const command of ['tasks', 'diagnostics', 'list_workflows']) expect(isMockTrackerCommand(command)).toBe(false);
  });

  it('plays a set-up tracker: open issues by default, closed ones on request, filters by search', () => {
    const status = mockTrackerCommand('tracker_status') as TrackerStatus;
    expect(status.state).toBe('ready');
    expect(status.key).toBe('saved');
    expect(JSON.stringify(status)).not.toMatch(/lin_api/);
    const open = mockTrackerCommand('tracker_view', {}) as TrackerView;
    expect(open.rows.every(r => r.statusKind !== 'completed' && r.statusKind !== 'canceled')).toBe(true);
    expect(open.states.map(s => s.kind)).toContain('backlog');
    const everything = mockTrackerCommand('tracker_view', { query: { includeClosed: true } }) as TrackerView;
    expect(everything.total).toBeGreaterThan(open.total);
    const found = mockTrackerCommand('tracker_view', { query: { search: 'bank' } }) as TrackerView;
    expect(found.rows.map(r => r.key)).toEqual(['ENG-31', 'ENG-33']);
  });

  it("gives this build's real answers when the engine is off, and the setup step when one is missing", () => {
    seed({ engine: false });
    const view = mockTrackerCommand('tracker_view', {}) as TrackerView;
    expect(view.rows).toEqual([]);
    expect(view.note).toMatch(/sync engine isn't part of this build/);
    expect(() => mockTrackerCommand('tracker_sync')).toThrow(/sync engine/);

    seed({ key: 'missing' });
    expect((mockTrackerCommand('tracker_status') as TrackerStatus).state).toBe('needsSetup');
    expect(() => mockTrackerCommand('tracker_team_meta')).toThrow('Save an API key.');
    expect((mockTrackerCommand('set_tracker_key', { key: 'abc123' }) as TrackerStatus).key).toBe('saved');
    expect(() => mockTrackerCommand('set_tracker_key', { key: 'two words' })).toThrow(/one line/);

    seed({ provider: null });
    expect((mockTrackerCommand('tracker_status') as TrackerStatus).state).toBe('off');
    expect(() => mockTrackerCommand('set_tracker_settings', { update: { pollSeconds: 30 } })).toThrow(/1 and 60 minutes/);
  });

  it('plays X1: tracking agent work is off until switched on, and an issue lists its seeded runs', () => {
    expect((mockTrackerCommand('tracker_status') as TrackerStatus).trackAgentWork).toBe(false);
    const on = mockTrackerCommand('set_tracker_settings', { update: { trackAgentWork: true } }) as TrackerStatus;
    expect(on.trackAgentWork).toBe(true);
    expect(mockTrackerCommand('tracker_issue_work', { issueId: 'i-1' })).toEqual({ runs: [], audit: [] });
    const run: WorkRun = {
      sessionKey: 's-20261110-sub00001', title: 'Fix the export', agent: 'claude', model: 'claude-sonnet-5-5', effort: 'high',
      status: 'done', startedAt: '2026-11-10T10:00:00Z', endedAt: '2026-11-10T10:20:00Z', workingMs: 1_200_000, tokens: 9_000,
      costUsd: 0.42, linkedBy: 'matched', accepted: false,
    };
    seed({ workRuns: { 'i-1': [run] } });
    expect((mockTrackerCommand('tracker_issue_work', { issueId: 'i-1' }) as TrackerIssueWork).runs).toEqual([run]);
    // X2: the issue's audit rows, as seeded.
    const row: AuditEntry = {
      id: 'a019a6f0c0000ffffffffffffffff', at: '2026-11-10T10:20:00Z', kind: 'tracker_write', sessionKey: 's-20261110-sub00001',
      parentKey: null, rootKey: null, askedBy: 's-20261110-sub00001', agent: null, model: null, effort: null, account: null,
      workspaceId: 'proj', issueId: 'i-1', issueKey: 'ENG-1', action: 'comment', fields: null, outcome: 'ok', detail: null, costUsd: null,
      tokensIn: null, tokensOut: null, workingMs: null, requestKey: 'work-1', prevHash: '', hash: 'h',
    };
    seed({ workAudit: { 'i-1': [row] } });
    expect((mockTrackerCommand('tracker_issue_work', { issueId: 'i-1' }) as TrackerIssueWork).audit).toEqual([row]);
  });

  it('gives ENG-31 a change history, the others none, and refuses when told to', () => {
    const { events, more } = mockTrackerCommand('tracker_issue_history', { id: 'i-31' }) as TrackerHistory;
    expect(events.find(e => e.kind === 'state')).toMatchObject({ actor: 'Alex Rivera', from: 'Todo', to: 'In Progress' });
    expect(more).toBe(false);
    expect(mockTrackerCommand('tracker_issue_history', { id: 'i-32' })).toEqual({ events: [], more: false });
    seed({ historyMore: true });
    expect((mockTrackerCommand('tracker_issue_history', { id: 'i-31' }) as TrackerHistory).more).toBe(true);
    expect(() => mockTrackerCommand('tracker_issue_history', { id: 'i-999' })).toThrow(/Sync first/);
    seed({ historyFails: true });
    expect(() => mockTrackerCommand('tracker_issue_history', { id: 'i-31' })).toThrow(/Couldn't reach/);
  });
});

describe('mock tracker conflicts (T2)', () => {
  it('seeded conflicts flag the row and fill the detail; saving the field settles it', () => {
    seed({ conflicts: { 'i-32': [{ field: 'title', local: 'Mine', remote: 'Theirs', at: '2026-11-11T09:00:00Z' }] } });
    const rows = (mockTrackerCommand('tracker_view', {}) as TrackerView).rows;
    expect(rows.filter(r => r.conflict).map(r => r.key)).toEqual(['ENG-32']);
    const detail = mockTrackerCommand('tracker_issue', { id: 'i-32' }) as { conflicts: { field: string }[] };
    expect(detail.conflicts.map(c => c.field)).toEqual(['title']);
    mockTrackerCommand('edit_tracker_item', { edit: { id: 'i-32', values: { title: 'Mine' }, expected: {} } });
    expect((mockTrackerCommand('tracker_view', {}) as TrackerView).rows.some(r => r.conflict)).toBe(false);
  });
});

describe('mock tracker personal team (Personal)', () => {
  const view = (query: Record<string, unknown> = {}) => mockTrackerCommand('tracker_view', { query: { includeClosed: true, ...query } }) as TrackerView;

  it('serves one team by default, with team keys on the rows', () => {
    const status = mockTrackerCommand('tracker_status') as TrackerStatus;
    expect(status.personalTeamKey).toBeNull();
    expect(status.personalTeamId).toBeNull();
    const v = view();
    expect(v.teamKeys).toEqual(['ENG']);
    expect(v.rows.every(r => r.teamKey === 'ENG' && r.key.startsWith('ENG-'))).toBe(true);
    expect(v.projects.every(p => p.teamKey === 'ENG')).toBe(true);
    expect(() => mockTrackerCommand('create_tracker_task', { task: { title: 'x', teamKey: 'PER' } })).toThrow(/isn't set up/);
  });

  it('adds HOL issues and a project once the personal team is set, and filters by team', () => {
    mockTrackerCommand('set_tracker_settings', { update: { personalTeamId: '22222222-2222-4222-8222-222222222222', personalTeamKey: 'PER' } });
    const status = mockTrackerCommand('tracker_status') as TrackerStatus;
    expect(status.personalTeamKey).toBe('PER');
    const both = view();
    expect(both.teamKeys).toEqual(['ENG', 'PER']);
    expect(both.rows.filter(r => r.teamKey === 'PER').map(r => r.key)).toEqual(['PER-1', 'PER-2', 'PER-3']);
    expect(both.projects.map(p => p.teamKey)).toContain('PER');
    const hol = view({ teamKey: 'PER' });
    expect(hol.rows.every(r => r.teamKey === 'PER')).toBe(true);
    expect(hol.projects.map(p => p.name)).toEqual(['Personal ops']);
    expect(hol.teamKeys).toEqual(['ENG', 'PER']);
    expect(view({ teamKey: 'ENG' }).rows.some(r => r.teamKey === 'PER')).toBe(false);
    mockTrackerCommand('set_tracker_settings', { update: { personalTeamId: '', personalTeamKey: '' } });
    expect(view().teamKeys).toEqual(['ENG']);
  });

  it('creates in the chosen team and refuses any other key', () => {
    seed({ personalTeamId: '22222222-2222-4222-8222-222222222222', personalTeamKey: 'PER' });
    const made = mockTrackerCommand('create_tracker_task', { task: { title: 'Note to self', teamKey: 'PER' } }) as { key: string };
    expect(made.key).toMatch(/^PER-/);
    const plain = mockTrackerCommand('create_tracker_task', { task: { title: 'Shared' } }) as { key: string };
    expect(plain.key).toMatch(/^ENG-/);
    expect(() => mockTrackerCommand('create_tracker_task', { task: { title: 'x', teamKey: 'LP' } })).toThrow(/ENG or PER only/);
    const project = mockTrackerCommand('create_tracker_project', { project: { name: 'Mine', teamKey: 'PER' } }) as { key: string };
    expect(project.key).toMatch(/^P-PER-/);
  });
});
