import { For, Show } from 'solid-js';
import { percentUsed, planLevel, resetText, type PlanWindow } from '../chatUsage';
import '../styles/chat-usage-menu.css';

/** One convention in chat and Settings: used percentage and used fill. */
export function PlanWindows(props: { windows: PlanWindow[] }) {
  return <For each={props.windows}>{w =>
    <div class="usage-menu-window" data-window={w.id}>
      <div class="usage-menu-window-head">
        <span>{w.label}</span><strong>{percentUsed(w.usedPercent)}</strong>
      </div>
      <div class="usage-menu-bar" role="meter" aria-label={w.label}
        aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.min(w.usedPercent, 100)}
        aria-valuetext={percentUsed(w.usedPercent)}
        data-level={planLevel(w.usedPercent)}>
        <span style={{ width: `${Math.min(Math.max(w.usedPercent, 0), 100)}%` }} />
      </div>
      <Show when={resetText(w.resetsAt)}>{text => <span class="usage-menu-reset">{text()}</span>}</Show>
    </div>
  }</For>;
}
