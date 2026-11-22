import { describe, expect, it } from 'vitest';
import { remoteRunNotice } from './remoteRunNotice';
import { runsOnForSave, agentSettingsPayload, draftFromSettings, unattendedRunsOnForSave } from './agentSettingsPayload';

describe('the remote runner notices', () => {
  it('say where a turn ran and how its work came back', () => {
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host' })).toBe(
      'Running on the runner (runner-host) in its own copy of this worktree. Tools, History and Stop stay on this Mac.',
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', kept: ['work-mini-2'] })).toContain("An earlier turn's work that never came back is on branch work-mini-2.");
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', kept: ['work-mini-1', 'work-mini-2'] })).toContain("Earlier turns' work that never came back is on branches work-mini-1, work-mini-2.");
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', pruned: { count: 2, freeGb: 31, floorGb: 40 } })).toBe(
      'Running on the runner (runner-host) in its own copy of this worktree. Tools, History and Stop stay on this Mac. The runner had 31 GB free, under its 40 GB floor, so it removed 2 folders other turns left there, unused for 12 hours or more.',
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', pruned: { count: 1, freeGb: 39, floorGb: 40 } })).toContain('so it removed 1 folder other turns left there');
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', pruned: { count: 0 } })).not.toContain('floor');
    // 11-02: commits there carry this Mac's identity; without one, the start says so once.
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', noIdentity: true })).toBe(
      "Running on the runner (runner-host) in its own copy of this worktree. Tools, History and Stop stay on this Mac. This Mac has no complete git name and email for this worktree, so commits made there carry the runner's default (set user.name and user.email here to change that).",
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', noIdentity: false })).not.toContain('git name');
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', noIdentity: true })).not.toContain('git name');
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'back', host: 'runner-host', outcome: 'applied', head: '0123456789abcdef' })).toBe(
      "The runner's work is back in this worktree (HEAD 01234567).",
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'back', host: 'runner-host', outcome: 'unchanged' })).toBe('The turn on the runner changed no files.');
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'back', host: 'runner-host', outcome: 'keptAside', branch: 'work-mini-1' })).toContain('branch work-mini-1');
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'back', host: 'runner-host', outcome: 'failed', message: 'no route' })).toBe('no route');
  });

  it('say how a workspace folder ran there and came back (Phase 3)', () => {
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder' })).toBe(
      "Running on the runner (runner-host) in its own copy of this workspace's folder. Tools, History and Stop stay on this Mac.",
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', earlier: { changed: 1, merged: [], copies: [] } })).toContain(
      "An earlier turn's changes that never came back are here now (1 file).",
    );
    // Overnight P3L-5: a dead run's copy that couldn't be kept for an hour.
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', aside: ['/Users/runner/hollow-remote/aside/w-1a2b-1793612265-411'] })).toBe(
      "Running on the runner (runner-host) in its own copy of this workspace's folder. Tools, History and Stop stay on this Mac. What a turn that stopped there left couldn't be brought back for an hour, so the runner kept it aside at /Users/runner/hollow-remote/aside/w-1a2b-1793612265-411 (for up to 14 days) and started fresh.",
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', aside: [] })).not.toContain('aside');
    // Overnight P3L-4: a result on a base this Mac never sent, as copies only.
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', earlier: { unsure: ['new (conflicted copy 2026-11-02 0605 ab12).md'] } })).toContain(
      "An earlier turn's changes that never came back are here now (made on a copy this Mac can't confirm it sent, so only kept beside your files, nothing here written over: new (conflicted copy 2026-11-02 0605 ab12).md).",
    );
    const many = Array.from({ length: 13 }, (_, i) => `f${i} (conflicted copy).md`);
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', earlier: { unsure: many } })).toContain(
      'f9 (conflicted copy).md and 3 more).',
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'back', host: 'runner-host', scope: 'folder', outcome: 'applied', changed: 2, merged: ['WORKSPACE.md'], copies: ['PLAN (conflicted copy 2026-11-02 0005 ab12).md'] })).toBe(
      "The runner's changes to this workspace's folder are here (2 files; merged with changes made here: WORKSPACE.md; changed on both sides, so both kept (the runner's as PLAN (conflicted copy 2026-11-02 0005 ab12).md)).",
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'back', host: 'runner-host', scope: 'folder', outcome: 'unchanged' })).toBe(
      "The turn on the runner changed nothing in this workspace's folder.",
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'back', host: 'runner-host', scope: 'folder', outcome: 'failed', message: 'later' })).toBe('later');
  });

  it('name what of the folder stayed on this Mac (overnight R3)', () => {
    expect(remoteRunNotice({
      type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder',
      leftHere: { secret: { count: 3, first: ['.env', 'keys/', 'secrets.pem'] }, big: { count: 1, first: ['outputs/render.mov'] }, repos: { count: 0, first: [] } },
    })).toBe(
      "Running on the runner (runner-host) in its own copy of this workspace's folder. Tools, History and Stop stay on this Mac. Left on this Mac, so the agent there doesn't see them: 3 secret-looking items (.env, keys/, secrets.pem) and 1 file over 50 MB (outputs/render.mov).",
    );
    const first = ['a.key', 'b.key', 'c.key', 'd.key', 'e.key'];
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', leftHere: { secret: { count: 7, first }, repos: { count: 2, first: ['app/', 'site/'] } } })).toContain(
      ': 7 secret-looking items (a.key, b.key, c.key, d.key, e.key and 2 more) and 2 repositories of their own (app/, site/).',
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', leftHere: { secret: { count: 1, first: ['.env'] }, big: { count: 2, first: ['a.mov', 'b.mov'] }, repos: { count: 1, first: ['app/'] } } })).toContain(
      ': 1 secret-looking item (.env), 2 files over 50 MB (a.mov, b.mov) and 1 repository of its own (app/).',
    );
    // A build workspace's vault folder goes beside the worktree.
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', leftHere: { secret: { count: 1, first: ['.env'] } } })).toBe(
      "Running on the runner (runner-host) in its own copy of this worktree. Tools, History and Stop stay on this Mac. Left on this Mac from this workspace's vault folder, so the agent there doesn't see them: 1 secret-looking item (.env).",
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', leftHere: null })).not.toContain('Left on');
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', leftHere: { secret: { count: 0, first: [] } } })).not.toContain('Left on');
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', leftHere: { budget: { count: 3120, first: ['build/a', 'build/b'] } } })).toContain(
      ': 3120 files only a .gitignore names, past the 2,000 files or 200 MB a turn takes (build/a, build/b and 3118 more).',
    );
    // What didn't come back, by why, each the first few and a count.
    expect(remoteRunNotice({
      type: 'remoteRun', phase: 'back', host: 'runner-host', scope: 'folder', outcome: 'applied', changed: 1,
      notBrought: { skipped: { count: 0, first: [] }, refused: { count: 2, first: ['.env', '.ssh/config'] }, tooBig: { count: 1, first: ['render.mov'] }, pastBudget: { count: 2400, first: ['out/1', 'out/2'] } },
    })).toBe(
      "The runner's changes to this workspace's folder are here (1 file; never brought back (key, credential and tool settings files, such as .env): .env, .ssh/config; left on the runner (over 50 MB): render.mov; left on the runner (past the 2,000 files or 200 MB a turn brings of files only a .gitignore names): out/1, out/2 and 2398 more).",
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'back', host: 'runner-host', scope: 'folder', outcome: 'applied', notBrought: { skipped: { count: 9, first: ['a', 'b', 'c', 'd', 'e'] } } })).toContain(
      'not brought (a link, an unsafe path, or junk or a cache this Mac keeps out): a, b, c, d, e and 4 more',
    );
    // A turn recorded before carries a plain list.
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'back', host: 'runner-host', scope: 'folder', outcome: 'applied', skipped: ['link.md'] })).toContain(
      'not brought (a link, an unsafe path, or junk or a cache this Mac keeps out): link.md',
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', earlier: { changed: 1, notBrought: { refused: { count: 1, first: ['.npmrc'] } } } })).toContain(
      '(1 file; never brought back (key, credential and tool settings files, such as .env): .npmrc)',
    );
  });
});

