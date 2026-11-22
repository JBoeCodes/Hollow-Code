/**
 * Settings, Usage, Limits (Settings PR 8): everything that can stop or slow
 * a run, as meters. Claude per account and its login, Codex's plan windows,
 * the agents that report no plan limits, daily and weekly budgets (monthly
 * below) with their warn and hard-stop marks, and the two spending caps,
 * edited in place. Then Spread usage.
 */
import { For, Show, onMount } from 'solid-js';
import { budgetsView, enableBudgetsResource } from '../store';
import { budgetPeriodWords, limitBudgets, type BudgetMeterRow } from './budgetMeters';
import { NO_PLAN_LIMITS } from './usageDetail';
import { createClaudeLimitsState } from './claudeLimitsState';
import { ClaudeMeters, SpreadUsage } from './ClaudeLimits';
import { ProviderQuotaCard } from './ProviderQuotaCard';
import { UsageMeter } from './UsageMeter';
import { BuiltInCaps } from '../components/BuiltInCaps';
import '../styles/budgets.css';

function BudgetRow(props: { row: BudgetMeterRow }) {
  const marks = () => [
    { at: props.row.warnAt, kind: 'warn' as const, title: `Warns at ${Math.round(props.row.warnAt * 100)}%` },
    ...(props.row.hardStop ? [{ at: 1, kind: 'stop' as const, title: 'Hard stop: automations pause here' }] : []),
  ];
  return (
    <UsageMeter
      label={`${props.row.label}, ${budgetPeriodWords(props.row.window).toLowerCase()}`}
      fill={props.row.fill}
      level={props.row.level}
      value={props.row.percent == null ? 'Not read' : `${props.row.percent}%`}
      meta={`${props.row.amount}${props.row.hardStop ? '' : ' · warn only'}`}
      marks={marks()}
    />
  );
}

/** Daily and weekly budgets as meters, monthly ones below. */
export function BudgetMeters(props: { onAll: () => void }) {
  onMount(enableBudgetsResource);
  const groups = () => limitBudgets(budgetsView()?.budgets ?? []);
  return (
    <section class="usage-block usage-limits-budgets" aria-label="Budgets">
      <header class="usage-limits-head">
        <h3 class="usage-block-title">Budgets</h3>
        <button type="button" class="btn-link" onClick={() => props.onAll()}>All budgets</button>
      </header>
      <Show when={groups().soon.length + groups().monthly.length}
        fallback={<p class="usage-empty">No budgets of your own. Add one on the Budgets tab.</p>}>
        <For each={groups().soon}>{row => <BudgetRow row={row} />}</For>
        <Show when={groups().monthly.length}>
          <h4 class="usage-limits-sub">Monthly</h4>
          <For each={groups().monthly}>{row => <BudgetRow row={row} />}</For>
        </Show>
      </Show>
    </section>
  );
}

export function UsageLimits(props: { onAllBudgets: () => void }) {
  const claude = createClaudeLimitsState();
  onMount(claude.load);
  return (
    <div class="usage-limits">
      <ClaudeMeters s={claude} />
      <ProviderQuotaCard />
      <section class="usage-block" aria-label="Agents without plan limits">
        <For each={NO_PLAN_LIMITS}>
          {name => <p class="usage-limits-none"><strong>{name}</strong> Doesn't report plan limits.</p>}
        </For>
      </section>
      <BudgetMeters onAll={props.onAllBudgets} />
      <section class="usage-block" aria-label="Spending caps">
        <h3 class="usage-block-title">Spending caps</h3>
        <BuiltInCaps />
      </section>
      <SpreadUsage s={claude} />
    </div>
  );
}
