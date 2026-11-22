import { describe, expect, it } from 'vitest';
import { runsOnHint } from './runsOnHint';
import agentDefaults from '../settings/AgentDefaults.tsx?raw';

// Lane S follow-ups (2026-11-03): a build workspace on Default names
// Overnight and worker chats while night and worker chats don't follow Runs on.
describe('the Runs on hint', () => {
  const build = { vaultWorkspace: false, runner: 'runner-host' };
  it('names Overnight and worker chats on Default only when that setting is the runner', () => {
    const both = 'Follows Settings → Devices → Runs on. Night and worker chats use the Overnight and worker chats setting there instead.';
    // M7: Follow Runs on is the default (absent), so only the runner names it.
    expect(runsOnHint({ ...build, choice: 'default', unattended: 'runner' })).toBe(both);
    expect(runsOnHint({ ...build, choice: 'default', unattended: 'mac-mini' })).toBe(both);
    for (const unattended of [undefined, null, 'same']) expect(runsOnHint({ ...build, choice: 'default', unattended })).toBe('Follows Settings → Devices → Runs on.');
  });
  it('a workspace that chooses for itself says where its turns run, with no Settings clause', () => {
    expect(runsOnHint({ ...build, choice: 'this-mac' })).toBe('Its turns run on this Mac.');
    expect(runsOnHint({ ...build, choice: 'runner' })).toBe("Claude and Codex turns run on the runner (runner-host); their work comes back here when each turn ends. A self-hosted model's turns stay on this Mac.");
    expect(runsOnHint({ vaultWorkspace: true, runner: null, choice: 'default' })).toBe("Its chats run on this Mac. A workspace's chats move only when it chooses the runner here.");
    expect(runsOnHint({ vaultWorkspace: true, runner: null, choice: 'runner' })).toContain('once one is set up (Settings, Devices, Remote runner) in a copy of this workspace');
  });
  // Self-hosted models Q7 review: wherever the runner is chosen, a
  // self-hosted model's turns are named as staying here, in the workspace's
  // hint and in Settings' Runs on and Overnight and worker chats help.
  it("says a self-hosted model's turns stay on this Mac wherever the runner is chosen", () => {
    for (const vaultWorkspace of [false, true]) {
      for (const runner of ['runner-host', null]) {
        expect(runsOnHint({ vaultWorkspace, runner, choice: 'runner' })).toContain("A self-hosted model's turns stay on this Mac.");
      }
    }
    expect(runsOnHint({ ...build, choice: 'this-mac' })).not.toContain('self-hosted');
    expect(agentDefaults).toContain("Where a build workspace's agent turns run.");
    expect(agentDefaults).toContain("A self-hosted model's turns stay on this Mac. Another workspace's chats run on the runner");
    expect(agentDefaults).toContain('Follow Runs on keeps them with Runs on. A chat on a self-hosted model runs on this Mac either way, and its first line says why.');
  });
});
