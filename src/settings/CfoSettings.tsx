/**
 * The CFO's Settings rows (COO rebuild L7, docs/CFO.md). In Agents: Effort
 * (Automatic or Fixed; Fixed shows each agent's saved level) and Models the
 * CFO may use (each installed agent's real models as checkboxes, never free
 * text; none checked means every installed agent on its saved model). In
 * Usage: Reserve for your own prompts, each account's room for unattended
 * work, Extra usage for unattended work, the Budget for each subagent and
 * Project budgets (L9). Each saves on change.
 */
import { For, Show, createResource, createSignal, type JSX } from 'solid-js';
import { api } from '../api';
import { AGENT_LABELS } from '../store';
import { Select } from '../components/Select';
import { Button, PlainErrorText } from '../components/ui';
import { SettingsSheet, openerRef } from './SettingsSheet';
import { errMsg } from '../utils';
import { accountRoomLine, allowRows, allowSummary, formatTokens, parseTokenAmount, setAny, setModel, tokenInput } from '../cfo';
import { workspaces } from '../store/workspaces';
import { isMainAgent, isQuickChats } from '../workspaceTypes';
import type { CfoPatch, CfoProjectBudget, CfoSettings, CfoView } from '../generated/ipc';
import { flashSaved } from './settingsDirty';

/** "5-hour 42%, weekly 10%": the windows that have a reading. */
function windowsLine(account: { fiveHourPercent: number | null; weeklyPercent: number | null }): string {
  return [
    account.fiveHourPercent !== null ? `5-hour ${Math.round(account.fiveHourPercent)}%` : '',
    account.weeklyPercent !== null ? `weekly ${Math.round(account.weeklyPercent)}%` : '',
  ].filter(Boolean).join(', ');
}

const agentName = (agent: string) => AGENT_LABELS[agent as keyof typeof AGENT_LABELS] ?? agent;

// One copy for the Effort and Models rows, so a save in one shows in both.
const [cfo, setCfo] = createSignal<CfoSettings | null>(null);
const [cfoError, setCfoError] = createSignal('');
let loading: Promise<void> | null = null;
function loadCfo() {
  loading ??= api.cfoSettings().then(settings => { setCfo(settings); }, error => { setCfoError(errMsg(error)); }).finally(() => { loading = null; });
  return loading;
}

async function saveCfo(patch: CfoPatch, what: string): Promise<CfoView | null> {
  setCfoError('');
  try {
    const view = await api.cfoSaveSettings(patch);
    setCfo(view.settings);
    flashSaved(what);
    return view;
  } catch (error) {
    setCfoError(`Could not save: ${errMsg(error)}`);
    return null;
  }
}

/** Effort: Automatic (the CFO picks per job) or Fixed (`children`: each
 *  agent's saved level, shown only then). */
export function CfoEffortRow(props: { children: JSX.Element }) {
  void loadCfo();
  const mode = () => cfo()?.effortMode ?? 'automatic';
  return (
    <>
      <div class="settings-row">
        <div class="settings-row-label">
          <label for="cfo-effort-mode" class="settings-row-k">Effort</label>
          <div class="settings-row-v">{mode() === 'automatic'
            ? 'Automatic, or a fixed level per agent. A chat’s own effort wins over both.'
            : 'Fixed: every turn uses its agent’s saved level below. A chat’s own effort wins.'}</div>
        </div>
        <Select id="cfo-effort-mode" label="Effort" value={mode()} disabled={!cfo()} searchable={false}
          onChange={value => { const current = cfo(); if (current && value !== current.effortMode) void saveCfo({ effortMode: value as CfoSettings['effortMode'] }, 'Effort'); }}
          options={[{ value: 'automatic', label: 'Automatic', detail: 'The CFO picks per job' }, { value: 'fixed', label: 'Fixed', detail: 'Each agent’s saved level' }]} />
      </div>
      <Show when={mode() === 'fixed'}>{props.children}</Show>
      <Show when={cfoError()}><p class="settings-row-warn" role="alert"><PlainErrorText text={cfoError()} plain /></p></Show>
    </>
  );
}

/** Models the CFO may use, for subagents and unattended work: one row with
 *  what it allows now; Choose opens the checkboxes in a sheet (Settings S5b,
 *  plan 3.1 rule 8), each saving as you change it. */
