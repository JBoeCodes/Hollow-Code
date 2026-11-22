/**
 * Settings, Usage, Limits (Settings PR 8): Claude's rows and Spread usage,
 * moved from Agents, Accounts. Each added account shows its 5-hour and
 * weekly meters with the Move on at line drawn on them, when each resets,
 * its In rotation switch and its last run; Claude Code's login shows the
 * same two meters when it is in use. "Read live from Claude" is the old
 * Show plan usage opt-in (it may ask Claude's usage service with the
 * login); the readings Hollow's own runs report show without it.
 *
 * Account rotation is automatic; per-account exclusions and the ceiling remain.
 */
import { For, Show } from 'solid-js';
import { api } from '../api';
import { requestOpenSettingsSection, trackTask } from '../store';
import { quotaOptIn, setQuotaOptIn } from '../store/prefs';
import type { ClaudeAccountWindow, ClaudeAccountsView } from '../types';
import {
  STATUS_LABELS, STEP_ASIDE_NOTE, failoverSummary, fillFraction, formatPercent, loginPinNote, loginRowBadge, loginStatus,
  nextRunBadge, resetLabel, routingSummary, sourceLabel, windowLevel,
} from '../claudeAccounts';
import { ceilingChoices, spreadUsage } from './spreadUsageActions';
import { saveCheckbox, type ClaudeLimitsState } from './claudeLimitsState';
import { UsageMeter } from './UsageMeter';
import { LeadText } from './settingsLayout';
import { ClaudeQuotaCard } from '../components/ClaudeQuotaCard';
import { Select } from '../components/Select';
import { ErrorText, Skeleton } from '../components/ui';
import '../styles/claude-accounts.css';

function WindowMeter(props: { label: string; w: ClaudeAccountWindow | null; view: ClaudeAccountsView }) {
  const ceiling = () => props.view.ceilingPercent;
  return (
    <UsageMeter
      label={props.label}
      fill={props.w ? fillFraction(props.w.usedPercent) : 0}
      level={windowLevel(props.w, ceiling())}
      value={props.w ? formatPercent(props.w.usedPercent) : 'Not read yet'}
      meta={props.w ? resetLabel(props.w.resetsAt) : null}
      marks={props.view.routing === 'spread' ? [{ at: ceiling() / 100, kind: 'move', title: `Move on at ${ceiling()}%` }] : []}
    />
  );
}

/** Claude's rows on Limits: each account, then Claude Code's login. */
export function ClaudeMeters(props: { s: ClaudeLimitsState }) {
  const s = props.s;
  const openAccounts = () => requestOpenSettingsSection('accounts', 'Accounts');
  const openConnectors = () => requestOpenSettingsSection('connectors', 'Your connectors');
  return (
    <section class="usage-block usage-limits-agent" aria-label="Claude limits">
      <header class="usage-limits-head">
        <h3 class="usage-block-title">Claude</h3>
        <label class="usage-switch" title="Ask Claude's usage service with Claude Code's login (macOS may ask for Keychain access). Without it, the readings Hollow's own Claude runs report still show.">
          <input type="checkbox" role="switch" checked={quotaOptIn()} onChange={e => setQuotaOptIn(e.currentTarget.checked)} />
          Read live from Claude
        </label>
      </header>
      <Show when={s.error()}>{msg => <ErrorText class="settings-note claude-acct-error" text={msg()} />}</Show>
      <Show when={s.view()} fallback={<Show when={!s.error()}><Skeleton label="Loading Claude accounts" /></Show>}>
        {v => (
          <>
            <Show when={v().error}>{msg => <ErrorText class="settings-note claude-acct-error" text={msg()} />}</Show>
            <ul class="claude-acct-list">
              <For each={v().accounts}>
                {a => (
                  <li class="claude-acct" data-status={a.status} data-enabled={a.enabled ? 'yes' : 'no'}>
                    <header class="claude-acct-head">
                      <span class="claude-acct-name">{a.label}</span>
                      <Show when={v().nextAccount === a.id}><span class="claude-acct-next">{nextRunBadge(v(), s.pin())}</span></Show>
                      <span class="claude-acct-status" data-status={a.status}>{STATUS_LABELS[a.status]}</span>
                      <span class="claude-acct-spacer" />
                      <label class="claude-acct-use" title="Use this account">
                        <input
                          type="checkbox"
                          checked={a.enabled}
                          disabled={s.busy()}
                          onChange={e => { const box = e.currentTarget; void saveCheckbox(box, () => s.view()?.accounts.find(x => x.id === a.id)?.enabled ?? a.enabled, () => s.run(() => spreadUsage.inRotation(api, a.id, box.checked), 'Could not update the account')); }}
                        />
                        In rotation
                      </label>
                    </header>
                    <div class="claude-acct-windows">
                      <WindowMeter label="5-hour" w={a.usage?.fiveHour ?? null} view={v()} />
                      <WindowMeter label="Weekly" w={a.usage?.sevenDay ?? null} view={v()} />
                    </div>
                    <div class="claude-acct-meta">
                      <Show when={a.usage}>{u => <span title={u().observedAt}>Usage {sourceLabel(u().source)}</span>}</Show>
                      <Show when={a.lastUsedAt}>{t => <span title={t()}>Last run {new Date(t()).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}</span>}</Show>
                      <Show when={a.usage?.overageDisabledReason}>{r => <span>Extra usage off: {r()}</span>}</Show>
                    </div>
                    <Show when={a.note}>{n => <p class="claude-acct-note">{n()}</p>}</Show>
                  </li>
                )}
              </For>
              {/* In use: no added account (or none can run now), or a claude.ai connector set to Always pins chats to it. */}
              <Show when={v().accounts.length === 0 || s.pin()}>
                <li class="claude-acct" data-kind="login" data-status={loginStatus(v())}>
                  <header class="claude-acct-head">
                    <span class="claude-acct-name">Claude Code's login</span>
                    <Show when={v().accounts.length > 0}><span class="claude-acct-next">{loginRowBadge(v())}</span></Show>
                    <Show when={loginStatus(v())}>{st => <span class="claude-acct-status" data-status={st()}>{STATUS_LABELS[st()]}</span>}</Show>
                    <Show when={s.pin()}>
                      <span class="claude-acct-spacer" />
                      <button type="button" class="btn" onClick={openConnectors}>Open Connectors</button>
                    </Show>
                  </header>
                  <p class="claude-acct-meta">{s.pin()?.login ?? 'The account Claude Code is signed in to on this Mac (claude auth login).'}</p>
                  <div class="claude-acct-windows">
                    <WindowMeter label="5-hour" w={v().loginUsage?.fiveHour ?? null} view={v()} />
                    <WindowMeter label="Weekly" w={v().loginUsage?.sevenDay ?? null} view={v()} />
                  </div>
                  <Show when={!v().loginUsage}><p class="claude-acct-meta">Usage not seen yet: it shows after this login's next run.</p></Show>
                  <Show when={s.pin()}>{p => <p class="claude-acct-note">{loginPinNote(p(), v().failover)}</p>}</Show>
                </li>
              </Show>
            </ul>
            <Show when={v().accounts.length === 0}>
              <p class="settings-note">Add more Claude plans in <button type="button" class="btn-link" onClick={openAccounts}>Agents, Accounts</button> to spread usage across them.</p>
            </Show>
          </>
        )}
      </Show>
      {/* Paperclip PR 8c: the live reading, only once asked for. */}
      <Show when={quotaOptIn()}><ClaudeQuotaCard /></Show>
    </section>
  );
}

