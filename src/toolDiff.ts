/**
 * Line diffs of agent edits: the red/green body an Edit row expands to, and
 * the `+a −d` counts its header shows. Pure, so the node vitest suite can
 * load it.
 */

function obj(v: unknown): Record<string, unknown> {
  return v && typeof v === 'object' ? (v as Record<string, unknown>) : {};
}
function str(v: unknown): string {
  return typeof v === 'string' ? v : '';
}

export type DLineT = 'ctx' | 'add' | 'del' | 'gap';
export interface DLine { t: DLineT; v: string }

export function editDiff(name: string, input: unknown): DLine[] | null {
  const i = obj(input);
  if (name === 'Edit') return lineDiff(str(i.old_string), str(i.new_string));
  if (name === 'NotebookEdit') return lineDiff(str(i.old_source), str(i.new_source));
  if (name === 'MultiEdit' && Array.isArray(i.edits)) {
    const out: DLine[] = [];
    (i.edits as unknown[]).forEach((e, idx) => {
      const ed = obj(e);
      if (idx > 0) out.push({ t: 'gap', v: '' });
      out.push(...lineDiff(str(ed.old_string), str(ed.new_string)));
    });
    return out.length ? out : null;
  }
  return null;
}

/** Line-level LCS diff. Bounded: for very large blocks the O(n·m) table is
 *  skipped in favor of a plain delete-all / add-all so the UI never stalls. */
export function lineDiff(a: string, b: string): DLine[] {
  // An empty side has no lines: inserting into nothing is not "−1".
  const al = a === '' ? [] : a.split('\n');
  const bl = b === '' ? [] : b.split('\n');
  const n = al.length, m = bl.length;
  if (n * m > 200_000) {
    return [...al.map((v): DLine => ({ t: 'del', v })), ...bl.map((v): DLine => ({ t: 'add', v }))];
  }
  // dp[i][j] = LCS length of al[i:], bl[j:]
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = al[i] === bl[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const out: DLine[] = [];
  let i = 0, j = 0;
  while (i < n && j < m) {
    if (al[i] === bl[j]) { out.push({ t: 'ctx', v: al[i] }); i++; j++; }
    else if (dp[i + 1][j] >= dp[i][j + 1]) { out.push({ t: 'del', v: al[i] }); i++; }
    else { out.push({ t: 'add', v: bl[j] }); j++; }
  }
  while (i < n) { out.push({ t: 'del', v: al[i] }); i++; }
  while (j < m) { out.push({ t: 'add', v: bl[j] }); j++; }
  return out;
}

/** Lines added and removed by an edit call; null for any other tool. A
 *  `Write` counts every line of the new file as added. */
export function editStats(name: string, input: unknown): { additions: number; deletions: number } | null {
  if (name === 'Write') {
    const content = str(obj(input).content);
    return { additions: content ? content.replace(/\n$/, '').split('\n').length : 0, deletions: 0 };
  }
  const diff = editDiff(name, input);
  if (!diff) return null;
  let additions = 0, deletions = 0;
  for (const ln of diff) {
    if (ln.t === 'add') additions++;
    else if (ln.t === 'del') deletions++;
  }
  return { additions, deletions };
}
