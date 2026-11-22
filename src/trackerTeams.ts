// Two Linear teams in the Tasks page and its create flows: the shared team
// (Engineering, key AI) and the person's own (Personal, key HOL). The
// pure choices behind `pages/tasks/TeamChoice.tsx`: the team filter's
// options and query value, and which create choices to offer. Both controls
// show only while the mirror holds two teams, so a single-team setup looks
// exactly as before.

import type { TrackerViewRow } from './generated/ipc';
import { isTeamKey } from './trackerSettings';

/** Team keys shown as choices, in the order given (shared first); keys Hollow doesn't accept are dropped. */
export function teamChoices(teamKeys: readonly string[] | null | undefined): string[] {
  return [...new Set((teamKeys ?? []).filter(isTeamKey))];
}

/** Whether to show the team filter, chips and create choice: two or more teams. */
export const hasTwoTeams = (teamKeys: readonly string[] | null | undefined) => teamChoices(teamKeys).length > 1;

/** The `teamKey` for `tracker_view`: the chosen key, or null (both teams). */
export function teamQuery(choice: string | null | undefined, teamKeys: readonly string[] | null | undefined): string | null {
  return choice && teamChoices(teamKeys).includes(choice) ? choice : null;
}

/** The `teamKey` for a create call: only sent when two teams exist (the backend defaults to AI). */
export function createTeamKey(choice: string | null | undefined, teamKeys: readonly string[] | null | undefined): string | undefined {
  return hasTwoTeams(teamKeys) && isTeamKey(choice) && teamChoices(teamKeys).includes(choice) ? choice : undefined;
}

/** A row's team key chip text, only with two teams. */
export function rowTeam(row: Pick<TrackerViewRow, 'teamKey'>, show: boolean): string | null {
  return show && row.teamKey ? row.teamKey : null;
}

export const TEAM_NAMES: Record<string, string> = { ENG: 'Engineering', PER: 'Personal' };
export const teamName = (key: string) => TEAM_NAMES[key] ?? key;

/** The team keys a status says are set up (shared first), for screens that hold no view. */
export function statusTeamKeys(s: { teamKey?: string | null; personalTeamKey?: string | null } | null | undefined): string[] {
  return teamChoices([s?.teamKey ?? '', s?.personalTeamKey ?? '']);
}

/** The keys a page shows: the view's own list, or the status's when it names more. */
export function knownTeamKeys(viewKeys: readonly string[] | null | undefined, fromStatus: readonly string[]): string[] {
  const a = teamChoices(viewKeys);
  return fromStatus.length > a.length ? [...fromStatus] : a;
}
