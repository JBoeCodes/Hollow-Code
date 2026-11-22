import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { onNative } from '../nativeEvents';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { flashFooter, reportPhoneModes, showConfirm } from '../store';
import type { CompanionBackendStatus, CompanionBackendTest, CompanionScope, CompanionStatus, PhoneAccessStatus } from '../types';
import { errMsg, fmtTime, openExternal } from '../utils';
import { Button, ErrorText } from '../components/ui';
import { Switch } from '../components/Switch';
import { registerDirty } from './settingsDirty';
import { PhoneLidClosed } from './PhoneLidClosed';
import { PhoneApprovalsRow } from './PhoneApprovalsRow';
import { PUSH_TEAM_ID, deviceRows, forwardLines, keyIdFromPath, pairBlocked, phoneStateLabel, publishChoice, pushLines, pushStateLabel, qrImageSrc, revokeConfirm, serverLine, spelledCode, tailscaleLine, type DeviceRow } from '../phoneAccess';

/// Settings, Devices, Companion devices (companion apps T0): pair the
/// Mac menu-bar app with a code that exists only in Hollow's memory, see what
/// is paired, and revoke a device. Paired devices send signed changes that
/// Hollow applies as your own edits (docs/COMPANION_PROTOCOL.md). The iPhone
/// app (the phone chat API, 2026-11-04, docs/PHONE_CHAT_API.md) has its own
/// section below, and its phones join the same device list.
export function CompanionDevicesPanel() {
  const [status, setStatus] = createSignal<CompanionStatus | null>(null);
  const [busy, setBusy] = createSignal(false);
  const phone = createPhoneAccess();

  async function refresh() {
    try {
      setStatus(await api.companionStatus());
    } catch (e) {
      flashFooter(`× ${errMsg(e)}`, 3500);
    }
  }

  onMount(() => {
    void refresh();
    void phone.refresh();
    // A phone's New chat starts in the mode for new chats, and its send runs
    // in the mode this window shows for its chat: the window's whole record
    // (`phone_access_new_chat_mode`, `phone_access_chat_modes`; each choice
    // made since reports itself).
    reportPhoneModes();
    // A device answering the code, or a change applied, refreshes the list.
    const off = onNative('companion-changed', () => void refresh());
    const offPhone = onNative('phone-access-changed', () => void phone.refresh());
    // A code expires on its own: re-read while one is open.
    const timer = window.setInterval(() => {
      if (status()?.pairing) void refresh();
      if (phone.status()?.pairing) void phone.refresh();
    }, 5000);
    onCleanup(() => { off(); offPhone(); window.clearInterval(timer); });
  });

  /** Every paired device, companion and phone, in one list. */
  const rows = () => deviceRows(status()?.devices ?? [], phone.status()?.devices ?? []);

  async function pair() {
    setBusy(true);
    try {
      await api.companionPairingStart();
      await refresh();
    } catch (e) {
      flashFooter(`× ${errMsg(e)}`, 3500);
    } finally {
      setBusy(false);
    }
  }

  async function cancel() {
    try {
      await api.companionPairingCancel();
    } catch (e) {
      flashFooter(`× ${errMsg(e)}`, 3500);
    }
    await refresh();
  }

  async function revoke(row: DeviceRow) {
    const ok = await showConfirm({ ...revokeConfirm(row), confirmLabel: 'Revoke', danger: true });
    if (!ok) return;
    if (row.kind === 'phone') {
      await phone.act('revoke', () => api.phoneAccessRevoke(row.deviceId), `${row.name} is revoked.`);
      return;
    }
    try {
      setStatus(await api.companionRevokeDevice(row.deviceId));
    } catch (e) {
      flashFooter(`× ${errMsg(e)}`, 3500);
    }
  }

  return (
    <div class="companion-devices">
      <Show
        when={status()?.pairing}
        fallback={
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Menu bar app</div>
              <div class="settings-row-v">In it, choose Pair with Hollow, then type the code. <Show when={rows().length === 0}><span>No devices are paired.</span></Show></div>
            </div>
            <Button onClick={pair} disabled={busy()}>Pair a device</Button>
          </div>
        }
      >
        {code => (
          <div class="settings-row" role="status">
            <p class="settings-note">
              Type this code on the device. It works once, until {new Date(code().expiresAt).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}.
            </p>
            <code class="companion-code" aria-label={`Pairing code ${code().code.split('').join(' ')}`}>{code().code}</code>
            <Button variant="ghost" onClick={cancel}>Cancel</Button>
          </div>
        )}
      </Show>
      <Show when={rows().length > 0}>
        <ul class="companion-device-list" aria-label="Paired devices">
          <For each={rows()}>
            {device => (
              <li class="settings-row">
                <span>
                  <strong>{device.name}</strong>
                  {' · '}{device.what}
                  {' · paired '}{fmtTime(device.pairedAt)}
                  <Show when={device.last}>{last => <>{` · ${last().label} `}{fmtTime(last().at)}</>}</Show>
                </span>
                <Button variant="ghost" onClick={() => void revoke(device)} disabled={device.kind === 'phone' && !!phone.busy()} aria-label={`Revoke ${device.name}`}>Revoke</Button>
                <Show when={device.kind === 'phone'}>
                  <PhoneApprovalsRow deviceId={device.deviceId} status={phone.status()} busy={!!phone.busy()} act={phone.act} />
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <Show when={(status()?.waiting ?? 0) > 0}>
        <p class="settings-note">{status()!.waiting} change{status()!.waiting === 1 ? '' : 's'} waiting to apply (a run holds the vault).</p>
      </Show>
      <PhoneAppSection phone={phone} />
      <CompanionBackend />
    </div>
  );
}

/** Welcome reuses the phone controls and their existing click-only pairing flow. */
export function WelcomePhone() {
  const phone = createPhoneAccess();
  onMount(() => {
    void phone.refresh();
    reportPhoneModes();
    const off = onNative('phone-access-changed', () => void phone.refresh());
    const timer = window.setInterval(() => { if (phone.status()?.pairing) void phone.refresh(); }, 5000);
    onCleanup(() => { off(); window.clearInterval(timer); });
  });
  return <PhoneAppSection phone={phone} headingId="welcome-phone-title" />;
}

type PhoneAccess = ReturnType<typeof createPhoneAccess>;

/// The iPhone app's status and the person's clicks on it, shared by the
/// device list (a phone's Revoke) and the iPhone app section. A status read
/// never starts while a click is in flight and never overlaps another read;
/// a click's answer replaces any read it overtook.
function createPhoneAccess() {
  const [status, setStatus] = createSignal<PhoneAccessStatus | null>(null);
  const [loading, setLoading] = createSignal(true);
  /** The click in flight, if any. */
  const [busy, setBusy] = createSignal<string | null>(null);
  const [error, setError] = createSignal('');
  const [notice, setNotice] = createSignal('');
  let alive = true, refreshing = false;
  const requests = latestRequest();
  onCleanup(() => { alive = false; requests.cancel(); });

  async function refresh(fresh = false) {
    if (busy() || refreshing) return;
    refreshing = true;
    const seq = requests.start();
    try {
      const next = await api.phoneAccessStatus(fresh);
      if (!alive || !requests.isLatest(seq)) return;
      setStatus(next); setError('');
    } catch (e) {
      if (alive && requests.isLatest(seq)) setError(`Could not read the iPhone app's status. ${errMsg(e)}`);
    } finally {
      refreshing = false;
      if (alive && requests.isLatest(seq)) setLoading(false);
    }
  }

  /** A click; `reread` reads the status again after a failure, for a click
   *  that may have done part of its work (Stop publishing tries every step,
   *  third review 2026-11-05), keeping the error. */
  async function act(name: string, action: () => Promise<PhoneAccessStatus>, done: string | (() => string), reread = false) {
    if (busy()) return;
    const seq = requests.start();
    setBusy(name); setError(''); setNotice('');
    try {
      const next = await action();
      if (!alive || !requests.isLatest(seq)) return;
      setStatus(next); setNotice(typeof done === 'function' ? done() : done);
    } catch (e) {
      if (!alive || !requests.isLatest(seq)) return;
      setError(errMsg(e));
      if (reread) {
        try {
          const next = await api.phoneAccessStatus(true);
          if (alive && requests.isLatest(seq)) setStatus(next);
        } catch { /* the error above stands */ }
      }
    } finally {
      if (alive && requests.isLatest(seq)) { setBusy(null); setLoading(false); }
    }
  }

  return { status, loading, busy, error, notice, refresh, act };
}

/// Settings, Devices, Companion devices, iPhone app (the phone chat
/// API, `phone_access.rs`): the switch for the server on 127.0.0.1:47680,
/// whether Tailscale publishes it to the tailnet (Publish on Tailscale / Stop
/// publishing), and Pair an iPhone with its code, address and QR code. The
/// paired phones are in the device list above.
function PhoneAppSection(props: { phone: PhoneAccess; headingId?: string }) {
  const p = props.phone;
  const state = () => phoneStateLabel(p.status(), p.loading());
  const lines = () => { const s = p.status(); return s?.available ? [serverLine(s), tailscaleLine(s), ...forwardLines(s)] : []; };
  const publish = () => publishChoice(p.status());
  const pairWhy = () => pairBlocked(p.status());
  const until = (at: string) => new Date(at).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  const togglePublish = (kind: 'publish' | 'unpublish') => kind === 'publish'
    ? p.act('publish', () => api.phoneAccessPublish(), 'Published on Tailscale, for your tailnet only.')
    : p.act('unpublish', () => api.phoneAccessUnpublish(), 'Stopped publishing on Tailscale.', true);
  return (
    <section class="companion-phone" aria-labelledby={props.headingId ?? 'companion-phone-title'}>
      <header class="companion-backend-heading">
        <h3 id={props.headingId ?? 'companion-phone-title'}>iPhone app</h3>
        <span role="status" class="companion-connection-state" classList={{ 'is-connected': state().tone === 'ok' }}>{state().label}</span>
      </header>
      <Show when={p.error()}><ErrorText text={p.error()} class="settings-note" /></Show>
      <Show when={p.status() && !p.status()!.available}>
        <p class="settings-note">{p.status()!.unavailableReason ?? 'The iPhone app is unavailable in this copy of Hollow.'}</p>
      </Show>
      <Show when={p.status()?.available ? p.status() : null}>
        {s => <>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Serve the iPhone app</div>
              <div class="settings-row-v">Chats on your iPhone, over your tailnet only. Listens on 127.0.0.1:{s().port}.</div>
            </div>
            <div class="settings-row-actions">
              <Show when={publish()}>{choice => (
                <Button variant="ghost" disabled={!!p.busy() || choice().disabled} onClick={() => void togglePublish(choice().kind)}>
                  {p.busy() === choice().kind ? 'Working…' : choice().label}
                </Button>
              )}</Show>
              <Button variant="ghost" disabled={!!p.busy()} onClick={() => void p.refresh(true)}>Check again</Button>
              <Show when={!s().pairing}>
                <Button onClick={() => void p.act('pair', () => api.phoneAccessPairingStart(), 'Pairing is open for 10 minutes.')} disabled={!!p.busy() || !!pairWhy()}>Pair an iPhone</Button>
              </Show>
              <Switch checked={s().enabled} label="Serve the iPhone app" disabled={!!p.busy()}
                onChange={on => void p.act('switch', () => api.phoneAccessSetEnabled(on), on ? 'The iPhone app is on.' : 'The iPhone app is off. Open chats on paired iPhones stop updating.')} />
            </div>
          </div>
          <Show when={s().recordError}>{message => <ErrorText text={message()} class="settings-note" />}</Show>
          <Show when={s().pairing}>
            {pairing => (
              <div class="companion-phone-pairing" role="group" aria-label="Pair an iPhone">
                <Show when={qrImageSrc(pairing().qrSvg)}>
                  {src => <img class="companion-phone-qr" src={src()} width="200" height="200" alt="QR code of the pairing link. Scan it with the iPhone’s Camera." />}
                </Show>
                <div class="companion-phone-pairing-text">
                  <p class="settings-note">Scan the QR code with the iPhone’s Camera, or open the Hollow app and type the address and code. The code works once, until {until(pairing().expiresAt)}.</p>
                  <code class="companion-code" aria-label={`Pairing code ${spelledCode(pairing().code)}`}>{pairing().code}</code>
                  <Show
                    when={s().baseUrl}
                    fallback={<p class="settings-note">Tailscale hasn’t named this Mac yet, so there is no address for the iPhone. Start Tailscale and sign in, then choose Check again.</p>}
                  >
                    {url => <p class="companion-mono companion-url">{url()}</p>}
                  </Show>
                  <Show when={!s().tailscale.published}>
                    <p class="settings-note">Publish on Tailscale first, or the iPhone can’t reach this Mac.</p>
                  </Show>
                  <Button variant="ghost" disabled={!!p.busy()} onClick={() => void p.act('cancel', () => api.phoneAccessPairingCancel(), 'Pairing is closed.')}>Cancel pairing</Button>
                </div>
              </div>
            )}
          </Show>
          {/* S5b-3: closed at rest; the summary names what is inside and the push state. */}
          <details class="settings-fold companion-phone-more">
            <summary>Status, push notifications and lid closed · {pushStateLabel(s().push ?? null).label}</summary>
            <p class="settings-note">{pairWhy() ?? 'Pair an iPhone shows a code and a QR code for the Hollow app on your iPhone.'}</p>
            <Show when={publish()?.why}>{why => <p class="settings-note">{why()}</p>}</Show>
            <ul class="companion-phone-lines" aria-label="iPhone app status">
            <For each={lines()}>{line => (
              <li classList={{ 'is-warning': line.tone === 'warn' }}>
                {line.text}
                <Show when={line.command}>{command => <>{' '}<code class="companion-mono">{command()}</code></>}</Show>
              </li>
            )}</For>
          </ul>
            <PushBlock phone={p} status={s()} />
            <PhoneLidClosed />
          </details>
        </>}
      </Show>
      <p class="settings-note" role="status">{p.notice()}</p>
    </section>
  );
}

/// Push notifications (phone API v1.1, `phone_access/push.rs`): Hollow tells
/// a paired iPhone when a turn finishes or a question card waits, straight
/// through Apple's push service, signed with the APNs key the person imports
/// here (kept only in the Keychain). Import key, Send test notification and
/// Remove key are the person's own clicks.
function PushBlock(props: { phone: PhoneAccess; status: PhoneAccessStatus }) {
  const p = props.phone;
  const push = () => props.status.push ?? null;
  const state = () => pushStateLabel(push());
  const [keyId, setKeyId] = createSignal('');
  const [teamId, setTeamId] = createSignal(PUSH_TEAM_ID);
  async function importKey() {
    if (p.busy()) return;
    let picked: string | string[] | null = null;
    try {
      picked = await openDialog({ multiple: false, directory: false, title: 'Choose the APNs key (.p8)', filters: [{ name: 'APNs key', extensions: ['p8'] }] });
    } catch (e) {
      flashFooter(`× ${errMsg(e)}`, 3500);
      return;
    }
    if (typeof picked !== 'string') return;
    const path = picked;
    const id = keyId().trim() || keyIdFromPath(path);
    await p.act('push-import', () => api.phonePushImportKey(path, id, teamId().trim() || null), 'The push key is in the Keychain. Paired iPhones can get notifications now.');
    if (!p.error()) setKeyId('');
  }
  async function removeKey() {
    const ok = await showConfirm({ title: 'Remove the push key?', body: 'Hollow sends no notifications until you import a key again. The key stays in your Apple developer account.', confirmLabel: 'Remove', danger: true });
    if (!ok) return;
    await p.act('push-remove', () => api.phonePushRemoveKey(), 'The push key is removed. Hollow sends no notifications.');
  }
  let said = '';
  const sendTest = () => p.act('push-test', async () => { said = await api.phonePushTest(); return api.phoneAccessStatus(false); }, () => said);
  return (
    <div class="companion-phone-push" role="group" aria-labelledby="companion-push-title">
      <header class="companion-backend-heading">
        <h4 id="companion-push-title">Push notifications</h4>
        <span role="status" class="companion-push-state" classList={{ 'is-connected': state().tone === 'ok' }}>{state().label}</span>
      </header>
      <p class="settings-note">Your iPhone hears when a turn finishes or a question card waits, through Apple’s push service. Nothing goes for a chat you have open on the phone.</p>
      <ul class="companion-phone-lines" aria-label="Push notification status">
        <For each={pushLines(push())}>{line => <li>{line}</li>}</For>
        <Show when={push()?.lastSentAt}>{at => <li>Last sent {fmtTime(at())}.</li>}</Show>
      </ul>
      <Show when={push()?.lastError}>{error => <ErrorText text={`Last error: ${error()}${push()?.lastErrorAt ? ` (${fmtTime(push()!.lastErrorAt!)})` : ''}`} class="settings-note" />}</Show>
      <form class="settings-row companion-backend-form" aria-label="Import the push key" onSubmit={e => { e.preventDefault(); void importKey(); }}>
        <label><span>Key ID</span><input value={keyId()} onInput={e => setKeyId(e.currentTarget.value)} placeholder="From the file name" autocapitalize="characters" spellcheck={false} maxlength={10} disabled={!!p.busy()} /></label>
        <label><span>Team ID</span><input value={teamId()} onInput={e => setTeamId(e.currentTarget.value)} autocapitalize="characters" spellcheck={false} maxlength={10} disabled={!!p.busy()} /></label>
        <Button type="submit" variant="ghost" disabled={!!p.busy()}>{p.busy() === 'push-import' ? 'Importing…' : push()?.state === 'ready' ? 'Replace key…' : 'Import key…'}</Button>
      </form>
      <div class="companion-phone-actions">
        <Button variant="ghost" disabled={!!p.busy() || push()?.state !== 'ready' || !push()?.phones} onClick={() => void sendTest()}>{p.busy() === 'push-test' ? 'Sending…' : 'Send test notification'}</Button>
        <Show when={push()?.state === 'ready'}>
          <Button variant="ghost" disabled={!!p.busy()} onClick={() => void removeKey()}>Remove key</Button>
        </Show>
      </div>
    </div>
  );
}

/// The backend for the web app (companion apps W1, `companion_bridge.rs`):
/// `hollow-companiond`'s URL and applier token. Hollow polls it for device
/// changes and pushes a read-only copy of your tasks to it. The token is
/// never shown again once saved.
function CompanionBackend() {
  const [backend, setBackend] = createSignal<CompanionBackendStatus | null>(null);
  const [url, setUrl] = createSignal(''), [token, setToken] = createSignal(''), [code, setCode] = createSignal('');
  const [busy, setBusy] = createSignal(false), [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal(''), [notice, setNotice] = createSignal('');
  // A test's answer, shown beside the button that asked (saved or typed).
  const [tested, setTested] = createSignal<(CompanionBackendTest & { typed: boolean }) | null>(null);
  const [testing, setTesting] = createSignal<'saved' | 'typed' | null>(null);
  let alive = true, urlEdited = false, refreshing = false;
  const requests = latestRequest();
  async function refresh() {
    if (busy() || refreshing) return;
    refreshing = true;
    const seq = requests.start();
    try {
      const next = await api.companionBackendStatus();
      if (!alive || !requests.isLatest(seq)) return;
      setBackend(next); setError('');
      if (!urlEdited) setUrl(next.url ?? '');
    } catch (e) {
      if (alive && requests.isLatest(seq)) setError(`Could not read the backend status. Any status shown is from the last successful check. ${errMsg(e)}`);
    } finally { refreshing = false; if (alive && requests.isLatest(seq)) setLoading(false); }
  }
  onMount(() => {
    void refresh();
    const timer = window.setInterval(() => { if (document.visibilityState === 'visible' && backend()?.url && !busy()) void refresh(); }, 5000);
    onCleanup(() => { alive = false; requests.cancel(); window.clearInterval(timer); });
  });
  async function change(action: () => Promise<CompanionBackendStatus>, message: string) {
    if (busy()) return;
    const seq = requests.start(); setBusy(true); setError(''); setNotice('');
    try {
      const next = await action();
      if (!alive || !requests.isLatest(seq)) return;
      setBackend(next); setUrl(next.url ?? ''); urlEdited = false; setToken(''); setCode(''); setTested(null); setNotice(message);
    } catch (e) {
      if (alive && requests.isLatest(seq)) setError(`Could not change the connection. Your entries are still here. ${errMsg(e)}`);
    } finally { if (alive && requests.isLatest(seq)) { setBusy(false); setLoading(false); } }
  }
  const save = (nextUrl: string) => { const suppliedToken = token(); return change(() => api.companionBackendSet(nextUrl, suppliedToken), nextUrl ? 'Backend saved.' : 'Backend turned off.'); };
  const connect = () => { const nextUrl = url().trim(), suppliedCode = code(); return change(() => api.companionBackendClaim(nextUrl, suppliedCode), 'Connected to the backend.'); };
  const setupUrl = () => {
    try { const parsed = new URL(url().trim()); if (parsed.username || parsed.password || parsed.search || parsed.hash || (parsed.protocol !== 'https:' && !(parsed.protocol === 'http:' && ['localhost','127.0.0.1','[::1]'].includes(parsed.hostname)))) return null; return `${parsed.href.replace(/\/+$/, '')}/setup`; }
    catch { return null; }
  };
  // Test connection (ST5): the saved connection, or the typed address and
  // token before saving. Saves nothing; the answer is one plain sentence.
  // It touches no status, so it leaves status reads (and their errors) alone.
  const tests = latestRequest();
  async function testConnection(typed: boolean) {
    if (busy()) return;
    const seq = tests.start(); setBusy(true); setTesting(typed ? 'typed' : 'saved'); setNotice(''); setTested(null);
    try { const result = await api.companionBackendTest(typed ? url().trim() : '', typed ? token() : ''); if (alive && tests.isLatest(seq)) setTested({ ...result, typed }); }
    catch (e) { if (alive && tests.isLatest(seq)) setTested({ ok: false, message: errMsg(e), version: null, typed }); }
    finally { if (alive && tests.isLatest(seq)) { setBusy(false); setTesting(null); } }
  }
  /** The answer for one button, in a status line that is always there (a live region must exist before its text changes). */
  const TestResult = (props: { typed: boolean }) => <p class="settings-note companion-test-result" classList={{ 'is-ok': !!tested()?.ok }} role="status">
    <Show when={tested()?.typed === props.typed ? tested() : null}>{t => <>{t().message}<Show when={t().version}>{v => <span class="companion-mono"> Backend {v()}.</span>}</Show></>}</Show>
  </p>;
  async function repush() {
    if (busy()) return;
    const seq = requests.start(); setBusy(true); setError(''); setNotice(''); setTested(null);
    try { const next = await api.companionBackendRepush(); if (alive && requests.isLatest(seq)) {setBackend(next);setNotice('Sending every task again.');} }
    catch (e) {if (alive && requests.isLatest(seq)) setError(`Could not re-send tasks. ${errMsg(e)}`);}
    finally {if (alive && requests.isLatest(seq)) setBusy(false);}
  }
  const editUrl = (value: string) => {urlEdited = true;setUrl(value);setTested(null);};
  // A typed address, code or token is a draft a section switch would drop (P1-21).
  registerDirty('companion-connection', () => (urlEdited && url().trim() !== (backend()?.url ?? '')) || !!code().trim() || !!token().trim(), 'Backend connection');
  // B05: a backend that failed to answer says so; "Waiting" is only for before its first answer. Other errors (a
  // refused token, an HTTP answer, a local record error) mean it was reached, so they read as needing attention.
  const unreachable = (message: string) => /^(Hollow couldn’t reach|Hollow couldn't reach) |didn['’]t answer within \d+ seconds/.test(message);
  const connectionLabel = () => loading() && !backend() ? 'Checking connection…' : !backend() ? 'Status unavailable' : !backend()!.url ? 'Not connected' : backend()!.connected ? 'Connected' : backend()!.lastError ? (unreachable(backend()!.lastError!) ? "Can't reach the backend" : 'Backend needs attention') : 'Waiting for the backend';
  /** The connection error shown under the address, when there is one. */
  const connectionError = () => (!backend()?.connected && backend()?.lastError) || null;
  /** What the "needs attention" details list: every error except the one already shown above (a share failing against the same unreachable backend repeats it word for word). */
  const otherErrors = () => [...new Set([backend()?.lastError, backend()?.mirrorError, backend()?.docsError, backend()?.scopeError])].filter((m): m is string => !!m && m !== connectionError());
  return <section class="companion-backend" aria-labelledby="companion-backend-title">
    <header class="companion-backend-heading"><h3 id="companion-backend-title">Backend for the web app</h3><span role="status" class="companion-connection-state" classList={{'is-connected':!!backend()?.connected}}>{connectionLabel()}</span></header>
    <Show when={error()}><div class="companion-feedback"><ErrorText text={error()} class="settings-note" /><Button variant="ghost" onClick={()=>void refresh()} disabled={busy()}>Retry status</Button></div></Show>
    <p class="settings-note" role="status">{notice()}</p>
    <Show when={backend()?.url}>{_configured => <div class="companion-backend-overview">
      <p class="companion-mono companion-url">{backend()!.url}</p>
      <dl class="companion-mirror-counts"><div><dt>Tasks shared</dt><dd>{backend()!.mirroredTasks}</dd></div><div><dt>Documents shared</dt><dd>{backend()!.mirroredDocs ?? 'Not reported'}</dd></div><div><dt>Scope</dt><dd>{backend()!.scope ?? 'Not reported'}</dd></div></dl>
      <p class="settings-note">{backend()!.lastPushAt ? `Last sent ${fmtTime(backend()!.lastPushAt!)}` : 'No successful task share reported yet.'}</p>
      <Show when={backend()!.docsSupported === false}><p class="settings-note">This backend needs an update to receive notes and workspaces. Task sharing still works.</p></Show>
      <Show when={connectionError()}>{message => <><ErrorText text={message()} class="settings-note" /><p class="settings-note companion-last-contact">{backend()!.lastContactAt ? `Last reached ${fmtTime(backend()!.lastContactAt)}` : 'Not reached yet'}</p></>}</Show>
      <Show when={otherErrors().length > 0}><details class="companion-scope"><summary>Connection or sharing needs attention</summary><For each={otherErrors()}>{message=><p class="settings-note">{message}</p>}</For></details></Show>
      <div class="settings-row companion-actions"><Button variant="ghost" onClick={()=>void testConnection(false)} disabled={busy()}>{testing() === 'saved' ? 'Testing…' : 'Test connection'}</Button><Button variant="ghost" onClick={repush} disabled={busy()}>Re-send all tasks</Button><Button variant="ghost" onClick={()=>void save('')} disabled={busy()}>Turn off</Button></div>
      <TestResult typed={false} />
    </div>}</Show>
    {/* Closed at rest, even before a backend is set (Settings PR 5b: a fold opens only when asked); its summary says what it is. */}
    <details class="companion-connection-editor">
      <summary>{backend()?.url ? 'Change connection' : 'Connect to the homelab'}</summary>
      <p class="settings-note">Use Hollow from a paired browser. Your Mac applies its changes to the original files.</p>
      <p class="settings-note">Enter your backend address, open its setup page, then use its one-time code. The code appears only while no Hollow is connected. Keep an existing connection unless you intend to replace it.</p>
      <form class="settings-row companion-backend-form" aria-label="Connect to the homelab" onSubmit={e=>{e.preventDefault();void connect();}}>
        <label><span>Backend URL</span><input type="url" value={url()} onInput={e=>editUrl(e.currentTarget.value)} placeholder="https://hollow-companion.<tailnet>.ts.net" spellcheck={false} disabled={busy()} required /></label>
        <label><span>Code from its setup page</span><input value={code()} onInput={e=>setCode(e.currentTarget.value)} placeholder="K7M3-Q9TX" autocomplete="one-time-code" autocapitalize="characters" spellcheck={false} maxlength={12} required disabled={busy()} /></label>
        <Button type="submit" disabled={busy() || !setupUrl() || !code().trim()}>{busy() ? 'Working…' : 'Connect'}</Button>
      </form>
      <div class="settings-row companion-actions"><Button variant="ghost" disabled={!setupUrl() || busy()} onClick={()=>openExternal(setupUrl())}>Open setup page</Button><Show when={setupUrl()}>{address=><span class="settings-note companion-url">{address()}</span>}</Show></div>
      <details class="companion-scope"><summary>Advanced: paste an applier token</summary>
        <form class="settings-row companion-backend-form" aria-label="Connect with an applier token" onSubmit={e=>{e.preventDefault();void save(url().trim());}}>
          <label><span>Backend URL</span><input type="url" value={url()} onInput={e=>editUrl(e.currentTarget.value)} placeholder="http://127.0.0.1:8787" spellcheck={false} disabled={busy()} required /></label>
          <label><span>Applier token</span><input type="password" value={token()} onInput={e=>{setToken(e.currentTarget.value);setTested(null);}} placeholder={backend()?.url ? 'Saved (paste to replace)' : 'Paste the backend token'} autocomplete="off" spellcheck={false} disabled={busy()} /></label>
          <Button type="submit" disabled={busy() || !setupUrl()}>Save</Button>
          <Button type="button" variant="ghost" disabled={busy() || !setupUrl()} onClick={()=>void testConnection(true)}>{testing() === 'typed' ? 'Testing…' : 'Test connection'}</Button>
        </form>
        <TestResult typed={true} />
        <p class="settings-note">A blank token keeps the saved token only for the same backend address.</p>
      </details>
    </details>
    <Show when={backend()?.url}><CompanionScopeEditor /></Show>
  </section>;
}

/// What the backend copies (companion backend v2, `companion_docs.rs`):
/// personal by default. Only this list puts a workspace on the homelab;
/// work workspaces can't be checked unless "Include work data" is on.
function CompanionScopeEditor() {
  const [scopeError, setScopeError] = createSignal('');
  let alive = true;
  onCleanup(()=>{alive=false;});
  const [scope, setScope] = createSignal<CompanionScope | null>(null);
  const [includeWork, setIncludeWork] = createSignal(false);
  const [chosen, setChosen] = createSignal<Set<string>>(new Set());
  const [busy, setBusy] = createSignal(false);

  function take(next: CompanionScope) {
    setScope(next);
    setIncludeWork(next.includeWorkData);
    setChosen(new Set(next.workspaces.filter(w => w.personal).map(w => w.id)));
  }

  async function load() {
    setScopeError('');
    try {const next=await api.companionScopeGet();if(alive)take(next);}
    catch (e) {if(alive)setScopeError(`Could not read what the backend copies. Retry before changing its scope. ${errMsg(e)}`);}
  }
  onMount(()=>void load());

  function toggle(id: string, on: boolean) {
    const next = new Set(chosen());
    if (on) next.add(id); else next.delete(id);
    setChosen(next);
  }

  const saved = () => scope()?.includeWorkData ?? false;
  const dirty = () => {
    const s = scope();
    if (!s) return false;
    const was = new Set(s.workspaces.filter(w => w.personal).map(w => w.id));
    const now = chosen();
    return includeWork() !== s.includeWorkData || was.size !== now.size || [...now].some(id => !was.has(id));
  };
  registerDirty('companion-scope', dirty, 'What the backend copies');

  async function save() {
    if (busy() || !scope()) return;
    const nextWork = includeWork(), nextChosen = [...chosen()];
    setBusy(true);setScopeError('');
    try {
      if (nextWork && !saved()) {
        const ok = await showConfirm({ title:'Send work data to the backend?', body:'The homelab backend will also get the work wiki, client notes, learnings and every workspace, imported projects included. Turn it off again here to remove them.', confirmLabel:'Include work data' });
        if (!ok || !alive) return;
      }
      const next = await api.companionScopeSet(nextWork, nextChosen);
      if (alive) {take(next);flashFooter(nextWork ? 'The backend now gets work data too' : 'Saved what the backend copies',2500);}
    } catch (e) {if(alive)setScopeError(`Could not save what the backend copies. Your selection is still here. ${errMsg(e)}`);}
    finally {if(alive)setBusy(false);}
  }

  const count = () => chosen().size;

  return (
    <details class="companion-scope">
      <summary>What the backend copies · {!scope() ? 'Selection not loaded' : saved() ? 'everything, work data included' : `personal: ${scope()?.workspaces.filter(w => w.personal).length ?? 0} workspace${scope()?.workspaces.filter(w => w.personal).length === 1 ? '' : 's'}`}</summary>
      <Show when={scopeError()}><ErrorText text={scopeError()} class="settings-note" /><Show when={!scope()}><Button variant="ghost" onClick={()=>void load()}>Retry selection</Button></Show></Show>
      <Show when={scope()?.error}>
        <ErrorText text={scope()?.error ?? ''} class="settings-note" />
      </Show>
      <p class="settings-note">
        Always: tasks and project names. Personal: your notes, and for each workspace checked below its brief, plan,
        sources, outputs and recording transcripts. Never: your profile (me/), chat sessions or Hollow's own files.
      </p>
      <label class="settings-note companion-scope-work">
        <input type="checkbox" checked={includeWork()} disabled={busy() || !scope()} onChange={e => setIncludeWork(e.currentTarget.checked)} />
        Include work data (the work wiki, client notes, learnings and every workspace, imported projects included)
      </label>
      <ul class="companion-scope-list" aria-label="Personal workspaces the backend copies">
        <For each={scope()?.workspaces ?? []}>
          {w => (
            <li>
              <label class={w.work ? 'is-work' : ''} title={w.work ? 'A work workspace: copied only with work data on' : undefined}>
                <input
                  type="checkbox"
                  checked={includeWork() || (!w.work && chosen().has(w.id))}
                  disabled={busy() || includeWork() || w.work}
                  onChange={e => toggle(w.id, e.currentTarget.checked)}
                />
                <span>{w.title}</span>
                <span class="companion-scope-tag">{w.work ? 'work' : w.status}</span>
              </label>
            </li>
          )}
        </For>
      </ul>
      <div class="settings-row">
        <Button onClick={() => void save()} disabled={busy() || !scope() || (!dirty() && !scope()?.error)}>Save what the backend copies</Button>
        <Show when={dirty()}><span class="settings-note">Not saved yet · {includeWork() ? 'work data included' : `${count()} personal workspace${count() === 1 ? '' : 's'}`}</span></Show>
      </div>
    </details>
  );
}
