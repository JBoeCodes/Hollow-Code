import { For, Show, createSignal } from 'solid-js';
import { api } from '../api';
import { draftFromSettings, savePayload } from '../agentSettingsPayload';
import { WHAT_RUNS_THERE, whatRunsTherePatch, type RunsThereChoice } from '../runnerSetup';
import { refreshAgentSettings } from '../store';
import { bumpSettingsRevision } from '../store/settingsRevision';
import { serializeAgentSettingsWrite } from './agentSettingsQueue';
import type { RunnerStatus } from '../types';
import { errMsg } from '../utils';
import { Button, ErrorText } from '../components/ui';
import { runnerStateLabel } from '../runnerStatus';
import { RunnerStatusBlock } from './RunnerStatusBlock';
import { RunnerSetupWizard } from './RunnerSetupWizard';
import { MoreFold } from './settingsLayout';
import { SettingsSheet, openerRef } from './SettingsSheet';
import '../styles/runs-on.css';

/** Settings, Devices, Remote runner (remote runners 2,
 *  2026-11-03). Off until the person names a runner: Hollow ships no host,
 *  so a new Hollow never reaches anyone's machine. Set up asks for the SSH
 *  host (a Host entry in ~/.ssh/config for the runner's own account),
 *  checks it read-only and saves it as this Mac's runner
 *  (~/.config/hollow/slot-remote). Once set: its host, its state and owner
 *  (RunnerStatusBlock, with Record me as owner) and Turn off. Set up also
 *  holds RunnerSetupWizard (remote runners 3): a runner from scratch, step
 *  by step; Set up again opens it once a runner is set. Naming a runner
 *  either way ends on What runs there? (M7, PER-21): nothing is ticked, and
 *  naming it turns nothing on. */