export function CfoAllowRow(props: { installed: string[] }) {
  void loadCfo();
  // Each installed agent's real models (the list the model picker shows).
  const [catalogs] = createResource(() => props.installed.join(','), async () => {
    const entries = await Promise.all(props.installed.map(async agent => {
      try { return [agent, (await api.agentModels(agent)).models] as const; }
      catch { return [agent, []] as const; }
    }));
    return Object.fromEntries(entries) as Record<string, { id: string; label: string }[]>;
  });
  const allow = () => cfo()?.allow ?? [];
  const rows = () => allowRows(allow(), props.installed);
  const label = (agent: string, model: string) => catalogs()?.[agent]?.find(m => m.id === model)?.label ?? model;
  const save = (next: CfoSettings['allow']) => { if (cfo()) void saveCfo({ allow: next }, 'Models the CFO may use'); };
  const [open, setOpen] = createSignal(false);
  const opener = openerRef('[data-sheet-opener="cfo-allow"]');
  return (
    <div class="settings-row">
      <div class="settings-row-label">
        <div class="settings-row-k">Models the CFO may use</div>
        <div class="settings-row-v">Now: {allowSummary(allow(), agentName, label)}</div>
      </div>
      <Button size="sm" data-sheet-opener="cfo-allow" aria-label="Choose the models the CFO may use" disabled={!cfo()} onClick={e => { opener.mark(e); setOpen(true); }}>Choose…</Button>
      <Show when={open()}>
        <SettingsSheet id="settings-sheet-cfo-allow" title="Models the CFO may use" returnTo={opener.get} onClose={() => setOpen(false)}>
          <p class="settings-note">For subagents and unattended work; your own chats use what you pick. None checked means every installed agent on its saved model. Each box saves at once.</p>
          <div class="cfo-allow" role="group" aria-label="Models the CFO may use">
            <For each={rows()} fallback={<div class="settings-row-v">No agent is installed.</div>}>{row => (
              <fieldset class="cfo-allow-agent" disabled={!cfo()}>
                <legend class="settings-row-v">{agentName(row.agent)}</legend>
                <label class="settings-watchdog-off">
                  <input type="checkbox" checked={row.any} onChange={e => save(setAny(allow(), row.agent, e.currentTarget.checked))} />
                  Any model of this agent
                </label>
                <For each={catalogs()?.[row.agent] ?? []}>{model => (
                  <label class="settings-watchdog-off">
                    <input type="checkbox" checked={row.any || row.models.includes(model.id)} disabled={row.any}
                      onChange={e => save(setModel(allow(), row.agent, model.id, e.currentTarget.checked, (catalogs()?.[row.agent] ?? []).map(m => m.id)))} />
                    {model.label}
                  </label>
                )}</For>
              </fieldset>
            )}</For>
          </div>
          <Show when={cfoError()}><p class="settings-row-warn" role="alert"><PlainErrorText text={cfoError()} plain /></p></Show>
        </SettingsSheet>
      </Show>
    </div>
  );
}

