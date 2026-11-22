import { Show, createSignal } from 'solid-js';
import { api } from '../api';
import { agentSettings, refreshAgentSettings } from '../store';
import { agentSettingsPayload } from '../agentSettingsPayload';
import { ErrorText } from '../components/ui';
import { Switch } from '../components/Switch';
import { MoreFold } from './settingsLayout';
import { sleepHint } from '../store/chatMoveWords';

/** Settings, Devices, Macs (M5 UI, 2026-11-15): Move running chats to the
 *  runner when this Mac sleeps (`agents.moveOnSleep`, off unless saved on).
 *  Shown in the Macs panel once the runner takes chats (`sleepTo`), or while
 *  it is on, so it can be turned off. Saved at once with the whole payload;
 *  `moveOnSleep` is sent either way, since a save without it keeps the old
 *  choice (`carry_over`). */
export function MoveOnSleepRow(props: { runner: string | null }) {
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal('');
  const on = () => agentSettings().moveOnSleep === true;
  async function save(next: boolean) {
    if (saving()) return;
    setSaving(true);
    setError('');
    try {
      const live = await api.agentSettings();
      await api.setAgentSettings({ ...agentSettingsPayload(live), moveOnSleep: next });
      await refreshAgentSettings();
    } catch (e) {
      setError(`Could not save it: ${String(e)}`);
    } finally { setSaving(false); }
  }
  return (
    <Show when={props.runner || on()}>
      <div class="settings-row machines-move-row">
        <div class="settings-row-label">
          <div class="settings-row-k" id="move-on-sleep">Move running chats to the runner when this Mac sleeps</div>
          <div class="settings-row-v">{sleepHint(props.runner, on())}</div>
          <MoreFold>
            <div class="settings-row-v">When this Mac is about to sleep, each running chat stops at a safe point and moves to the runner with its history, then keeps going there. Chats that are idle stay here. The runner’s own Claude and Codex logins run those turns, so its account pays, not this Mac’s. Files, attachments and uncommitted changes stay here. With Keep Hollow running with the lid closed on, this Mac doesn’t sleep and nothing moves.</div>
          </MoreFold>
        </div>
        <Switch checked={on()} label="Move running chats to the runner when this Mac sleeps" disabled={saving()} onChange={next => void save(next)} />
      </div>
      <Show when={error()}>{msg => <ErrorText class="settings-row-warn" text={msg()} />}</Show>
    </Show>
  );
}
