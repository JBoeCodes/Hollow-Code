// Shared "What happened" head: normalized facts in all receipt hosts. The
// automation host supplies its own cost/retry model; Chat supplies its one
// retry controller. Changed paths live only in the receipt disclosure.
import { For, Show, createMemo, createUniqueId, type JSX } from 'solid-js';
import { runOutcome, unknownLabel, type RunOutcome as RunOutcomeData, type RunReceiptFacts } from '../runOutcome';
import { Button, StatusPill } from './ui';

export interface RunOutcomeProps {
  outcome?: RunOutcomeData;
  receipt?: RunReceiptFacts;
  /** "Latest run · 3h ago · failed". */
  when?: string;
  running?: boolean;
  /** Run again; `retryOf` set for the guarded retry. */
  onRetry?: (retryOf: string | null) => void;
  /** Chat supplies its one existing retry controller and provider menu. */
  retryActions?: JSX.Element;
  /** Open what it changed in Vault History (its first changed file). */
  onOpenHistory?: () => void;
}

const OUTCOME_TONE: Record<string, string> = { done: 'ok', failed: 'off', uncertain: 'warn' };

export function RunOutcome(props: RunOutcomeProps) {
  const id = createUniqueId();
  // No automation retry inference for Chat or Activity. Those hosts own
  // their actions; verification failure is not a failed provider turn.
  const o = createMemo(() => props.outcome ?? runOutcome({ status: null, receipt: props.receipt ?? null, cost: null }));
  return (
    <section class="workflow-outcome" aria-labelledby={`${id}-outcome`} data-changed={o().changedAnything} data-retry={o().retry.kind}>
      <header class="workflow-outcome-head">
        <h3 id={`${id}-outcome`}>What happened</h3>
        <span class="workflow-outcome-when">{props.when}</span>
      </header>
      <dl class="workflow-outcome-grid">
        <div class="workflow-outcome-line">
          <dt>Changed</dt>
          <dd>{o().changed}</dd>
        </div>
        <div class="workflow-outcome-line">
          <dt>Checks</dt>
          <dd><StatusPill quiet label={o().checks.label} tone={o().checks.tone} hint={o().checks.hint} /></dd>
        </div>
        <Show when={props.outcome}><div class="workflow-outcome-line">
          <dt>Cost</dt>
          <dd>{o().cost}</dd>
        </div></Show>
        <div class="workflow-outcome-line">
          <dt>Outside its files</dt>
          <dd>
            <Show when={o().effects.length || o().moreEffects} fallback={<span class="dim">{o().outsideRecorded ? 'Nothing recorded.' : 'Not recorded for this run.'}</span>}>
              <ul class="workflow-outcome-list" aria-label="Recorded outside effects">
                <For each={o().effects}>
                  {e => (
                    <li class="workflow-outcome-effect" data-outcome={e.outcome}>
                      <span>{e.label}</span>
                      <StatusPill quiet label={e.outcomeLabel} tone={OUTCOME_TONE[e.outcome] ?? 'neutral'} />
                      <Show when={props.onRetry && e.guarded && e.outcome !== 'failed'}>
                        <span class="workflow-outcome-guard" title="A guarded retry refuses this exact call">won’t repeat</span>
                      </Show>
                    </li>
                  )}
                </For>
                <Show when={o().moreEffects > 0}><li class="dim">{o().moreEffects} more recorded</li></Show>
              </ul>
            </Show>
          </dd>
        </div>
          <div class="workflow-outcome-line">
            <dt>Not visible to Hollow</dt>
            <dd>
              <Show when={o().unknown.length} fallback={<span class="dim">{o().outsideRecorded ? 'No unobserved calls recorded.' : 'Coverage not recorded.'}</span>}><ul class="workflow-outcome-list" aria-label="Calls Hollow can’t see into">
                <For each={o().unknown}>
                  {u => (
                    <li>
                      {u.count} {unknownLabel(u.label, u.count)}
                      <Show when={u.examples.length}><span class="workflow-outcome-examples mono">{u.examples.join(' · ')}</span></Show>
                    </li>
                  )}
                </For>
              </ul></Show>
            </dd>
          </div>
      </dl>
      <p class="workflow-outcome-coverage">{o().coverage}</p>
      {props.retryActions}
      <Show when={props.outcome && o().retry.kind !== 'none'}>
        <div class="workflow-outcome-retry" role="group" aria-label="Running it again">
          <p>{o().retry.detail}</p>
          <div class="workflow-outcome-actions">
            <Show when={o().retry.label}>
              {label => (
                <Button size="sm" disabled={props.running || !props.onRetry} onClick={() => props.onRetry?.(o().retry.retryOf)}>{label()}</Button>
              )}
            </Show>
            <Show when={o().retry.kind === 'currentFiles' && props.onOpenHistory}>
              <Button size="sm" variant="ghost" onClick={() => props.onOpenHistory!()}>Open in History</Button>
            </Show>
          </div>
        </div>
      </Show>
    </section>
  );
}
