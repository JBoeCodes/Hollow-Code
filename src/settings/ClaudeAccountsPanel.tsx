/**
 * Settings, Claude accounts: several Claude plans, and which one each
 * Claude run uses (`claude_accounts.rs`).
 *
 * Only accounts added here are listed and routed. Each is a
 * token minted with `claude setup-token` while signed in as that account,
 * stored in Hollow's Keychain item; it goes in once and never comes back
 * over IPC. Routing (One account or Spread usage), each account's meters
 * and In rotation are in Usage, Limits since Settings PR 8; this list keeps
 * add, edit, remove and a link row there.
 *
 * A plain signal loaded when the section opens, never a resource, so an
 * update can't suspend the Settings page. Lazy chunk with its own CSS.
 */
import { For, Show, createSignal, onMount } from 'solid-js';
import { api } from '../api';
import { flashFooter, requestOpenSettingsSection, showConfirm, trackTask } from '../store';
import type { ClaudeAccount, ClaudeAccountsView } from '../types';
import { STATUS_LABELS, tokenProblem } from '../claudeAccounts';
import { errMsg } from '../utils';
import '../styles/claude-accounts.css';
import { ErrorText, Skeleton } from '../components/ui';
import { SettingsSheet, openerRef } from './SettingsSheet';

export function ClaudeAccountsPanel() {
  const [view, setView] = createSignal<ClaudeAccountsView | null>(null);
  const [error, setError] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [adding, setAdding] = createSignal(false);
  const opener = openerRef();
  const [label, setLabel] = createSignal('');
  const [token, setToken] = createSignal('');
  const [editing, setEditing] = createSignal<string | null>(null);
  const [editLabel, setEditLabel] = createSignal('');
  const [editToken, setEditToken] = createSignal('');

  async function run(action: () => Promise<ClaudeAccountsView>, failure?: string): Promise<boolean> {
    setBusy(true);
    try {
      setView(await action());
      setError('');
      return true;
    } catch (e) {
      const message = errMsg(e);
      if (failure) flashFooter(`× ${failure}: ${message}`, 5000);
      else setError(message);
      return false;
    } finally {
      setBusy(false);
    }
  }

  onMount(() => { void run(() => api.claudeAccounts()); });
  // Settings PR 8: Spread usage, the meters and In rotation moved to Usage, Limits.
  const openLimits = () => requestOpenSettingsSection('usage', 'Automatic account rotation');

  async function add() {
    const ok = await run(
      () => trackTask('Add Claude account', () => api.addClaudeAccount(label(), token())),
      'Could not add the account',
    );
    if (ok) {
      setLabel('');
      setToken('');
      setAdding(false);
    }
  }

  function startEdit(a: ClaudeAccount) {
    setEditing(a.id);
    setEditLabel(a.label);
    setEditToken('');
  }

  async function saveEdit(a: ClaudeAccount) {
    const ok = await run(
      () => api.updateClaudeAccount({ id: a.id, label: editLabel(), token: editToken() || null }),
      'Could not save the account',
    );
    if (ok) setEditing(null);
  }

  async function remove(a: ClaudeAccount) {
    const confirmed = await showConfirm({
      title: `Remove ${a.label}?`,
      body: 'Hollow deletes its token from the Keychain and stops using the account. The token itself stays valid until you revoke it on claude.ai.',
      confirmLabel: 'Remove account',
      danger: true,
    });
    if (confirmed) void run(() => api.removeClaudeAccount(a.id), 'Could not remove the account');
  }

  return (
    <div class="claude-accts">
      <Show when={error()}>{msg => <ErrorText class="settings-note claude-acct-error" text={msg()} />}</Show>
      <Show when={view()} fallback={<Show when={!error()}><Skeleton label="Loading accounts" /></Show>}>
        {v => (
          <>
            <Show when={v().error}>{msg => <ErrorText class="settings-note claude-acct-error" text={msg()} />}</Show>
            <p class="settings-note" data-jump-skip>
              Limits and automatic account rotation are in Usage. <button type="button" class="btn-link" onClick={openLimits}>Open Usage, Limits</button>
            </p>

            <ul class="claude-acct-list">
              <For each={v().accounts}>
                {a => (
                  <li class="claude-acct" data-status={a.status} data-enabled={a.enabled ? 'yes' : 'no'}>
                    <header class="claude-acct-head">
                      <span class="claude-acct-name">{a.label}</span>
                      <span class="claude-acct-status" data-status={a.status}>{STATUS_LABELS[a.status]}</span>
                      <span class="claude-acct-spacer" />
                      <button class="btn" disabled={busy()} onClick={() => (editing() === a.id ? setEditing(null) : startEdit(a))}>
                        {editing() === a.id ? 'Cancel' : 'Edit'}
                      </button>
                      <button class="btn" disabled={busy()} onClick={() => void remove(a)}>Remove</button>
                    </header>
                    <div class="claude-acct-meta">
                      <Show when={a.lastUsedAt}>{t => <span title={t()}>Last run {new Date(t()).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}</span>}</Show>
                    </div>
                    <Show when={a.note}>{n => <p class="claude-acct-note">{n()}</p>}</Show>
                    <Show when={editing() === a.id}>
                      <div class="claude-acct-edit">
                        <label class="claude-acct-field">
                          <span>Name</span>
                          <input class="settings-input" value={editLabel()} onInput={e => setEditLabel(e.currentTarget.value)} />
                        </label>
                        <label class="claude-acct-field">
                            <span>New token</span>
                            <input
                              class="settings-input"
                              type="password"
                              autocomplete="off"
                              placeholder="Leave blank to keep the current token"
                              value={editToken()}
                              onInput={e => setEditToken(e.currentTarget.value)}
                            />
                          </label>
                          <Show when={tokenProblem(editToken())}>{p => <span class="claude-acct-hint">{p()}</span>}</Show>
                        <button
                          class="btn primary"
                          disabled={busy() || !editLabel().trim() || !!tokenProblem(editToken())}
                          onClick={() => void saveEdit(a)}
                        >Save</button>
                      </div>
                    </Show>
                  </li>
                )}
              </For>
            </ul>

            <button class="btn" disabled={busy()} onClick={e => { opener.mark(e); setAdding(true); }}>Add a Claude account…</button>
            {/* Settings PR 5: the add form is a sheet; closing it puts focus back on this button. */}
            <Show when={adding()}>
              <SettingsSheet id="settings-sheet-claude-account" title="Add a Claude account" returnTo={opener.get} locked={busy} onClose={() => { setAdding(false); setToken(''); }}>
              <Show when={error()}>{msg => <ErrorText class="settings-note claude-acct-error" text={msg()} />}</Show>
              <div class="claude-acct-add">
                <ol class="claude-acct-steps">
                  <li>In a terminal, run <code>claude auth login</code> and sign in as the other account.</li>
                  <li>Run <code>claude setup-token</code>. It prints a token starting <code>sk-ant-oat01-</code> (valid for a year).</li>
                  <li>Paste it below. Line breaks from a narrow terminal are removed for you.</li>
                  <li>Run <code>claude auth login</code> again to sign back in as yourself. The token keeps working.</li>
                </ol>
                <label class="claude-acct-field">
                  <span>Name</span>
                  <input class="settings-input" placeholder="e.g. Second Max plan" value={label()} onInput={e => setLabel(e.currentTarget.value)} />
                </label>
                <label class="claude-acct-field">
                  <span>Token</span>
                  <input
                    class="settings-input"
                    type="password"
                    autocomplete="off"
                    placeholder="sk-ant-oat01-…"
                    value={token()}
                    onInput={e => setToken(e.currentTarget.value)}
                  />
                </label>
                <Show when={tokenProblem(token())}>{p => <span class="claude-acct-hint">{p()}</span>}</Show>
                <p class="claude-acct-note">
                  The token is stored in your macOS Keychain and never shown again. Hollow checks it with a
                  one-token request, then uses it only for Claude runs Hollow starts.
                </p>
                <div class="claude-acct-actions">
                  <button
                    class="btn primary"
                    disabled={busy() || !label().trim() || !token().trim() || !!tokenProblem(token())}
                    onClick={() => void add()}
                  >Add account</button>
                  <button class="btn" disabled={busy()} onClick={() => { setAdding(false); setToken(''); }}>Cancel</button>
                </div>
              </div>
              </SettingsSheet>
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}
