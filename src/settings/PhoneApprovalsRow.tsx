import { Show } from 'solid-js';
import { api } from '../api';
import type { PhoneAccessStatus } from '../types';
import { fmtTime } from '../utils';
import { Button } from '../components/ui';
import { approvalsLine, APPROVAL_WAIT_CHOICES } from '../phoneApprovals';

/// B4: one paired iPhone's "Approvals from this iPhone" row, under its entry in
/// Settings, Devices, Companion devices. Turn on opens a code for 10 minutes
/// (it exists only in Hollow's memory); the phone types it in its own Settings,
/// Approvals, and proves it is the genuine app. Turn off drops its keys.
export function PhoneApprovalsRow(props: {
  deviceId: string;
  status: PhoneAccessStatus | null;
  busy: boolean;
  act: (name: string, action: () => Promise<PhoneAccessStatus>, done: string) => Promise<void>;
}) {
  const device = () => props.status?.devices.find(d => d.deviceId === props.deviceId);
  const code = () => {
    const open = props.status?.approvalsCode;
    return open && open.deviceId === props.deviceId && Date.parse(open.expiresAt) > Date.now() ? open : null;
  };
  const dropped = () => props.status?.approvalsDropped?.find(([id]) => id === props.deviceId)?.[1] ?? null;
  return (
    <Show when={props.status && device() && device()!.approvals !== undefined}>
      <div class="phone-approvals" role="group" aria-label="Approvals from this iPhone">
        <span class="settings-note">{approvalsLine(device()!.approvals ?? null, fmtTime)}</span>
        <Show when={dropped() && !device()!.approvals}>
          <span class="settings-note">Its approval keys were turned off after five bad answers. Turn them on again to use approvals.</span>
        </Show>
        <Show
          when={device()!.approvals}
          fallback={
            <Show
              when={code()}
              fallback={
                <Button variant="ghost" disabled={props.busy} onClick={() => void props.act('approvals-start', () => api.phoneAccessApprovalsStart(props.deviceId), 'Type the code in the iPhone app.')}>
                  Turn on approvals
                </Button>
              }
            >
              {open => (
                <>
                  <span class="settings-note">
                    In the iPhone app, open Settings, Approvals and type this code. It works once, until {new Date(open().expiresAt).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}.
                  </span>
                  <code class="companion-code" aria-label={`Approvals code ${open().code.split('').join(' ')}`}>{open().code}</code>
                  <Button variant="ghost" disabled={props.busy} onClick={() => void props.act('approvals-cancel', () => api.phoneAccessApprovalsCancel(), 'The code is closed.')}>Cancel</Button>
                </>
              )}
            </Show>
          }
        >
          <label class="settings-note">
            A card waits{' '}
            <select
              aria-label="How long a card waits for this iPhone"
              value={String(props.status!.approvalWaitSecs ?? 300)}
              disabled={props.busy}
              onChange={e => void props.act('approvals-wait', () => api.phoneAccessApprovalsWait(Number(e.currentTarget.value)), 'Saved.')}
            >
              {APPROVAL_WAIT_CHOICES.map(c => <option value={String(c.seconds)}>{c.label}</option>)}
            </select>
          </label>
          <Button variant="ghost" disabled={props.busy} onClick={() => void props.act('approvals-off', () => api.phoneAccessApprovalsOff(props.deviceId), 'Approvals from this iPhone are off.')}>Turn off approvals</Button>
        </Show>
      </div>
    </Show>
  );
}
