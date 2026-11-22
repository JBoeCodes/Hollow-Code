import type { DecisionRecord } from './recordTypes';

/** Replaced or withdrawn: kept in the records, out of the lists. */
const isGone = (d: DecisionRecord) => d.status === 'superseded' || d.status === 'retracted';

/** Master plan #20: a workspace's own decisions, settled ones first, then
 *  those marked for revisit; superseded and retracted ones stay in the records
 *  but leave the list. Newest first within each. */
export function decisionRows(list: DecisionRecord[], workspace: string): DecisionRecord[] {
  const rank = (d: DecisionRecord) => (d.status === 'settled' ? 0 : 1);
  return list
    .filter(d => d.scope === workspace && !isGone(d))
    .sort((a, b) => rank(a) - rank(b) || b.decidedAt.localeCompare(a.decidedAt) || a.path.localeCompare(b.path));
}

/** "Settled 2026-10-04 by Ana", "Revisit (decided 2026-10-04)". */
export function decisionLabel(d: DecisionRecord): string {
  const by = d.decidedBy && d.decidedBy !== 'unknown' ? ` by ${d.decidedBy}` : '';
  return d.status === 'revisit' ? `Revisit (decided ${d.decidedAt}${by})` : `Settled ${d.decidedAt}${by}`;
}

/** Library, Records' Decisions (COO rebuild L4: they were on a workspace's
 *  Overview): every scope's, those marked for revisit first, then settled,
 *  newest first within each; superseded and retracted ones leave the list. */
export function libraryDecisionRows(list: DecisionRecord[]): DecisionRecord[] {
  const rank = (d: DecisionRecord) => (d.status === 'revisit' ? 0 : 1);
  return list
    .filter(d => !isGone(d))
    .sort((a, b) => rank(a) - rank(b) || b.decidedAt.localeCompare(a.decidedAt) || a.path.localeCompare(b.path));
}

/** Where a decision applies: its workspace's title, "Everywhere" for a
 *  global one, else the scope as written. */
export function decisionScope(scope: string, titleOf: (id: string) => string | null | undefined): string {
  return scope === 'global' ? 'Everywhere' : titleOf(scope) ?? scope;
}

/** DR1: what a proposed decision's `rejected` text (`option -> reason; option
 *  -> reason`) says, as plain text for the review card: "Rejected: Use Redis
 *  (too heavy); Use SQLite". Empty when there is none. */
export function rejectedLine(raw: unknown): string {
  if (typeof raw !== 'string') return '';
  const items = raw.split(';').map(part => {
    const [option, ...reason] = part.split('->');
    const why = reason.join('->').trim();
    return option.trim() ? (why ? `${option.trim()} (${why})` : option.trim()) : '';
  }).filter(Boolean);
  return items.length ? `Rejected: ${items.join('; ')}` : '';
}
