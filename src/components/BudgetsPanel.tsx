/**
 * Settings, Usage budgets (Paperclip PR 8a, gate D2).
 *
 * Lists every budget with this window's usage, its state and whether it can
 * actually be enforced, and edits them. Above them, the caps Hollow keeps for
 * you (BuiltInCaps, P1-07): this is their one editor. Hollow ships with no
 * budgets of your own; "Suggest
 * a limit" appears only after a week of ledger data and only prefills the
 * form. A hard-stop budget pauses automations when it is reached (chat only
 * warns); "Resume…" raises its limit above this window's usage.
 *
 * Reads `budgetsView` (a store resource behind preserveDuringRefetch, so a
 * `budget-changed` refetch never blanks the Settings page). Backend refusals
 * are plain strings (Tauri), read through errMsg and shown inline.
 */
import { For, Show, createSignal, onMount } from 'solid-js';
import { api } from '../api';
import {
  budgetsView, enableBudgetsResource, enableWorkflowsResource, refetchBudgets, refetchWorkflows,
  showConfirm, trackTask, workflowsList,
} from '../store';
import type { BudgetInput, BudgetMetric, BudgetStatus, BudgetWindow } from '../types';
import {
  ENFORCEMENT_COPY, allowedMetrics, budgetOfLimit, budgetStateLabel, canResume, enforceabilityLabel,
  formatBudgetAmount, suggestionPrefill, windowLabel,
} from '../budgets';
import { errMsg } from '../utils';
import '../styles/budgets.css';
import { Select } from './Select';
import { BudgetGauge } from './BudgetGauge';
import { BuiltInCaps } from './BuiltInCaps';
import { ErrorText, PlainErrorText } from './ui';

interface Draft {
  id: string | null;
  scope: string;
  metric: BudgetMetric;
  window: BudgetWindow;
  limit: string;
  warnPercent: string;
  hardStop: boolean;
}

const EMPTY: Draft = { id: null, scope: 'global', metric: 'tokens', window: 'day', limit: '', warnPercent: '80', hardStop: true };

const FIXED_SCOPES: { value: string; label: string }[] = [
  { value: 'global', label: 'All usage' },
  { value: 'heartbeat', label: 'The heartbeat' },
  { value: 'agent:claude', label: 'Claude runs' },
  { value: 'agent:codex', label: 'Codex runs' },
  { value: 'agent:antigravity', label: 'Antigravity runs' },
  { value: 'agent:grok', label: 'Grok runs' },
];

function draftOf(status: BudgetStatus): Draft {
  return {
    id: status.id,
    scope: status.scope,
    metric: status.metric,
    window: status.window,
    limit: String(status.limit),
    warnPercent: String(status.warnPercent),
    hardStop: status.hardStop,
  };
}

/** `caps: false` (Usage, Budgets tab, Settings PR 8): the caps are edited on
 *  Limits instead, so this list leaves them out. */