export function RemoteRunnerRow(props: { runner: string | null | undefined; onChange: (host: string | null) => void }) {
  const [setup, setSetup] = createSignal(false);
  const opener = openerRef('[data-sheet-opener="runner"]');
  const [host, setHost] = createSignal('');
  const [checked, setChecked] = createSignal<RunnerStatus | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [checking, setChecking] = createSignal(false);
  const [error, setError] = createSignal('');
  // M7: the runner just named, while What runs there? shows.
  const [named, setNamed] = createSignal<string | null>(null);
  const close = () => { setSetup(false); setChecked(null); setError(''); setNamed(null); };
  async function act<T>(run: () => Promise<T>, then: (value: T) => void) {
    if (busy()) return;
    setBusy(true);
    setError('');
    try {
      then(await run());
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  }
  const typed = () => host().trim();
  const check = () => {
    if (busy()) return;
    setChecking(true);
    void act(() => api.runnerCheckHost(typed()), setChecked).finally(() => setChecking(false));
  };
  const use = () => void act(() => api.runnerSet(typed()), now => { setChecked(null); setHost(''); props.onChange(now); if (now) setNamed(now); else close(); });
  const turnOff = () => void act(() => api.runnerSet(null), now => props.onChange(now));
  return (
    <Show when={props.runner !== undefined}>
    <div class="settings-row remote-runner">
      <div class="settings-row-label">
        <span class="settings-row-k" id="remote-runner-k">Remote runner</span>
        <div class="settings-row-v">
          <Show when={props.runner} fallback={<>
            <strong class="remote-runner-off">Off.</strong> A computer of yours that stays on (another Mac today) can take builds, tests and whole agent turns, so this Mac stays free.
            <MoreFold>
              <div>Until one is set up, everything runs here, and the settings that choose what runs on a runner (Runs on, Builds and tests, App builds, Overnight and worker chats, Runner GitHub access) appear below once one is. A runner should be yours alone: anyone who can log in to its runner account can read what runs there.</div>
            </MoreFold>
          </>}>
            {host => <>
              <strong>{host()}</strong>: builds, tests and turns set to run on the runner go there (Runs on, below).
              <MoreFold><div>It should be yours alone: only this Mac's key should open its runner account.</div></MoreFold>
            </>}
          </Show>
        </div>
        <Show when={props.runner}>
          <RunnerStatusBlock />
        </Show>
        <Show when={setup()}>
          {/* Settings PR 5: the setup is a sheet; closing it puts focus back on Set up. */}
          <SettingsSheet id="settings-sheet-runner" title="Set up a remote runner" returnTo={opener.get} locked={busy} onClose={close}>
          <Show when={named()} fallback={
          <div class="remote-runner-setup" role="group" aria-label="Set up a remote runner">
            <p class="remote-runner-note">The SSH host is the Host name in ~/.ssh/config for the runner's own account: a standard user there whose ~/.ssh/authorized_keys holds this Mac's key. Check asks it how it is and changes nothing; Use this runner saves it here.</p>
            <div class="remote-runner-fields">
              <input class="input remote-runner-input" aria-label="SSH host" placeholder="SSH host" autocomplete="off" spellcheck={false}
                value={host()} onInput={e => { setHost(e.currentTarget.value); setChecked(null); }} disabled={busy()} />
              <Button variant="secondary" disabled={busy() || !typed()} onClick={check}>{checking() ? 'Checking…' : 'Check'}</Button>
              <Button variant="primary" disabled={busy() || !typed()} onClick={use}>Use this runner</Button>
              <Button variant="ghost" disabled={busy()} onClick={close}>Cancel</Button>
            </div>
            <div aria-live="polite">
            <Show when={checked()}>
              {s => (
                <div class="remote-runner-checked">
                  <span class="runner-status-state" data-state={s().state}>{runnerStateLabel(s().state)}</span>
                  <Show when={s().machine}>{m => <span> · {m()}</span>}</Show>
                  <Show when={s().preflight}>{p => <p class="runner-status-line">{p()}</p>}</Show>
                  <Show when={s().message}>{m => <p class="settings-row-warn runner-status-why">{m()}</p>}</Show>
                  <Show when={s().owner?.state === 'other'}><p class="settings-row-warn runner-status-why">Another Hollow has recorded itself as this runner's owner.</p></Show>
                  <Show when={s().owner?.otherKeys}>{n => <p class="settings-row-warn runner-status-why">{n()} other SSH {n() === 1 ? 'key' : 'keys'} can log in to its runner account.</p>}</Show>
                </div>
              )}
            </Show>
            </div>
            <RunnerSetupWizard onUse={now => { props.onChange(now); if (now) setNamed(now); }} />
            <Show when={error()}>{msg => <ErrorText as="p" class="runs-on-error" text={msg()} />}</Show>
          </div>
          }>
            {host => <WhatRunsThere host={host()} onDone={close} />}
          </Show>
          </SettingsSheet>
        </Show>
        <Show when={!setup() ? error() : ''}>{msg => <ErrorText as="p" class="runs-on-error" text={msg()} />}</Show>
      </div>
      <Show when={props.runner} fallback={
        <Button variant="secondary" data-sheet-opener="runner" aria-describedby="remote-runner-k" onClick={e => { opener.mark(e); setSetup(true); }}>Set up</Button>
      }>
        {/* One block, so the row keeps its two tracks: a third child put Turn off under the text. */}
        <div class="settings-row-actions">
          <Button variant="secondary" data-sheet-opener="runner" aria-describedby="remote-runner-k" onClick={e => { opener.mark(e); setSetup(true); }}>Set up again</Button>
          <Button variant="ghost" disabled={busy()} aria-describedby="remote-runner-k" onClick={turnOff}>Turn off</Button>
        </div>
      </Show>
    </div>
    </Show>
  );
}

/** Set up's last step (M7, PER-21): what the runner just named takes. Four
 *  unticked choices; Save writes only the ticked ones through the normal
 *  agent settings save (one queue with the Settings forms), and Not now, or
 *  Save with nothing ticked, writes nothing. */
function WhatRunsThere(props: { host: string; onDone: () => void }) {
  const [ticked, setTicked] = createSignal<ReadonlySet<RunsThereChoice>>(new Set());
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal('');
  const toggle = (id: RunsThereChoice, on: boolean) => setTicked(old => {
    const next = new Set(old);
    if (on) next.add(id); else next.delete(id);
    return next;
  });
  async function save() {
    const patch = whatRunsTherePatch(ticked());
    if (!Object.keys(patch).length) { props.onDone(); return; }
    setSaving(true);
    setError('');
    try {
      await serializeAgentSettingsWrite(async () => {
        const live = await api.agentSettings();
        await api.setAgentSettings(savePayload({ ...draftFromSettings(live), ...patch }, live));
      });
      await refreshAgentSettings();
      bumpSettingsRevision();
      props.onDone();
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setSaving(false);
    }
  }
  return (
    <div class="remote-runner-setup" role="group" aria-label="What runs there?">
      <p class="remote-runner-note"><strong>What runs there?</strong> {props.host} is this Mac's runner. Nothing runs there until you choose it; you can change these later in Settings, Devices.</p>
      <For each={WHAT_RUNS_THERE}>{choice => (
        <label class="runner-setup-field">
          <input type="checkbox" checked={ticked().has(choice.id)} disabled={saving()}
            onChange={e => toggle(choice.id, e.currentTarget.checked)} />
          <span class="runner-setup-label">{choice.label}</span>
          <span class="runner-setup-note">{choice.detail}</span>
        </label>
      )}</For>
      <div class="remote-runner-fields">
        <Button variant="primary" disabled={saving()} onClick={() => void save()}>{saving() ? 'Saving…' : 'Save'}</Button>
        <Button variant="ghost" disabled={saving()} onClick={props.onDone}>Not now</Button>
      </div>
      <Show when={error()}>{msg => <ErrorText as="p" class="runs-on-error" text={msg()} />}</Show>
    </div>
  );
}
