import { For, Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import { api } from '../../api';
import { ErrorText, Skeleton } from '../../components/ui';
import { errMsg } from '../../utils';
import { openSettingsFromOutside } from '../../store';
import { rememberWindowLabels } from '../../windowLabels';
import type { Workflow } from '../../types';
import { scheduleWindow } from '../../workflowSchedule';
import { guardNavigation, registerNavigationGuard } from '../../store/navigationGuard';
import { showConfirm } from '../../store/overlays';
import { requestOpenWorkflow } from '../../store/runtime';
import { resetAutomationsView, setAutomationWindowsOpen } from '../../store/automationsViews';
import { setNewAutomationWindow } from './newRequest';
import type { AutomationWindows } from '../../generated/ipc';
import {
  BUILT_IN_USES, DAY_KEYS, DAY_NAMES, createLoadGate, draftFromView, draftProblems, hasProblems, isDirty, newWindow,
  placeBackendError, rebaseDraft, toSettings, toggleDay, windowSummary, windowUseCount, withTitle,
  type DraftProblems, type WindowDraft, type WindowField, type WindowsDraft,
} from '../../automationWindows';
import '../../styles/automation-windows.css';

const NO_PROBLEMS: DraftProblems = { windows: {} };

/**
 * Automations → Automation windows (overnight Spare S10; moved from Settings in PR 4, 11-12): the named
 * windows (`schedule_windows.rs`) and the overnight cap, edited as a draft and
 * saved with the person's own `set_automation_windows` (user-only). Loads when
 * its fold opens (never at launch). A refetch (opening, window focus) only
 * replaces the draft while it has no unsaved edits, and a refused save keeps
 * them, with the refusal next to the control it names. Edits live only here:
 * leaving the page asks first (a navigation guard).
 */
export function AutomationWindowsPanel() {
  const [base, setBase] = createSignal<WindowsDraft | null>(null);
  const [draft, setDraft] = createSignal<WindowsDraft | null>(null);
  const [zone, setZone] = createSignal('');
  const [loadError, setLoadError] = createSignal('');
  const [problems, setProblems] = createSignal<DraftProblems>(NO_PROBLEMS);
  const [saving, setSaving] = createSignal(false);
  const [note, setNote] = createSignal('');
  const dirty = createMemo(() => { const d = draft(); return !!d && isDirty(d, base()); });
  // Leaving the Automations page asks before dropping these changes (it was
  // Settings' section-switch question before PR 4).
  onCleanup(registerNavigationGuard(() => !dirty() || showConfirm({
    title: 'Discard unsaved automation window changes?',
    body: 'These window edits have not been saved.',
    confirmLabel: 'Discard', cancelLabel: 'Keep editing', danger: true,
  })));
  // Automations' schedules, read when the panel opens, for "N automations use
  // this window" and the confirm before removing a used one.
  const [workflows, setWorkflows] = createSignal<Workflow[] | null>(null);
  const [workflowError, setWorkflowError] = createSignal('');
  let listRead = 0;
  onCleanup(() => { listRead++; });
  async function loadWorkflows() {
    const ticket = ++listRead;
    try {
      const rows = await api.listWorkflows();
      if (ticket === listRead) { setWorkflows(rows); setWorkflowError(''); }
    } catch (error) { if (ticket === listRead) setWorkflowError(errMsg(error)); }
  }
  const linked = (name: string) => (workflows() ?? []).filter(w => scheduleWindow(w.schedule) === name);
  const [confirming, setConfirming] = createSignal<string | null>(null);
  const uses = (name: string) => windowUseCount((workflows() ?? []).map(w => w.schedule), name);
  function openAutomation(name: string, slug?: string) {
    if (dirty() || saving()) return;
    // Already on Automations: no page change, just the editor (an unsaved
    // automation draft is asked about by the guard). The fold closes so the
    // editor has the page: in a short window (480 px, large text) the open
    // fold left the editor's split one pixel tall (2026-11-15).
    void guardNavigation(() => {
      setAutomationWindowsOpen(false);
      resetAutomationsView();
      if (slug) requestOpenWorkflow(slug);
      else setNewAutomationWindow(name);
    });
  }
  const gate = createLoadGate();
  let list: HTMLOListElement | undefined;
  let panel: HTMLDivElement | undefined;
  let addButton: HTMLButtonElement | undefined;

  function adopt(view: AutomationWindows, replaceDraft: boolean) {
    rememberWindowLabels(view.windows);
    const next = draftFromView(view);
    setZone(view.timeZone);
    setBase(next);
    if (replaceDraft) setDraft(next);
  }

  async function load() {
    const ticket = gate.load();
    try {
      const view = await api.automationWindows();
      // A read that started before the last save may predate it.
      if (!gate.accepts(ticket)) return;
      setLoadError('');
      // Edits in progress win over a refetch.
      adopt(view, !dirty() && !saving());
    } catch (e) {
      if (gate.accepts(ticket) && !draft()) setLoadError(errMsg(e));
    }
  }

  onMount(() => {
    void load();
    void loadWorkflows();
    const onFocus = () => { if (!dirty()) void load(); };
    window.addEventListener('focus', onFocus);
    onCleanup(() => window.removeEventListener('focus', onFocus));
  });

  /** Apply an edit and clear only the errors it can have fixed (`clears`). */
  function edit(key: string, change: (w: WindowDraft) => WindowDraft, clears: WindowField[]) {
    setDraft(d => d && { ...d, windows: d.windows.map(w => (w.key === key ? change(w) : w)) });
    setNote('');
    if (!clears.length) return;
    setProblems(p => {
      const row = p.windows[key];
      if (!row && !p.general) return p;
      const rest = { ...row };
      for (const f of clears) delete rest[f];
      return { ...p, general: undefined, windows: { ...p.windows, [key]: rest } };
    });
  }

  const focusTitle = (key: string) => list?.querySelector<HTMLInputElement>(`[data-key="${key}"] input[data-field="title"]`);

  function addWindow() {
    const d = draft();
    if (!d) return;
    const w = newWindow(d.windows);
    setDraft({ ...d, windows: [...d.windows, w] });
    setNote('');
    queueMicrotask(() => focusTitle(w.key)?.select());
  }

  /** Remove; a window automations use asks first, inline. */
  function askRemove(w: WindowDraft) {
    if ((uses(w.name) > 0 || !workflows() || workflowError()) && confirming() !== w.key) {
      setConfirming(w.key);
      queueMicrotask(() => list?.querySelector<HTMLButtonElement>(`[data-key="${w.key}"] .aw-confirm button`)?.focus());
      return;
    }
    removeWindow(w);
  }

  function removeWindow(w: WindowDraft) {
    const rows = draft()?.windows ?? [];
    const next = rows[rows.findIndex(x => x.key === w.key) + 1];
    setConfirming(null);
    setDraft(d => d && { ...d, windows: d.windows.filter(x => x.key !== w.key) });
    setNote(w.saved ? `${w.title || w.name} is removed when you save.` : '');
    setProblems(p => ({ ...p, windows: { ...p.windows, [w.key]: {} } }));
    // Focus goes on, not to the top of the page: the next row, else Add.
    queueMicrotask(() => (next ? focusTitle(next.key) : addButton)?.focus());
  }

  function discard() {
    setDraft(base());
    setProblems(NO_PROBLEMS);
    setNote('');
  }

  async function save() {
    const d = draft();
    if (!d || saving()) return;
    const found = draftProblems(d);
    if (hasProblems(found)) {
      setProblems(found);
      setNote('Nothing saved yet: fix the marked fields.');
      queueMicrotask(() => panel?.querySelector<HTMLElement>('[aria-invalid="true"], .aw-days[data-invalid] > button')?.focus());
      return;
    }
    setSaving(true);
    setNote('');
    gate.save();
    try {
      const view = await api.setAutomationWindows(toSettings(d));
      gate.save();
      // Edits typed while the save ran stay; what it saved becomes the base.
      const typedSince = draft() !== d;
      adopt(view, !typedSince);
      if (typedSince) setDraft(cur => cur && rebaseDraft(cur, draftFromView(view)));
      setProblems(NO_PROBLEMS);
      setNote(`Saved. Windows run in ${view.timeZone}.`);
    } catch (e) {
      // The draft stays as typed; the refusal goes next to what it names.
      gate.save();
      setProblems(placeBackendError(errMsg(e), d));
      setNote('Not saved: your changes are still here.');
    } finally {
      setSaving(false);
    }
  }

  const errId = (key: string, field: string) => `aw-${key.replace(/[^a-z0-9-]/gi, '-')}-${field}-error`;
  const fieldError = (key: string, field: WindowField) => problems().windows[key]?.[field];

  return (
    <div class="aw-panel" ref={panel}>
      <Show when={draft()} fallback={
        <Show when={loadError()} fallback={<Skeleton label="Loading automation windows" rows={4} />}>
          <ErrorText class="settings-row-warn" text={loadError()} />
          <div class="settings-actions"><button class="btn" onClick={() => void load()}>Try again</button></div>
        </Show>
      }>{d => <>
        <p class="aw-zone">Times are in this Mac’s zone{zone() ? <>, <strong>{zone()}</strong></> : null}. An end at or before the start means the next day.</p>
        <Show when={workflowError()}><ErrorText class="settings-row-warn" text={workflowError()} /><button type="button" class="btn small" onClick={() => void loadWorkflows()}>Retry automations</button></Show>
        <Show when={dirty() || saving()}><p class="aw-note">Save or discard window changes before opening or adding an automation.</p></Show>
        <ol class="aw-list" ref={list} aria-label="Automation windows">
          {/* Keyed by row key, not by object: an edit makes a new object,
              and the row (and the input being typed in) must stay. */}
          <For each={d().windows.map(x => x.key)}>{key => {
            const row = createMemo(() => d().windows.find(x => x.key === key));
            return <Show when={row()}>{w => {
            const title = () => w().title.trim() || w().name;
            return (
              <li class="aw-window" data-key={key}>
                <fieldset class="aw-fieldset">
                  <legend class="aw-legend">
                    <span class="aw-title">{title()}</span>
                    <code class="aw-code">window:{w().name || '…'}</code>
                    <Show when={w().builtIn}><span class="settings-state-chip">built in</span></Show>
                    <Show when={!w().saved}><span class="settings-state-chip" data-state="warn">new</span></Show>
                  </legend>
                  <div class="aw-fields">
                    <label class="aw-field aw-field-title">
                      <span class="aw-field-k">Label</span>
                      <input class="settings-input" type="text" data-field="title" value={w().title}
                        onInput={e => edit(key, x => withTitle(x, e.currentTarget.value), w().saved ? [] : ['name'])} />
                    </label>
                    <Show when={!w().saved}>
                      <label class="aw-field aw-field-name">
                        <span class="aw-field-k">Name automations use</span>
                        <input class="settings-input aw-mono" type="text" value={w().name} spellcheck={false} autocapitalize="off"
                          aria-invalid={!!fieldError(key, 'name')} aria-describedby={fieldError(key, 'name') ? errId(key, 'name') : undefined}
                          onInput={e => edit(key, x => ({ ...x, name: e.currentTarget.value, nameTouched: true }), ['name'])} />
                      </label>
                    </Show>
                    <div class="aw-field aw-field-days">
                      <span class="aw-field-k" id={errId(key, 'days-label')}>Days</span>
                      <div class="ui-segmented aw-days" role="group" aria-labelledby={errId(key, 'days-label')} data-invalid={fieldError(key, 'days') ? '' : undefined}
                        aria-describedby={fieldError(key, 'days') ? errId(key, 'days') : undefined}>
                        <For each={DAY_KEYS}>{day =>
                          <button type="button" aria-pressed={w().days.includes(day)} onClick={() => edit(key, x => toggleDay(x, day), ['days'])}>{DAY_NAMES[day]}</button>}
                        </For>
                      </div>
                    </div>
                    <label class="aw-field aw-field-time">
                      <span class="aw-field-k">Starts</span>
                      <input class="settings-input" type="time" value={w().start} required
                        aria-invalid={!!fieldError(key, 'hours')} aria-describedby={fieldError(key, 'hours') ? errId(key, 'hours') : undefined}
                        onInput={e => edit(key, x => ({ ...x, start: e.currentTarget.value }), ['hours'])} />
                    </label>
                    <label class="aw-field aw-field-time">
                      <span class="aw-field-k">Ends</span>
                      <input class="settings-input" type="time" value={w().end} required
                        aria-invalid={!!fieldError(key, 'hours')} aria-describedby={fieldError(key, 'hours') ? errId(key, 'hours') : undefined}
                        onInput={e => edit(key, x => ({ ...x, end: e.currentTarget.value }), ['hours'])} />
                    </label>
                    <Show when={uses(w().name) > 0 || !w().builtIn}>
                      <span class="aw-row-end">
                        <Show when={uses(w().name) > 0}>
                          <span class="aw-uses">{uses(w().name) === 1 ? '1 automation uses this window' : `${uses(w().name)} automations use this window`}</span>
                        </Show>
                        <Show when={!w().builtIn && confirming() !== key}>
                          <button type="button" class="btn small ghost" aria-label={`Remove ${title()}`} onClick={() => askRemove(w())}>Remove</button>
                        </Show>
                      </span>
                    </Show>
                  </div>
                  <Show when={confirming() === key}>
                    <p class="aw-confirm" role="group" aria-label={`Remove ${title()}?`}>
                      <span>{!workflows() || workflowError() ? 'Automation usage is unavailable. Removing this window may stop automations that use it. Remove it?' : uses(w().name) === 1 ? 'An automation runs on this window; removing it stops it until it gets another window. Remove it?' : `${uses(w().name)} automations run on this window; removing it stops them until they get another window. Remove it?`}</span>
                      <button type="button" class="btn small" onClick={() => removeWindow(w())}>Remove anyway</button>
                      <button type="button" class="btn small ghost" onClick={() => { setConfirming(null); queueMicrotask(() => focusTitle(key)?.focus()); }}>Keep it</button>
                    </p>
                  </Show>
                  <For each={(['name', 'days', 'hours'] as const).filter(f => fieldError(key, f))}>{f =>
                    <div id={errId(key, f)}><ErrorText class="settings-row-warn" text={fieldError(key, f)!} /></div>}
                  </For>
                  <p class="aw-note">
                    <span class="aw-summary">{windowSummary(w())}</span>
                    {' · '}
                    {w().builtIn ? BUILT_IN_USES[w().name]
                      : <>Choose this window in an automation’s Schedule picker.</>}
                  </p>
                  <Show when={w().saved}>
                    <div class="aw-automation-links">
                      <Show when={workflows() && !workflowError()} fallback={<p class="aw-note">Automation usage is unavailable.</p>}>
                        <For each={linked(w().name).map(row => row.slug)}>{slug => <button type="button" class="btn small ghost"
                          disabled={dirty() || saving()} aria-label={`Open ${workflows()?.find(row => row.slug === slug)?.title}`}
                          onClick={() => openAutomation(w().name, slug)}>{workflows()?.find(row => row.slug === slug)?.title}</button>}</For>
                      </Show>
                      <button type="button" class="btn small" disabled={dirty() || saving()} onClick={() => openAutomation(w().name)}>Add automation</button>
                    </div>
                  </Show>
                </fieldset>
              </li>
            );
            }}</Show>;
          }}</For>
        </ol>
        <div class="settings-actions"><button type="button" class="btn small ghost" ref={addButton} onClick={addWindow}>Add a window</button></div>

        <div class="settings-row aw-cap">
          <div class="settings-row-label">
            <div class="settings-row-k">Overnight cap</div>
            <div class="settings-row-v">
              The most one night of <code>tonight</code> may cost, counting the night queue and every other unattended run. The cap is edited only in Usage.
            </div>
          </div>
          {/* Leaving the section would drop an unsaved windows draft: say so instead. */}
          <button type="button" class="btn-link" onClick={() => {
            if (dirty()) { setNote('Save or discard your window changes first, then set the cap in Usage → Budgets.'); return; }
            openSettingsFromOutside('usage', 'Overnight cap');
          }}>Set in Usage → Budgets</button>
        </div>
        <Show when={problems().cap}>{msg => <div id="aw-cap-error"><ErrorText class="settings-row-warn" text={msg()} /></div>}</Show>

        <Show when={problems().general}>{msg => <ErrorText class="settings-row-warn" text={msg()} />}</Show>
        <div class="settings-actions aw-actions">
          <button type="button" class="btn primary" aria-disabled={!dirty() || saving()} aria-busy={saving()}
            onClick={() => { if (dirty()) void save(); }}>{saving() ? 'Saving…' : 'Save windows'}</button>
          <button type="button" class="btn ghost" aria-disabled={!dirty() || saving()} onClick={() => { if (dirty() && !saving()) discard(); }}>Discard changes</button>
          <p class="aw-status" role="status">{note() || (dirty() ? 'Unsaved changes. Save windows keeps them; leaving this page asks first.' : '')}</p>
        </div>
      </>}</Show>
    </div>
  );
}
