// M2 PR 4: Settings, Devices, Macs (`docs/reference/peers.md`, "The UI").
// The other Macs this one opens chats on: each paired Mac (its name, state and
// Forget on one line; slug, last seen and its SSH aliases in the order tried
// in its closed Details and SSH aliases fold), Check now, Pair a Mac
// (an alias; Hollow asks that Mac for a code over SSH and pairs), and the
// server side in one closed fold: Let another Mac connect (a code for 10
// minutes) and the Macs paired to this one, each with Revoke. Pairing and
// Forget are the person's clicks only (`peer_pair`, `peer_forget` are user-only commands). Lazy.
// M2 leftovers: a Mac's fold also adds and removes SSH aliases (another
// address for the same Mac), and a Mac that is connecting offers Cancel.
import { For, Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js';
import { invoke } from '../api';
import { onNative } from '../nativeEvents';
import { flashFooter, showConfirm } from '../store';
import type { PeerMachine } from '../generated/ipc';
import type { PhoneAccessStatus } from '../types';
import { errMsg, fmtTime } from '../utils';
import { Button, ErrorText } from '../components/ui';
import { Icon } from '../icons';
import { MachineGlyph } from '../components/MachineBadge';
import { agoLabel, stateText } from '../components/machineStrip';
import { cancelPeerConnect, followPeerSessions, isPeerFailure, loadPeerMachines, peerMachines } from '../store/peerSessions';
import { pairRequest, takePairRequest } from '../store/peerUi';
import { parsePairAddress } from '../store/pairAddress';
import { workspaceSessions } from '../store/workspaces';
import { loadChatHomes } from '../store/chatMoves';
import { forgetBody, macName } from '../store/chatMoveWords';
import { MoveOnSleepRow } from './MoveOnSleepRow';
import '../styles/peer-machines.css';

/** SSH aliases per Mac, at most (`peer_machines::HOSTS_MAX`). */
const HOSTS_MAX = 8;

const failure = (e: unknown) => (isPeerFailure(e) ? e.message : errMsg(e));

export function MachinesPanel() {
  const [now, setNow] = createSignal(Date.now());
  const [pairing, setPairing] = createSignal(false);
  const [alias, setAlias] = createSignal('');
  const [busy, setBusy] = createSignal<string | null>(null);
  const [progress, setProgress] = createSignal('');
  // Cancel was clicked while a pairing ran: it ends as a quiet return, not an error.
  const [cancelling, setCancelling] = createSignal(false);
  const [error, setError] = createSignal('');
  const [server, setServer] = createSignal<PhoneAccessStatus | null>(null);
  const [serverError, setServerError] = createSignal('');
  // The alias being typed in each Mac's fold, by machine id.
  const [newAlias, setNewAlias] = createSignal<Readonly<Record<string, string>>>({});
  let aliasBox: HTMLInputElement | undefined;
  let section: HTMLElement | undefined;

  async function readServer() {
    try {
      setServer(await invoke<PhoneAccessStatus>('phone_access_status'));
      setServerError('');
    } catch (e) {
      setServerError(`Could not read the Macs paired to this one. ${errMsg(e)}`);
    }
  }

  onMount(() => {
    const stop = followPeerSessions();
    void loadPeerMachines(false);
    void readServer();
    const off = onNative('phone-access-changed', () => void readServer());
    const timer = window.setInterval(() => { setNow(Date.now()); if (server()?.machinePairing) void readServer(); }, 5000);
    onCleanup(() => { stop(); off(); window.clearInterval(timer); });
  });

  // The strip's Pair again: open Pair a Mac with its alias, once.
  createEffect(() => {
    if (!pairRequest()) return;
    const ask = takePairRequest();
    if (!ask) return;
    setAlias(ask.alias);
    setPairing(true);
    setError(''); setProgress('');
    queueMicrotask(() => { section?.scrollIntoView({ block: 'start' }); aliasBox?.focus(); });
  });

  const machines = () => peerMachines()?.machines ?? [];
  const startPairing = () => {
    setAlias(alias() || peerMachines()?.suggestedAlias || '');
    setPairing(true); setError(''); setProgress('');
    queueMicrotask(() => aliasBox?.focus());
  };

  async function pair() {
    const host = alias().trim();
    if (!host || busy()) return;
    // Caught here first, in plain words; the backend checks again and decides.
    const typed = parsePairAddress(host);
    if (!typed.ok) { setError(typed.message); return; }
    setBusy('pair'); setError(''); setCancelling(false);
    setProgress(`Asking ${host} for a code, then connecting…`);
    try {
      const m = await invoke<PeerMachine>('peer_pair', { alias: host });
      setProgress(`Paired with ${m.name}.`);
      setPairing(false);
      flashFooter(`Paired with ${m.name}`);
      await loadPeerMachines(false);
    } catch (e) {
      setProgress('');
      // Cancelled: back to where it was before Pair, nothing kept.
      if (cancelling()) { setPairing(false); setError(''); flashFooter('Pairing cancelled'); await loadPeerMachines(false); }
      else setError(failure(e));
    } finally {
      setBusy(null);
      setCancelling(false);
    }
  }

  /** Cancel on the Pairing step: closes the form, or while a pairing runs
   *  stops it at the backend (`peer_pair_cancel`; nothing is kept) and waits
   *  for `pair()` to come back. */
  async function cancelPairing() {
    if (busy() !== 'pair') { setPairing(false); setError(''); setProgress(''); return; }
    if (cancelling()) return;
    setCancelling(true);
    setProgress('Cancelling…');
    try {
      const was = await invoke<boolean>('peer_pair_cancel');
      // Nothing was running (it just finished): its own result stands.
      if (!was) setCancelling(false);
    } catch (e) {
      setCancelling(false);
      setError(failure(e));
    }
  }

  /** A local chat's title by key, for Forget's list of chats it releases. */
  const titleOf = (key: string) => Object.values(workspaceSessions()).flatMap(g => g?.sessions ?? []).find(x => x.key === key)?.title?.trim() || 'Untitled chat';

  async function forget(m: PeerMachine) {
    if (busy()) return;
    // Busy before the homes read, so a second click can't open a second confirm.
    setBusy(`forget:${m.machineId}`);
    // M5: Forget releases the chats still moving there; the ones that moved stay there.
    const homes = await loadChatHomes();
    const ok = await showConfirm({
      title: `Forget ${m.name}?`,
      body: forgetBody(m.name, m.machineId, homes, titleOf),
      confirmLabel: 'Forget', danger: true,
    });
    if (!ok) { setBusy(null); return; }
    setError('');
    try {
      await invoke<boolean>('peer_forget', { machineId: m.machineId });
      flashFooter(`${m.name} is forgotten`);
      await loadPeerMachines(false);
    } catch (e) {
      setError(failure(e));
    } finally {
      setBusy(null);
      // Released even when unpairing there failed.
      void loadChatHomes();
    }
  }

  /** Save a Mac's aliases (order, an added one, one removed); `true` when saved. */
  async function saveHosts(m: PeerMachine, hosts: string[]): Promise<boolean> {
    setBusy(`hosts:${m.machineId}`); setError('');
    try {
      await invoke<PeerMachine>('peer_reorder_hosts', { machineId: m.machineId, hosts });
      await loadPeerMachines(false);
      return true;
    } catch (e) {
      setError(failure(e));
      return false;
    } finally {
      setBusy(null);
    }
  }

  async function move(m: PeerMachine, from: number, to: number) {
    const hosts = [...m.sshHosts];
    const [h] = hosts.splice(from, 1);
    hosts.splice(to, 0, h);
    await saveHosts(m, hosts);
  }

  /** Another address for the same Mac, tried after the others (the backend
   *  checks the name and refuses a repeat). */
  async function addAlias(m: PeerMachine) {
    const host = (newAlias()[m.machineId] ?? '').trim();
    if (!host || busy()) return;
    if (m.sshHosts.includes(host)) { setError(`${host} is already one of ${m.name}’s aliases.`); return; }
    if (await saveHosts(m, [...m.sshHosts, host])) {
      setNewAlias(all => ({ ...all, [m.machineId]: '' }));
      flashFooter(`${m.name} can be reached through ${host} too`);
    }
  }

  async function removeAlias(m: PeerMachine, host: string) {
    if (m.sshHosts.length < 2 || busy()) return;
    await saveHosts(m, m.sshHosts.filter(h => h !== host));
  }

  async function cancelConnect(m: PeerMachine) {
    setBusy(`cancel:${m.machineId}`); setError('');
    try { await cancelPeerConnect(m.machineId); } catch (e) { setError(failure(e)); } finally { setBusy(null); }
  }

  async function check() {
    setBusy('check');
    try { await loadPeerMachines(true); } finally { setBusy(null); }
  }

  async function serverAct(command: string, args?: Record<string, unknown>) {
    setBusy(command); setServerError('');
    try {
      setServer(await invoke<PhoneAccessStatus>(command, args));
    } catch (e) {
      setServerError(errMsg(e));
    } finally {
      setBusy(null);
    }
  }

  async function revoke(deviceId: string, name: string) {
    const ok = await showConfirm({ title: `Revoke ${name}?`, body: `${name} can no longer open this Mac’s chats. Pair again to let it back.`, confirmLabel: 'Revoke', danger: true });
    if (ok) await serverAct('phone_access_revoke', { deviceId });
  }

  const until = (at: string) => new Date(at).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

  const serverMachines = () => server()?.machines ?? [];
  // The server fold's summary says what is inside it, so a closed fold still
  // shows an open code and how many Macs open this one's chats.
  const serverState = () => {
    const code = server()?.machinePairing;
    if (code) return `code open until ${until(code.expiresAt)}`;
    const n = serverMachines().length;
    return n === 0 ? 'no Mac paired to this one' : `${n} paired to this one`;
  };

  // Settings height (1.3 screens a section): at rest the panel is one row
  // (Macs, its state, Pair a Mac) plus a one-line row per paired Mac and one
  // closed fold for the server side. Each Mac's details and its SSH aliases sit
  // in that Mac's own closed fold. Nothing is dropped; each fold's summary names
  // what is inside it.
  return (
    <section class="machines-panel" ref={el => (section = el)} aria-labelledby="machines-title">
      <div class="settings-row machines-head">
        <div class="settings-row-label">
          <h3 id="machines-title" class="settings-row-k">Macs</h3>
          <div class="settings-row-v">Steer another Mac’s chats over SSH. Only chats cross, no files or settings.</div>
        </div>
        <div class="settings-row-actions">
          <span class="companion-connection-state">{machines().length === 0 ? 'None paired' : `${machines().length} paired`}</span>
          <Show when={machines().length > 0}>
            <Button variant="ghost" disabled={!!busy()} onClick={() => void check()}>{busy() === 'check' ? 'Checking…' : 'Check now'}</Button>
          </Show>
          <Show when={!pairing()}>
            <Button onClick={startPairing} disabled={!!busy()}>Pair a Mac</Button>
          </Show>
        </div>
      </div>
      <Show when={error()}><ErrorText text={error()} class="settings-note" /></Show>
      <Show when={pairing()}>
        <>
          <form class="settings-row companion-backend-form machines-pair" aria-label="Pair a Mac" onSubmit={e => { e.preventDefault(); void pair(); }}>
            <label><span>SSH alias or address</span>
              <input ref={el => (aliasBox = el)} value={alias()} onInput={e => setAlias(e.currentTarget.value)} placeholder="my-runner or runner.tailnet.ts.net" spellcheck={false} autocapitalize="off" disabled={busy() === 'pair'} required />
            </label>
            <Button type="submit" disabled={!!busy() || !alias().trim()}>{busy() === 'pair' ? 'Pairing…' : 'Pair'}</Button>
            <Button variant="ghost" disabled={cancelling() || (!!busy() && busy() !== 'pair')} onClick={() => void cancelPairing()}>{cancelling() ? 'Cancelling…' : 'Cancel'}</Button>
          </form>
          <p class="settings-note">Hollow asks the other Mac for a code over SSH, so you need an SSH alias or address that reaches it. If the suggested name is wrong, type its address instead: a tailnet name, user@host, or host:port (the port Hollow answers on there, 47680 unless you changed it). Cancel stops a pairing in progress and keeps nothing.</p>
        </>
      </Show>
      <p class="settings-note machines-progress" role="status">{progress()}</p>
      <Show when={machines().length > 0}>
        <ul class="machines-list" aria-label="Paired Macs">
          <For each={machines()}>
            {m => (
              <li class="machines-row">
                <div class="machines-row-head">
                  <MachineGlyph size={15} />
                  <strong>{m.name}</strong>
                  <span class="machine-dot" data-dot={m.state === 'online' ? 'online' : m.state === 'connecting' ? 'connecting' : m.state === 'offline' ? 'offline' : 'attention'} aria-hidden="true" />
                  <span class="machines-row-state">{stateText(m, 0, now())}</span>
                  <span class="machines-row-actions">
                    <Show when={m.state === 'connecting'}>
                      <Button variant="ghost" size="sm" disabled={!!busy()} onClick={() => void cancelConnect(m)} aria-label={`Cancel connecting to ${m.name}`}>Cancel</Button>
                    </Show>
                    <Button variant="ghost" size="sm" disabled={!!busy()} onClick={() => void forget(m)} aria-label={`Forget ${m.name}`}>Forget</Button>
                  </span>
                </div>
                <Show when={m.message}>{msg => <p class="settings-note machines-row-message">{msg()}</p>}</Show>
                <details class="settings-fold machines-row-more">
                  <summary>Details and SSH aliases · {m.sshHosts.length === 1 ? '1 alias' : `${m.sshHosts.length} aliases`}{m.slug ? '' : ' · no home slug'}</summary>
                  <p class="settings-note">
                    <Show when={m.slug} fallback={<>No home slug on {m.name}: set one there.</>}>{slug => <>Home slug <code class="companion-mono">{slug()}</code>.</>}</Show>
                    {' '}Paired {fmtTime(m.pairedAt)}.
                    {m.lastSeenAt ? ` Last seen ${agoLabel(m.lastSeenAt, now())}.` : ' Not seen yet.'}
                    {m.lastVersion ? ` Hollow ${m.lastVersion}.` : ''}
                  </p>
                  <ol class="machines-hosts" aria-label={`SSH aliases for ${m.name}, in the order tried`}>
                    <For each={m.sshHosts}>
                      {(host, i) => (
                        <li>
                          <code class="companion-mono">{host}</code>
                          <Show when={m.alias === host}><span class="machines-host-up">connected</span></Show>
                          <button type="button" class="icon-btn" disabled={!!busy() || i() === 0} aria-label={`Try ${host} earlier`} title="Try earlier" onClick={() => void move(m, i(), i() - 1)}><Icon name="arrowUp" size={13} /></button>
                          <button type="button" class="icon-btn" disabled={!!busy() || m.sshHosts.length < 2} aria-label={`Remove ${host}`} title={m.sshHosts.length < 2 ? 'A Mac needs one alias' : 'Remove this alias'} onClick={() => void removeAlias(m, host)}><Icon name="x" size={13} /></button>
                        </li>
                      )}
                    </For>
                  </ol>
                  <Show when={m.sshHosts.length < HOSTS_MAX}>
                    <form class="machines-alias-add" aria-label={`Add an SSH alias for ${m.name}`} onSubmit={e => { e.preventDefault(); void addAlias(m); }}>
                      <input value={newAlias()[m.machineId] ?? ''} onInput={e => { const v = e.currentTarget.value; setNewAlias(all => ({ ...all, [m.machineId]: v })); }}
                        aria-label={`Another SSH alias for ${m.name}`} placeholder="another alias, e.g. my-runner-lan" spellcheck={false} autocapitalize="off" disabled={!!busy()} />
                      <Button type="submit" variant="ghost" size="sm" disabled={!!busy() || !(newAlias()[m.machineId] ?? '').trim()}>Add alias</Button>
                    </form>
                  </Show>
                </details>
              </li>
            )}
          </For>
        </ul>
      </Show>
      {/* M5: the sleep move, beside the Mac it moves chats to. */}
      <MoveOnSleepRow runner={macName(peerMachines(), peerMachines()?.sleepTo ?? '')} />

      <Show when={server()?.available}>
        <details class="settings-fold machines-server" aria-label="Macs that open this Mac’s chats">
          <summary>Let another Mac connect · {serverState()}</summary>
          <Show when={serverError()}><ErrorText text={serverError()} class="settings-note" /></Show>
          <Show
            when={server()?.machinePairing}
            fallback={
              <div class="settings-row">
                <div class="settings-row-label">
                  <div class="settings-row-k">Let another Mac connect</div>
                  <div class="settings-row-v">Shows a code the other Mac types in its Pair a Mac. It works once, for 10 minutes.</div>
                </div>
                <Button variant="ghost" disabled={!!busy()} onClick={() => void serverAct('phone_access_machine_pairing_start')}>Let another Mac connect</Button>
              </div>
            }
          >
            {code => (
              <div class="settings-row" role="status" aria-label={`Pairing code ${code().code.split('').join(' ')}`}>
                <p class="settings-note">Type this code on the other Mac. It works once, until {until(code().expiresAt)}.</p>
                <code class="companion-code">{code().code}</code>
                <Button variant="ghost" disabled={!!busy()} onClick={() => void serverAct('phone_access_machine_pairing_cancel')}>Cancel</Button>
              </div>
            )}
          </Show>
          <Show when={serverMachines().length > 0} fallback={<p class="settings-note">No Mac opens this Mac’s chats.</p>}>
            <ul class="companion-device-list" aria-label="Macs paired to this one">
              <For each={serverMachines()}>
                {d => (
                  <li class="settings-row">
                    <span><strong>{d.name}</strong>{' · paired '}{fmtTime(d.pairedAt)}<Show when={d.lastSeenAt}>{at => <>{' · last seen '}{fmtTime(at())}</>}</Show></span>
                    <Button variant="ghost" disabled={!!busy()} onClick={() => void revoke(d.deviceId, d.name)} aria-label={`Revoke ${d.name}`}>Revoke</Button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </details>
      </Show>
    </section>
  );
}
