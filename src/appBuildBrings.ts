/** What the Build app panel shows for "what this install brings" (RF-787-F1/F2).
 *  Pure, unit-tested. When the comparison has a note and lists nothing, the note
 *  (a failure such as an unavailable base commit, or "adds no commits") is the
 *  visible line: there is no disclosure and no "0 pull requests, 0 direct commits",
 *  so a failed comparison never reads as "nothing new". Otherwise the counts head a
 *  closed disclosure and any note (for example the uncommitted-changes warning)
 *  is shown outside it. */
import type { InstallBringsReport } from './workspaceTypes';

type Brings = Pick<InstallBringsReport, 'note' | 'pullRequests' | 'commits'>;

export const plural = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;

/** The note stands in for the list: nothing was listed and the backend said why. */
export const noteOnly = (r: Brings): boolean => !!r.note && !r.pullRequests.length && !r.commits.length;

/** `summary`: the disclosure's summary line, or null when there is no disclosure.
 *  `note`: text shown visibly outside any disclosure, or null. */
export function bringsDisplay(r: Brings): { summary: string | null; note: string | null } {
  if (noteOnly(r)) return { summary: null, note: r.note };
  return {
    summary: `What this install brings · ${plural(r.pullRequests.length, 'pull request')}, ${plural(r.commits.length, 'direct commit')}`,
    note: r.note || null,
  };
}
