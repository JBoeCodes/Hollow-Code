// Settings, Connectors: Hollow’s own Plaud row (L4, X18: extracted from
// pages/Settings.tsx).
import { Show, createSignal, onMount } from 'solid-js';
import { PlainErrorText } from '../components/ui';
import { plaudStatus, plaudMessage, connectPlaud, disconnectPlaud, checkPlaudStatus } from '../store/connectors';
import { showConfirm } from '../store';
import { RECORDINGS_SIGN_IN_ROW } from '../meetingsCopy';
import { errMsg } from '../utils';
import { api } from '../api';

/// Plaud's two accesses in one block (P1-20): Recordings in Hollow (Hollow's own
/// Plaud sign-in, read on demand: Connect / Test / Disconnect, which asks
/// first) and Plaud tools for agents (the connector's own sign-in; its status
/// comes from `serviceHealth`, passed in as `agents`). Drives the global
/// `plaudStatus`. Recordings' sign-in button opens this row by its label
/// (`RECORDINGS_SIGN_IN_ROW`); the jump focuses the row's `data-jump-focus`
/// control, Connect (Reconnect when a stored sign-in Plaud rejected is still
/// on disk), whatever else the row shows.
export function PlaudConnectorRow() {
  const [testMsg, setTestMsg] = createSignal<string | null>(null);
  // A test that threw (overnight A09): the raw error, shown in plain words
  // with the raw text under Details. The ✓ / × outcomes stay in `testMsg`,
  // shown as written.
  const [testError, setTestError] = createSignal<string | null>(null);
  const note = () => testMsg() ?? (plaudMessage() || null);

  onMount(() => {
    void checkPlaudStatus();
  });

  const test = async () => {
    setTestError(null);
    setTestMsg('Checking Plaud…');
    try {
      const ok = await api.plaudPing();
      setTestMsg(ok ? '✓ Plaud accepted the token' : '× Plaud rejected the token');
    } catch (e) {
      setTestMsg(null);
      setTestError(errMsg(e));
    }
  };

  const connected = () => plaudStatus() === 'ok';
  const busy = () => plaudStatus() === 'authenticating';
  const isWarn = () => plaudStatus() === 'needs-auth';
  const label = () => {
    switch (plaudStatus()) {
      case 'ok': return 'connected';
      case 'authenticating': return 'connecting…';
      case 'needs-auth': return 'not connected';
      default: return 'unknown';
    }
  };
  const conduit = () =>
    plaudStatus() === 'ok' ? 'ok'
        : plaudStatus() === 'authenticating' ? 'busy'
          : plaudStatus() === 'needs-auth' ? 'warn'
            : 'off';

  const disconnect = async () => {
    if (!await showConfirm({
      title: 'Sign out of Plaud for Recordings?',
      body: 'Library → Recordings stops listing your recordings until you sign in again. Transcript copies you kept stay where they are. Plaud tools for agents keep their own sign-in.',
      confirmLabel: 'Sign out',
      danger: true,
    })) return;
    setTestMsg(null);
    setTestError(null);
    await disconnectPlaud();
  };

  return (
    <div class="settings-row">
      <span class="status-conduit settings-conduit" data-state={conduit()} aria-hidden="true" />
      <div class="settings-row-label">
        <div class="settings-row-k">{RECORDINGS_SIGN_IN_ROW}</div>
        <div class={`settings-row-v ${isWarn() ? 'warn' : ''}`}>
          {label()} · Hollow’s own Plaud sign-in, read on demand
        </div>
        {/* The agents' Plaud connector shows its state once, on its own row in Your connectors (Settings PR 7). */}
        <Show when={testError()} fallback={<Show when={note()}>{n => <div class="settings-row-warn">{n()}</div>}</Show>}>
          {e => <div class="settings-row-warn" role="alert">× <PlainErrorText text={e()} /></div>}
        </Show>
      </div>
      <div style="display:flex; gap:6px;">
        <Show when={connected()}>
          <button class="btn ghost" onClick={test} title="Checks that Plaud still accepts Hollow’s sign-in">Test</button>
          <button class="btn ghost" onClick={() => void disconnect()} title="Signs Hollow’s Recordings out of Plaud; asks first">Disconnect</button>
        </Show>
        <button
          class={`btn ${connected() ? 'ghost' : 'primary'}`}
          data-jump-focus
          onClick={() => connectPlaud()}
          disabled={busy()}
          title="Sign in to Plaud (opens your browser)"
        >{busy() ? 'Connecting…' : connected() ? 'Reconnect' : 'Connect'}</button>
      </div>
    </div>
  );
}