describe('a Hollow that quit and its runs on the runner (overnight R1)', () => {
  it('says Continue picked the run up, and how an earlier turn came back', () => {
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'worktree', reattached: true }))
      .toBe('Picked up the turn that was still running on the runner (runner-host) when Hollow quit: the rest of its output follows, and its work comes back to this worktree when it ends. Tools, History and Stop stay on this Mac.');
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', scope: 'folder', reattached: true }))
      .toContain("comes back to this workspace's folder when it ends");
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'back', host: 'runner-host', outcome: 'applied', head: '0123456789abcdef', earlier: true }))
      .toBe("The turn that was running on the runner (runner-host) when Hollow quit was stopped there before this one. The runner's work is back in this worktree (HEAD 01234567).");
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'back', host: 'runner-host', outcome: 'applied', head: '0123456789abcdef' }))
      .toBe("The runner's work is back in this worktree (HEAD 01234567).");
  });
});

describe('Runs on in Settings → Devices', () => {
  it('saves only the runner (this Mac is the default and never sent)', () => {
    expect(runsOnForSave('runner')).toBe('runner');
    expect(runsOnForSave('mac-mini')).toBe('runner');
    expect(runsOnForSave('this-mac')).toBeUndefined();
    expect(runsOnForSave('elsewhere')).toBeUndefined();
    expect(runsOnForSave(undefined)).toBeUndefined();
    expect(agentSettingsPayload({ defaultAgent: null, models: {}, runsOn: 'mac-mini' }).runsOn).toBe('runner');
    expect('runsOn' in agentSettingsPayload({ defaultAgent: null, models: {}, runsOn: 'this-mac' })).toBe(false);
    expect(draftFromSettings({ defaultAgent: null, models: {}, runsOn: 'mac-mini' }).runsOn).toBe('mac-mini');
  });

  it('saves the runner or Follow Runs on for overnight and worker chats (absent follows Runs on, M7)', () => {
    expect(unattendedRunsOnForSave('same')).toBe('same');
    expect(unattendedRunsOnForSave('mac-mini')).toBe('runner');
    expect(unattendedRunsOnForSave('runner')).toBe('runner');
    expect(unattendedRunsOnForSave('moon')).toBeUndefined();
    expect(unattendedRunsOnForSave(null)).toBeUndefined();
    expect(agentSettingsPayload({ defaultAgent: null, models: {}, unattendedRunsOn: 'same' }).unattendedRunsOn).toBe('same');
    expect(agentSettingsPayload({ defaultAgent: null, models: {}, unattendedRunsOn: 'mac-mini' }).unattendedRunsOn).toBe('runner');
    expect('unattendedRunsOn' in agentSettingsPayload({ defaultAgent: null, models: {}, unattendedRunsOn: 'moon' })).toBe(false);
    expect(draftFromSettings({ defaultAgent: null, models: {}, unattendedRunsOn: 'same' }).unattendedRunsOn).toBe('same');
  });
});

