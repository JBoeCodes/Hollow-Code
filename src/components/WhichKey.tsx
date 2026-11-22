import { For, Show } from 'solid-js';
import { GO_TARGETS, LEADER, leaderKeyOf } from '../keyboardLeader';
import { shortcutSettings } from '../shortcuts';
import { leaderPending, whichKeyOpen } from '../store/leader';
import '../styles/which-key.css';

/** The which-key card (five asks A4): after `g`, a moment's wait shows
 *  every key it can take next. The one live region says the leader is
 *  waiting; the card itself is a glance (the `?` card lists them too). */
export function WhichKey() {
  const rows = () => GO_TARGETS
    .map(t => ({ label: t.label, key: leaderKeyOf(t.id, shortcutSettings().leader ?? {}) }))
    .filter(r => r.key);
  return (
    <>
      <p class="sr-only" aria-live="polite">{leaderPending() ? `Go to: press a key, or Escape. The ? card lists them.` : ''}</p>
      <Show when={whichKeyOpen()}>
        <div class="which-key" aria-hidden="true">
          <div class="which-key-head"><kbd>{LEADER}</kbd> go to… <span class="which-key-esc">esc cancels</span></div>
          <ul class="which-key-grid">
            <For each={rows()}>{r => <li><kbd>{r.key}</kbd><span>{r.label}</span></li>}</For>
          </ul>
        </div>
      </Show>
    </>
  );
}
