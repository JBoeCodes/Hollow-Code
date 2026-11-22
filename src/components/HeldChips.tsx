// Sends held for after a running turn (mid-turn steering, chatSteer.ts):
// one chip each, oldest first, with Cancel, under the chat's box.
import { For, Show } from 'solid-js';
import { Icon } from '../icons';
import type { HeldChip } from '../chatSteer';

export function HeldChips(props: { chips: HeldChip[]; onCancel: (id: string) => void }) {
  return (
    <Show when={props.chips.length > 0}>
      <ul class="chat-held-list" aria-label="Waiting for this turn to end">
        <For each={props.chips}>
          {(chip) => (
            <li class="chat-held-chip">
              <Icon name={chip.after === 'stop' ? 'stop' : 'clock'} size={12} />
              <span class="chat-held-kind">{chip.after === 'stop' ? 'Stopping, then sending' : 'Queued for after this turn'}</span>
              <span class="chat-held-text">{chip.text}</span>
              <button type="button" class="btn ghost sm" aria-label={`Cancel: ${chip.text.slice(0, 60)}`} title="Don't send it; put it back in the box" onClick={() => props.onCancel(chip.id)}>Cancel</button>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}
