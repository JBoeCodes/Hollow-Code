import { For, Show, createMemo } from 'solid-js';
import '../styles/documents.css';
import { diffLines, diffStats, splitLines } from '../textMerge';

/**
 * A line diff between two texts (next-level P05/P08): unchanged runs fold to
 * a "N unchanged lines" divider with a little context around each change, so
 * a long document shows only what moved. Read-only; screen readers get the
 * +/− as text, not only colour.
 */
export function DiffView(props: { before: string; after: string; beforeLabel?: string; afterLabel?: string; context?: number; class?: string }) {
  const ops = createMemo(() => diffLines(splitLines(props.before), splitLines(props.after)));
  const stats = createMemo(() => diffStats(ops()));
  const rows = createMemo(() => {
    const ctx = props.context ?? 3;
    const list = ops();
    const keep = new Array(list.length).fill(false);
    list.forEach((op, i) => { if (op.kind !== 'same') for (let k = Math.max(0, i - ctx); k <= Math.min(list.length - 1, i + ctx); k++) keep[k] = true; });
    const out: ({ kind: 'same' | 'add' | 'del'; text: string } | { kind: 'fold'; count: number })[] = [];
    let folded = 0;
    list.forEach((op, i) => {
      if (keep[i]) { if (folded) { out.push({ kind: 'fold', count: folded }); folded = 0; } out.push(op); }
      else folded++;
    });
    if (folded) out.push({ kind: 'fold', count: folded });
    return out;
  });
  return <div class={`diff-view${props.class ? ` ${props.class}` : ''}`}>
    <div class="diff-view-head">
      <span>{props.beforeLabel ?? 'Before'} → {props.afterLabel ?? 'After'}</span>
      <span class="diff-view-stats"><b class="add">+{stats().add}</b> <b class="del">−{stats().del}</b></span>
    </div>
    <Show when={stats().add + stats().del > 0} fallback={<p class="diff-view-same">No differences.</p>}>
      {/* Focusable so the keyboard can scroll a long diff (overnight B8). */}
      <ol class="diff-view-lines" aria-label="Changed lines" tabIndex={0}>
        <For each={rows()}>{row => row.kind === 'fold'
          ? <li class="diff-fold">{row.count} unchanged line{row.count === 1 ? '' : 's'}</li>
          : <li class={`diff-line ${row.kind}`}><span class="diff-mark" aria-label={row.kind === 'add' ? 'added' : row.kind === 'del' ? 'removed' : undefined}>{row.kind === 'add' ? '+' : row.kind === 'del' ? '−' : ' '}</span><span class="diff-text">{row.text || ' '}</span></li>}
        </For>
      </ol>
    </Show>
  </div>;
}
