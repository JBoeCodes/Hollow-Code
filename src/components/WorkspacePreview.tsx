import { Show } from 'solid-js';
import { BuildChanges } from './BuildChanges';
import { StatePanel } from './ui';
import { workspaces } from '../store/workspaces';

/**
 * A workspace view in the Preview pane (COO rebuild L4, 2026-11-10): a build
 * workspace's Changes (changed files, diff, commit, push, pull request,
 * Services, Build app). It was a section of the workspace panel, which is
 * gone; the Files picker's Changed files group, the top bar's branch and the
 * palette's Open Changes open it.
 */
export function WorkspacePreview(props: { view: 'changes'; workspaceId: string }) {
  const workspace = () => workspaces().find(w => w.id === props.workspaceId) ?? null;
  return <div class="workspace-preview">
    <Show when={workspace()?.build?.exists ? workspace() : null} keyed fallback={<StatePanel compact kind="empty" title="No worktree to show">This workspace has no build worktree now.</StatePanel>}>
      {w => <BuildChanges workspace={w} />}
    </Show>
  </div>;
}
