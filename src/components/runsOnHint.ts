// The line under a workspace's Runs on control (RunsOnControl.tsx). Pure, so
// the node vitest suite can check each case without the component.
import { RUNNER, unattendedRunsOnForSave } from '../agentSettingsPayload';

/** A workspace's Runs on as the control shows it: the runner is `runner`
 *  (an old saved `mac-mini` is read as it, M6). */
export type RunsOn = 'default' | 'this-mac' | 'runner';

/** What a workspace's Runs on choice means. `runner` is the runner's SSH
 *  host (null: none set up). `unattended` is Settings, Devices, Overnight
 *  and worker chats as saved: `runner` (the runner when it answers), or
 *  `same` or absent (Follow Runs on, the default since M7). A build
 *  workspace on Default follows Runs on, its night and worker chats too
 *  unless that setting is the runner (lane S follow-ups, 2026-11-03). */
export function runsOnHint(o: { choice: RunsOn; vaultWorkspace: boolean; runner: string | null; unattended?: string | null }): string {
  if (o.vaultWorkspace && o.choice !== 'runner') return "Its chats run on this Mac. A workspace's chats move only when it chooses the runner here.";
  if (o.choice === 'this-mac') return 'Its turns run on this Mac.';
  const where = o.runner ? `the runner (${o.runner})` : 'the remote runner, once one is set up (Settings, Devices, Remote runner)';
  // Self-hosted models Q7 review: a self-hosted model's turns never go there.
  if (o.choice === 'runner') return o.vaultWorkspace
    ? `Claude and Codex turns run on ${where} in a copy of this workspace's folder; their changes come back here when each turn ends. A self-hosted model's turns stay on this Mac.`
    : `Claude and Codex turns run on ${where}; their work comes back here when each turn ends. A self-hosted model's turns stay on this Mac.`;
  return unattendedRunsOnForSave(o.unattended) === RUNNER ? 'Follows Settings → Devices → Runs on. Night and worker chats use the Overnight and worker chats setting there instead.' : 'Follows Settings → Devices → Runs on.';
}
