/**
 * Settings, Models, Self-hosted models (self-hosted models Q4): models
 * you run on your own GPU or server (`self_hosted_registry.rs`). Each row
 * has its served model and window, an on/off switch, Test, Edit and Remove;
 * Add a self-hosted model opens the form; Use the Qwen on Cloud Run preset,
 * beside it, opens the form already filled, and inside the add form fills it
 * again (one `PresetButton`, so Settings search lands on a real button
 * either way). Every action is a button here; there is no key.
 *
 * Test goes through Hollow's local forwarder the way a run will: the health
 * check (a cold start takes minutes), the model list, one 1-token answer.
 * Its state is module-level, so leaving Settings mid-test and coming back
 * shows it still running, then its result. A result belongs to the entry
 * as it was tested (`testKey`): a saved edit or a remove clears it, and one
 * whose entry changed while it ran is dropped. While a row's Test runs its
 * switch, Edit and Remove are off; while the form is open every row's
 * switch and Edit are off, so the form's copy of the switch is never stale
 * and an open draft is never replaced.
 *
 * Plain signals loaded when the part mounts, never a resource, so a change
 * can't suspend the Settings page. Lazy chunk.
 *
 * Q5: each switched-on entry is in the chat's model menu under Claude as
 * "<name> · self-hosted"; a save, switch or removal here drops the cached
 * Claude list so the menu shows it. Q9: the API style field picks the
 * agent: Anthropic's Messages API runs on Claude, OpenAI's Responses API on
 * Codex (listed under Codex), so a change drops the cached Codex list too.
 *
 * Q10: Background jobs (in Advanced, Hollow's own model calls, since
 * Settings S5b; the same component with `part="jobs"`): one row per kind of Hollow's own
 * model calls with a select (As now, or a switched-on model), the reason a
 * kind's jobs are skipped when its model can't run, and per model "N jobs
 * waiting for <name>" with Run them now (`self_hosted_jobs.rs`). It reads
 * again on `self-hosted-jobs-changed` and after every change above. Held
 * jobs are listed with Discard, and Run as now when their kind is back to
 * As now, or Try again when they failed three times (review round 2: each
 * row's words name only its own buttons). It sits outside the list's own
 * loading, so a list that fails to load never hides it.
 */
