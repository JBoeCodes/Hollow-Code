/**
 * Line diffs and a three-way merge for file conflicts (next-level P08).
 *
 * When a save finds the file changed on disk, the editor has three texts:
 * the base the draft started from, the file now ("theirs") and the draft
 * ("mine"). `merge3` keeps every non-overlapping change from both sides and
 * marks overlapping ones as conflicts for the user to settle; it never picks
 * a side silently. Pure, unit-tested. Bounded: beyond MAX_LINES per side the
 * merge reports one whole-file conflict instead of diffing.
 *
 * Lines are compared without their line breaks, and the merged text is joined
 * with the file's own line break (`eol`: the first one in the file on disk,
 * else the base, else the draft), so resolving a conflict in a CRLF file
 * doesn't convert it to LF (review A, slice 4 item 1d).
 */

export type DiffOp = { kind: 'same' | 'add' | 'del'; text: string };

const MAX_LINES = 4000;

export const splitLines = (text: string) => text === '' ? [] : text.replace(/\r\n/g, '\n').split('\n');

/** Longest-common-subsequence line diff from `a` to `b`. */
export function diffLines(a: string[], b: string[]): DiffOp[] {
  const n = a.length, m = b.length;
  // Trim the common prefix and suffix first: most edits are local.
  let start = 0;
  while (start < n && start < m && a[start] === b[start]) start++;
  let endA = n, endB = m;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) { endA--; endB--; }
  const head: DiffOp[] = a.slice(0, start).map(text => ({ kind: 'same', text }));
  const tail: DiffOp[] = a.slice(endA).map(text => ({ kind: 'same', text }));
  const A = a.slice(start, endA), B = b.slice(start, endB);
  if (A.length * B.length > 16_000_000) {
    return [...head, ...A.map(text => ({ kind: 'del' as const, text })), ...B.map(text => ({ kind: 'add' as const, text })), ...tail];
  }
  const w = B.length + 1;
  const dp = new Uint32Array((A.length + 1) * w);
  for (let i = A.length - 1; i >= 0; i--) for (let j = B.length - 1; j >= 0; j--) {
    dp[i * w + j] = A[i] === B[j] ? dp[(i + 1) * w + j + 1] + 1 : Math.max(dp[(i + 1) * w + j], dp[i * w + j + 1]);
  }
  const mid: DiffOp[] = [];
  let i = 0, j = 0;
  while (i < A.length && j < B.length) {
    if (A[i] === B[j]) { mid.push({ kind: 'same', text: A[i] }); i++; j++; }
    else if (dp[(i + 1) * w + j] >= dp[i * w + j + 1]) { mid.push({ kind: 'del', text: A[i++] }); }
    else mid.push({ kind: 'add', text: B[j++] });
  }
  while (i < A.length) mid.push({ kind: 'del', text: A[i++] });
  while (j < B.length) mid.push({ kind: 'add', text: B[j++] });
  return [...head, ...mid, ...tail];
}

/** A change to a range of base lines: replace base[start, end) with `lines`. */
interface Hunk { start: number; end: number; lines: string[] }

function hunks(base: string[], other: string[]): Hunk[] {
  const out: Hunk[] = [];
  let at = 0;
  let cur: Hunk | null = null;
  for (const op of diffLines(base, other)) {
    if (op.kind === 'same') { if (cur) { out.push(cur); cur = null; } at++; continue; }
    if (!cur) cur = { start: at, end: at, lines: [] };
    if (op.kind === 'del') { cur.end = ++at; } else cur.lines.push(op.text);
  }
  if (cur) out.push(cur);
  return out;
}

export interface MergeConflict { base: string[]; mine: string[]; theirs: string[] }
export type MergeChunk = { kind: 'clean'; lines: string[] } | ({ kind: 'conflict' } & MergeConflict);
export type Eol = '\n' | '\r\n';
export interface MergeResult { chunks: MergeChunk[]; conflicts: number; /** The merged text with conflict markers, for hand editing. */ text: string; /** The line break `text` uses. */ eol: Eol }

/** The first line break in `text`, if it has one. */
const firstEol = (text: string): Eol | undefined => (/\r?\n/.exec(text)?.[0] as Eol | undefined);