describe('a self-hosted turn whose Runs on is the runner (self-hosted models Q7)', () => {
  it('says it runs here and why, and never that the runner might take it later', () => {
    const qwen = { model: 'Qwen3.8 27B', signIn: 'gcloud' as const };
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host: 'runner-host', selfHosted: qwen })).toBe(
      "The runner (runner-host) can't use Qwen3.8 27B: Hollow serves it through this Mac's local forwarder and signs in with this Mac's gcloud, so this turn runs on this Mac.",
    );
    // Q7 review: Runs on is the runner but none is set up: its own sentence,
    // never a login the runner lacks.
    for (const host of ['', '  ']) {
      const none = remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host, selfHosted: qwen });
      expect(none).toBe('Runs on is the remote runner, but none is set up on this Mac; Qwen3.8 27B runs on this Mac either way, so this turn runs here.');
      expect(none).not.toContain('login');
    }
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host: 'runner-host', selfHosted: { model: 'Llama', signIn: 'command' } })).toBe(
      "The runner (runner-host) can't use Llama: Hollow serves it through this Mac's local forwarder and signs in with its token command on this Mac, so this turn runs on this Mac.",
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host: 'runner-host', selfHosted: { model: 'Local', signIn: 'none' } })).toBe(
      "The runner (runner-host) can't use Local: Hollow serves it through this Mac's local forwarder, so this turn runs on this Mac.",
    );
    for (const signIn of ['gcloud', 'command', 'none', null] as const) {
      const line = remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host: 'runner-host', reason: 'ignored', selfHosted: { model: 'Qwen3.8 27B', signIn } });
      expect(line).not.toContain('Overnight and worker chats');
      expect(line).not.toContain('ignored');
      // Q7 review: a Google Cloud runner may have a login of its own, so
      // the reason is the forwarder, which always holds.
      expect(line).not.toContain('Google Cloud login');
      expect(line).toContain("this Mac's local forwarder");
      expect(line.endsWith('so this turn runs on this Mac.')).toBe(true);
    }
  });
  it('says what of an earlier turn is still on the runner, by why (Q7 review)', () => {
    const qwen = { model: 'Qwen3.8 27B', signIn: 'gcloud' as const };
    const line = (leftOnRunner: 'pending' | 'runner' | null | undefined, host = 'runner-host') =>
      remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host, selfHosted: { ...qwen, leftOnRunner } });
    // A failed return, or a run that is gone: only a turn there brings it back.
    expect(line('runner')).toBe(
      "The runner (runner-host) can't use Qwen3.8 27B: Hollow serves it through this Mac's local forwarder and signs in with this Mac's gcloud, so this turn runs on this Mac. Work an earlier turn left on the runner stays there until a turn there on a model that isn't self-hosted brings it back.",
    );
    // The runner didn't answer, or (Q7 follow-up) wasn't asked because it
    // didn't answer minutes ago: the next message here tries again.
    expect(line('pending')).toBe(
      "The runner (runner-host) can't use Qwen3.8 27B: Hollow serves it through this Mac's local forwarder and signs in with this Mac's gcloud, so this turn runs on this Mac. The turn that was running on the runner when Hollow quit wasn't brought back: the runner didn't answer (or wasn't asked, because it didn't answer minutes ago), so its work is still there and it may still be running; a later message in this chat brings it back once the runner answers.",
    );
    expect(line('pending')).not.toContain("couldn't be reached");
    expect(line('pending')).not.toContain('Claude model');
    expect(line('runner', '')).toContain('either way, so this turn runs here. Work an earlier turn left on the runner stays there');
    for (const leftOnRunner of [null, undefined]) {
      expect(line(leftOnRunner)).not.toContain('earlier turn');
      expect(line(leftOnRunner)).not.toContain('when Hollow quit');
    }
  });
});

