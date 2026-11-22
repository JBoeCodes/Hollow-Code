// Repo features R5: a Run record's Cites row, loaded only when a receipt has
// cites (`ExecutionReceipt.tsx` lazy-loads it, keeping it out of the App
// chunk). Opening it lists the problem cites, `path:line · status`.
import { For, Show } from 'solid-js';
import type { ReceiptCitesWire } from '../executionReceipt';
import { citeItemLine, citesLine } from '../receiptCites';
import { openFileInSidePane } from '../store/chatRuntime';

export default function ReceiptCites(props: { cites: ReceiptCitesWire }) {
  return (
    <details class="execution-receipt-row execution-receipt-cites">
      <summary>
        <span class="toolcall-verb">Cites</span>
        <span class="toolcall-extra mono"> · {citesLine(props.cites)}</span>
      </summary>
      <ul class="execution-receipt-list">
        <For each={props.cites.items ?? []}>
          {(item) => <li class="execution-receipt-row mono">
            <Show when={item.target?.startsWith('/')} fallback={<span>{citeItemLine(item)} · file location not recorded</span>}>
              <button type="button" class="panel-link" onClick={() => openFileInSidePane(item.target!, { line: item.line, end: item.end })}>{citeItemLine(item)}</button>
            </Show>
          </li>}
        </For>
        <li class="execution-receipt-row toolcall-extra">Checked when the run ended.</li>
      </ul>
    </details>
  );
}
