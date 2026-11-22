// Shared overnight summary words for Home and Today.
// Pure, so the unit tests import it directly.
import type { OvernightSummary } from './types';

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** One line under the heading: what ran, then what needs a look. `waiting`
 *  is the live count (`waitingBadge`), not the summary's snapshot. */
export function overnightHeadline(s: OvernightSummary, waiting: number): string {
  const merged = s.pullRequests.filter(p => p.action === 'merged').length;
  const parts = [`${plural(s.runs, 'run')} in ${plural(s.conversations, 'conversation')}`];
  if (merged) parts.push(`${plural(merged, 'pull request')} merged`);
  if (s.failed.length) parts.push(`${s.failed.length} failed`);
  if (waiting) parts.push(`${waiting} waiting on you`);
  return parts.join(' · ');
}

/** One repository's moved head: `Hollow: aaaaaaaa → cccccccc (2 runs)`. */
export function commitLabel(c: OvernightSummary['commits'][number]): string {
  return `${c.repo}: ${c.from} → ${c.to}${c.runs > 1 ? ` (${c.runs} runs)` : ''}`;
}

/** `#451 · owner/repo`, with the conversation that did it. */
export function prLabel(p: OvernightSummary['pullRequests'][number]): string {
  const verb = p.action === 'merged' ? 'Merged' : 'Opened';
  return `${verb} #${p.number}${p.repo ? ` in ${p.repo}` : ''}${p.sessionTitle ? ` · ${p.sessionTitle}` : ''}`;
}

const RESULT_WORDS: Record<string, string> = {
  error: 'stopped with an error', timedOut: 'timed out', maxTurns: 'hit its turn limit',
  providerQuota: 'ran out of provider quota', auth: 'needed a sign-in', stalled: 'stalled',
  budgetStop: 'hit its budget', permissionDenied: 'was denied permission',
  transient: 'hit a temporary error', resumeFailed: "couldn't resume", refusal: 'was refused',
};

/** What went wrong, in words: never just a status code. */
export function failedLabel(f: OvernightSummary['failed'][number]): string {
  const what = f.failedChecks > 0
    ? `${plural(f.failedChecks, 'check')} failed`
    : RESULT_WORDS[f.resultKind ?? ''] ?? RESULT_WORDS[f.status] ?? 'did not finish';
  const who = f.sessionTitle
    ?? (f.source === 'chat' ? 'A conversation' : `${/^[aeiou]/i.test(f.source) ? 'An' : 'A'} ${f.source} run`);
  return `${who}: ${what}`;
}