/** Settings, Usage, Limits: the reserve, each account's room, extra usage, and the subagent and project budgets (L9). */
export function CfoUsageRows() {
  const [view, { mutate, refetch }] = createResource(() => api.cfoView());
  const [error, setError] = createSignal('');
  async function act(run: () => Promise<CfoView>, what: string): Promise<boolean> {
    setError('');
    try { mutate(await run()); flashSaved(what); return true; }
    catch (e) { setError(`Could not save: ${errMsg(e)}`); return false; }
  }
  const extraUntil = () => {
    const until = view()?.settings.extraUsageUntil;
    return until && view()?.extraUsageOpen ? new Date(until).toLocaleString([], { weekday: 'short', hour: 'numeric', minute: '2-digit' }) : null;
  };
  // Subagent budget drafts (text) and the project add drafts.
  const [subTokens, setSubTokens] = createSignal<string | null>(null);
  const [subMinutes, setSubMinutes] = createSignal<string | null>(null);
  const [addId, setAddId] = createSignal('');
  const [addFive, setAddFive] = createSignal('');
  const [addWeek, setAddWeek] = createSignal('');
  const title = (id: string) => {
    const w = workspaces().find(x => x.id === id);
    return w ? (isMainAgent(w) ? 'COO' : w.title) : id;
  };
  const candidates = (v: CfoView) => workspaces().filter(w => !isQuickChats(w) && !v.settings.projectBudgets.some(b => b.workspaceId === w.id));
  async function saveSubagent(v: CfoView) {
    const tokens = parseTokenAmount(subTokens() ?? tokenInput(v.settings.subagentBudget.tokens));
    const minutesText = (subMinutes() ?? String(v.settings.subagentBudget.minutes ?? '')).trim();
    const minutes = minutesText === '' ? null : /^\d+$/.test(minutesText) ? Number(minutesText) : 'invalid';
    if (tokens === 'invalid') { setError('Could not save: tokens are a whole number, or a number with k or M (2M, 500k).'); return; }
    if (minutes === 'invalid' || (minutes !== null && (minutes < 1 || minutes > 10080))) { setError('Could not save: minutes are a whole number from 1 to 10080.'); return; }
    // The typed text stays when the save fails (L9 review).
    if (await act(() => api.cfoSaveSettings({ subagentBudget: { ...(tokens !== null ? { tokens } : {}), ...(minutes !== null ? { minutes } : {}) } }), 'Budget for each subagent')) {
      setSubTokens(null); setSubMinutes(null);
    }
  }
  /** One project's caps from two typed fields; null (with the error shown) when they are not valid. */
  function caps(five: string, week: string): Pick<CfoProjectBudget, 'fiveHourTokens' | 'weekTokens'> | null {
    const f = parseTokenAmount(five), w = parseTokenAmount(week);
    if (f === 'invalid' || w === 'invalid') { setError('Could not save: a cap is a whole number, or a number with k or M (2M, 500k).'); return null; }
    if (f === null && w === null) { setError('Could not save: give the 5-hour cap, the 7-day cap, or both.'); return null; }
    return { ...(f !== null ? { fiveHourTokens: f } : {}), ...(w !== null ? { weekTokens: w } : {}) };
  }
  const saveBudgets = (next: CfoProjectBudget[]) => act(() => api.cfoSaveSettings({ projectBudgets: next }), 'Project budgets');
  return (
    <div class="usage-block cfo-usage">
      <h3 class="usage-block-title">Unattended work</h3>
      <Show when={view.error}><p class="settings-row-warn" role="alert">The CFO's limits could not be read: <PlainErrorText text={errMsg(view.error)} /> <button class="btn small" onClick={() => void refetch()}>Retry</button></p></Show>
      <Show when={view()}>{v => (
        <>
          <div class="settings-row">
            <div class="settings-row-label">
              <label for="cfo-reserve" class="settings-row-k">Reserve for your own prompts</label>
              <div class="settings-row-v">Room kept in each account’s windows for your prompts. Other work starts below {v().poolCeilingPercent}%.</div>
            </div>
            <Select id="cfo-reserve" label="Reserve for your own prompts" value={String(v().settings.reservePercent)} searchable={false}
              onChange={value => void act(() => api.cfoSaveSettings({ reservePercent: Number(value) }), 'Reserve for your own prompts')}
              options={(v().reserveChoices.includes(v().settings.reservePercent) ? v().reserveChoices : [...v().reserveChoices, v().settings.reservePercent].sort((a, b) => a - b))
                .map(percent => ({ value: String(percent), label: `${percent}%` }))} />
          </div>
          <For each={v().accounts}>{account => (
            <div class="settings-row">
              <div class="settings-row-label">
                <div class="settings-row-k">{account.label}</div>
                <div class="settings-row-v">{[accountRoomLine(account.room, account.roomAt, account.note), windowsLine(account)].filter(Boolean).join(' · ')}</div>
              </div>
            </div>
          )}</For>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Extra usage for unattended work</div>
              <div class="settings-row-v">{extraUntil() ? `Approved until ${extraUntil()}: unattended work may run past an account's included usage.` : 'Off: unattended work stops at included usage and waits for its reset.'}</div>
            </div>
            <div class="settings-actions">
              <button class="btn" onClick={() => void act(() => api.cfoApproveExtraUsage(4), 'Extra usage for unattended work')}>Approve for 4 h</button>
              <button class="btn" onClick={() => void act(() => api.cfoApproveExtraUsage(24), 'Extra usage for unattended work')}>Approve for 24 h</button>
              <Show when={extraUntil()}><button class="btn ghost" onClick={() => void act(() => api.cfoApproveExtraUsage(0), 'Extra usage for unattended work')}>Withdraw</button></Show>
            </div>
          </div>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Budget for each subagent</div>
              <div class="settings-row-v">Each subagent’s cap on tokens and minutes. Reaching it pauses it. Blank: no cap.</div>
            </div>
            <div class="settings-actions">
              <input class="settings-input cfo-budget-input" type="text" inputmode="text" placeholder="Tokens" aria-label="Subagent budget, tokens"
                value={subTokens() ?? tokenInput(v().settings.subagentBudget.tokens)}
                onInput={e => setSubTokens(e.currentTarget.value)} />
              <input class="settings-input cfo-budget-input" type="text" inputmode="numeric" placeholder="Minutes" aria-label="Subagent budget, minutes"
                value={subMinutes() ?? String(v().settings.subagentBudget.minutes ?? '')}
                onInput={e => setSubMinutes(e.currentTarget.value)} />
              <button class="btn" aria-label="Save budget for each subagent" onClick={() => void saveSubagent(v())}>Save</button>
            </div>
          </div>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Project budgets</div>
              <div class="settings-row-v">Tokens a project’s agents may use over 5 hours and 7 days. Your prompts still run.</div>
              <For each={v().settings.projectBudgets}>{budget => {
                const usage = () => v().projectUsage.find(u => u.workspaceId === budget.workspaceId);
                const line = () => [
                  budget.fiveHourTokens !== undefined ? `${formatTokens(usage()?.fiveHourTokens ?? 0)} of ${formatTokens(budget.fiveHourTokens)} in the last 5 hours` : '',
                  budget.weekTokens !== undefined ? `${formatTokens(usage()?.weekTokens ?? 0)} of ${formatTokens(budget.weekTokens)} in the last 7 days` : '',
                ].filter(Boolean).join(' · ');
                let five!: HTMLInputElement, week!: HTMLInputElement;
                const change = () => {
                  const next = caps(five.value, week.value);
                  if (!next) { five.value = tokenInput(budget.fiveHourTokens); week.value = tokenInput(budget.weekTokens); return; }
                  void saveBudgets(v().settings.projectBudgets.map(b => b.workspaceId === budget.workspaceId ? { workspaceId: b.workspaceId, ...next } : b));
                };
                return (
                  <div class="cfo-budget" role="group" aria-label={`Budget for ${title(budget.workspaceId)}`}>
                    <div class="settings-row-k">{title(budget.workspaceId)}</div>
                    <div class="settings-actions">
                      <input ref={five} class="settings-input cfo-budget-input" type="text" placeholder="5-hour tokens" aria-label={`${title(budget.workspaceId)}, 5-hour tokens`}
                        value={tokenInput(budget.fiveHourTokens)} onChange={change} />
                      <input ref={week} class="settings-input cfo-budget-input" type="text" placeholder="7-day tokens" aria-label={`${title(budget.workspaceId)}, 7-day tokens`}
                        value={tokenInput(budget.weekTokens)} onChange={change} />
                      <button class="btn ghost" aria-label={`Remove the budget for ${title(budget.workspaceId)}`}
                        onClick={() => void saveBudgets(v().settings.projectBudgets.filter(b => b.workspaceId !== budget.workspaceId))}>Remove</button>
                    </div>
                    <div class="settings-row-v">{line()}</div>
                    <Show when={usage()?.reached}><div class="settings-row-warn">Reached: its unattended work waits</div></Show>
                  </div>
                );
              }}</For>
              <div class="settings-actions" role="group" aria-label="Add a project budget">
                <Select id="cfo-budget-project" label="Project to budget" value={addId()} searchable={candidates(v()).length > 8}
                  onChange={value => setAddId(value)}
                  options={candidates(v()).map(w => ({ value: w.id, label: isMainAgent(w) ? 'COO' : w.title }))} />
                <input class="settings-input cfo-budget-input" type="text" placeholder="5-hour tokens" aria-label="New budget, 5-hour tokens"
                  value={addFive()} onInput={e => setAddFive(e.currentTarget.value)} />
                <input class="settings-input cfo-budget-input" type="text" placeholder="7-day tokens" aria-label="New budget, 7-day tokens"
                  value={addWeek()} onInput={e => setAddWeek(e.currentTarget.value)} />
                <button class="btn" aria-label="Add project budget" disabled={!addId()} onClick={() => {
                  const next = caps(addFive(), addWeek());
                  if (!next) return;
                  void saveBudgets([...v().settings.projectBudgets, { workspaceId: addId(), ...next }]).then(saved => { if (saved) { setAddId(''); setAddFive(''); setAddWeek(''); } });
                }}>Add</button>
              </div>
            </div>
          </div>
        </>
      )}</Show>
      <Show when={error()}><p class="settings-row-warn" role="alert"><PlainErrorText text={error()} plain /></p></Show>
    </div>
  );
}