export function BudgetsPanel(props: { caps?: boolean } = {}) {
  const [draft, setDraft] = createSignal<Draft | null>(null);
  const [formError, setFormError] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);
  const [resumeFor, setResumeFor] = createSignal<string | null>(null);
  const [resumeValue, setResumeValue] = createSignal('');
  const [resumeError, setResumeError] = createSignal<string | null>(null);

  onMount(() => {
    enableBudgetsResource();
    enableWorkflowsResource();
  });

  const scopes = () => [
    ...FIXED_SCOPES,
    ...(workflowsList() ?? []).map(w => ({ value: `workflow:${w.slug}`, label: `Automation: ${w.title || w.slug}` })),
  ];

  function update<K extends keyof Draft>(key: K, value: Draft[K]) {
    setDraft(previous => {
      if (!previous) return previous;
      const next = { ...previous, [key]: value };
      // A scope that cannot report the chosen metric switches to one it can.
      const allowed = allowedMetrics(next.scope);
      if (allowed.length && !allowed.includes(next.metric)) next.metric = allowed[0];
      return next;
    });
    setFormError(null);
  }

  async function save() {
    const d = draft();
    if (!d || saving()) return;
    const limit = Number(d.limit);
    const warnPercent = Number(d.warnPercent);
    if (!Number.isFinite(limit) || limit <= 0) {
      setFormError('Enter a limit above zero.');
      return;
    }
    if (!Number.isInteger(warnPercent) || warnPercent < 1 || warnPercent > 100) {
      setFormError('The warning threshold is a whole percent from 1 to 100.');
      return;
    }
    const input: BudgetInput = { id: d.id, scope: d.scope, metric: d.metric, window: d.window, limit, warnPercent, hardStop: d.hardStop };
    setSaving(true);
    try {
      await trackTask(d.id ? 'Save budget' : 'Add budget', () => api.saveBudget(input));
      setDraft(null);
      await refetchBudgets();
      void refetchWorkflows();
    } catch (error) {
      setFormError(errMsg(error));
    } finally {
      setSaving(false);
    }
  }

  async function remove(status: BudgetStatus) {
    const ok = await showConfirm({
      title: `Delete the ${windowLabel(status.window).toLowerCase()} budget for ${status.scopeLabel}?`,
      body: status.pause ? 'Its pause ends and the automations under it can run again.' : 'Usage keeps being recorded; only the limit goes.',
      confirmLabel: 'Delete budget',
      danger: true,
    });
    if (!ok) return;
    try {
      await trackTask('Delete budget', () => api.deleteBudget(status.id));
      await refetchBudgets();
      void refetchWorkflows();
    } catch {
      /* trackTask surfaced it */
    }
  }

  function openResume(status: BudgetStatus) {
    setResumeFor(status.id);
    const prefill = suggestionPrefill(status.observed, status.suggestedLimit);
    setResumeValue(prefill == null ? '' : String(prefill));
    setResumeError(null);
  }

  async function resume(status: BudgetStatus) {
    const value = Number(resumeValue());
    if (!canResume(value, status.observed)) {
      setResumeError(`Enter a limit above this ${status.window}'s usage (${formatBudgetAmount(status.metric, status.observed, status.estimated)}).`);
      return;
    }
    try {
      await trackTask('Resume budget', () => api.resumeBudget(status.id, value));
      setResumeFor(null);
      await refetchBudgets();
      void refetchWorkflows();
    } catch (error) {
      setResumeError(errMsg(error));
    }
  }

  return (
    <section class="budgets-panel" aria-label="Usage budgets">
      <header class="budgets-head">
        <h3 class="budgets-title">Budgets</h3>
        <button class="btn" onClick={() => { setDraft({ ...EMPTY }); setFormError(null); }} disabled={!!budgetsView()?.storeError}>
          Add budget
        </button>
      </header>
      <p class="budgets-note">{budgetsView()?.enforcement || ENFORCEMENT_COPY}</p>

      <Show when={budgetsView()?.storeError}>
        {error => (
          <div class="settings-banner danger" role="alert">
            <p>Automations are held: <PlainErrorText text={error()} /></p>
          </div>
        )}
      </Show>
      <Show when={budgetsView()?.foreignEdit}>
        <div class="settings-banner" role="status">
          <p>The budget settings file was changed outside Hollow. Check the budgets below.</p>
        </div>
      </Show>

      <Show when={draft()}>
        {d => (
          <form class="budgets-form" onSubmit={event => { event.preventDefault(); void save(); }}>
            <label class="budgets-field">
              <span>Applies to</span>
              <Select label="Applies to" value={d().scope} onChange={v => update('scope', v)} options={scopes().map(scope => ({ value: scope.value, label: scope.label }))} />
            </label>
            <label class="budgets-field">
              <span>Measure</span>
              <Select label="Measure" value={d().metric} searchable={false} onChange={v => update('metric', v as BudgetMetric)}
                options={[{ value: 'tokens', label: 'Tokens', disabled: !allowedMetrics(d().scope).includes('tokens') }, { value: 'costUsd', label: 'Spending (USD)', disabled: !allowedMetrics(d().scope).includes('costUsd') }]} />
            </label>
            <label class="budgets-field">
              <span>Period</span>
              <Select label="Period" value={d().window} searchable={false} onChange={v => update('window', v as BudgetWindow)}
                options={[{ value: 'day', label: 'Day' }, { value: 'week', label: 'Week' }, { value: 'month', label: 'Month' }]} />
            </label>
            <label class="budgets-field">
              <span>Limit</span>
              <input type="number" min="0" step="any" inputMode="decimal" value={d().limit} onInput={event => update('limit', event.currentTarget.value)} />
            </label>
            <label class="budgets-field">
              <span>Warn at (%)</span>
              <input type="number" min="1" max="100" step="1" value={d().warnPercent} onInput={event => update('warnPercent', event.currentTarget.value)} />
            </label>
            <label class="budgets-check">
              <input type="checkbox" checked={d().hardStop} onChange={event => update('hardStop', event.currentTarget.checked)} />
              <span>Hard stop: pause automations at the limit</span>
            </label>
            <Show when={allowedMetrics(d().scope).length === 0}>
              <p class="settings-error">No budget can be set on this scope: its runs report no usage Hollow can count.</p>
            </Show>
            <Show when={formError()}>{error => <ErrorText class="settings-error" text={error()} />}</Show>
            <div class="budgets-form-actions">
              <Show when={budgetsView()?.suggestionsAvailable && d().id}>
                <button
                  type="button"
                  class="btn ghost"
                  onClick={() => {
                    const current = budgetsView()?.budgets.find(b => b.id === d().id);
                    if (current?.suggestedLimit != null) update('limit', String(current.suggestedLimit));
                  }}
                >Suggest a limit</button>
              </Show>
              <button type="button" class="btn ghost" onClick={() => setDraft(null)}>Cancel</button>
              <button type="submit" class="btn primary" disabled={saving() || allowedMetrics(d().scope).length === 0}>
                {saving() ? 'Saving…' : 'Save budget'}
              </button>
            </div>
          </form>
        )}
      </Show>

      <Show when={props.caps !== false}><BuiltInCaps /></Show>

      <Show
        when={(budgetsView()?.budgets ?? []).length}
        fallback={
          <p class="budgets-empty">
            No budgets of your own.
            <Show when={!budgetsView()?.suggestionsAvailable}>
              {' '}Suggestions appear after a week of usage data ({budgetsView()?.ledgerDays ?? 0} of 7 days so far).
            </Show>
          </p>
        }
      >
        <ul class="budgets-list">
          <For each={budgetsView()?.budgets ?? []}>
            {status => (
              <li class="budgets-row" data-state={status.state}>
                <BudgetGauge percent={status.percent} unavailable={!!status.ledgerError} />
                <div class="budgets-row-head">
                  <span class="budgets-scope">{status.scopeLabel}</span>
                  <span class="budgets-window">{windowLabel(status.window)}</span>
                  <span class="settings-state-chip budgets-chip" data-state={status.state}>
                    {budgetStateLabel(status.state, status.pause?.held)}
                  </span>
                  <Show when={!status.hardStop}><span class="budgets-soft">Warn only</span></Show>
                </div>
                <div class="budgets-row-meta">
                  <span>{budgetOfLimit(status)}</span>
                  <Show when={status.percent != null}><span>{Math.round(status.percent!)}%</span></Show>
                  <Show when={status.cachedInputTokens > 0 && status.metric === 'tokens'}>
                    <span title="Cache reads are shown but never counted">{formatBudgetAmount('tokens', status.cachedInputTokens)} cached (not counted)</span>
                  </Show>
                </div>
                <Show when={enforceabilityLabel(status.enforceability)}>
                  {label => <p class="budgets-warn">{label()}: {status.enforceabilityReason}</p>}
                </Show>
                <Show when={status.scopeMissing}>
                  <p class="budgets-warn">This automation no longer exists.</p>
                </Show>
                <Show when={status.ledgerError}>
                  {error => <p class="settings-error">Usage could not be read: <PlainErrorText text={error()} /></p>}
                </Show>
                <Show when={status.unknownRuns + status.partialRuns + status.unpricedRuns + (status.gpuTimeRuns ?? 0) > 0}>
                  <p class="budgets-counts">
                    {[
                      status.unknownRuns ? `${status.unknownRuns} with no usage figures` : '',
                      status.partialRuns ? `${status.partialRuns} partial` : '',
                      status.unpricedRuns ? `${status.unpricedRuns} unpriced` : '',
                      status.gpuTimeRuns ? `${status.gpuTimeRuns} billed by GPU time` : '',
                    ].filter(Boolean).join(' · ')}
                  </p>
                </Show>
                <div class="budgets-row-actions">
                  {/* A hard-stop budget over its limit holds automations even
                      before a claim has opened its pause card, so it offers
                      Resume… too. */}
                  <Show when={status.pause || (status.hardStop && status.state === 'over')}>
                    <button class="btn sm" onClick={() => openResume(status)}>Resume…</button>
                  </Show>
                  <button class="btn ghost sm" onClick={() => { setDraft(draftOf(status)); setFormError(null); }}>Edit</button>
                  <button class="btn ghost sm" onClick={() => void remove(status)}>Delete</button>
                </div>
                <Show when={resumeFor() === status.id}>
                  <div class="budgets-resume">
                    <label class="budgets-field">
                      <span>New limit</span>
                      <input
                        type="number"
                        min="0"
                        step="any"
                        inputMode="decimal"
                        aria-label="New limit"
                        value={resumeValue()}
                        onInput={event => { setResumeValue(event.currentTarget.value); setResumeError(null); }}
                      />
                    </label>
                    <button
                      class="btn primary sm"
                      disabled={!canResume(Number(resumeValue()), status.observed)}
                      onClick={() => void resume(status)}
                    >Raise and resume</button>
                    <button class="btn ghost sm" onClick={() => setResumeFor(null)}>Cancel</button>
                    <Show when={resumeError()}>{error => <ErrorText class="settings-error" text={error()} />}</Show>
                  </div>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </section>
  );
}