describe('unattended work the runner could not take (overnight N6)', () => {
  it('says it runs here, why, and where the choice lives', () => {
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host: 'runner-host', reason: "When checked 2 minutes ago, runner-host didn't answer: ssh: Operation timed out." })).toBe(
      "Running on this Mac: the runner (runner-host) couldn't take this turn. When checked 2 minutes ago, runner-host didn't answer: ssh: Operation timed out. Overnight and worker chats go there when it can (Settings → Devices → Runs on).",
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host: 'runner-host', reason: 'Codex has no API key to hand over there' })).toContain(
      "couldn't take this turn. Codex has no API key to hand over there. Overnight",
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host: 'runner-host' })).toBe(
      "Running on this Mac: the runner (runner-host) couldn't take this turn. Overnight and worker chats go there when it can (Settings → Devices → Runs on).",
    );
  });
});

describe('routing decisions record their reason (RT1)', () => {
  it('a fallback from an unreachable runner says it ran here and why, once', () => {
    const route = { target: 'here', host: 'runner-host', cause: 'unreachable', reason: 'the runner did not answer', fallback: true } as const;
    const tail = ' Overnight and worker chats go there when it can (Settings → Devices → Runs on).';
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host: 'runner-host', reason: "The runner (runner-host) didn't answer: ssh: Operation timed out. Nothing ran there.", route })).toBe(
      `Running on this Mac because the runner (runner-host) did not answer: ssh: Operation timed out. Nothing ran there.${tail}`,
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host: 'runner-host', reason: "When checked 2 minutes ago, runner-host didn't answer: ssh: Operation timed out.", route })).toBe(
      `Running on this Mac because the runner (runner-host) did not answer when checked 2 minutes ago: ssh: Operation timed out.${tail}`,
    );
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host: 'runner-host', reason: "When checked a minute ago, runner-host didn't finish the check in 10 s (a stalled connection, or a check stuck there). It may be off.", route })).toBe(
      `Running on this Mac because the runner (runner-host) did not answer when checked a minute ago (a stalled connection, or a check stuck there). It may be off.${tail}`,
    );
    const refusing = { ...route, cause: 'refusing', reason: "the runner wasn't taking work" } as const;
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host: 'runner-host', reason: "runner-host isn't taking work now (no: free=12GB).", route: refusing })).toBe(
      `Running on this Mac because the runner (runner-host) wasn't taking work (no: free=12GB).${tail}`,
    );
    const busy = { ...route, cause: 'busy', reason: 'another turn of this worktree was still on the runner' } as const;
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'stayed', host: 'runner-host', reason: "Another turn of this worktree is still on it, and two can't share its copy there.", route: busy })).toBe(
      `Running on this Mac instead of on the runner (runner-host): Another turn of this worktree is still on it, and two can't share its copy there.${tail}`,
    );
  });

  it('a turn sent to the runner says why it went there', () => {
    const route = { target: 'runner', host: 'runner-host', cause: 'chosen', reason: 'Runs on is set to the runner', fallback: false } as const;
    expect(remoteRunNotice({ type: 'remoteRun', phase: 'start', host: 'runner-host', route })).toBe(
      'Running on the runner (runner-host) in its own copy of this worktree. Tools, History and Stop stay on this Mac. It went there because Runs on is set to the runner.',
    );
  });
});
