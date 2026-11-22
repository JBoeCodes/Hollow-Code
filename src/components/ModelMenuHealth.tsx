import { Show } from 'solid-js';
import type { AgentCliStatus } from '../generated/ipc';
import type { SteerSupportRow } from '../chatSteer';
import { openAgentUpdates } from '../store/agentHealth';

/** Actions are siblings of the model listbox, so they keep native key handling. */
export default function ModelMenuHealth(props: {
  label: string; warning?: string; loading: boolean; refresh: () => void;
  row?: AgentCliStatus; support?: SteerSupportRow; close: () => void;
}) {
  const status = () => props.row?.state === 'updating' ? 'Updating · new runs unavailable'
    : props.row?.state === 'waiting' ? 'Update waiting for running work'
    : props.row?.updateAvailable ? 'Update available' : '';
  const qualification = () => props.support?.agent === 'codex' && props.support.support === 'none' ? props.support.note : '';
  return <Show when={props.warning || status() || qualification()}>
    <div class="model-menu-health" role="group" aria-label={`${props.label} status`}>
      <Show when={status()}><span class="model-menu-health-state">{status()}</span></Show>
      <Show when={props.warning}><p role="status">{props.warning}</p></Show>
      <Show when={qualification()}><p>{qualification()}</p></Show>
      <div class="model-menu-health-actions">
        <Show when={props.warning}><button type="button" class="btn small" disabled={props.loading} onClick={props.refresh}>{props.loading ? 'Refreshing…' : 'Refresh'}</button></Show>
        <button type="button" class="btn small ghost" onClick={() => { props.close(); openAgentUpdates(); }}>Agent updates</button>
      </div>
    </div>
  </Show>;
}
