/**
 * Settings, Accounts, below Claude: one row per other agent CLI (ChatGPT via
 * Codex, Antigravity, Grok) with its own sign-in state and its own sign-in /
 * sign-out commands (`provider_accounts.rs`), then the Jev API key. Hollow
 * stores none of these credentials; each CLI keeps its own.
 *
 * Plain signals loaded when the section opens, never resources, so a
 * refresh can't suspend the Settings page.
 */
import { For, Show, createSignal, onMount } from 'solid-js';
import { api } from '../api';
import { flashFooter, showConfirm, trackTask } from '../store';
import type { JevStatus, ProviderAccount } from '../types';
import { AgentMark } from '../components/AgentMark';
import { errMsg } from '../utils';
import '../styles/claude-accounts.css';
import { PlainErrorText } from '../components/ui';

function statusText(a: ProviderAccount): string {
  if (!a.installed) return 'Not installed';
  if (a.signedIn === null) return 'Could not check';
  return a.signedIn ? 'Signed in' : 'Signed out';
}

function statusState(a: ProviderAccount): string {
  if (!a.installed || a.signedIn === false) return 'missingToken';
  if (a.signedIn === null) return 'unknown';
  return 'ok';
}

export function ProviderAccountsPanel() {
  const [accounts, setAccounts] = createSignal<ProviderAccount[] | null>(null);
  const [error, setError] = createSignal('');
  const [pending, setPending] = createSignal<string | null>(null);

  async function load() {
    try {
      setAccounts(await api.providerAccounts());
      setError('');
    } catch (e) {
      setError(errMsg(e));
    }
  }
  onMount(() => void load());

  async function signIn(a: ProviderAccount) {
    setPending(a.id);
    try {
      setAccounts(await trackTask(`Sign in to ${a.label}`, () => api.providerSignIn(a.id), { detail: 'Finish in your browser' }));
    } catch (e) {
      flashFooter(`× ${errMsg(e)}`, 6000);
    } finally {
      setPending(null);
    }
  }

  async function signOut(a: ProviderAccount) {
    const ok = await showConfirm({
      title: `Sign out of ${a.label}?`,
      body: `${a.label} runs in Hollow and in your terminal stop working until you sign in again.`,
      confirmLabel: 'Sign out',
      danger: true,
    });
    if (!ok) return;
    setPending(a.id);
    try {
      setAccounts(await api.providerSignOut(a.id));
    } catch (e) {
      flashFooter(`× ${errMsg(e)}`, 6000);
    } finally {
      setPending(null);
    }
  }

  return (
    <div class="claude-accts provider-accts">
      <Show when={error()}>{msg => <p class="settings-note claude-acct-error"><PlainErrorText text={msg()} /></p>}</Show>
      <Show when={accounts()} fallback={<Show when={!error()}><p class="settings-loading">Checking sign-ins…</p></Show>}>
        {list => (
          <ul class="claude-acct-list">
            <For each={list()}>
              {a => (
                <li class="claude-acct provider-acct" data-provider={a.id}>
                  <header class="claude-acct-head">
                    <AgentMark agent={a.id} />
                    <span class="claude-acct-name">{a.label}</span>
                    <span class="claude-acct-status" data-status={statusState(a)}>{statusText(a)}</span>
                    <Show when={a.plan}>{p => <span class="claude-acct-kind">{p()}</span>}</Show>
                    <span class="claude-acct-spacer" />
                    <Show when={a.installed && a.signIn === 'browser'}>
                      <button class="btn" disabled={pending() !== null} onClick={() => void signIn(a)}>
                        {pending() === a.id ? 'Waiting for browser…' : a.signedIn ? 'Switch account' : 'Sign in'}
                      </button>
                    </Show>
                    <Show when={a.signIn === 'terminal'}>
                      <button class="btn" disabled={pending() !== null} onClick={() => void load()}>Check again</button>
                    </Show>
                    <Show when={a.canSignOut}>
                      <button class="btn" disabled={pending() !== null} onClick={() => void signOut(a)}>Sign out</button>
                    </Show>
                  </header>
                  <Show when={a.email}>{e => <div class="claude-acct-meta"><span>{e()}</span></div>}</Show>
                  <Show when={a.note}>{n => <p class="claude-acct-note">{n()}</p>}</Show>
                </li>
              )}
            </For>
          </ul>
        )}
      </Show>
      <JevKeyRow />
    </div>
  );
}

/** Jev's TypeSafe API key (the switches stay in Context prioritizer). */
function JevKeyRow() {
  const [status, setStatus] = createSignal<JevStatus | null>(null);
  const [key, setKey] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal('');
  const [failed, setFailed] = createSignal(false);

  onMount(async () => {
    try { setStatus(await api.jevStatus()); } catch (e) { setFailed(true); setMessage(errMsg(e)); }
  });

  async function run(action: () => Promise<void>, done: string) {
    setBusy(true);
    try { await action(); setFailed(false); setMessage(done); } catch (e) { setFailed(true); setMessage(errMsg(e)); } finally { setBusy(false); }
  }
  // Removing it switches Jev off too, so it asks first (P0-13).
  async function removeKey() {
    const ok = await showConfirm({
      title: 'Remove the Jev key?',
      body: 'The key leaves your macOS Keychain and Jev switches off: context ranking and the duplicate check go back to local only. To turn Jev on again you paste the key again.',
      confirmLabel: 'Remove key', danger: true, focusCancel: true,
    });
    if (ok) await run(async () => { setStatus(await api.removeJevKey()); }, 'Key removed and Jev switched off.');
  }

  return (
    // The whole card is the target of Manage key's jump (P1-22).
    <div class="claude-acct provider-acct" data-provider="jev" data-settings-control="Jev (TypeSafe)">
      <header class="claude-acct-head">
        <span class="claude-acct-name">Jev (TypeSafe)</span>
        <span class="claude-acct-status" data-status={status()?.keyPresent ? 'ok' : 'unknown'}>{status()?.keyPresent ? 'Key saved' : 'No key'}</span>
        <span class="claude-acct-kind">API key</span>
      </header>
      <p class="claude-acct-note">Jev ranks context when on in Context prioritizer. Its key stays in your Keychain.</p>
      <div class="claude-acct-edit">
        <label class="claude-acct-field">
          <span>Key</span>
          <input
            class="settings-input"
            type="password"
            autocomplete="off"
            aria-label="Jev API key"
            placeholder={status()?.keyPresent ? 'Key saved; paste to replace' : 'TypeSafe API key'}
            value={key()}
            onInput={e => setKey(e.currentTarget.value)}
          />
        </label>
        <div class="claude-acct-actions">
          <button class="btn primary" disabled={busy() || !key().trim()}
            onClick={() => void run(async () => {
              setStatus(await api.saveJevSettings(Boolean(status()?.enabled), key().trim()));
              setKey('');
            }, 'Key saved.')}>Save key</button>
          <Show when={status()?.keyPresent}>
            <button class="btn" disabled={busy()} onClick={() => void run(() => api.testJevKey(), 'Jev answered. The key works.')}>Test</button>
            <button class="btn" disabled={busy()} onClick={() => void removeKey()}>Remove key</button>
          </Show>
        </div>
      </div>
      <Show when={message()}><p class="claude-acct-note" role="status"><PlainErrorText text={message()} plain={failed()} /></p></Show>
    </div>
  );
}
