// Repo features R5: the words of a Run record's Cites row
// (`cite_check::ReceiptCites`). Pure, and imported only by the lazy
// `components/ReceiptCites.tsx`, so none of it is in the App chunk.
import type { ReceiptCitesWire, ReceiptCiteWire } from './executionReceipt';

/** Each status, in the row's order, and how it reads after a count or on a
 *  listed cite. `unknown` is a file Hollow saw nothing open while a subagent,
 *  Workflow run or teammate ran, or for an agent that reports no tool calls. */
const CITE_WORDS = {
  seen: 'seen',
  fileOnly: 'outside the lines read',
  earlier: 'read in an earlier turn',
  notOpened: 'not opened',
  unknown: 'not seen here (a subagent or teammate may have read it)',
  pastEnd: 'past end of file',
  missing: 'no such file',
  unresolved: 'unresolved name',
} as const;

type CiteStatus = keyof typeof CITE_WORDS;

/** "9 checked · 7 seen · 1 not opened · 1 past end of file": zero parts left
 *  out. Null when nothing was checked. */
export function citesLine(cites: ReceiptCitesWire | null | undefined): string | null {
  if (!cites || !(cites.checked > 0)) return null;
  const parts = [`${cites.checked} checked`];
  for (const key of Object.keys(CITE_WORDS) as CiteStatus[]) {
    const n = cites[key];
    if (typeof n === 'number' && n > 0) parts.push(`${n} ${CITE_WORDS[key]}`);
  }
  return parts.join(' · ');
}

/** "src/chat.rs:12-20 · past end of file". */
export function citeItemLine(item: ReceiptCiteWire): string {
  const words = Object.hasOwn(CITE_WORDS, item.status) ? CITE_WORDS[item.status as CiteStatus] : item.status;
  const end = item.end && item.end > item.line ? `-${item.end}` : '';
  return `${item.path}:${item.line}${end} · ${words}`;
}
