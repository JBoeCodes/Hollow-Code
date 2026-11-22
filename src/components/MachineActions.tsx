// M2 leftovers (2026-11-15): what a paired Mac's state offers, in one place
// for the machine strip's popover and Library, Conversations' "On <Mac>":
// Retry while it is unreachable (asks it now, skipping the tunnel's wait) and
// Cancel while its tunnel connects (the tunnel stays closed until Retry).
// The words come from `popoverInfo` (machineStrip.ts).
import { Show, createSignal } from 'solid-js';
import { flashFooter } from '../store';
import { cancelPeerConnect, isPeerFailure, loadPeerMachines, refreshPeerSessions } from '../store/peerSessions';
import { Button } from './ui';
import type { PopoverInfo } from './machineStrip';

export function MachineActions(props: { machineId: string; info: PopoverInfo }) {
  const [busy, setBusy] = createSignal<'retry' | 'cancel' | null>(null);

  async function retry() {
    setBusy('retry');
    try {
      await loadPeerMachines(true);
      await refreshPeerSessions(props.machineId);
    } finally {
      setBusy(null);
    }
  }

  async function cancel() {
    setBusy('cancel');
    try {
      await cancelPeerConnect(props.machineId);
    } catch (e) {
      flashFooter(`× ${isPeerFailure(e) ? e.message : 'Could not cancel connecting'}`, 3500);
    } finally {
      setBusy(null);
    }
  }

  return (
    <Show when={props.info.retry || props.info.cancel}>
      <Show when={props.info.cancel} fallback={
        <Button variant="ghost" size="sm" icon="refresh" disabled={!!busy()} onClick={() => void retry()}>{busy() === 'retry' ? 'Checking…' : 'Retry'}</Button>
      }>
        <Button variant="ghost" size="sm" icon="x" disabled={!!busy()} onClick={() => void cancel()}>{busy() === 'cancel' ? 'Cancelling…' : 'Cancel'}</Button>
      </Show>
    </Show>
  );
}
