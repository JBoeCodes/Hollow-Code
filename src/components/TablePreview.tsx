import { For, Show, createMemo, createSignal } from 'solid-js';
import { drawnRow, summarizeTable, tableWindow } from '../tablePreview';
import { Button } from './ui';

const PAGE = 200;

/**
 * A read-only CSV/TSV preview (next-level P08). Every row is parsed, so the
 * count and the numeric totals row cover the whole file; only a window of
 * rows is rendered, extended on request, within `tableWindow`'s column and
 * cell limits (run 15 P1 second pass). The file itself is the export.
 */
export function TablePreview(props: { text: string; name: string }) {
  const table = createMemo(() => summarizeTable(props.text, props.name));
  const view = createMemo(() => tableWindow(table(), PAGE));
  const [shown, setShown] = createSignal(PAGE);
  const drawn = () => Math.min(shown(), view().maxRows);
  const more = () => Math.min(PAGE * 5, table().rows.length - drawn(), view().maxRows - drawn());
  const totals = () => table().totals.slice(0, view().columns);
  const fmt = (n: number | null) => n == null ? '' : n.toLocaleString(undefined, { maximumFractionDigits: 2 });
  return <div class="table-preview">
    <div class="table-preview-meta">
      <span>{table().rows.length.toLocaleString()} row{table().rows.length === 1 ? '' : 's'} · {table().header.length.toLocaleString()} column{table().header.length === 1 ? '' : 's'}</span>
      <Show when={table().rows.length > drawn()}><span>Showing the first {drawn().toLocaleString()}{more() > 0 ? '' : ' (open the file for the rest)'}; totals cover every row.</span></Show>
      <Show when={view().hiddenColumns > 0}><span>Showing the first {view().columns} columns; open the file for the other {view().hiddenColumns.toLocaleString()}.</span></Show>
    </div>
    <div class="table-preview-scroll" tabindex="0" role="region" aria-label={`${props.name} table`}>
      <table>
        <thead><tr><For each={table().header.slice(0, view().columns)}>{h => <th scope="col">{h}</th>}</For></tr></thead>
        {/* The parsed rows themselves, so Show more adds rows and keeps the drawn ones. */}
        <tbody><For each={table().rows.slice(0, drawn())}>{row => <tr><For each={drawnRow(row, view().columns)}>{(cell, i) => <td classList={{ num: table().numeric[i()] }}>{cell}</td>}</For></tr>}</For></tbody>
        <Show when={totals().some(t => t != null)}>
          <tfoot><tr><For each={totals()}>{(t, i) => <td classList={{ num: table().numeric[i()] }}>{i() === 0 && t == null ? 'Total' : fmt(t)}</td>}</For></tr></tfoot>
        </Show>
      </table>
    </div>
    <Show when={more() > 0}>
      <div class="table-preview-more"><Button size="sm" onClick={() => setShown(drawn() + more())}>Show {more().toLocaleString()} more rows</Button></div>
    </Show>
  </div>;
}
