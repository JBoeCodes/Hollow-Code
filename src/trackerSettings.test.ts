import { describe, expect, it } from 'vitest';
import type { TrackerStatus } from './generated/ipc';
import { ALLOWED_TEAMS, ALLOWED_TEAM_KEYS, PERSONAL_TEAMS, isTeamKey, personalTeamLabel, canSyncNow, intervalChoice, requestsLeft, syncWhen, teamAllowed, teamLabel, trackerHeadline, trackerHealth } from './trackerSettings';

const AI = ALLOWED_TEAMS.linear;
const base: TrackerStatus = {
  state: 'ready', provider: 'linear', displayName: 'Linear', providers: [], teamId: AI.id, teamKey: 'ENG', personalTeamId: null, personalTeamKey: null, transport: 'graphql',
  pollSeconds: 120, trackAgentWork: false, fridayStatus: false, postStatusUpdates: false, key: 'saved', missing: null, blockedBy: null, lastSync: null, nextSync: null, lastError: null, rate: null,
  outbox: 0, problems: [], restartNeeded: false, engine: true,
};
const s = (over: Partial<TrackerStatus>): TrackerStatus => ({ ...base, ...over });

describe('task tracker settings words', () => {
  it('names the allowed team in full and nothing else', () => {
    expect(teamLabel(base)).toBe('Engineering (ENG)');
    expect(teamAllowed(base)).toBe(true);
    expect(teamAllowed(s({ teamId: 'other-team' }))).toBe(false);
    expect(teamLabel(s({ teamId: 'other-team', teamKey: 'LP' }))).toBe('team LP');
    expect(teamLabel(s({ teamId: null, teamKey: null }))).toBe('no team yet');
  });

  it('leads with what the person has to do', () => {
    expect(trackerHeadline(null)).toMatch(/Reading/);
    expect(trackerHeadline(s({ state: 'off' }))).toMatch(/^Off\./);
    expect(trackerHeadline(s({ state: 'needsSetup', missing: 'Save an API key.' }))).toBe('Linear is chosen but not set up: Save an API key.');
    expect(trackerHeadline(s({ state: 'blocked', blockedBy: 'Notion two-way sync is on.' }))).toBe('Notion two-way sync is on.');
    expect(trackerHeadline(s({ state: 'error', lastError: 'Linear refused the key.' }))).toMatch(/failed: Linear refused the key\./);
    expect(trackerHeadline(base)).toBe('Linear is on and syncing Engineering (ENG).');
  });

  it('dots: off, working, on, needs you', () => {
    expect(trackerHealth(undefined)).toBe('off');
    expect(trackerHealth(s({ state: 'off' }))).toBe('off');
    expect(trackerHealth(s({ state: 'syncing' }))).toBe('busy');
    expect(trackerHealth(base)).toBe('ok');
    expect(trackerHealth(s({ lastError: 'x' }))).toBe('attention');
    expect(trackerHealth(s({ state: 'needsSetup' }))).toBe('attention');
  });

  it('says when, both ways', () => {
    const now = Date.parse('2026-11-10T12:00:00Z');
    expect(syncWhen(null, now)).toBe('not yet');
    expect(syncWhen('2026-11-10T11:57:00Z', now)).toBe('3m ago');
    expect(syncWhen('2026-11-10T12:02:00Z', now)).toBe('in 2m');
    expect(syncWhen('2026-11-10T12:00:02Z', now)).toBe('now');
    expect(syncWhen('2026-11-10T09:00:00Z', now)).toBe('3h ago');
    expect(syncWhen('garbage', now, 'never')).toBe('never');
  });

  it('reports requests left only when the provider did', () => {
    expect(requestsLeft(base)).toBeNull();
    expect(requestsLeft(s({ rate: { requestsRemaining: 2431, requestsLimit: 2500, complexityRemaining: null, complexityLimit: null, resetAt: null } })))
      .toBe('2,431 of 2,500 requests left this hour');
    expect(requestsLeft(s({ rate: { requestsRemaining: 12, requestsLimit: null, complexityRemaining: null, complexityLimit: null, resetAt: null } })))
      .toBe('12 requests left this hour');
  });

  it('offers Sync now only when a pass can run', () => {
    expect(canSyncNow(base)).toBe(true);
    expect(canSyncNow(s({ state: 'error' }))).toBe(true);
    expect(canSyncNow(s({ state: 'needsSetup' }))).toBe(false);
    expect(canSyncNow(s({ state: 'blocked' }))).toBe(false);
    expect(canSyncNow(s({ engine: false }))).toBe(false);
    expect(canSyncNow(null)).toBe(false);
  });

  it('snaps the saved interval to a choice', () => {
    expect(intervalChoice(120)).toBe(2);
    expect(intervalChoice(60)).toBe(1);
    expect(intervalChoice(3600)).toBe(60);
    expect(intervalChoice(400)).toBe(5);
  });
});

describe('the personal team (Personal)', () => {
  it('offers exactly the shared team ENG and the personal team PER, with fixed ids', () => {
    expect([...ALLOWED_TEAM_KEYS]).toEqual(['ENG', 'PER']);
    expect(PERSONAL_TEAMS.linear).toEqual({ id: '22222222-2222-4222-8222-222222222222', key: 'PER', name: 'Personal' });
    expect(ALLOWED_TEAMS.linear.key).toBe('ENG');
    expect(Object.values(ALLOWED_TEAMS).concat(Object.values(PERSONAL_TEAMS)).map(t => t.key).every(isTeamKey)).toBe(true);
    expect(isTeamKey('LP')).toBe(false);
    expect(isTeamKey(null)).toBe(false);
  });
  it('labels the personal team, or null when none was added', () => {
    expect(personalTeamLabel(base)).toBeNull();
    expect(personalTeamLabel(s({ personalTeamId: PERSONAL_TEAMS.linear.id, personalTeamKey: 'PER' }))).toBe('Personal (PER)');
    expect(personalTeamLabel(s({ personalTeamId: 'other', personalTeamKey: 'XY' }))).toBe('team XY');
  });
});
