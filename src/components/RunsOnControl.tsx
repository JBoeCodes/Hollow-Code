import { Show, createEffect, createSignal, on } from 'solid-js';
import { isRunner } from '../agentSettingsPayload';
import { api } from '../api';
import { agentSettings, flashFooter } from '../store';
import { refreshWorkspaces } from '../store/workspaces';
import { errMsg } from '../utils';
import { runsOnHint, type RunsOn } from './runsOnHint';
import { RunnerAccessControl } from './RunnerAccessControl';
import { ErrorText, Segmented } from './ui';
import '../styles/runs-on.css';

/** The remote runner: a workspace's Runs on. A build workspace's (Phase 2,
 *  2026-11-01) shows in its Changes tab and a new conversation's empty state
 *  (`build_set_runs_on`); a vault workspace's (Phase 3, 2026-11-02) in a new
 *  conversation's empty state (`workspace_set_runs_on`, read with
 *  `workspace_runs_on`). A build workspace's Default follows Settings,
 *  Devices → Runs on (its night and worker chats, Overnight and worker chats);
 *  a vault workspace moves only on its own choice (This Mac / Mac mini),
 *  never on that default. A choice saved as `mac-mini` (before M6) reads as
 *  the runner; choosing again saves `runner`. */
export function RunsOnControl(props: { workspaceId: string; saved?: string | null; class?: string; kind?: 'build' | 'workspace' }) {
  const [saved, setSaved] = createSignal<string | null | undefined>(undefined);
  const [runner, setRunner] = createSignal<string | null>(null);
  const [error, setError] = createSignal('');
  void api.remoteRunner().then(setRunner, () => setRunner(null));
  const vaultWorkspace = () => props.kind === 'workspace';
  // A vault workspace's choice isn't in the workspace list: read it here.
  createEffect(on(() => [props.workspaceId, vaultWorkspace()] as const, ([id, own]) => {
    setSaved(undefined);
    if (own) void api.workspaceRunsOn(id).then(value => { if (props.workspaceId === id) setSaved(value ?? null); }, () => {});
  }));
  const value = (): RunsOn => {
    const current = saved() === undefined ? props.saved : saved();
    return current === 'this-mac' ? current : isRunner(current) ? 'runner' : 'default';
  };
  // The Default hint names Overnight and worker chats while night and worker
  // chats don't follow Runs on (the app-wide settings, kept current by Settings).
  const hint = () => runsOnHint({ choice: value(), vaultWorkspace: vaultWorkspace(), runner: runner(), unattended: agentSettings().unattendedRunsOn });
  async function choose(next: RunsOn) {
    if (next === value()) return;
    setError('');
    try {
      if (vaultWorkspace()) {
        setSaved((await api.workspaceSetRunsOn(props.workspaceId, next === 'default' ? null : next)) ?? null);
      } else {
        const info = await api.buildSetRunsOn(props.workspaceId, next === 'default' ? null : next);
        setSaved(info.runsOn ?? null);
      }
      // The other place the choice shows (Changes, a new chat) reads the list.
      void refreshWorkspaces();
      flashFooter(next === 'default' ? 'Runs on follows Settings again' : next === 'runner' ? 'This workspace runs on the runner' : 'This workspace runs on this Mac');
    } catch (e) {
      setError(errMsg(e));
    }
  }
  return (
    <div class={`runs-on-control${props.class ? ` ${props.class}` : ''}`}>
      <span class="runs-on-label">Runs on</span>
      <Show when={vaultWorkspace()} fallback={
        <Segmented label="Runs on" level="secondary"
          items={[{ value: 'default', label: 'Default' }, { value: 'this-mac', label: 'This Mac' }, { value: 'runner', label: 'Remote runner' }] as const}
          value={value()} onChange={next => void choose(next)} />
      }>
        <Segmented label="Runs on" level="secondary"
          items={[{ value: 'this-mac', label: 'This Mac' }, { value: 'runner', label: 'Remote runner' }] as const}
          value={value() === 'runner' ? 'runner' : 'this-mac'} onChange={next => void choose(next === 'this-mac' ? 'default' : next)} />
      </Show>
      <span class="runs-on-hint">{hint()}</span>
      <Show when={error()}>{msg => <ErrorText as="p" class="runs-on-error" text={msg()} />}</Show>
      {/* Remote runners (2026-11-03): GitHub and Google Cloud for this
          workspace's runner turns, once a runner is set up. */}
      <Show when={runner()}><RunnerAccessControl workspaceId={props.workspaceId} /></Show>
    </div>
  );
}