/** Spread usage, on Limits below the meters (moved from Agents, Accounts). */
export function SpreadUsage(props: { s: ClaudeLimitsState }) {
  const s = props.s;
  const openConnectors = () => requestOpenSettingsSection('connectors', 'Your connectors');
  function checkUsage() {
    void s.run(() => trackTask('Check Claude account usage', () => spreadUsage.checkUsage(api)), 'Could not check usage');
  }
  return (
    <section class="usage-block claude-acct-routing" aria-label="Automatic account rotation">
      <h3 class="usage-block-title">Automatic account rotation</h3>
      <Show when={s.view()}>
        {v => (
          <>
            <p class="claude-acct-summary">{routingSummary(v(), s.pin())}</p>
            <div class="claude-acct-controls">
              <Show when={v().routing === 'spread'}>
                <label class="claude-acct-field" title="Accounts at or over this share of either window are skipped while another has room.">
                  <span>Move on at</span>
                  <Select label="Move on at" value={String(v().ceilingPercent)} disabled={s.busy()} searchable={false} onChange={value => void s.run(() => spreadUsage.moveOnAt(api, Number(value)), 'Could not change the ceiling')}
                    options={ceilingChoices(v().ceilingPercent).map(c => ({ value: String(c), label: `${c}%` }))} />
                </label>
              </Show>
              <button class="btn" disabled={s.busy()} onClick={checkUsage} title="Check every account's 5-hour and weekly usage now.">
                Check usage
              </button>
            </div>
            <p class="claude-acct-note">Hollow refreshes stale usage and selects the account with the most room across its 5-hour and weekly limits. A quota rejection overrides the meter until reset. Use In rotation above to exclude an account.</p>
            <LeadText class="claude-acct-summary" text={failoverSummary(v())} />
            {/* Turn guards (2026-10-09): with Spread usage, a running chat
                also moves at the ceiling, at a safe point, once per turn. */}
            <Show when={v().routing === 'spread' && v().failover}>
              <p class="claude-acct-note">A running chat also moves to another account once its account passes {v().ceilingPercent}%.</p>
            </Show>
            <Show when={v().failover && v().accounts.length > 0 && s.pin()}>
              <p class="claude-acct-note" data-kind="step-aside">{STEP_ASIDE_NOTE}</p>
            </Show>
            <Show when={v().accounts.length > 0 && s.loginLine()}>
              {line => <p class="claude-acct-note" data-kind="login-override">{line()} <button type="button" class="btn-link" onClick={openConnectors}>Change it in Connectors</button></p>}
            </Show>
          </>
        )}
      </Show>
    </section>
  );
}
