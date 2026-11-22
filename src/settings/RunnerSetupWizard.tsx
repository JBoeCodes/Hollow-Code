import { For, Index, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { showConfirm } from '../store';
import { errMsg } from '../utils';
import { Button, ErrorText, Segmented, StatusPill } from '../components/ui';
import { Select } from '../components/Select';
import {
  applyConfirm, blankSetupParams, createConfirm, estimateLines, fillBlanks, onLabel, setupStateLabel, setupStateTone,
  type RunnerSetupCmd, type RunnerSetupParams, type RunnerSetupPath, type RunnerSetupPlan, type RunnerSetupStep, type RunnerSetupStepState,
} from '../runnerSetup';
import '../styles/runs-on.css';

type TextKey = { [K in keyof RunnerSetupParams]: RunnerSetupParams[K] extends string ? K : never }[keyof RunnerSetupParams];
type NumberKey = { [K in keyof RunnerSetupParams]: RunnerSetupParams[K] extends number ? K : never }[keyof RunnerSetupParams];

const PATHS = [
  { value: 'mac' as const, label: 'A Mac you own, always on' },
  { value: 'cloud' as const, label: 'A Google Cloud machine' },
];

/** One step's commands under a heading, each with who runs it and Copy. */
function CmdList(props: { title: string; cmds: RunnerSetupCmd[]; note?: string | null }) {
  const copy = (text: string) => void navigator.clipboard?.writeText(text).catch(() => undefined);
  return (
    <Show when={props.cmds.length}>
      <div class="runner-setup-cmdlist">
        <p class="runner-setup-cmdlist-title">{props.title}</p>
        <Show when={props.note}>{n => <p class="runner-setup-note">{n()}</p>}</Show>
        <For each={props.cmds}>{c => (
          <div class="runner-setup-cmd">
            <div class="runner-setup-cmd-head">
              <span>{onLabel(c)}{c.sudo ? ', with sudo -n' : ''}</span>
              <Button size="sm" variant="ghost" title="Copy this command" onClick={() => copy(c.shown)}>Copy</Button>
            </div>
            <pre class="runner-setup-pre">{c.shown}</pre>
          </div>
        )}</For>
      </div>
    </Show>
  );
}

/** Settings, Devices, Remote runner, Set up (remote runners 3,
 *  2026-11-03): a runner from scratch, on a Mac you own or a Google Cloud
 *  machine. Every step lists its exact commands. Check runs every step's
 *  read-only commands and changes nothing; each step's Apply is the only
 *  thing that changes anything, and those that change accounts, keys,
 *  sleep or sshd ask first (showConfirm). Google Cloud's Create shows the
 *  monthly estimate in its confirm and hands that estimate back, so the
 *  backend creates nothing without it. The last step names the runner
 *  (`onUse`). Commands come from `runner_setup_plan`; the words from
 *  runnerSetup.ts. */
export function RunnerSetupWizard(props: { onUse: (host: string | null) => void }) {
  const [params, setParams] = createSignal<RunnerSetupParams>(blankSetupParams('mac'));
  const [plan, setPlan] = createSignal<RunnerSetupPlan | null>(null);
  const [states, setStates] = createSignal<Record<string, RunnerSetupStepState>>({});
  const [busy, setBusy] = createSignal('');
  const [error, setError] = createSignal('');
  // A field changed and its plan hasn't come back: what is shown is old.
  const [pending, setPending] = createSignal(true);
  let asked = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  async function replan(fill: boolean) {
    const mine = ++asked;
    setPending(true);
    try {
      const next = await api.runnerSetupPlan(params());
      if (mine !== asked) return;
      if (fill) setParams(p => fillBlanks(p, next.params));
      setPlan(next);
      setPending(false);
      setError('');
    } catch (e) {
      // The last plan stays, and what runs is what it shows (`shownParams`).
      if (mine === asked) { setError(errMsg(e)); setPending(false); }
    }
  }
  onMount(() => void replan(true));

  const cloud = () => params().path === 'cloud';
  const blocked = () => !!busy() || pending() || !plan() || (plan()?.problems.length ?? 0) > 0;
  /** The fields the shown commands were built from: what Check, Apply and Create send. */
  const shownParams = () => plan()?.params ?? params();
  function choosePath(path: RunnerSetupPath) {
    if (path === params().path || busy()) return;
    clearTimeout(timer);
    setParams(blankSetupParams(path));
    setStates({});
    setError('');
    void replan(true);
  }
  function setField(key: TextKey | NumberKey, value: string | number) {
    setParams(p => ({ ...p, [key]: value }));
    setStates({});
    setPending(true);
    asked++; // a plan already on its way is for the old fields
    clearTimeout(timer);
    timer = setTimeout(() => void replan(false), 250);
  }
  async function run<T>(what: string, go: () => Promise<T>, then: (value: T) => void) {
    if (busy()) return;
    setBusy(what);
    setError('');
    try {
      then(await go());
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy('');
    }
  }
  const keep = (s: RunnerSetupStepState) => setStates(all => ({ ...all, [s.id]: s }));
  const checkAll = () => void run('check', () => api.runnerSetupCheck(shownParams()), list => setStates(Object.fromEntries(list.map(s => [s.id, s]))));
  async function applyStep(step: RunnerSetupStep) {
    const sent = shownParams();
    const ask = applyConfirm(step);
    if (ask && !await showConfirm({ ...ask, focusCancel: true })) return;
    await run(step.id, () => api.runnerSetupApply(sent, step.id), s => {
      keep(s);
      if (step.id === 'use' && s.state === 'done') props.onUse(sent.alias);
    });
  }
  async function create() {
    const sent = shownParams();
    await run('cloud', async () => {
      const shown = await api.runnerCloudEstimate(sent);
      if (!await showConfirm({ ...createConfirm(sent, shown), focusCancel: true })) return null;
      return api.runnerCloudCreate(sent, shown.totalCents);
    }, s => { if (s) keep(s); });
  }

  function text(key: TextKey, label: string, placeholder?: string) {
    return (
      <label class="runner-setup-field">
        <span class="runner-setup-label">{label}</span>
        <input class="input runner-setup-input" value={params()[key]} placeholder={placeholder ?? (plan()?.params[key] || '')}
          autocomplete="off" spellcheck={false} disabled={!!busy()} onInput={e => setField(key, e.currentTarget.value.trim())} />
      </label>
    );
  }
  function number(key: NumberKey, label: string) {
    return (
      <label class="runner-setup-field">
        <span class="runner-setup-label">{label}</span>
        <input class="input runner-setup-input runner-setup-number" type="number" min="1" value={params()[key] || ''} placeholder={String(plan()?.params[key] || '')}
          disabled={!!busy()} onInput={e => setField(key, Math.max(0, Math.floor(Number(e.currentTarget.value) || 0)))} />
      </label>
    );
  }

  return (
    <div class="runner-setup" role="group" aria-label="Set up a runner">
      <p class="remote-runner-note"><strong>Or set one up from scratch.</strong> Every step lists the commands it runs. Check runs only the read commands and changes nothing; a step's Apply is the only thing that changes anything, and Applies that change accounts, keys, sleep or sshd ask first.</p>
      <Segmented label="Where the runner is" items={PATHS} value={params().path} onChange={choosePath} />
      <div class="runner-setup-fields">
        <Show when={cloud()} fallback={<>
          {text('admin', 'Admin login (setup only)', 'admin@your-mac.local')}
          {text('hostName', 'Its address', 'your-mac.local')}
          {text('alias', 'Runner alias')}
          {text('account', 'Runner account')}
          {text('keyFile', 'Key file')}
        </>}>
          {text('project', 'Project id', 'your-project')}
          {text('zone', 'Zone')}
          <div class="runner-setup-field">
            <span class="runner-setup-label">Machine type</span>
            <Select label="Machine type" value={params().machineType || plan()?.params.machineType || ''} disabled={!!busy()} onChange={v => setField('machineType', v)}
              options={(plan()?.machineTypes ?? []).map(m => ({ value: m.id, label: `${m.id} (${m.label})`, detail: `about $${m.hourlyUsd.toFixed(3)} an hour` }))} />
          </div>
          {number('diskGb', 'Boot disk (GB)')}
          {number('idleMinutes', 'Stop after idle minutes')}
          {number('hoursPerMonth', 'Expected hours a month')}
          {text('billingAccount', 'Billing account id', '0X0X0X-0X0X0X-0X0X0X')}
          {number('budgetUsd', 'Monthly budget alert (USD)')}
          {text('alias', 'Runner alias')}
          {text('instance', 'Instance name')}
          {text('serviceAccount', 'Service account name')}
          {text('keyFile', 'Key file')}
        </Show>
      </div>
      <Show when={plan()?.problems.length}>
        <ul class="runner-setup-problems">
          <For each={plan()?.problems ?? []}>{p => <li>{p}</li>}</For>
        </ul>
      </Show>
      <Show when={plan()?.notes.length}>
        <ul class="runner-setup-notes">
          <For each={plan()?.notes ?? []}>{n => <li>{n}</li>}</For>
        </ul>
      </Show>
      <Show when={cloud() && plan()?.estimate}>{e => (
        <div class="runner-setup-estimate" role="group" aria-label="Monthly estimate">
          <For each={estimateLines(e())}>{line => <p>{line}</p>}</For>
          <p class="runner-setup-note">Creating the machine was built and tested against a stand-in gcloud, not yet against a real project.</p>
        </div>
      )}</Show>
      <div class="runner-setup-actions">
        <Button variant="secondary" disabled={blocked()} onClick={checkAll}>{busy() === 'check' ? 'Checking…' : 'Check'}</Button>
        <span class="runner-setup-note">Check asks every step what is already there, one at a time.</span>
      </div>
      <ol class="runner-setup-steps">
        <Index each={plan()?.steps ?? []}>{(step, i) => {
          const found = () => states()[step().id];
          return (
            <li class="runner-setup-step" data-state={found()?.state ?? 'unchecked'}>
              <div class="runner-setup-step-head">
                <span class="runner-setup-step-title">{i + 1}. {step().title}</span>
                <StatusPill quiet label={setupStateLabel(found()?.state)} tone={setupStateTone(found()?.state)} />
                <For each={step().changes}>{c => <span class="runner-setup-change">{c}</span>}</For>
              </div>
              <p class="runner-setup-why">{step().why}</p>
              <Show when={found()}>{s => (
                <div class="runner-setup-found" aria-live="polite">
                  <p>{s().summary}</p>
                  <Show when={s().details.length}>
                    <ul><For each={s().details}>{d => <li>{d}</li>}</For></ul>
                  </Show>
                </div>
              )}</Show>
              <details class="runner-setup-cmds" open={!!found()?.needsByHand}>
                <summary>Commands</summary>
                <CmdList title="Check runs (reads only)" cmds={step().check} />
                <CmdList title={step().id === 'cloud' ? 'Create runs, in order' : 'Apply runs, in order'} cmds={step().apply} />
                <CmdList title="By hand, in Terminal" cmds={step().byHand} note={step().byHandNote} />
                <Show when={found()?.ran.length}>
                  <div class="runner-setup-cmdlist">
                    <p class="runner-setup-cmdlist-title">What ran last</p>
                    <For each={found()?.ran ?? []}>{r => (
                      <div class="runner-setup-cmd">
                        <div class="runner-setup-cmd-head"><span>{r.code === null ? 'Not finished' : `Exit ${r.code}`}</span></div>
                        <pre class="runner-setup-pre">{r.shown.split('\n')[0]}{r.output ? `\n${r.output}` : ''}</pre>
                      </div>
                    )}</For>
                  </div>
                </Show>
              </details>
              <div class="runner-setup-step-actions">
                <Show when={step().id === 'cloud'} fallback={
                  <Show when={step().apply.length}>
                    <Button size="sm" variant="secondary" disabled={blocked()} onClick={() => void applyStep(step())}>{busy() === step().id ? 'Applying…' : 'Apply'}</Button>
                  </Show>
                }>
                  <Button size="sm" variant="primary" disabled={blocked()} onClick={() => void create()}>{busy() === 'cloud' ? 'Creating…' : 'Create…'}</Button>
                </Show>
              </div>
            </li>
          );
        }}</Index>
      </ol>
      <Show when={error()}>{msg => <ErrorText as="p" class="runs-on-error" text={msg()} />}</Show>
    </div>
  );
}
