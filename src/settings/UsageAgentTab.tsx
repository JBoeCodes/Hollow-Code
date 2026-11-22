/**
 * Settings, Usage, one agent tab or All agents (Settings PR 8): Totals, By
 * day, By model, By account, By where (workspace, surface, automation and
 * the ten costliest conversations, each opening its chat) and Outcomes.
 * Every list is from the same ledger rows as Totals, so they sum to it.
 */
import { For, Show, Suspense, lazy } from 'solid-js';
import type { UsageBucket, UsageSummary } from '../types';
import { SURFACE_LABELS, bucketCost, formatTokens } from '../usageFormat';
import { agentNote, allTokens, averageCost, cacheShare, costKind, notMetered } from './usageDetail';
import { openSessionByKey } from '../store/workspaces';

const UsageInstruments = lazy(() => import('../components/UsageInstruments'));

export function tokenLine(b: UsageBucket): string {
  return `${formatTokens(b.inputTokens)} in · ${formatTokens(b.cachedInputTokens)} cached · ${formatTokens(b.outputTokens)} out`;
}

export function UsageTable(props: { title: string; rows: UsageBucket[]; label?: (b: UsageBucket) => string; open?: (b: UsageBucket) => void }) {
  return (
    <div class="usage-block">
      <h3 class="usage-block-title">{props.title}</h3>
      <Show when={props.rows.length} fallback={<p class="usage-empty">No runs in this period.</p>}>
        <table class="usage-table">
          <thead>
            <tr>
              <th scope="col">{props.title.replace(/^By /, '')}</th>
              <th scope="col" class="num">Runs</th>
              <th scope="col" class="num">Tokens</th>
              <th scope="col" class="num">Cost</th>
            </tr>
          </thead>
          <tbody>
            <For each={props.rows}>
              {row => (
                <tr>
                  <th scope="row">
                    <Show when={props.open} fallback={props.label ? props.label(row) : row.label}>
                      {open => <button type="button" class="btn-link" onClick={() => open()(row)}>{props.label ? props.label(row) : row.label}</button>}
                    </Show>
                  </th>
                  <td class="num">{row.runs}</td>
                  <td class="num" title={tokenLine(row)}>{formatTokens(allTokens(row))}</td>
                  <td class="num">{bucketCost(row)}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </Show>
    </div>
  );
}

function Outcomes(props: { rows: UsageBucket[] }) {
  return (
    <div class="usage-block">
      <h3 class="usage-block-title">Outcomes</h3>
      <Show when={props.rows.length} fallback={<p class="usage-empty">No runs in this period.</p>}>
        <ul class="usage-outcomes">
          <For each={props.rows}>{o => <li data-outcome={o.key}><strong>{o.runs}</strong> {o.label}</li>}</For>
        </ul>
      </Show>
    </div>
  );
}

function Totals(props: { s: UsageSummary }) {
  const t = () => props.s.total;
  const kind = () => costKind(t());
  return (
    <div class="usage-block usage-totals">
      <div class="usage-total">
        <div class="usage-total-runs"><strong>{t().runs}</strong> run{t().runs === 1 ? '' : 's'}</div>
        <div class="usage-total-tokens">{tokenLine(t())}</div>
        <div class="usage-total-cost">{bucketCost(t())}</div>
      </div>
      <p class="usage-counts">
        {[
          cacheShare(t()) ? `${cacheShare(t())} of input from cache` : '',
          kind() === 'reported' ? 'Cost as the agent reported it' : kind() === 'estimated' ? 'Cost estimated at list prices' : kind() === 'both' ? 'Reported and estimated costs kept apart' : '',
          averageCost(t()) ? `${averageCost(t())} a run on average` : '',
        ].filter(Boolean).join(' · ')}
      </p>
      <Show when={notMetered(t()).length}>
        <p class="usage-counts">{notMetered(t()).join(' · ')}</p>
      </Show>
      <Show when={agentNote(props.s.agent)}>{note => <p class="usage-explainer">{note()}</p>}</Show>
      <Show when={props.s.partialRuns && props.s.partialFloorUsd}>
        <p class="usage-counts">At least ${props.s.partialFloorUsd!.toFixed(2)} more in {props.s.partialRuns} incomplete runs.</p>
      </Show>
    </div>
  );
}

export function UsageAgentTab(props: { s: UsageSummary }) {
  const agent = () => props.s.agent ?? null;
  const showsAccounts = () => !agent() || agent() === 'claude' || agent() === 'codex';
  return (
    <div class="usage-agent-tab">
      <Totals s={props.s} />
      <Show when={agent() !== 'antigravity'}>
        <Suspense><UsageInstruments summary={props.s} /></Suspense>
      </Show>
      <Show when={!agent()}><UsageTable title="By agent" rows={props.s.byAgent} /></Show>
      <UsageTable title="By model" rows={props.s.byModel ?? []} />
      <Show when={showsAccounts()}><UsageTable title="By account" rows={props.s.byAccount} /></Show>
      <h3 class="usage-block-title usage-where-title">By where</h3>
      <UsageTable title="By workspace" rows={props.s.byProject} />
      <UsageTable title="By surface" rows={props.s.bySurface} label={b => SURFACE_LABELS[b.key] ?? b.label} />
      <Show when={props.s.byWorkflow.length}><UsageTable title="By automation" rows={props.s.byWorkflow} /></Show>
      <Show when={(props.s.topConversations ?? []).length}>
        <UsageTable title="Costliest conversations" rows={props.s.topConversations ?? []} open={b => void openSessionByKey(b.key)} />
      </Show>
      <Show when={props.s.bySubagent.length}><UsageTable title="By subagent" rows={props.s.bySubagent} /></Show>
      <Outcomes rows={props.s.outcomes ?? []} />
    </div>
  );
}
