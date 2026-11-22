// Where an item lives, said the same way on every list and detail (overhaul
// H4b): "Tracked in Linear", "Stored in Hollow", "Read-only from Notion",
// "Linear unreachable since 9:40 AM", plus "Was tracked in Linear" after a
// switch. The words come from Rust (`sources::label`); this only draws them.
// Open in Linear opens the item's own link. Used by lazy pages only.
import { Show } from 'solid-js';
import type { SourceLabel } from '../generated/ipc';
import { labelTone } from '../sourcesSettings';
import { openExternal } from '../utils';

export function SourceLabelChip(props: { label: SourceLabel | null | undefined; withOpen?: boolean; class?: string }) {
  return (
    <Show when={props.label}>
      {label => (
        <span class={`source-label${props.class ? ` ${props.class}` : ''}`} data-kind={label().kind}>
          <span class="settings-state-chip" data-state={labelTone(label().kind)} title={label().wasTracked ?? undefined}>{label().text}</span>
          <Show when={label().wasTracked}>{was => <span class="source-label-was">{was()}</span>}</Show>
          <Show when={props.withOpen && label().url && label().openLabel}>
            <button type="button" class="btn-link source-label-open" onClick={() => void openExternal(label().url!)}>{label().openLabel}</button>
          </Show>
        </span>
      )}
    </Show>
  );
}
