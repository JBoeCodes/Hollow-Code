// Settings, Devices, COO on this Mac (M3a PR2): the row's pure parts, kept
// out of the component so they are unit tested.
import type { DispatcherStatus } from '../mainAgent';

/** COO on this Mac's two choices (`agents.cooDispatcher`; Rust
 *  `agent_settings::COO_DISPATCHER`). Auto is absent when saved. */
export type CooDispatcherChoice = 'auto' | 'off';

/** The saved choice as the row shows it: `off`, else `auto` (the default). */
export function cooDispatcherShown(value: string | null | undefined): CooDispatcherChoice {
  return value?.trim() === 'off' ? 'off' : 'auto';
}

/** The row's status line from the COO's status: where it sends long work
 *  now and since when, or why it doesn't. Nothing before the first read. */
export function dispatcherLine(d: DispatcherStatus | null | undefined, when: (at: string) => string = shortWhen): string {
  if (!d) return '';
  if (d.dispatching) return `Now: the dispatcher, sending long work to ${d.machine ?? 'the runner'}${d.since ? ` (its runner COO since ${when(d.since)})` : ''}.`;
  return d.whyOff ? `Now: the full COO. ${d.whyOff}` : 'Now: the full COO.';
}

function shortWhen(at: string): string {
  const date = new Date(at);
  return Number.isNaN(date.getTime()) ? at : date.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
}

/** Whether Settings, Devices shows the row: once a paired Mac runs (or ran)
 *  a runner COO, or while it is saved off so the person can always undo it.
 *  With no runner COO there is nothing to choose (as the runner rows hide
 *  until a runner is set up, overnight 11-07 D3). */
export function dispatcherRowShown(d: DispatcherStatus | null | undefined, saved: string | null | undefined): boolean {
  if (cooDispatcherShown(saved) === 'off') return true;
  return !!d && (d.machine !== null || d.off === 'pairing-needed' || d.off === 'runner-quiet' || d.off === 'runner-not-started');
}

/** M3a follow-up: the paired runner whose runner COO has no conversation
 *  yet, which the row's Start its COO starts (`peer_start_coo`). */
export function startableRunner(d: DispatcherStatus | null | undefined): { machineId: string; machine: string } | null {
  if (!d || d.off !== 'runner-not-started' || !d.machineId) return null;
  return { machineId: d.machineId, machine: d.machine ?? 'the runner' };
}
