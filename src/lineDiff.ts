/** One line of a diff: kept, removed (only then) or added (only now). */
export interface DiffLine { kind: 'same' | 'removed' | 'added'; text: string }

/** A line diff from `before` to `after` (longest common subsequence),
 *  bounded: past `maxLines` lines on either side it reports the whole
 *  texts as removed and added rather than spending quadratic time. */
export function lineDiff(before: string, after: string, maxLines = 2000): DiffLine[] {
  const a = before.split('\n');
  const b = after.split('\n');
  if (a.length > maxLines || b.length > maxLines) {
    return [...a.map(text => ({ kind: 'removed' as const, text })), ...b.map(text => ({ kind: 'added' as const, text }))];
  }
  const lcs: number[][] = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) { out.push({ kind: 'same', text: a[i] }); i++; j++; }
    else if (lcs[i + 1][j] >= lcs[i][j + 1]) { out.push({ kind: 'removed', text: a[i] }); i++; }
    else { out.push({ kind: 'added', text: b[j] }); j++; }
  }
  while (i < a.length) out.push({ kind: 'removed', text: a[i++] });
  while (j < b.length) out.push({ kind: 'added', text: b[j++] });
  return out;
}

/** Only the changed lines, each with one line of context around it. */
export function changedOnly(lines: DiffLine[]): DiffLine[] {
  const keep = new Set<number>();
  lines.forEach((l, i) => { if (l.kind !== 'same') for (const k of [i - 1, i, i + 1]) if (k >= 0 && k < lines.length) keep.add(k); });
  return lines.filter((_, i) => keep.has(i));
}
