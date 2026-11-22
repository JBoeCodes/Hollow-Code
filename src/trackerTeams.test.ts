import { describe, expect, it } from 'vitest';
import { createTeamKey, hasTwoTeams, knownTeamKeys, rowTeam, statusTeamKeys, teamChoices, teamQuery } from './trackerTeams';

describe('team choices on the Tasks page', () => {
  it('shows the controls only with two accepted teams', () => {
    expect(hasTwoTeams(['ENG'])).toBe(false);
    expect(hasTwoTeams(undefined)).toBe(false);
    expect(hasTwoTeams(['ENG', 'PER'])).toBe(true);
    expect(hasTwoTeams(['ENG', 'LP'])).toBe(false);
    expect(teamChoices(['ENG', 'PER', 'PER', 'XX'])).toEqual(['ENG', 'PER']);
  });
  it('queries one team or both', () => {
    expect(teamQuery(null, ['ENG', 'PER'])).toBeNull();
    expect(teamQuery('PER', ['ENG', 'PER'])).toBe('PER');
    expect(teamQuery('PER', ['ENG'])).toBeNull();
    expect(teamQuery('LP', ['ENG', 'PER'])).toBeNull();
  });
  it('sends a create team only with two teams', () => {
    expect(createTeamKey('PER', ['ENG'])).toBeUndefined();
    expect(createTeamKey('PER', ['ENG', 'PER'])).toBe('PER');
    expect(createTeamKey('ENG', ['ENG', 'PER'])).toBe('ENG');
    expect(createTeamKey('LP', ['ENG', 'PER'])).toBeUndefined();
  });
  it('chips a row only with two teams', () => {
    expect(rowTeam({ teamKey: 'PER' }, true)).toBe('PER');
    expect(rowTeam({ teamKey: 'PER' }, false)).toBeNull();
    expect(rowTeam({ teamKey: null }, true)).toBeNull();
  });
  it('reads the teams from a status and prefers the longer list', () => {
    expect(statusTeamKeys({ teamKey: 'ENG', personalTeamKey: 'PER' })).toEqual(['ENG', 'PER']);
    expect(statusTeamKeys({ teamKey: 'ENG', personalTeamKey: null })).toEqual(['ENG']);
    expect(statusTeamKeys(null)).toEqual([]);
    expect(knownTeamKeys(['ENG'], ['ENG', 'PER'])).toEqual(['ENG', 'PER']);
    expect(knownTeamKeys(['ENG', 'PER'], ['ENG'])).toEqual(['ENG', 'PER']);
  });
});
