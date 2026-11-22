/**
 * Task proposals from a recording (next-level P11). Pure, unit-tested.
 *
 * Only what the recording's summary lists as an action, next step, to-do or
 * follow-up becomes a proposal; nothing is inferred from conversation alone.
 * Each proposal carries the transcript timestamp it most likely came from
 * (by shared words), an owner only when the text names one explicitly
 * ("Dana: …", "(owner: Dana)"), and a due date only when written as a date.
 * Unknown stays unknown. Creation uses an idempotency key derived from the
 * recording and the text, so retrying never makes a twin.
 */
export interface Segment { start: string; end: string; speaker: string | null; text: string }
export interface TaskProposal { text: string; owner: string | null; due: string | null; at: string | null; quote: string | null; idempotencyKey: string; origin: string }

const KEYS = '(?:action (?:items?|points?)|actions|next steps?|to[- ]?dos?|follow[- ]?ups?|tasks)';
/** A heading that opens the actions section: a Markdown or bold heading, or a
 *  plain line starting with one of the words ("Action items:"). */
const HEAD = new RegExp(`^(#{1,6}\\s*|\\*\\*)?\\s*${KEYS}\\b`, 'i');
/** An outline-numbered heading ("3. Action Items", "2) Next steps:"): the
 *  whole line is the heading words, so a numbered sentence never counts
 *  (durability D4). */
const NUMBERED_HEAD = new RegExp(`^(?:\\d{1,2}|[ivx]{1,4}|[a-h])[.)]\\s+${KEYS}\\s*:?$`, 'i');
/** A plain heading line of another section ("Risks:", "Open questions:"). */
const OTHER_HEAD = /^[A-Z][^:.!?]{0,60}:$/;
/** An outline-numbered line: a heading or a numbered item. */
const NUMBERED_LINE = /^(?:\d{1,2}|[ivx]{1,4}|[a-h])[.)]\s+\S/i;
const STOP = new Set('the a an and or to of for in on with we our be is are will this that it at by from as need needs should can'.split(' '));
const words = (s: string) => new Set(s.toLowerCase().replace(/[^a-z0-9 ]+/g, ' ').split(/\s+/).filter(w => w.length > 2 && !STOP.has(w)));
export const normalizeTask = (s: string) => s.replace(/\s+/g, ' ').trim().replace(/[.;]+$/, '');

/** "4. Risks", "5. Open Questions", "6. Notes:": the words after the number
 *  are capitalized (small joining words aside) or end with a colon. */
function titleLike(line: string): boolean {
  const body = line.replace(/^\S+\s+/, '');
  if (body.endsWith(':')) return true;
  return body.split(/\s+/).every(w => /^[A-Z&]/.test(w) || /^(?:and|of|to|&)$/.test(w));
}

export function extractProposals(recordingId: string, summary: string | null, transcript: Segment[] | null): TaskProposal[] {
  if (!summary) return [];
  const out: TaskProposal[] = [];
  let inside = false;
  // Opened by an outline-numbered heading: the next one closes it.
  let numbered = false;
  for (const raw of summary.split('\n')) {
    const line = raw.trim();
    if (!line) continue;
    const plain = line.replace(/[*_]/g, '');
    if (NUMBERED_HEAD.test(plain)) { inside = true; numbered = true; continue; }
    if (HEAD.test(plain)) { inside = true; numbered = false; continue; }
    if (/^#{1,6}\s/.test(line) || (/^\*\*.+\*\*:?$/.test(line)) || OTHER_HEAD.test(plain)
      || (numbered && NUMBERED_LINE.test(plain) && !/[.!?]$/.test(plain) && titleLike(plain))) { inside = false; numbered = false; continue; }
    if (!inside) continue;
    const m = /^(?:[-*•]|\d+[.)]|\[ \])\s+(.+)$/.exec(line);
    if (!m) continue;
    let text = normalizeTask(m[1].replace(/\*\*/g, ''));
    let owner: string | null = null;
    const lead = /^([A-Z][a-zA-Z]+(?: [A-Z][a-zA-Z]+)?):\s+(.+)$/.exec(text);
    if (lead) { owner = lead[1]; text = lead[2]; }
    const tagged = /\((?:owner|assigned to)[:\s]+([^)]+)\)/i.exec(text);
    if (tagged) { owner = tagged[1].trim(); text = normalizeTask(text.replace(tagged[0], '')); }
    const due = /\b(20\d\d-\d\d-\d\d)\b/.exec(text)?.[1] ?? null;
    const want = words(text);
    let best: Segment | null = null, score = 0;
    for (const s of transcript ?? []) {
      const have = words(s.text); let n = 0;
      for (const w of want) if (have.has(w)) n++;
      if (n > score) { score = n; best = s; }
    }
    const hit = best && score >= Math.min(3, Math.max(2, Math.floor(want.size / 2))) ? best : null;
    if (!text || out.some(p => p.text.toLowerCase() === text.toLowerCase())) continue;
    out.push({ text, owner, due, at: hit?.start ?? null, quote: hit ? hit.text.slice(0, 240) : null, idempotencyKey: `recording:${recordingId}\n${text.toLowerCase()}`, origin: `meeting:${recordingId}` });
  }
  return out;
}
