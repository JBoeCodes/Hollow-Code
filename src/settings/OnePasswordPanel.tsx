/**
 * Settings, Accounts, 1Password (t3code parity row 7.5, 2026-11-20): an
 * optional second secret path beside the Keychain, off by default. With the
 * switch on, a secret you point at an `op://Vault/Item/field` reference is
 * read from 1Password at the moment Hollow needs it (the `op` command line
 * tool, or the 1Password app's sign-in), never saved by Hollow. The Keychain
 * stays the default and the fallback. The status never carries a secret:
 * references, a state and fixed messages (`onepassword.rs`).
 *
 * Plain signals loaded when the section opens, never resources, so a
 * refresh can't suspend the Settings page.
 */
import { For, Show, createSignal, onMount } from 'solid-js';
import { api } from '../api';
import { Switch } from '../components/Switch';
import { PlainErrorText } from '../components/ui';
import type { OnePasswordSlot, OnePasswordStatus } from '../types';
import { errMsg } from '../utils';
import '../styles/claude-accounts.css';

const CHIP: Record<OnePasswordStatus['cli']['state'], { text: string; state: string }> = {
  off: { text: 'Off', state: 'unknown' },
  'not-installed': { text: 'op not installed', state: 'missingToken' },
  unchecked: { text: 'On', state: 'unknown' },
  ready: { text: 'Ready', state: 'ok' },
  locked: { text: 'Locked', state: 'missingToken' },
  'not-signed-in': { text: 'Not signed in', state: 'missingToken' },
  'timed-out': { text: 'No answer', state: 'missingToken' },
  error: { text: 'Error', state: 'missingToken' },
};

function SlotRow(props: { slot: OnePasswordSlot; on: boolean; busy: boolean; save: (key: string, reference: string) => void; clear: (key: string) => void }) {
  const [draft, setDraft] = createSignal('');
  const saved = () => props.slot.reference;
  return (
    <div class="claude-acct-edit" data-slot={props.slot.key}>
      <label class="claude-acct-field">
        <span>{props.slot.label}</span>
        <input
          class="settings-input"
          type="text"
          autocomplete="off"
          spellcheck={false}
          aria-label={`1Password reference for ${props.slot.label}`}
          placeholder={saved() ?? 'op://Vault/Item/field'}
          value={draft()}
          onInput={e => setDraft(e.currentTarget.value)}
        />
      </label>
      <div class="claude-acct-actions">
        <button class="btn primary" disabled={props.busy || !draft().trim()}
          onClick={() => { props.save(props.slot.key, draft()); setDraft(''); }}>
          {saved() ? 'Replace reference' : 'Save reference'}
        </button>
        <Show when={saved()}>
          <button class="btn" disabled={props.busy} onClick={() => props.clear(props.slot.key)}>Use the Keychain</button>
        </Show>
      </div>
      <Show when={saved()}>
        <p class="claude-acct-note">
          Reads <code>{saved()}</code>{props.on ? ' when it is needed.' : ' once 1Password secrets are on.'}
        </p>
      </Show>
      <Show when={saved() && props.slot.lastError}>
        {why => <p class="claude-acct-note" role="status">The last read through 1Password failed (the Keychain value was used, if there is one): {why()}</p>}
      </Show>
    </div>
  );
}

export function OnePasswordPanel() {
  const [status, setStatus] = createSignal<OnePasswordStatus | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal('');
  const [failed, setFailed] = createSignal(false);

  onMount(() => { void api.onePasswordStatus().then(setStatus, e => { setFailed(true); setMessage(errMsg(e)); }); });

  async function act(run: () => Promise<OnePasswordStatus>, done = '') {
    if (busy()) return;
    setBusy(true);
    try {
      setStatus(await run());
      setFailed(false);
      setMessage(done);
    } catch (e) {
      setFailed(true);
      setMessage(errMsg(e));
    } finally {
      setBusy(false);
    }
  }

  const on = () => status()?.enabled ?? false;
  const chip = () => CHIP[status()?.cli.state ?? 'off'];

  return (
    // The whole card is the target of a jump from search (P1-22).
    <div class="claude-acct provider-acct" data-provider="onepassword" data-settings-control="1Password">
      <header class="claude-acct-head">
        <span class="claude-acct-name">1Password secrets</span>
        <span class="claude-acct-status" data-status={chip().state}>{chip().text}</span>
        <span class="claude-acct-kind">Optional</span>
      </header>
      <p class="claude-acct-note">
        Read a secret from 1Password when Hollow needs it, instead of the Keychain. Hollow saves only the
        reference you give it, never what 1Password returns, and the Keychain stays the fallback when 1Password
        can't answer. Off until you turn it on.
      </p>
      <div class="claude-acct-controls">
        <Switch checked={on()} label="Use 1Password for secrets" disabled={busy() || !status()}
          onChange={next => void act(() => api.onePasswordSetEnabled(next), next ? '1Password secrets are on.' : 'Back to the Keychain for every secret.')} />
        <span class="claude-acct-note" role="status">{status()?.cli.message ?? 'Checking…'}</span>
        <Show when={on()}>
          <button class="btn" disabled={busy()} onClick={() => void act(() => api.onePasswordCheck())}>Check</button>
        </Show>
      </div>
      <Show when={status()?.cli.version}>
        {v => <p class="claude-acct-note">1Password CLI {v()}</p>}
      </Show>
      <Show when={on() && status() && !status()!.installed}>
        <p class="claude-acct-note">
          Install the 1Password command line tool (<code>brew install 1password-cli</code>), then turn on
          Integrate with 1Password CLI in the 1Password app (Settings, Developer).
        </p>
      </Show>
      <Show when={status()}>
        <For each={status()!.slots}>
          {slot => (
            <SlotRow slot={slot} on={on()} busy={busy()}
              save={(key, reference) => void act(() => api.onePasswordSetReference(key, reference), 'Reference saved.')}
              clear={key => void act(() => api.onePasswordClearReference(key), 'Using the Keychain for that secret.')} />
          )}
        </For>
      </Show>
      <Show when={message()}><p class="claude-acct-note" role="status"><PlainErrorText text={message()} plain={failed()} /></p></Show>
    </div>
  );
}
