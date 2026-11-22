import { Show, createEffect, createSignal, on } from 'solid-js';
import { api } from '../api';
import { flashFooter } from '../store';
import type { RunnerAccess, RunnerCloudAccess } from '../types';
import { errMsg } from '../utils';
import { Button, ErrorText } from './ui';
import { runnerAccessHint } from './runnerAccessHint';
import '../styles/runs-on.css';

/** Runs on → On the runner (remote runners, 2026-11-03): what this
 *  workspace's runner turns may reach besides their checkout. GitHub (push,
 *  pull requests) and Google Cloud, both off until turned on here. Only the
 *  person sets these: no agent action does. Shown beside Runs on wherever it
 *  shows, once a runner is set up. */
export function RunnerAccessControl(props: { workspaceId: string }) {
  const [access, setAccess] = createSignal<RunnerAccess | null>(null);
  // "A service account" picked but not yet given an address: shown, not saved.
  const [pickingAccount, setPickingAccount] = createSignal(false);
  const [account, setAccount] = createSignal('');
  const [error, setError] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  createEffect(on(() => props.workspaceId, id => {
    setAccess(null);
    setError('');
    setPickingAccount(false);
    void api.runnerAccess(id).then(value => {
      if (props.workspaceId !== id) return;
      setAccess(value);
      setAccount(value.serviceAccount ?? '');
    }, e => { if (props.workspaceId === id) setError(errMsg(e)); });
  }));
  /** Save `next` for this workspace: `failed` (the caller puts its control
   *  back), or `moved` when the person went to another workspace meanwhile
   *  (its controls now show that one's, so nothing is put back). */
  async function save(next: RunnerAccess): Promise<'saved' | 'failed' | 'moved'> {
    if (busy()) return 'failed';
    const id = props.workspaceId;
    setBusy(true);
    setError('');
    try {
      const saved = await api.runnerAccessSet(id, next);
      if (props.workspaceId !== id) return 'moved';
      setAccess(saved);
      setAccount(saved.serviceAccount ?? '');
      setPickingAccount(false);
      flashFooter("Saved what this workspace's runner turns may reach");
      return 'saved';
    } catch (e) {
      if (props.workspaceId !== id) return 'moved';
      setError(errMsg(e));
      return 'failed';
    } finally {
      setBusy(false);
    }
  }
  const saved = (): RunnerAccess => access() ?? { github: false, cloud: 'off' };
  const cloudOn = () => saved().cloud !== 'off' || pickingAccount();
  const shownCloud = (): RunnerCloudAccess => (pickingAccount() ? 'service-account' : saved().cloud);
  /** A checkbox the save didn't take goes back to what is saved. */
  const toggle = async (box: HTMLInputElement, next: RunnerAccess) => {
    if ((await save(next)) === 'failed') box.checked = !box.checked;
  };
  return (
    <div class="runner-access" role="group" aria-label="On the runner">
      <span class="runs-on-label">On the runner</span>
      <label class="cx-check"><input type="checkbox" checked={saved().github} disabled={!access() || busy()}
        onChange={e => void toggle(e.currentTarget, { ...saved(), github: e.currentTarget.checked })} />GitHub</label>
      <label class="cx-check"><input type="checkbox" checked={cloudOn()} disabled={!access() || busy()}
        onChange={e => {
          const box = e.currentTarget;
          if (!box.checked && pickingAccount() && saved().cloud === 'off') { setPickingAccount(false); return; }
          void toggle(box, { ...saved(), cloud: box.checked ? 'limited' : 'off', serviceAccount: undefined });
        }} />Google Cloud</label>
      <Show when={cloudOn()}>
        <label class="runner-access-cloud">
          <span class="sr-only">Google Cloud access</span>
          <select class="input" value={shownCloud()} disabled={busy()}
            onChange={e => {
              const select = e.currentTarget;
              const cloud = select.value as RunnerCloudAccess;
              // A service account needs its address first: saved with it.
              if (cloud === 'service-account' && !account().trim()) { setPickingAccount(true); return; }
              setPickingAccount(false);
              void save({ ...saved(), cloud, serviceAccount: cloud === 'service-account' ? account().trim() : undefined })
                .then(result => { if (result === 'failed') select.value = shownCloud(); });
            }}>
            <option value="limited">My account, no production changes</option>
            <option value="full">My account (full)</option>
            <option value="service-account">A service account</option>
          </select>
        </label>
        <Show when={shownCloud() === 'service-account'}>
          <input class="input runner-access-account" aria-label="Service account address" placeholder="name@project.iam.gserviceaccount.com"
            value={account()} onInput={e => setAccount(e.currentTarget.value)} disabled={busy()} />
          <Button variant="ghost" disabled={busy() || !account().trim() || (!pickingAccount() && account().trim() === (saved().serviceAccount ?? ''))}
            onClick={() => void save({ ...saved(), cloud: 'service-account', serviceAccount: account().trim() })}>Use it</Button>
        </Show>
      </Show>
      <span class="runs-on-hint">{runnerAccessHint(pickingAccount() ? { ...saved(), cloud: 'service-account', serviceAccount: undefined } : saved())}</span>
      <Show when={error()}>{msg => <ErrorText as="p" class="runs-on-error" text={msg()} />}</Show>
    </div>
  );
}
