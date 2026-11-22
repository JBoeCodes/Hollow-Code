/**
 * Settings, Usage, Value (master plan #6): this month's cost against the
 * hours Hollow saved, by department and work kind, and the minutes-saved
 * baseline per work kind. Only the person sets baselines (`set_baseline` is
 * never an agent action). Hours saved count only accepted runs (an accepted
 * Result, or the run's tracker issue reached Done after it started; G4),
 * stay hidden until one is accepted, and are always labelled estimates; a
 * run with no cost is "unknown", never $0.
 */
import { For, Show, createResource } from 'solid-js';
import { api } from '../api';
import { flashFooter } from '../store';
import { WORK_KINDS, baselineSetOn, costLine, hoursShown, savedCell, savedLine, type WorkKind } from '../valueLedger';
import { unlessErrored } from '../stableResource';
import { ResourceReadError } from '../components/ResourceReadError';

export function ValuePanel() {
  const [summaryRead, { refetch: refetchSummary }] = createResource(() => api.valueSummary());
  const [baselineRead, { mutate, refetch: refetchBaselines }] = createResource(() => api.getValueBaselines());
  const summary = () => unlessErrored(summaryRead);
  const baselines = () => unlessErrored(baselineRead);
  const anyBaseline = () => Object.keys(baselines()?.baselines ?? {}).length > 0;
  const save = async (workKind: WorkKind, raw: string) => {
    const minutes = raw.trim() === '' ? null : Number(raw);
    if (minutes !== null && !Number.isFinite(minutes)) return;
    try {
      mutate(await api.setValueBaseline({ workKind, minutes }));
      refetchSummary();
    } catch (e) {
      flashFooter(`× ${e instanceof Error ? e.message : String(e)}`, 4000);
    }
  };
  return (
    <div class="usage-block">
      <h3 class="usage-block-title">Value this month</h3>
      <ResourceReadError label="value figures" read={summaryRead} retry={refetchSummary} />
      <Show when={summary()} fallback={<Show when={!summaryRead.error}><p class="usage-empty">No value figures yet.</p></Show>}>
        {s => (
          <>
            <p class="usage-total">
              <span>{s().totals.runs} runs</span>
              <span>{costLine(s().totals)}</span>
              <Show when={savedLine(s().totals, anyBaseline())}>{line => <span>{line()}</span>}</Show>
            </p>
            <Show when={s().groups.length}>
              <table class="usage-table">
                <thead>
                  <tr>
                    <th scope="col">Department</th>
                    <th scope="col">Work kind</th>
                    <th scope="col" class="num">Runs</th>
                    <th scope="col" class="num">Cost</th>
                    <Show when={hoursShown(s().totals)}><th scope="col" class="num">Saved (estimate)</th></Show>
                  </tr>
                </thead>
                <tbody>
                  <For each={s().groups}>
                    {g => (
                      <tr>
                        <th scope="row">{g.key.department ?? '—'}</th>
                        <td>{g.key.workKind ?? '—'}</td>
                        <td class="num">{g.totals.runs}</td>
                        <td class="num">{costLine(g.totals)}</td>
                        <Show when={hoursShown(s().totals)}><td class="num">{savedCell(g.totals)}</td></Show>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </Show>
          </>
        )}
      </Show>
      <h4 class="usage-block-title">Minutes saved per accepted run</h4>
      <ResourceReadError label="value baselines" read={baselineRead} retry={refetchBaselines} />
      <p class="usage-explainer">
        Your estimate of how long each kind of work takes you by hand. Nothing is filled in for you:
        until you set one, that work shows "baseline not set", never 0 hours. Only accepted runs count:
        a Result you accepted, or a run whose tracker issue reached Done after it started (each run
        once). Hours saved stay hidden until a run is accepted. Agents can't change these; leave a box
        empty to clear it.
      </p>
      <Show when={baselines()}><table class="usage-table">
        <tbody>
          <For each={WORK_KINDS}>
            {kind => (
              <tr>
                <th scope="row">{kind}</th>
                <td class="num">
                  <input class="settings-input" type="number" min="0" max="4800" step="5"
                    aria-label={`Minutes saved per accepted ${kind} run`} style={{ width: '5rem' }} placeholder="not set"
                    value={baselines()?.baselines[kind]?.minutes ?? ''}
                    onChange={e => save(kind, e.currentTarget.value)} />
                </td>
                <td class="value-set-on">{baselineSetOn(baselines()?.baselines[kind])}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table></Show>
    </div>
  );
}
