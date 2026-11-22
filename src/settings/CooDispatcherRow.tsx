import { Show, createSignal, onMount } from 'solid-js';
import { api, invoke } from '../api';
import { agentSettings, refreshAgentSettings } from '../store';
import { mainAgentStatus, refreshMainAgentStatus } from '../store/mainAgent';
import { agentSettingsPayload } from '../agentSettingsPayload';
import { cooDispatcherShown, dispatcherLine, dispatcherRowShown, startableRunner } from './cooDispatcher';
import { Button, ErrorText } from '../components/ui';
import { Select } from '../components/Select';
import { MoreFold } from './settingsLayout';

/** Settings, Devices, COO on this Mac (M3a PR2, 2026-11-15): whether this
 *  Mac's COO becomes the dispatcher while a paired Mac runs a runner COO
 *  (the default) or always stays the full COO. Shown once a paired Mac
 *  runs a runner COO, or while saved off (`dispatcherRowShown`). Saved at once into the agent
 *  settings (the whole payload, so nothing else changes); the next COO turn
 *  reads it. */
export function CooDispatcherRow() {
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal('');
  onMount(() => { void refreshMainAgentStatus(); });
  const current = () => cooDispatcherShown(agentSettings().cooDispatcher);
  const [starting, setStarting] = createSignal(false);
  // M3a follow-up: the runner's COO has no conversation yet, so nothing can
  // be routed there; this starts it from here, then opens it there.
  async function startCoo(machineId: string, machine: string) {
    if (starting()) return;
    setStarting(true);
    setError('');
    try {
      const key = await invoke<string>('peer_start_coo', { machineId });
      await refreshMainAgentStatus();
      void import('../store/peerChat').then(m => m.openRemoteChat(machineId, key));
    } catch (e) {
      const said = e && typeof e === 'object' && 'message' in e ? String((e as { message: unknown }).message) : String(e);
      setError(`Could not start ${machine}'s COO: ${said}`);
    } finally { setStarting(false); }
  }
  async function choose(choice: string) {
    if (saving() || choice === current()) return;
    setSaving(true);
    setError('');
    try {
      const live = await api.agentSettings();
      await api.setAgentSettings(agentSettingsPayload({ ...live, cooDispatcher: choice === 'off' ? 'off' : undefined }));
      await refreshAgentSettings();
      await refreshMainAgentStatus();
    } catch (e) {
      setError(`Could not save COO on this Mac: ${String(e)}`);
    } finally { setSaving(false); }
  }
  return (
    <Show when={dispatcherRowShown(mainAgentStatus()?.dispatcher, agentSettings().cooDispatcher)}>
      <div class="settings-row">
        <div class="settings-row-label">
          <label for="coo-dispatcher" class="settings-row-k">COO on this Mac</label>
          <div class="settings-row-v">With a runner COO paired, it answers quick questions and sends long work there.</div>
          <MoreFold>
            <div class="settings-row-v">As the dispatcher, this Mac's COO answers quick questions itself and sends long work to the paired Mac that runs a runner COO, asking you on a card first. Always the full COO keeps all its work on this Mac, as with no runner COO paired. A runner COO starts with hollow serve runner-coo on at that Mac, after you pair it. The next COO turn uses your choice.</div>
          </MoreFold>
          <Show when={dispatcherLine(mainAgentStatus()?.dispatcher)}>{line => <div class="settings-row-v">{line()}</div>}</Show>
          <Show when={startableRunner(mainAgentStatus()?.dispatcher)}>{r => (
            <div class="settings-row-v">
              <Button size="sm" disabled={starting()} onClick={() => void startCoo(r().machineId, r().machine)}>
                {starting() ? 'Starting…' : `Start ${r().machine}'s COO`}
              </Button>
            </div>
          )}</Show>
        </div>
        <Select id="coo-dispatcher" label="COO on this Mac" value={current()} disabled={saving()} searchable={false}
          onChange={value => void choose(value)}
          options={[{ value: 'auto', label: 'Dispatcher when a runner COO is paired', detail: 'The default' }, { value: 'off', label: 'Always the full COO' }]} />
      </div>
      <Show when={error()}>{msg => <ErrorText class="settings-row-warn" text={msg()}/>}</Show>
    </Show>
  );
}
