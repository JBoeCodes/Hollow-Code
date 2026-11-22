/**
 * Flight recorder (master plan #15): a finished turn's steps (the F6 step
 * spine) as a timeline beside what the run saw (its context manifest). The
 * first failed step and the steps that fed it are marked; every step offers
 * Retry from this step, which starts a NEW conversation seeded to that step
 * (never a replay) and puts files back through History first when needed,
 * refusing with nothing changed when that can't be exact.
 */
import { For, Show, createResource, createSignal } from 'solid-js';
import { api } from '../api';
import { flashFooter } from '../store';
import { openSessionByKey } from '../store/workspaces';
import { stepStatus, timelineRows, type StepsView } from '../flightRecorder';
import { errMsg } from '../utils';
import { runCostSuffix } from '../runOutcome';

export function FlightRecorder(props: { runId: string }) {
  const [view] = createResource(() => props.runId, (runId) => api.runSteps(runId));
  const [context] = createResource(() => props.runId, async (runId) => {
    try { return await api.runContextManifest(runId); } catch { return null; }
  });
  const [busy, setBusy] = createSignal<number | null>(null);
  const retry = async (step: number) => {
    setBusy(step);
    try {
      const started = await api.retryRunFromStep(props.runId, step);
      flashFooter(started.filesPutBack > 0
        ? `Retry started; ${started.filesPutBack} file(s) put back to step ${step} (undoable in History)`
        : 'Retry started in a new conversation', 4000);
      await openSessionByKey(started.sessionKey);
    } catch (e) {
      flashFooter(`× ${errMsg(e)}`, 6000);
    } finally {
      setBusy(null);
    }
  };
  const rows = () => (view() ? timelineRows(view() as StepsView) : []);
  return (
    <div class="flight-recorder">
      <Show when={view()} fallback={<p class="toolcall-extra">{view.error ? `Timeline unavailable: ${errMsg(view.error)}` : 'Loading the timeline…'}</p>}>
        {(v) => (
          <>
            <p class="toolcall-extra">
              {v().steps.length} steps
              {v().accounts.length ? ` · account ${v().accounts.join(' → ')}` : ''}
              {runCostSuffix(context()?.costRollup)}
              {' · per-step cost not recorded'}
            </p>
            <Show when={v().steps.length === 0}>
              <p class="toolcall-extra">No steps were recorded for this turn (it ran before the step spine).</p>
            </Show>
            <ol class="flight-recorder-steps" aria-label="Steps">
              <For each={rows()}>
                {(row) => {
                  const st = stepStatus(row.step.status);
                  return (
                    <li
                      class="flight-recorder-step"
                      classList={{ 'is-first-error': row.firstError, 'fed-the-error': row.fedTheError, 'is-bad': st.bad }}
                      style={{ 'padding-inline-start': `${row.depth * 1.25}rem` }}
                    >
                      <span class="toolcall-verb">{row.step.seq}. {row.label}</span>
                      <span class="toolcall-extra"> · {st.text} · {row.duration}</span>
                      <Show when={row.firstError}><span class="toolcall-extra"> · first error</span></Show>
                      <Show when={row.fedTheError}><span class="toolcall-extra"> · led to the error</span></Show>
                      <Show when={row.step.checkpoint?.result}>
                        <pre class="toolcall-result" classList={{ error: st.bad }}><code>{row.step.checkpoint!.result}</code></pre>
                      </Show>
                      <Show when={!v().live}>
                        <button type="button" class="btn small" disabled={busy() !== null}
                          aria-label={`Retry from after step ${row.step.seq}`}
                          onClick={() => retry(row.step.seq)}>Retry from this step</button>
                      </Show>
                    </li>
                  );
                }}
              </For>
            </ol>
            <Show when={!v().live}>
              <button type="button" class="btn small" disabled={busy() !== null} onClick={() => retry(0)}>Retry from the start</button>
            </Show>
          </>
        )}
      </Show>
      <Show when={context()}>
        {(c) => (
          <details class="flight-recorder-context">
            <summary>What it saw ({c().manifest.layers.length} context layers)</summary>
            <ul class="execution-receipt-list">
              <For each={c().manifest.layers}>
                {(layer) => <li class="execution-receipt-row"><span class="toolcall-verb">{layer.name}</span><span class="toolcall-extra mono"> · {layer.bytes} bytes</span></li>}
              </For>
              <For each={c().manifest.fallbacks}>
                {(f) => <li class="execution-receipt-row"><span class="toolcall-verb">Fallback · {f.stage}</span><span class="toolcall-extra"> · {f.detail}</span></li>}
              </For>
            </ul>
          </details>
        )}
      </Show>
    </div>
  );
}
