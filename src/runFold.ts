/**
 * Finished-run folding (frontier F5). Once a reply's run is over, its work
 * (every tool step, and the narration between them) collapses into one quiet
 * row, "Worked for 1m 10s ›" (overnight 11-12, C1), above the answer, the way Claude
 * desktop and Codex show a finished turn. Pure (type-only imports), so the
 * node vitest suite can load it.
 */
import { parseSegments } from './chatSegments';
import { durationText, typicalLine } from './turnFooter';

export interface FoldableTurn {
  role: string;
  text: string;
  tools: readonly { status?: string; name?: string; result?: string; isError?: boolean; endedAt?: string }[];
  partial?: boolean;
  runId?: string;
  receipt?: { startedAt?: string; finishedAt?: string };
}

export interface RunFold {
  /** The reply's first assistant turn: the fold row renders here. */
  start: number;
  /** Last turn of the reply (inclusive). */
  end: number;
  /** The assistant turn whose rendered text is the answer (shown below the
   *  fold), or null when the reply draws no text. A hidden fence is not
   *  text. Its own tools go in the fold. */
  answer: number | null;
  /** The assistant turn whose text was written at or just before a question
   *  card, when that turn is not already the answer. Shown on its own, not
   *  inside "Worked for". Null when the reply asked no card, or the preface
   *  is the answer. */
  preface: number | null;
  /** The assistant turn whose text was written before the reply's first
   *  tool call, when it is not the answer (PER-13): what the agent said to
   *  the person before it started work stays visible, not folded into
   *  "Worked for". Null when the reply opened with a tool call. */
  opening: number | null;
  /** Assistant turns whose narration and tools the fold holds, in order. */
  members: number[];
  steps: number;
  failed: number;
  /** From the receipt's start to the reply's last step (the newest tool
   *  result time a live turn recorded), else to the receipt's finish. Null
   *  without a start. */
  durationMs: number | null;
}

/** What a turn renders as, given the folds (ChatBody reads this per turn). */
export type FoldRole = 'normal' | 'head' | 'hidden' | 'answer' | 'head-answer';

/** Every finished reply that used at least one tool, keyed by its first
 *  assistant turn. A reply is the run of non-user turns after a user turn
 *  (turns before the first user turn never fold). It is still live
 *  (never folded) while any of its turns streams, while it carries the live
 *  run id, or while it is the last reply and a turn is busy. */
export function runFolds(turns: readonly FoldableTurn[], live: { busy: boolean; liveRunId: string | null }): Map<number, RunFold> {
  const folds = new Map<number, RunFold>();
  let i = 0;
  while (i < turns.length) {
    if (turns[i].role === 'user') { i++; continue; }
    const start = i;
    while (i < turns.length && turns[i].role !== 'user') i++;
    const end = i - 1;
    const range = turns.slice(start, end + 1);
    const last = i >= turns.length;
    // Only a reply to a message folds: events with no user turn before them
    // (an adopted run's tail, a test harness) stay as they are.
    if (start === 0) continue;
    const running = range.some(t => t.partial || (!!live.liveRunId && t.runId === live.liveRunId)) || (last && live.busy);
    const assistants = range.map((t, k) => ({ t, k: start + k })).filter(x => x.t.role === 'assistant');
    const steps = assistants.reduce((n, x) => n + x.t.tools.length, 0);
    if (running || steps === 0) continue;
    const answerAt = [...assistants].reverse().find(x => hasShownText(x.t.text));
    const answer = answerAt ? answerAt.k : null;
    const preface = cardPreface(assistants, answer);
    const opening = openingText(assistants, answer);
    const members = assistants.filter(x => x.k !== answer || x.t.tools.length > 0).map(x => x.k);
    const failed = assistants.reduce((n, x) => n + x.t.tools.filter(tool => tool.status === 'error').length, 0);
    const receipt = [...range].reverse().find(t => t.receipt)?.receipt;
    const from = Date.parse(receipt?.startedAt ?? '');
    const to = lastStepAt(assistants.map(x => x.t), from) ?? Date.parse(receipt?.finishedAt ?? '');
    const durationMs = Number.isFinite(from) && Number.isFinite(to) && to >= from ? to - from : null;
    const head = assistants[0].k;
    folds.set(head, { start: head, end, answer, preface, opening, members, steps, failed, durationMs });
  }
  return folds;
}

/** The newest tool result time at or after `from` (C1: "Worked for" runs
 *  to the last step), or null when no step recorded one (a reloaded
 *  transcript), so the receipt's finish is used. */
function lastStepAt(turns: readonly FoldableTurn[], from: number): number | null {
  let last: number | null = null;
  for (const t of turns) for (const tool of t.tools) {
    const at = Date.parse(tool.endedAt ?? '');
    if (Number.isFinite(at) && at >= from && (last == null || at > last)) last = at;
  }
  return last;
}