/** `text` with every line break written as `eol` (a hand-edited merge comes
 *  back from a textarea with LF). */
export const withEol = (text: string, eol: Eol) => text.replace(/\r?\n/g, eol);

export const MARK_MINE = '<<<<<<< Your draft';
export const MARK_SPLIT = '=======';
export const MARK_THEIRS = '>>>>>>> File on disk';

/** Three-way merge of line arrays (`base`, the file now, the draft). */
export function merge3(baseText: string, theirsText: string, mineText: string): MergeResult {
  const base = splitLines(baseText), theirs = splitLines(theirsText), mine = splitLines(mineText);
  const eol = firstEol(theirsText) ?? firstEol(baseText) ?? firstEol(mineText) ?? '\n';
  if (base.length > MAX_LINES || theirs.length > MAX_LINES || mine.length > MAX_LINES) {
    const chunk: MergeChunk = { kind: 'conflict', base, mine, theirs };
    return { chunks: [chunk], conflicts: 1, text: render([chunk], eol), eol };
  }
  const a = hunks(base, mine), b = hunks(base, theirs);
  const chunks: MergeChunk[] = [];
  let pos = 0, i = 0, j = 0;
  const clean = (lines: string[]) => { if (!lines.length) return; const last = chunks[chunks.length - 1]; if (last?.kind === 'clean') last.lines.push(...lines); else chunks.push({ kind: 'clean', lines: [...lines] }); };
  while (i < a.length || j < b.length) {
    const ha = a[i], hb = b[j];
    // Take the earlier hunk; group every hunk that overlaps it from either side.
    const first = !hb || (ha && ha.start <= hb.start) ? ha : hb;
    let start = first.start, end = first.end;
    const groupA: Hunk[] = [], groupB: Hunk[] = [];
    let grew = true;
    while (grew) {
      grew = false;
      while (i < a.length && overlaps(a[i], start, end)) { groupA.push(a[i]); start = Math.min(start, a[i].start); end = Math.max(end, a[i].end); i++; grew = true; }
      while (j < b.length && overlaps(b[j], start, end)) { groupB.push(b[j]); start = Math.min(start, b[j].start); end = Math.max(end, b[j].end); j++; grew = true; }
    }
    clean(base.slice(pos, start));
    const side = (group: Hunk[]) => apply(base, start, end, group);
    if (!groupA.length) clean(side(groupB));
    else if (!groupB.length) clean(side(groupA));
    else {
      const m = side(groupA), t = side(groupB);
      if (m.join('\n') === t.join('\n')) clean(m);
      else chunks.push({ kind: 'conflict', base: base.slice(start, end), mine: m, theirs: t });
    }
    pos = end;
  }
  clean(base.slice(pos));
  return { chunks, conflicts: chunks.filter(c => c.kind === 'conflict').length, text: render(chunks, eol), eol };
}

function overlaps(h: Hunk, start: number, end: number) {
  // Insertions at the same point overlap too; touching ranges do not.
  return h.start < end || (h.start === start && h.start === h.end) || (h.start === end && start === end) || (h.start <= start && h.end > start);
}

function apply(base: string[], start: number, end: number, group: Hunk[]): string[] {
  const out: string[] = [];
  let pos = start;
  for (const h of group) { out.push(...base.slice(pos, h.start), ...h.lines); pos = h.end; }
  out.push(...base.slice(pos, end));
  return out;
}

function render(chunks: MergeChunk[], eol: Eol): string {
  const out: string[] = [];
  for (const c of chunks) {
    if (c.kind === 'clean') out.push(...c.lines);
    else out.push(MARK_MINE, ...c.mine, MARK_SPLIT, ...c.theirs, MARK_THEIRS);
  }
  return out.join(eol);
}

/** Whether a hand-merged text still has conflict markers. */
export const hasConflictMarkers = (text: string) => splitLines(text).some(l => l === MARK_MINE || l === MARK_THEIRS);

/** Added/removed line counts, for a compact "+3 −1" summary. */
export function diffStats(ops: DiffOp[]) {
  let add = 0, del = 0;
  for (const op of ops) { if (op.kind === 'add') add++; else if (op.kind === 'del') del++; }
  return { add, del };
}
