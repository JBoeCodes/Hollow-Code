import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { onNative } from '../nativeEvents';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import type { PhoneLidStatus } from '../types';
import { errMsg } from '../utils';
import { Button, ErrorText } from '../components/ui';
import { Switch } from '../components/Switch';
import { BATTERY_WARNING, LID_ABOUT, lidDone, lidLines, lidOrphan, lidSwitch, lidUpdate } from '../phoneLid';

/// Settings, Devices, Companion devices, iPhone app, Keep Hollow
/// running with the lid closed (2026-11-05, `phone_access/lid.rs`): the
/// switch and On battery too, each of which asks macOS for the person's
/// password, Update the helper for one from another version of Hollow, and
/// in words whether the lid can close now (a helper that has stopped
/// included). Reads its own status on open, when the iPhone app changes and
/// every 15 seconds while visible (power plugged or unplugged).
export function PhoneLidClosed() {
  const [status, setStatus] = createSignal<PhoneLidStatus | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [notice, setNotice] = createSignal('');
  /** On battery too while the setting is off: the choice the switch installs. */
  const [battery, setBattery] = createSignal(false);
  let alive = true;
  const requests = latestRequest();

  async function refresh() {
    if (busy()) return;
    const seq = requests.start();
    try {
      const next = await api.phoneLidStatus();
      if (alive && requests.isLatest(seq)) { setStatus(next); setError(''); }
    } catch (e) {
      if (alive && requests.isLatest(seq)) setError(`Could not read whether the lid can close. ${errMsg(e)}`);
    }
  }

  async function change(enabled: boolean, onBattery: boolean) {
    if (busy()) return;
    const was = status();
    const seq = requests.start();
    setBusy(true); setError(''); setNotice('');
    try {
      const next = await api.phoneLidSet(enabled, onBattery);
      if (!alive || !requests.isLatest(seq)) return;
      setStatus(next); setNotice(lidDone(enabled, onBattery, was, next));
    } catch (e) {
      if (alive && requests.isLatest(seq)) setError(errMsg(e));
    } finally {
      if (alive && requests.isLatest(seq)) setBusy(false);
    }
  }

  onMount(() => {
    void refresh();
    const off = onNative('phone-access-changed', () => void refresh());
    const timer = window.setInterval(() => { if (document.visibilityState === 'visible') void refresh(); }, 15_000);
    onCleanup(() => { alive = false; requests.cancel(); off(); window.clearInterval(timer); });
  });

  const lines = () => lidLines(status());
  const toggle = () => lidSwitch(status(), busy());
  const batteryChecked = () => (status()?.enabled ? status()!.onBattery : battery());

  return (
    <div class="companion-phone-lid" role="group" aria-labelledby="companion-phone-lid-title">
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k" id="companion-phone-lid-title">Keep Hollow running with the lid closed</div>
          <div class="settings-row-v">{LID_ABOUT}</div>
        </div>
        <Switch checked={!!status()?.enabled} label="Keep Hollow running with the lid closed" disabled={toggle().disabled}
          onChange={on => void change(on, on && battery())} />
      </div>
      <Show when={toggle().why}>{why => <p class="settings-note">{why()}</p>}</Show>
      <label class="companion-phone-lid-battery">
        <input type="checkbox" checked={batteryChecked()} disabled={!status()?.available || busy() || (!status()?.enabled && !status()?.iphoneOn)}
          onChange={e => {
            const next = e.currentTarget.checked;
            if (status()?.enabled) { e.currentTarget.checked = !next; void change(true, next); }
            else setBattery(next);
          }} />
        On battery too
      </label>
      <p class="settings-note">{BATTERY_WARNING}</p>
      <Show when={status() && !status()!.available}>
        <p class="settings-note">{status()!.unavailableReason ?? 'Keeping the Mac awake isn’t available in this copy of Hollow.'}</p>
      </Show>
      <Show when={lines().length > 0}>
        <ul class="companion-phone-lines" aria-label="Lid closed status">
          <For each={lines()}>{line => <li classList={{ 'is-warning': line.tone === 'warn' }}>{line.text}</li>}</For>
        </ul>
      </Show>
      <Show when={lidOrphan(status())}>
        <div class="companion-phone-actions">
          <Button variant="ghost" disabled={busy()} onClick={() => void change(false, false)}>Turn sleep back on</Button>
        </div>
      </Show>
      <Show when={lidUpdate(status())}>
        <div class="companion-phone-actions">
          <Button variant="ghost" disabled={busy()} onClick={() => void change(true, !!status()?.onBattery)}>Update the helper</Button>
        </div>
      </Show>
      <Show when={error()}><ErrorText text={error()} class="settings-note" /></Show>
      <p class="settings-note" role="status">{busy() ? 'Waiting for your password in macOS’s prompt…' : notice()}</p>
    </div>
  );
}
