import { For, Show, createEffect, createMemo, createSignal, on, type JSX } from 'solid-js';
import type { OfficePart } from './OfficePreview';
import { formulaBarText, isNumericCell } from '../sheetCells';

/** UN1: the Excel preview as a live sheet. One tab per sheet, a formula bar
 *  that shows the selected cell's formula (or its value), arrow-key and
 *  click selection, and numbers aligned right. Read-only: editing waits on a
 *  bundle measurement. Our own small grid, no spreadsheet library. */
export default function SheetGrid(props: { parts: OfficePart[] }): JSX.Element {
  const [tab, setTab] = createSignal(0);
  const [at, setAt] = createSignal<{ row: number; col: number } | null>(null);
  // A reloaded file is a new `parts`: keep the tab in range and clear the pick (its cell may be gone).
  createEffect(on(() => props.parts, parts => { if (tab() >= parts.length) setTab(0); setAt(null); }, { defer: true }));
  const part = createMemo(() => props.parts[Math.min(tab(), props.parts.length - 1)] as OfficePart | undefined);
  const cell = createMemo(() => {
    const pick = at();
    const row = pick && part()?.rows[pick.row];
    return pick && row ? { ref: `${part()!.columns[pick.col] ?? ''}${row.number}`, value: row.cells[pick.col] ?? '', formula: row.formulas?.[pick.col] } : null;
  });
  const move = (e: KeyboardEvent) => {
    const p = part();
    if (!p) return;
    const cur = at() ?? { row: 0, col: 0 };
    const d: Record<string, [number, number]> = { ArrowDown: [1, 0], ArrowUp: [-1, 0], ArrowRight: [0, 1], ArrowLeft: [0, -1] };
    const step = d[e.key];
    if (!step || !p.rows.length) return;
    e.preventDefault();
    const row = Math.max(0, Math.min(p.rows.length - 1, cur.row + step[0]));
    const col = Math.max(0, Math.min(p.columns.length - 1, cur.col + step[1]));
    setAt({ row, col });
    queueMicrotask(() => (e.currentTarget as HTMLElement).querySelector<HTMLElement>('[aria-current="true"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' }));
  };
  return <div class="sheet-grid">
    <div class="sheet-grid-tabs" role="tablist" aria-label="Sheets">
      <For each={props.parts}>{(p, i) => <button type="button" role="tab" class="sheet-grid-tab" aria-selected={tab() === i()} onClick={() => { setTab(i()); setAt(null); }}>{p.title}</button>}</For>
    </div>
    <div class="sheet-grid-bar">
      <span class="sheet-grid-ref" aria-label="Selected cell">{cell()?.ref ?? ''}</span>
      <span class="sheet-grid-fx" aria-hidden="true">fx</span>
      <output class="sheet-grid-formula" aria-label="Formula bar">{cell() ? formulaBarText(cell()!.value, cell()!.formula) : ''}</output>
    </div>
    <Show when={part()}>{p =>     <section class="office-preview-sheet" aria-label={`Sheet ${p().title}`}>
      <Show when={p().rows.length} fallback={<p class="office-preview-empty">No cells on this sheet.</p>}>
        <div class="office-preview-grid" tabIndex={0} role="region" aria-label={`${p().title} cells`} onKeyDown={move}>
          <table class="office-preview-table">
            <thead><tr><th scope="col"><span class="sr-only">Row</span></th>
              <For each={p().columns}>{column => <th scope="col">{column}</th>}</For>
            </tr></thead>
            <tbody><For each={p().rows}>{(row, r) => <tr>
              <th scope="row">{row.number}</th>
              <For each={row.cells}>{(text, c) => <td title={text} classList={{ 'sheet-grid-num': isNumericCell(text), 'sheet-grid-formula-cell': !!row.formulas?.[c()] }}
                aria-current={at()?.row === r() && at()?.col === c() ? 'true' : undefined}
                onClick={() => setAt({ row: r(), col: c() })}>{text}</td>}</For>
            </tr>}</For></tbody>
          </table>
        </div>
      </Show>
    </section>
    }</Show>
  </div>;
}
