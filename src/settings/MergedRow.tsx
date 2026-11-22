// One row, two or three small controls (Settings PR 5, plan 3.1 rule 2). Each
// control keeps its own name: `Mini` is a visible label that also names the
// control for Settings search (`data-settings-control`), so a search for
// "Max beats per day" still lands on its row.
import type { JSX } from 'solid-js';

export function Mini(props: { label: string; control?: string; children: JSX.Element }) {
  return (
    <label class="settings-mini" data-settings-control={props.control ?? props.label}>
      <span class="settings-row-v">{props.label}</span>
      {props.children}
    </label>
  );
}