/** Text the transcript draws. Question-card and next-message fences are
 *  hidden (`parseSegments`), so a fence-only turn is not an answer. */
function hasShownText(text: string): boolean {
  return parseSegments(text).length > 0;
}

/** A posted question card. A failed call, or a result that is not "Posted ",
 *  never put a card on screen (`AskRecorder` keeps the same rule). A call
 *  with no result text still counts: older transcripts stored only the name. */
function postedAsk(tool: { status?: string; name?: string; result?: string; isError?: boolean }): boolean {
  const name = tool.name ?? '';
  if (!name.includes('ask_user_questions') && !name.includes('hollow_questions')) return false;
  if (tool.isError || tool.status === 'error') return false;
  if (tool.result != null && tool.result !== '' && !tool.result.startsWith('Posted ')) return false;
  return true;
}

/** A question card: a posted hollow_questions tool, or a fenced hollow-questions block. */
function asksCard(turn: FoldableTurn): boolean {
  if (turn.tools.some(postedAsk)) return true;
  return /```hollow-questions\b/.test(turn.text);
}

/** The last rendered assistant text at or before the reply's last card,
 *  unless that turn is already the answer (it stays visible on its own).
 *  A fence-only card is not rendered text, so the explanation before it
 *  stays the preface. */
function cardPreface(assistants: readonly { t: FoldableTurn; k: number }[], answer: number | null): number | null {
  let cardAt: number | null = null;
  for (const row of assistants) if (asksCard(row.t)) cardAt = row.k;
  if (cardAt == null) return null;
  let preface: number | null = null;
  for (const row of assistants) {
    if (row.k > cardAt) break;
    if (hasShownText(row.t.text)) preface = row.k;
  }
  return preface == null || preface === answer ? null : preface;
}

/** The first assistant turn with rendered text, when no tool ran before it
 *  and it is not already the answer (PER-13). */
function openingText(assistants: readonly { t: FoldableTurn; k: number }[], answer: number | null): number | null {
  for (const row of assistants) {
    if (hasShownText(row.t.text)) return row.k === answer ? null : row.k;
    if (row.t.tools.length) return null;
  }
  return null;
}

/** Whether turn `i` of fold `f` keeps its text on screen beside the answer:
 *  a card's preface or the reply's opening words (PER-13). */
export function foldKeepsText(f: Pick<RunFold, 'answer' | 'preface'> & { opening?: number | null }, i: number): boolean {
  return i !== f.answer && (i === f.preface || i === f.opening);
}

/** Every turn's role under `folds` at once (overnight C5), in one pass over
 *  the folds' spans, so a long thread's rows don't each loop over every fold.
 *  System notices inside a folded reply stay visible. An index not in the map
 *  is 'normal'. */
export function foldRoles(folds: Map<number, RunFold>, turns: readonly { role: string }[]): Map<number, FoldRole> {
  const out = new Map<number, FoldRole>();
  for (const f of folds.values()) {
    for (let i = f.start; i <= f.end; i++) {
      if (out.has(i)) continue;
      const preface = foldKeepsText(f, i);
      out.set(i, turns[i]?.role !== 'assistant' ? 'normal'
        : i === f.start ? (f.answer === i || preface ? 'head-answer' : 'head')
        : i === f.answer || preface ? 'answer' : 'hidden');
    }
  }
  return out;
}

/** The fold row's words (overnight 11-12, C1): "Worked for 1m 10s", the
 *  way his reference reads; "Worked · 1 step" without a duration. A usual
 *  length for this kind of job, when one is known, follows. The step count
 *  of a timed fold moves to `foldSteps` (tooltip and screen readers). */
export function foldSummary(f: Pick<RunFold, 'steps' | 'durationMs'> & { typicalMs?: number | null; workKind?: string | null }): string {
  const base = f.durationMs == null ? `Worked · ${stepsText(f.steps)}` : `Worked for ${durationText(f.durationMs)}`;
  const typical = typicalLine(f.typicalMs, f.workKind);
  return typical ? `${base} · ${typical}` : base;
}

/** " · 12 steps" after a timed summary (read out, and in the row's
 *  tooltip); empty when the summary already counts them. */
export function foldSteps(f: Pick<RunFold, 'steps' | 'durationMs'>): string {
  return f.durationMs == null ? '' : ` · ${stepsText(f.steps)}`;
}

function stepsText(n: number): string {
  return `${n} ${n === 1 ? 'step' : 'steps'}`;
}

/** The reply's last assistant turn in `f`: the one whose footer carries the
 *  receipt, so its changed-file chips can sit above the fold row. */
export function foldTail(f: Pick<RunFold, 'start' | 'end'>, turns: readonly { role: string }[]): number {
  for (let i = f.end; i > f.start; i--) if (turns[i]?.role === 'assistant') return i;
  return f.start;
}
