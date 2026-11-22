import { For, Show, createEffect, createMemo, createSignal, on, onCleanup } from 'solid-js';
import { api } from '../api';
import { Icon } from '../icons';
import { DELIVERY_STEP } from '../workState';
import { deliveryRows } from '../delivery';
import { latestRequest } from '../latestRequest';
import { errMsg } from '../utils';
import { localBuildInButton, updateWaiting } from '../store/updates';
import type { AppBuildStatus, BuildStatus, SelfInstallReceipt, WorkWorkspace } from '../workspaceTypes';

/**
 * The delivery record at the top of a build workspace's Changes pane
 * (next-level P20): local → committed → PR → merged → built → install
 * scheduled → installed, each with the evidence that backs it. Remote
 * publication, the built candidate and the installed app stay distinct;
 * nothing here acts, it only reports. The app build is the Changes pane's
 * one signal (P1-46), so this and the Hollow app section never disagree; a
 * receipt that can't be read is "unknown" with Retry.
 */
export function DeliveryRecord(props: { workspace: WorkWorkspace; status: BuildStatus | null; build: AppBuildStatus | null }) {
  const [receipt, setReceipt] = createSignal<SelfInstallReceipt | null>(null);
  const [receiptError, setReceiptError] = createSignal<string | null>(null);
  const reads = latestRequest();
  async function readReceipt() {
    const ticket = reads.start();
    try {
      const next = await api.selfInstallReceipt();
      if (reads.isLatest(ticket)) { setReceipt(next); setReceiptError(null); }
    } catch (e) { if (reads.isLatest(ticket)) { setReceipt(null); setReceiptError(errMsg(e)); } }
  }
  // Only when HEAD or the build's state actually moves, not on every refresh.
  const moved = createMemo(() => `${props.status?.head ?? ''}|${props.build?.state ?? ''}`);
  createEffect(on(moved, () => void readReceipt()));
  onCleanup(() => reads.cancel());
  const waitingInUpdate = () => !!props.build?.receipt && updateWaiting() && localBuildInButton() === props.build.receipt.buildId;
  const rows = () => deliveryRows({ status: props.status, build: props.build, receipt: receipt(), receiptError: receiptError(), waitingInUpdate: waitingInUpdate() });
  return <Show when={rows().length}>
    <ol class="delivery-record" aria-label="Delivery record">
      <For each={rows()}>{r => <li data-state={r.state}>
        <span class="delivery-mark" aria-hidden="true"><Show when={r.state === 'done'} fallback={<Show when={r.state === 'failed'} fallback={<i />}><Icon name="x" size={10} /></Show>}><Icon name="check" size={10} /></Show></span>
        <span class="delivery-text"><strong>{DELIVERY_STEP[r.step].label}</strong><small>{r.evidence}</small></span>
        <Show when={r.state === 'unknown'}><button type="button" class="btn small ghost" onClick={() => void readReceipt()}>Retry</button></Show>
        <span class="sr-only">{r.state === 'done' ? 'done' : r.state === 'current' ? 'in progress' : r.state === 'failed' ? 'failed' : r.state === 'unknown' ? 'unknown' : 'not yet'}</span>
      </li>}</For>
    </ol>
  </Show>;
}
