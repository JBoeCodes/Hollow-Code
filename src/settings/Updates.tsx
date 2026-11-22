import { Show, Suspense, createResource, createSignal, lazy, onCleanup } from 'solid-js';
import { api } from '../api';
import { Select } from '../components/Select';
import { ErrorText } from '../components/ui';
import { ackLastUpdateResult, cancelUpdateWait, checkForUpdatesNow, lastUpdateResult, refreshUpdateState, updateError, updateState, updateWaiting } from '../store/updates';
import { errMsg } from '../utils';
import '../styles/update-button.css';
import { lastCheckedText, updateResultText } from '../updatesText';

// Go back (U6), People (U9) and Publish to Main (U8) load with their own code and strings.
const UpdatesGoBack = lazy(() => import('./UpdatesGoBack').then(m => ({ default: m.UpdatesGoBack })));
const UpdatesPeople = lazy(() => import('./UpdatesPeople').then(m => ({ default: m.UpdatesPeople })));
const UpdatesPublish = lazy(() => import('./UpdatesPublish').then(m => ({ default: m.UpdatesPublish })));

export function Updates() {
  const [settings, { mutate }] = createResource(() => api.updateGetSettings());
  const savedSettings = () => settings.error ? undefined : settings();
  const [channelChanged, setChannelChanged] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [now, setNow] = createSignal(Date.now());
  let tokenInput: HTMLInputElement | undefined;
  let alive = true;
  const timer = window.setInterval(() => setNow(Date.now()), 60_000);
  onCleanup(() => { alive = false; window.clearInterval(timer); if (tokenInput) tokenInput.value = ''; });
  void refreshUpdateState();
  const disabled = () => !updateState()?.configured || busy();
  const pending = () => !!updateState()?.pendingConnect;
  const reason = () => {
    const state = updateState();
    if (!state) return 'Reading update status.';
    if (state.note) return state.note;
    const phase = state.phase;
    if ('reason' in phase) return phase.reason;
    switch (phase.phase) {
      case 'idle': return 'Ready to check for updates.';
      case 'checking': return 'Checking for updates.';
      case 'upToDate': return 'Hollow is up to date.';
      case 'available': return `Update ${phase.release.version} is available.`;
      case 'downloading': return `Downloading ${phase.release.version}.`;
      case 'ready': return `Update ${phase.release.version} is ready.`;
      case 'installing': return `Installing ${phase.release.version}.`;
      case 'goingBack': return `Going back to ${phase.version}.`;
    }
  };
  async function act(action: () => Promise<unknown>, requiresConfigured = true, failureText = (_error: unknown) => 'Could not change update settings. Try again.') {
    if (busy() || (requiresConfigured && !updateState()?.configured)) return;
    setBusy(true); setError('');
    try { await action(); await refreshUpdateState(); }
    catch (error) { if (alive) setError(failureText(error)); }
    finally { if (alive) setBusy(false); }
  }
  function connect() {
    if (!tokenInput || disabled() || !tokenInput.value.trim()) return;
    // Keep the credential only for this request and its error check, never in
    // reactive state or logs. Clear the input before starting the request.
    const token = tokenInput.value;
    tokenInput.value = '';
    void act(() => api.updateConnect(token), true, error => {
      const generic = 'Could not connect Hollow. Try again.';
      const message = errMsg(error);
      if (message.includes(token) || message.includes(token.trim())) return generic;
      const firstLine = message.split(/\r?\n/, 1)[0].trim();
      return firstLine ? `${generic} ${firstLine}` : generic;
    });
  }
  // A teammate's picker offers only what their updates access allows (U9).
  const channelOptions = () => [{ value: 'main', label: 'Main' }, { value: 'nightly', label: 'Nightly' }].filter(o => (updateState()?.allowedChannels ?? ['main', 'nightly']).includes(o.value as 'main' | 'nightly'));
  return <>
    <p class="settings-row-k">Hollow {updateState()?.runningVersion ?? '...'} · {updateState()?.channel === 'nightly' ? 'Nightly' : 'Main'}</p>
    <p class="settings-note" role="status">{reason()}</p>
    <p class="settings-note">{lastCheckedText(updateState()?.lastCheckedAt, now())}</p>
    <Show when={updateState() && !updateState()!.configured}><p class="settings-note">Updates are not set up on this build.</p></Show>
    <div class="settings-row">
      <div class="settings-row-label"><div class="settings-row-k">Channel</div><div class="settings-row-v">Main gets approved updates. Nightly gets every change that passes its checks.</div></div>
      <Select label="Channel" value={updateState()?.channel ?? 'main'} disabled={disabled() || !channelOptions().length}
        options={channelOptions()}
        onChange={channel => void act(async () => {
          await api.updateSetChannel(channel === 'nightly' ? 'nightly' : 'main');
          if (alive) setChannelChanged(true);
        })} />
    </div>
    <Show when={channelChanged() || updateState()?.downgradeAllowed}><p class="settings-note">Switching channel can offer an older version once.</p></Show>
    <div class="settings-row">
      <div class="settings-row-label">
        <label class="settings-row-k"><input type="checkbox" checked={savedSettings()?.installOvernight ?? false} disabled={disabled() || !savedSettings()}
          onChange={event => { const enabled = event.currentTarget.checked; event.currentTarget.checked = savedSettings()?.installOvernight ?? false; void act(async () => { const next = await api.updateSetOvernight(enabled); if (alive) mutate(next); }); }} /> Install updates overnight</label>
        <div class="settings-row-v">Between 2 and 5 AM, when nothing runs and the Mac is on power.</div>
      </div>
    </div>
    <p><button type="button" class="btn" disabled={disabled() || updateState()?.phase.phase === 'checking'} onClick={() => void act(checkForUpdatesNow)}>Check now</button></p>
    <Show when={updateWaiting()}>
      <p class="settings-note">Hollow will restart to update when chats finish.</p>
      <p><button type="button" class="btn" disabled={busy()} onClick={() => void act(cancelUpdateWait, false)}>Cancel update wait</button></p>
    </Show>
    <Show when={updateState()?.connected} fallback={
      <div class="settings-row">
        <label class="settings-row-label"><span class="settings-row-k">Update token</span><input ref={tokenInput} type="password" autocomplete="off" disabled={disabled()} /></label>
        <button type="button" class="btn" disabled={disabled()} onClick={connect}>Connect Hollow</button>
      </div>
    }>
      <div class="settings-row"><p>Connected to the update server</p><button type="button" class="btn" disabled={disabled()} onClick={() => void act(api.updateDisconnect)}>Disconnect</button></div>
    </Show>
    <Show when={pending()}>
      <p>A link asked to connect Hollow with a different update token. Replace the token you have?</p>
      <p><button type="button" class="btn" disabled={disabled()} onClick={() => void act(() => api.updateConfirmConnect())}>Replace token</button>{' '}
        <button type="button" class="btn" disabled={disabled()} onClick={() => void act(() => api.updateDiscardConnect())}>Keep current token</button></p>
    </Show>
    <Show when={lastUpdateResult()}>{result => <div role="status"><p class="update-result-text">{updateResultText(result())}</p><button type="button" class="btn" disabled={busy()} onClick={() => void act(ackLastUpdateResult, false)}>OK</button></div>}</Show>
    <Show when={error() || updateError() || settings.error}><ErrorText class="settings-error" text={error() || updateError() || 'Could not read update settings. Reopen Updates to try again.'} /></Show>
    <Suspense>
      <UpdatesGoBack />
      <UpdatesPeople settings={savedSettings()} onSettings={next => mutate(next)} />
      <Show when={savedSettings()?.publisher}><UpdatesPublish /></Show>
    </Suspense>
  </>;
}
