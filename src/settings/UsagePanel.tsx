/**
 * Settings, Usage (Paperclip PR 6; in detail since Settings PR 8): how much
 * each agent was used, how close each limit is, and how runs share Claude
 * accounts. Usage opens on Limits (every agent's windows, budgets as meters,
 * the spending caps, Spread usage); then All agents and one tab per agent
 * with runs in the period, each with Totals, By day, By model, By account,
 * By where and Outcomes; then Budgets, the full list. The period and Refresh
 * sit above the tabs and apply to all of them.
 *
 * Reads the usage ledger through `usage_summary` (filtered by agent on an
 * agent tab). Runs whose totals are incomplete, unusable, never started or
 * still running are counted apart from metered runs, and a missing cost is
 * shown as a dash, never "$0".
 */
import { For, Match, Show, Switch, createEffect, createResource, createSignal } from 'solid-js';
import { api } from '../api';
import type { UsageWindow } from '../types';
import { BudgetsPanel } from '../components/BudgetsPanel';
import { ValuePanel } from './ValuePanel';
import { CfoUsageRows } from './CfoSettings';
import { UsageLimits } from './UsageLimits';
import { UsageAgentTab } from './UsageAgentTab';
import { figuresFor } from './usageDetail';
import { USAGE_PERIODS, agentTabLabel, agentTabs, requestUsageTab, usageTabRequest, type UsageTab } from './usageTabs';
import '../styles/usage-panel.css';
import '../styles/usage-limits.css';
import { PlainErrorText } from '../components/ui';
import { errMsg } from '../utils';

export function UsagePanel() {
  const [tab, setTab] = createSignal<UsageTab>(usageTabRequest() ?? 'limits');
  // A Settings jump to a control on another tab (usageTabFor) switches here.
  createEffect(() => {
    const wanted = usageTabRequest();
    if (wanted) { setTab(wanted); requestUsageTab(null); }
  });
  const [period, setPeriod] = createSignal<UsageWindow>('day');
  const agent = () => { const t = tab(); return t.startsWith('agent:') ? t.slice('agent:'.length) : null; };
  const [summary, { refetch }] = createResource(
    () => ({ window: period(), agent: agent() }),
    q => api.usageSummary(q.window, q.agent),
  );
  // `latest`: a refetch (another period or tab) keeps the last figures on
  // screen instead of suspending the Settings page.
  const latest = () => (summary.error ? undefined : summary.latest);
  // The figures under the open tab: never the previous agent's while this one loads.
  const figures = () => figuresFor(latest(), agent());
  const tabs = () => [
    { id: 'limits' as UsageTab, label: 'Limits' },
    { id: 'all' as UsageTab, label: 'All agents' },
    ...agentTabs(latest()?.agents).map(a => ({ id: `agent:${a.key}` as UsageTab, label: a.label })),
    // The open agent tab stays while its agent has no runs in a new period.
    ...(agent() && !agentTabs(latest()?.agents).some(a => a.key === agent()) ? [{ id: tab(), label: agentTabLabel(agent()!) }] : []),
    { id: 'budgets' as UsageTab, label: 'Budgets' },
  ];

  return (
    <div class="usage-panel" aria-busy={summary.loading}>
      <div class="usage-toolbar">
        <div class="seg-toggle" role="tablist" aria-label="Usage period">
          <For each={USAGE_PERIODS}>
            {p => (
              <button
                class={`seg-toggle-btn ${period() === p.id ? 'active' : ''}`}
                role="tab"
                aria-selected={period() === p.id}
                onClick={() => setPeriod(p.id)}
              >
                {p.label}
              </button>
            )}
          </For>
        </div>
        <button class="btn" onClick={() => void refetch()} disabled={summary.loading}>Refresh</button>
      </div>

      {/* Never a jump target: a jump to "Budgets" lands on its meters. */}
      <div class="usage-tabs" role="tablist" aria-label="Usage view" data-jump-skip>
        <For each={tabs()}>
          {t => (
            <button
              type="button"
              class="usage-tab"
              classList={{ active: tab() === t.id }}
              role="tab"
              aria-selected={tab() === t.id}
              data-tab={t.id}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          )}
        </For>
      </div>

      <Switch>
        <Match when={tab() === 'limits'}>
          <UsageLimits onAllBudgets={() => setTab('budgets')} />
        </Match>
        <Match when={tab() === 'budgets'}>
          {/* Usage budgets (Paperclip PR 8a): hard stops pause automations. */}
          <BudgetsPanel caps={false} />
          {/* The CFO's limits (COO rebuild L7): the reserve, each account's room, extra usage. */}
          <CfoUsageRows />
        </Match>
        <Match when={true}>
          <Show when={summary.error}>
            <div class="settings-banner danger" role="alert">
              <p>Usage could not be read: <PlainErrorText text={errMsg(summary.error)} /></p>
            </div>
          </Show>
          <Show when={figures()} fallback={<Show when={summary.loading && !summary.error}><p class="usage-empty" role="status">Loading usage</p></Show>}>{s => <UsageAgentTab s={s()} />}</Show>
          <Show when={tab() === 'all' ? figures() : undefined}>
            {s => (
              <>
                <p class="usage-explainer">
                  Claude Code reports a cost; on a subscription login it is an estimate at API list
                  prices, not a bill, so it is shown as “estimated”. Codex reports tokens but no price:
                  with a row for its model under System, Advanced, Prices, its spend is estimated at the price you entered.
                  A self-hosted model is billed by its endpoint's GPU time, not per token: its runs have their own
                  row with tokens and no price. Antigravity reports tokens per call and no price: its spend is estimated at Google's published Gemini rates (Prices), and the prepaid credit balance is only in AI Studio (ai.studio/projects). Tokens count metered and incomplete runs only. A
                  hard-stop budget (Budgets tab) pauses automations when it is reached; chat only warns.
                </p>
                <p class="usage-footer">
                  <Show when={s().ledgerSince} fallback={<>Nothing recorded yet.</>}>
                    {since => <>Recording since {new Date(since()).toLocaleDateString()}.</>}
                  </Show>
                </p>
                <Show when={s().ledgerError}>
                  {error => (
                    <div class="settings-banner" role="status">
                      <p>Some runs could not be recorded: {error()}</p>
                    </div>
                  )}
                </Show>
                {/* AI value ledger v1 (master plan #6): cost against estimated hours saved. */}
                <ValuePanel />
              </>
            )}
          </Show>
        </Match>
      </Switch>
    </div>
  );
}
