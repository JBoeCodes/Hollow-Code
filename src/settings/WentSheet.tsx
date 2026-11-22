import { For } from 'solid-js';
import { SettingsSheet } from './SettingsSheet';
import { WHERE_THINGS_WENT } from './whereThingsWent';

/** The "where things went" sheet opened by the Settings page header's link
 *  (Settings PR 9). The same list is in docs/WHATS_NEW.md, "Where everything in
 *  Settings went". Lazy: it mounts only when the link is clicked. */
export default function WhereThingsWent(props: { onClose: () => void; returnTo?: () => HTMLElement | null | undefined }) {
  return (
    <SettingsSheet id="settings-sheet-where-went" title="Where everything in Settings went" returnTo={props.returnTo} onClose={props.onClose}>
      <p class="settings-row-v">Every setting is still here. Old links and searches land on the new place.</p>
      <For each={WHERE_THINGS_WENT}>{g => (
        <section class="where-went-group">
          <h3>{g.from}</h3>
          <ul>
            <For each={g.rows}>{r => <li><strong>{r.what}</strong>: {r.now}</li>}</For>
          </ul>
        </section>
      )}</For>
    </SettingsSheet>
  );
}
