import { For, Show, createSignal, onMount } from 'solid-js';
import { api } from '../api';
import { capNote, draftFromView, toSettings } from '../automationWindows';
import { builtInCaps, parseCapInput, MAX_CAP_USD, type BuiltInCap, type CapState } from '../builtInCaps';
import type { MainAgentSettings } from '../mainAgent';
import type { AutomationWindows } from '../generated/ipc';
import { errMsg } from '../utils';
import { ErrorText } from './ui';

const STATE_WORDS: Record<CapState, string> = { ok: 'Under the cap', warn: 'Near the cap', stopped: 'Stopped at the cap', unknown: 'Spend not read' };

/** Settings, Usage, Budgets' fixed rows (P1-07): the caps Hollow keeps for
 *  you, each with its spend, limit and state. The Main agent hard stop and the
 *  overnight cap are edited here through their own setters (this is their one
 * editor; Automation links here). */
export function BuiltInCaps() {
  const [settings, setSettings] = createSignal<MainAgentSettings | null>(null);
  const [mainSpent, setMainSpent] = createSignal<number | null>(null);
  const [windows, setWindows] = createSignal<AutomationWindows | null>(null);
  const [lastNight, setLastNight] = createSignal<{ night: string; spentUsd: number; capUsd: number; unknownCosts: number; gpuTime: number } | null>(null);
  const [editing, setEditing] = createSignal<BuiltInCap['id'] | null>(null);
  const [value, setValue] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);

  onMount(() => {
    api.getMainAgentSettings().then(setSettings, () => {});
    api.mainAgentStatus().then(s => setMainSpent(s ? s.spentTodayUsd : null), () => {});
    api.automationWindows().then(setWindows, () => {});
  });

  const rows = () => builtInCaps({
    main: settings() ? { spentUsd: mainSpent(), capUsd: settings()!.dailyCapUsd } : null,
    overnightCapUsd: windows()?.overnightCapUsd ?? null,
  });

  let listEl: HTMLUListElement | undefined;
  /** Close the editor and put focus back on the row's Change limit. */
  const close = (id: BuiltInCap['id']) => {
    setEditing(null);
    queueMicrotask(() => listEl?.querySelector<HTMLElement>(`[data-cap="${id}"] .budgets-row-actions button`)?.focus());
  };
  const edit = (row: BuiltInCap) => { setEditing(row.id); setValue(row.limitUsd == null ? '' : String(row.limitUsd)); setError(null); };

  async function save(row: BuiltInCap) {
    if (saving()) return;
    const n = parseCapInput(value());
    if (n == null) { setError(`Type a dollar amount from 0 to ${MAX_CAP_USD}.`); return; }
    setSaving(true);
    try {
      if (row.id === 'main-agent') {
        const cur = settings() ?? await api.getMainAgentSettings();
        setSettings(await api.setMainAgentSettings({ ...cur, dailyCapUsd: n }));
      } else if (row.id === 'overnight') {
        const view = windows() ?? await api.automationWindows();
        setWindows(await api.setAutomationWindows(toSettings({ ...draftFromView(view), cap: String(n) })));
      }
      close(row.id);
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <ul class="budgets-list budgets-fixed" aria-label="Caps Hollow keeps for you" ref={listEl}>
      <For each={rows()}>
        {row => (
          <li class="budgets-fixed-row" data-cap={row.id} data-state={row.state}>
            <div class="budgets-row-head">
              <span class="budgets-scope settings-row-k">{row.label}</span>
              <span class="budgets-window">{row.limitUsd == null ? 'Limit not read yet' : `${row.id === 'overnight' ? 'Current limit' : 'Limit'} $${row.limitUsd.toFixed(row.limitUsd % 1 ? 2 : 0)}`}</span>
              <span class="budgets-state">{STATE_WORDS[row.state]}</span>
            </div>
            <p class="budgets-counts">{row.spentWords}</p>
            <Show when={row.meterPercent != null} fallback={row.costIncomplete ? <p class="budgets-counts">Cost incomplete</p> : undefined}>
              <div class="meter" role="meter" aria-label={row.label}
                aria-valuemin={0} aria-valuemax={100} aria-valuenow={row.meterPercent ?? undefined} aria-valuetext={row.spentWords}>
                <div class="meter-track"><span class="meter-fill" style={{ '--pct': (row.meterPercent ?? 0) / 100 }} /></div>
              </div>
            </Show>
            <p class="budgets-note">{row.covers}{row.id === 'overnight' && row.limitUsd != null ? ` ${capNote(String(row.limitUsd))}` : ''}</p>
            <Show when={row.editable}>
              <Show when={editing() === row.id} fallback={
                <div class="budgets-row-actions"><button type="button" class="btn small" onClick={() => edit(row)}>Change limit</button></div>
              }>
                <form class="budgets-resume" onSubmit={e => { e.preventDefault(); void save(row); }}>
                  <label class="budgets-field">
                    <span>{row.label} (USD)</span>
                    <input class="settings-input" type="text" inputmode="decimal" value={value()} aria-invalid={!!error()}
                      aria-describedby={error() ? `cap-error-${row.id}` : undefined} ref={el => queueMicrotask(() => el.focus())}
                      onInput={e => { setValue(e.currentTarget.value); setError(null); }} />
                  </label>
                  <div class="budgets-form-actions">
                    <button type="submit" class="btn small primary" aria-disabled={saving()}>{saving() ? 'Saving…' : 'Save'}</button>
                    <button type="button" class="btn small ghost" onClick={() => { close(row.id); setError(null); }}>Cancel</button>
                  </div>
                  <Show when={error()}>{msg => <div id={`cap-error-${row.id}`}><ErrorText class="budgets-warn" text={msg()} /></div>}</Show>
                </form>
              </Show>
            </Show>
          </li>
        )}
      </For>
    </ul>
  );
}