import { For, Show, createSignal, createUniqueId, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { flashFooter, showConfirm, trackTask } from '../store';
import type { SelfHostedEntry, SelfHostedHeldJob, SelfHostedJobRow, SelfHostedJobs, SelfHostedModels, SelfHostedTest, SelfHostedWaiting } from '../generated/ipc';
import { API_STYLE_OPTIONS, AS_NOW, AUTH_OPTIONS, JOBS_CLAUDE_ONLY, JOBS_DAMAGED, TEST_RUNNING, draftFrom, emptyDraft, entryFrom, entrySummary, heldDetail, heldLine, invalidText, jobOptions, levelsFor, removeBody, retryResult, testKey, testSummary, waitingDetail, waitingLine, windowText, withApiStyle, type SelfHostedDraft } from '../selfHostedModels';
import { onNative } from '../nativeEvents';
import { effortLabel } from '../modelMenu';
import { forgetCatalog } from '../modelCatalog';
import { errMsg } from '../utils';
import { Button, ErrorText, Skeleton } from '../components/ui';
import { MultiSelect, Select } from '../components/Select';
import { Switch } from '../components/Switch';
import { flashSaved, registerDirty } from './settingsDirty';
import { SettingsSheet, openerRef } from './SettingsSheet';
import { MoreFold } from './settingsLayout';

/** A Test running or done, and `key`: the entry it tested (`testKey`). */
type TestState =
  | { running: true; key: string }
  | { running: false; key: string; result: SelfHostedTest | null; error: string | null };

// Module-level: a Test outlives the panel that started it.
const [tests, setTests] = createSignal<Readonly<Record<string, TestState>>>({});
const setTest = (id: string, state: TestState) => setTests(all => ({ ...all, [id]: state }));
const dropTest = (id: string) => setTests(all => {
  const next: Record<string, TestState> = { ...all };
  delete next[id];
  return next;
});
/** Forget a finished Test (an edit saved, the entry removed). A running one
 *  stays, and its result is dropped when it ends if the entry changed. */
const clearTest = (id: string) => {
  if (tests()[id] && !tests()[id]!.running) dropTest(id);
};

/** One labelled field of the form: its name and hint left, the control right. */
function Field(props: { id: string; label: string; hint: string; children: any }) {
  return (
    <div class="settings-row">
      <div class="settings-row-label">
        <label class="settings-row-k" for={props.id}>{props.label}</label>
        <div class="settings-row-v">{props.hint}</div>
      </div>
      {props.children}
    </div>
  );
}

/** Use the Qwen on Cloud Run preset: beside Add it opens the form filled,
 *  inside the add form it fills it again. One component, so the label
 *  Settings search lists is always this button. */
function PresetButton(props: { disabled: boolean; onUse: (e: MouseEvent) => void }) {
  return <Button size="sm" disabled={props.disabled} onClick={e => props.onUse(e)}>Use the Qwen on Cloud Run preset</Button>;
}

/** `part`: `list` (Models, Self-hosted models: the list and Add) or `jobs`
 *  (Advanced, Hollow's own model calls: Background jobs, plan 3.3); both
 *  when absent. */
export function SelfHostedModelsPanel(props: { part?: 'list' | 'jobs' }) {
  const shows = (part: 'list' | 'jobs') => !props.part || props.part === part;
  const [view, setView] = createSignal<SelfHostedModels | null>(null);
  const [loadError, setLoadError] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  // The open form: adding (edit false) or editing one entry, its draft and
  // the draft it opened with (to tell whether it is unsaved).
  const [form, setForm] = createSignal<{ edit: boolean; draft: SelfHostedDraft; start: string } | null>(null);
  const [formError, setFormError] = createSignal('');
  const opener = openerRef();
  const uid = createUniqueId();
  const fieldId = (name: string) => `self-hosted-${name}-${uid}`;

  registerDirty('self-hosted-model', () => {
    const f = form();
    return !!f && JSON.stringify(f.draft) !== f.start;
  }, 'Self-hosted model');

  async function load() {
    try {
      setView(await api.selfHostedModels());
      setLoadError('');
    } catch (e) {
      setLoadError(errMsg(e));
    }
    void loadJobs();
  }
  onMount(() => void load());

  // Background jobs (Q10).
  const [jobs, setJobs] = createSignal<SelfHostedJobs | null>(null);
  const [jobsError, setJobsError] = createSignal('');
  const [jobsBusy, setJobsBusy] = createSignal(false);
  async function loadJobs() {
    try {
      setJobs(await api.selfHostedJobs());
      setJobsError('');
    } catch (e) {
      setJobsError(errMsg(e));
    }
  }
  onCleanup(onNative('self-hosted-jobs-changed', () => void loadJobs()));

  async function setJob(row: SelfHostedJobRow, entry: string) {
    if (jobsBusy() || (row.entry ?? AS_NOW) === entry) return;
    setJobsBusy(true);
    try {
      setJobs(await api.setSelfHostedJob(row.kind, entry === AS_NOW ? null : entry));
      flashSaved(row.label);
    } catch (e) {
      flashFooter(`× Could not change ${row.label}: ${errMsg(e)}`, 5000);
      void loadJobs();
    } finally {
      setJobsBusy(false);
    }
  }

  /** A held job: Run as now sends it where its kind runs today (only this
   *  click does), Try again runs one that gave up on its model once more
   *  and says whether that run started now or the next run takes it
   *  (`retryResult`), Discard drops it. */
  async function heldAction(h: SelfHostedHeldJob, action: 'run' | 'retry' | 'discard') {
    if (action === 'discard') {
      const confirmed = await showConfirm({
        title: `Discard this ${h.label} job?`,
        body: 'Hollow drops it without running it. Nothing else changes.',
        confirmLabel: 'Discard job',
        danger: true,
      });
      if (!confirmed) return;
    }
    setJobsBusy(true);
    try {
      if (action === 'retry') {
        const retry = await api.retrySelfHostedJob(h.id);
        setJobs(retry.jobs);
        flashFooter(retryResult(retry), 5000);
      } else {
        setJobs(await (action === 'run' ? api.runSelfHostedJobAsNow(h.id) : api.discardSelfHostedJob(h.id)));
      }
    } catch (e) {
      const verb = action === 'run' ? 'run' : action === 'retry' ? 'try again' : 'discard';
      flashFooter(`× Could not ${verb} the job: ${errMsg(e)}`, 5000);
      void loadJobs();
    } finally {
      setJobsBusy(false);
    }
  }

  async function runNow(w: SelfHostedWaiting) {
    setJobsBusy(true);
    try {
      setJobs(await api.runSelfHostedJobsNow(w.entry));
    } catch (e) {
      flashFooter(`× Could not run them: ${errMsg(e)}`, 5000);
      void loadJobs();
    } finally {
      setJobsBusy(false);
    }
  }

  /** A change; after a failed one the list is read again, so a file that
   *  changed or became unreadable shows as it is now. Q5: Claude's model
   *  list ends with the switched-on entries, so a change drops the cached
   *  list and the chat's model menu reads it afresh. */
  async function change(action: () => Promise<SelfHostedModels>, failure: string): Promise<boolean> {
    setBusy(true);
    try {
      setView(await action());
      forgetCatalog('claude');
      forgetCatalog('codex');
      void loadJobs();
      return true;
    } catch (e) {
      flashFooter(`× ${failure}: ${errMsg(e)}`, 5000);
      void load();
      return false;
    } finally {
      setBusy(false);
    }
  }

  // One form at a time: Add and every Edit are off while it is open, so an
  // unsaved draft is never replaced without a word.
  const open = (edit: boolean, draft: SelfHostedDraft) => {
    if (form()) return;
    setFormError('');
    setForm({ edit, draft, start: JSON.stringify(draft) });
  };
  const update = (fields: Partial<SelfHostedDraft>) => {
    const f = form();
    if (f) setForm({ ...f, draft: { ...f.draft, ...fields } });
  };
  const usePreset = () => {
    const v = view();
    const f = form();
    if (v && f) setForm({ ...f, draft: draftFrom(v.preset) });
  };

  async function save() {
    const f = form();
    if (!f || busy()) return;
    const v = view();
    const parsed = entryFrom(f.draft, v ? { min: v.minContextWindow, max: v.maxContextWindow } : undefined);
    if ('error' in parsed) { setFormError(parsed.error); return; }
    setBusy(true);
    try {
      setView(await api.saveSelfHostedModel(parsed.entry, f.edit));
      forgetCatalog('claude');
      forgetCatalog('codex');
      void loadJobs();
      setForm(null);
      setFormError('');
      // The last Test was of the endpoint as it was before the edit.
      if (f.edit) clearTest(parsed.entry.id);
      flashSaved(parsed.entry.name);
    } catch (e) {
      setFormError(errMsg(e));
      void load();
    } finally {
      setBusy(false);
    }
  }

  async function remove(entry: SelfHostedEntry) {
    const confirmed = await showConfirm({
      title: `Remove ${entry.name}?`,
      body: removeBody(entry),
      confirmLabel: 'Remove model',
      danger: true,
    });
    if (!confirmed) return;
    if (await change(() => api.removeSelfHostedModel(entry.id), 'Could not remove the model')) {
      clearTest(entry.id);
      if (form()?.edit && form()?.draft.id === entry.id) setForm(null);
    }
  }

  async function setEnabled(entry: SelfHostedEntry, enabled: boolean) {
    if (await change(() => api.setSelfHostedModelEnabled(entry.id, enabled), 'Could not change the model')) {
      flashSaved(entry.name);
    }
  }

  async function test(entry: SelfHostedEntry) {
    if (tests()[entry.id]?.running) return;
    const key = testKey(entry);
    setTest(entry.id, { running: true, key });
    let outcome: { result: SelfHostedTest | null; error: string | null };
    try {
      const result = await trackTask(`Test ${entry.name}`, () => api.testSelfHostedModel(entry.id), {
        onSuccessStatus: r => (r.ok ? 'success' : 'warning'),
      });
      outcome = { result, error: null };
    } catch (e) {
      outcome = { result: null, error: errMsg(e) };
    }
    // Edited or removed while it ran: the result is about something else now.
    const current = view()?.entries.find(e => e.id === entry.id);
    if (!current || testKey(current) !== key) dropTest(entry.id);
    else setTest(entry.id, { running: false, key, ...outcome });
  }

  /** The row's Test line: the wait while it runs, then what it found, while
   *  the entry is still the one it tested. The live region stays mounted so
   *  each change is announced. */
  function TestLine(props: { entry: SelfHostedEntry }) {
    const state = () => {
      const s = tests()[props.entry.id];
      return s && (s.running || s.key === testKey(props.entry)) ? s : undefined;
    };
    const done = () => {
      const s = state();
      return s && s.running === false ? s : null;
    };
    return (
      <div class="settings-row-v" aria-live="polite">
        <Show when={state()}>
          <Show when={done()} fallback={<span>{TEST_RUNNING}</span>}>
            {d => (
              <Show when={d().result} fallback={<ErrorText class="settings-row-warn" as="div" text={d().error ?? ''} />}>
                {r => {
                  const summary = () => testSummary(r(), props.entry);
                  return (
                    <>
                      <span class="settings-state-chip" data-state={summary().state}>{summary().chip}</span>
                      <For each={summary().lines}>{line => <div>{line}</div>}</For>
                    </>
                  );
                }}
              </Show>
            )}
          </Show>
        </Show>
      </div>
    );
  }

  function Form() {
    const f = () => form()!;
    const d = () => f().draft;
    // Q9: an OpenAI-style model offers Codex's levels only (no max).
    const levels = () => levelsFor(d().apiStyle, view()?.levels ?? []);
    const minWindow = () => windowText(view()?.minContextWindow ?? 110_000);
    return (
      <div class="self-hosted-form" role="group" aria-label={f().edit ? `Edit ${d().name || 'self-hosted model'}` : 'Add a self-hosted model'}>
        <Show when={!f().edit}>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Qwen on Cloud Run</div>
              <div class="settings-row-v">Fills every field for Qwen3.8 27B on Cloud Run. Nothing saves until you press Save.</div>
            </div>
            <PresetButton disabled={busy() || !view()} onUse={usePreset} />
          </div>
        </Show>
        <Field id={fieldId('name')} label="Name" hint="What Hollow shows for it, up to 80 characters.">
          <input id={fieldId('name')} class="settings-input" value={d().name} placeholder="Qwen3.8 27B" onInput={e => update({ name: e.currentTarget.value })} />
        </Field>
        <Field id={fieldId('id')} label="Id" hint={f().edit ? 'Fixed once added.' : 'Lowercase letters, digits and hyphens, up to 32. Hollow names the model by it.'}>
          <input id={fieldId('id')} class="settings-input" value={d().id} placeholder="qwen" disabled={f().edit} onInput={e => update({ id: e.currentTarget.value })} />
        </Field>
        <Field id={fieldId('url')} label="Address" hint="The server's base address: https://, or http:// for this Mac (localhost or 127.0.0.1).">
          <input id={fieldId('url')} class="settings-input" value={d().baseUrl} placeholder="https://" onInput={e => update({ baseUrl: e.currentTarget.value })} />
        </Field>
        <Field id={fieldId('style')} label="API style" hint="Anthropic style runs it on Claude; OpenAI style runs it on Codex.">
          <Select id={fieldId('style')} label="API style" value={d().apiStyle} searchable={false}
            options={API_STYLE_OPTIONS.map(o => ({ value: o.value, label: o.label }))}
            onChange={value => { const f = form(); if (f) setForm({ ...f, draft: withApiStyle(f.draft, value === 'openai' ? 'openai' : 'anthropic') }); }} />
        </Field>
        <Field id={fieldId('model')} label="Served model" hint="The model id the server serves, with no spaces.">
          <input id={fieldId('model')} class="settings-input" value={d().servedModel} placeholder="qwen3.8-27b" onInput={e => update({ servedModel: e.currentTarget.value })} />
        </Field>
        <Field id={fieldId('window')} label="Context window" hint={`Tokens the server takes, at least ${minWindow()}: Claude Code compacts at 100,000 at the earliest.`}>
          <input id={fieldId('window')} class="settings-input" inputmode="numeric" value={d().contextWindow} placeholder="131072" onInput={e => update({ contextWindow: e.currentTarget.value })} />
        </Field>
        <Field id={fieldId('effort')} label="Effort levels" hint="The reasoning efforts it offers. At least one.">
          <MultiSelect id={fieldId('effort')} label="Effort levels" value={d().effortLevels} searchable={false}
            options={levels().map(level => ({ value: level, label: effortLabel(level) ?? level }))}
            onChange={value => update({ effortLevels: levels().filter(level => value.includes(level)) })} />
        </Field>
        <Field id={fieldId('auth')} label="Sign-in" hint="Token command: Hollow runs it for a bearer token, kept in memory only.">
          <Select id={fieldId('auth')} label="Sign-in" value={d().authKind} searchable={false}
            options={AUTH_OPTIONS.map(o => ({ value: o.value, label: o.label }))}
            onChange={value => update({ authKind: value === 'none' ? 'none' : 'tokenCommand' })} />
        </Field>
        <Show when={d().authKind === 'tokenCommand'}>
          <Field id={fieldId('program')} label="Program" hint="Run without a shell, on the PATH agents get.">
            <input id={fieldId('program')} class="settings-input" value={d().program} placeholder="gcloud" onInput={e => update({ program: e.currentTarget.value })} />
          </Field>
          <Field id={fieldId('args')} label="Arguments" hint="Separated by spaces, at most 16. Quote an argument that has a space.">
            <input id={fieldId('args')} class="settings-input" value={d().args} placeholder="auth print-identity-token" onInput={e => update({ args: e.currentTarget.value })} />
          </Field>
        </Show>
        <Field id={fieldId('health')} label="Health path" hint="A GET that answers once the server is up. Test calls it first.">
          <input id={fieldId('health')} class="settings-input" value={d().healthPath} placeholder="/health" onInput={e => update({ healthPath: e.currentTarget.value })} />
        </Field>
        <div class="settings-row">
          <div class="settings-row-label">
            <div class="settings-row-k">Use this model</div>
            <div class="settings-row-v">Off keeps the entry and stops its local forwarder.</div>
          </div>
          <Switch checked={d().enabled} label="Use this model" onChange={next => update({ enabled: next })} />
        </div>
        <Show when={formError()}>{msg => <ErrorText class="settings-row-warn" text={msg()} />}</Show>
        <div class="settings-actions">
          <Button variant="primary" disabled={busy()} onClick={() => void save()}>Save</Button>
          <Button disabled={busy()} onClick={() => { setForm(null); setFormError(''); }}>Cancel</Button>
        </div>
      </div>
    );
  }

  /** Background jobs (Q10): each kind's model, and what waits for each. */
  function BackgroundJobs(props: { entries: SelfHostedEntry[] }) {
    return (
      <div class="self-hosted-jobs" role="group" aria-label="Background jobs">
        <div class="settings-row">
          <div class="settings-row-label">
            <div class="settings-row-k">Background jobs</div>
            <div class="settings-row-v">Which model Hollow’s own calls use. As now keeps a kind where it runs today.</div>
            <MoreFold><div class="settings-row-v">{JOBS_CLAUDE_ONLY}</div></MoreFold>
          </div>
        </div>
        <Show when={jobsError()}>
          {msg => (
            <div class="settings-row">
              <div class="settings-row-label"><ErrorText class="settings-row-warn" as="div" text={`Could not load background jobs: ${msg()}`} /></div>
              <Button size="sm" onClick={() => void loadJobs()}>Retry</Button>
            </div>
          )}
        </Show>
        <Show when={jobs()}>
          {j => (
            <>
              <Show when={j().error}>
                {msg => (
                  <div class="settings-row">
                    <div class="settings-row-label"><ErrorText class="settings-row-warn" as="div" text={`${JOBS_DAMAGED} ${msg()}`} /></div>
                    <Button size="sm" disabled={jobsBusy()} onClick={() => void loadJobs()}>Check again</Button>
                  </div>
                )}
              </Show>
              <Show when={j().queueError}>
                {msg => (
                  <div class="settings-row">
                    <div class="settings-row-label"><ErrorText class="settings-row-warn" as="div" text={`Jobs that would wait are skipped until this is fixed: ${msg()}`} /></div>
                  </div>
                )}
              </Show>
              <For each={j().kinds}>
                {row => (
                  <div class="settings-row" data-self-hosted-job={row.kind}>
                    <div class="settings-row-label">
                      <label class="settings-row-k" for={fieldId(`job-${row.kind}`)}>{row.label}</label>
                      <div class="settings-row-v">{row.about}</div>
                      <Show when={row.problem}>{p => <ErrorText class="settings-row-warn" as="div" text={p()} />}</Show>
                    </div>
                    <Select id={fieldId(`job-${row.kind}`)} label={`${row.label} model`} value={row.entry ?? AS_NOW} searchable={false}
                      disabled={jobsBusy() || !!j().error} options={jobOptions(row, props.entries)}
                      onChange={value => void setJob(row, value)} />
                  </div>
                )}
              </For>
              <For each={j().waiting}>
                {w => (
                  <div class="settings-row" data-self-hosted-waiting={w.entry} aria-live="polite">
                    <div class="settings-row-label">
                      <div class="settings-row-k">{waitingLine(w)}</div>
                      <div class="settings-row-v">{waitingDetail(w)}</div>
                    </div>
                    <Button size="sm" aria-label={`Run the jobs waiting for ${w.name ?? w.entry} now`} disabled={jobsBusy() || w.draining} onClick={() => void runNow(w)}>Run them now</Button>
                  </div>
                )}
              </For>
              <For each={j().held}>
                {h => (
                  <div class="settings-row" data-self-hosted-held={h.id} aria-live="polite">
                    <div class="settings-row-label">
                      <div class="settings-row-k">{heldLine(h)}</div>
                      <div class="settings-row-v">{heldDetail(h)}</div>
                    </div>
                    <div class="settings-row-actions">
                      <Show when={h.canRunAsNow}>
                        <Button size="sm" aria-label={`Run the held ${h.label} job as now`} disabled={jobsBusy() || h.running || !!j().error} onClick={() => void heldAction(h, 'run')}>Run as now</Button>
                      </Show>
                      <Show when={h.canTryAgain}>
                        <Button size="sm" aria-label={`Try the ${h.label} job again`} disabled={jobsBusy() || h.running || !!j().error} onClick={() => void heldAction(h, 'retry')}>Try again</Button>
                      </Show>
                      <Button size="sm" aria-label={`Discard the held ${h.label} job`} disabled={jobsBusy() || h.running} onClick={() => void heldAction(h, 'discard')}>Discard</Button>
                    </div>
                  </div>
                )}
              </For>
            </>
          )}
        </Show>
      </div>
    );
  }

  // Settings S5b: Background jobs alone, in Advanced. Its choices still list the switched-on models.
  if (!shows('list')) return <div class="self-hosted-models"><BackgroundJobs entries={view()?.entries ?? []} /></div>;
  return (
    <div class="self-hosted-models">
      <Show when={loadError()}>
        {msg => (
          <div class="settings-row">
            <div class="settings-row-label"><ErrorText class="settings-row-warn" as="div" text={`Could not load self-hosted models: ${msg()}`} /></div>
            <Button size="sm" onClick={() => void load()}>Retry</Button>
          </div>
        )}
      </Show>
      <Show when={view()} fallback={<Show when={!loadError()}><Skeleton label="Loading self-hosted models" rows={2} /></Show>}>
        {v => (
          <>
            <Show when={v().error}>
              {msg => (
                <div class="settings-row">
                  <div class="settings-row-label"><ErrorText class="settings-row-warn" as="div" text={msg()} /></div>
                  <Button size="sm" disabled={busy()} onClick={() => void load()}>Check again</Button>
                </div>
              )}
            </Show>
            <For each={v().entries}>
              {entry => {
                const testing = () => !!tests()[entry.id]?.running;
                return (
                  <div class="settings-row" data-self-hosted={entry.id}>
                    <div class="settings-row-label">
                      <div class="settings-row-k">{entry.name}</div>
                      <div class="settings-row-v">{entrySummary(entry)}</div>
                      {/* Q5 review: a hand-edited entry that fails the checks a save makes; chats refuse it with the same reason. */}
                      <Show when={v().invalid[entry.id]}>{why => <ErrorText class="settings-row-warn" as="div" text={invalidText(why())} />}</Show>
                      <TestLine entry={entry} />
                    </div>
                    <div class="settings-row-actions">
                      <Switch checked={entry.enabled} label={`${entry.name} on`} disabled={busy() || !!form() || testing()} onChange={next => void setEnabled(entry, next)} />
                      <Button size="sm" aria-label={`Test ${entry.name}`} disabled={testing()} onClick={() => void test(entry)}>Test</Button>
                      <Button size="sm" aria-label={`Edit ${entry.name}`} disabled={busy() || !!form() || testing()} onClick={e => { opener.mark(e, `[data-self-hosted="${entry.id}"] button[aria-label^="Edit "]`); open(true, draftFrom(entry)); }}>Edit</Button>
                      <Button size="sm" aria-label={`Remove ${entry.name}`} disabled={busy() || testing()} onClick={() => void remove(entry)}>Remove</Button>
                    </div>
                  </div>
                );
              }}
            </For>
            <>
              <div class="settings-row">
                <div class="settings-row-label">
                  <div class="settings-row-k">Add a self-hosted model</div>
                  <div class="settings-row-v">{v().entries.length ? 'Another server, such as vLLM, or the Qwen on Cloud Run preset.' : 'No self-hosted models yet. Add a server such as vLLM, or use the Qwen preset.'}</div>
                </div>
                {/* Browser-run fix (2026-11-03): Settings search lists the
                    preset, so it is a button beside Add, not only inside
                    the open form; here it opens the form already filled. */}
                <div class="settings-row-actions">
                  <Button size="sm" aria-label="Add a self-hosted model" data-sheet-opener="self-hosted-add" disabled={busy() || !!v().error || !!form()} onClick={e => { opener.mark(e); open(false, emptyDraft()); }}>Add…</Button>
                  <PresetButton disabled={busy() || !!v().error || !!form()} onUse={e => { opener.mark(e, '[data-sheet-opener="self-hosted-add"]'); open(false, draftFrom(v().preset)); }} />
                </div>
              </div>
              {/* Settings PR 5: the form is a sheet; saving or cancelling puts focus back on the row's button. */}
              <Show when={form()}>
                <SettingsSheet id="settings-sheet-self-hosted" title={form()?.edit ? 'Edit a self-hosted model' : 'Add a self-hosted model'} returnTo={opener.get} locked={busy} onClose={() => { setForm(null); setFormError(''); }}>
                  <Form />
                </SettingsSheet>
              </Show>
            </>
          </>
        )}
      </Show>
      {/* Q10 review: outside the list, so a list that fails to load never
          hides what waits or Run them now. */}
      <Show when={shows('jobs')}><BackgroundJobs entries={view()?.entries ?? []} /></Show>
    </div>
  );
}
