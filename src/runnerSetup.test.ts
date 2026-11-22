import { describe, expect, it } from 'vitest';
import { applyConfirm, blankSetupParams, createConfirm, estimateLines, fillBlanks, formatUsd, onLabel, setupStateLabel, setupStateTone, WHAT_RUNS_THERE, whatRunsTherePatch, type RunnerCloudEstimate, type RunnerSetupStep, type RunsThereChoice } from './runnerSetup';
import { agentSettingsPayload, draftFromSettings, savePayload } from './agentSettingsPayload';
import type { AgentSettings } from './types';

const estimate: RunnerCloudEstimate = {
  machineType: 'e2-standard-8', machineLabel: '8 vCPU, 32 GB', hourlyUsd: 0.268024, hours: 160, computeUsd: 42.88,
  diskGb: 200, diskUsd: 20, natUsd: 3.87, totalUsd: 66.75, totalCents: 6675, region: 'us-central1', pricesRegion: 'us-central1', pricesTaken: '2026-11-03',
};

describe('runner setup words', () => {
  it('labels every state, and an unchecked step', () => {
    expect(['done', 'missing', 'unknown', 'failed', undefined].map(s => setupStateLabel(s as never))).toEqual(['Done', 'Not yet', "Couldn't tell", 'Failed', 'Not checked']);
    expect(setupStateTone('done')).toBe('ok');
    expect(setupStateTone('failed')).toBe('error');
    expect(setupStateTone(undefined)).toBe('off');
  });

  it('says who runs a command', () => {
    expect(onLabel({ on: 'admin', who: 'admin@your-mac.local (admin login)', shown: '', sudo: false })).toBe('On admin@your-mac.local (admin login)');
    expect(onLabel({ on: 'local', who: 'this Mac', shown: '', sudo: false })).toBe('On this Mac');
    expect(onLabel({ on: 'hollow', who: 'Hollow', shown: '', sudo: false })).toBe('Hollow itself');
  });

  it('fills only blank fields', () => {
    const typed = { ...blankSetupParams('mac'), admin: 'admin@a.local', account: 'mine' };
    const filled = { ...blankSetupParams('mac'), admin: 'x', alias: 'hollow-runner', account: 'hollowworker', keyFile: '~/.ssh/id_ed25519_hollow-runner' };
    const out = fillBlanks(typed, filled);
    expect([out.admin, out.alias, out.account, out.keyFile]).toEqual(['admin@a.local', 'hollow-runner', 'mine', '~/.ssh/id_ed25519_hollow-runner']);
    const cloud = fillBlanks(blankSetupParams('cloud'), { ...blankSetupParams('cloud'), diskGb: 200, path: 'mac' });
    expect([cloud.diskGb, cloud.path]).toEqual([200, 'cloud']);
  });

  it('spells out the estimate, and other regions differ', () => {
    expect(formatUsd(62.88)).toBe('$62.88');
    const lines = estimateLines(estimate);
    expect(lines[0]).toBe('e2-standard-8 (8 vCPU, 32 GB): $0.268 an hour × 160 hours = $42.88');
    expect(lines[2]).toBe('Cloud NAT, its way out to the internet (no public address): $3.87, plus $0.045 per GB it carries');
    expect(lines[3]).toBe('About $66.75 a month.');
    expect(lines[4]).toContain('taken 2026-11-03');
    expect(lines).toHaveLength(5);
    expect(estimateLines({ ...estimate, region: 'europe-west4' }).at(-1)).toContain('europe-west4 differ');
  });

  it('names the project, the machine and the estimate before Create', () => {
    const params = { ...blankSetupParams('cloud'), project: 'my-project-123', instance: 'hollow-runner', zone: 'us-central1-a', budgetUsd: 80, idleMinutes: 30 };
    const c = createConfirm(params, estimate);
    expect(c.title).toBe('Create hollow-runner in my-project-123?');
    expect(c.body).toContain('Project: my-project-123');
    expect(c.body).toContain('Machine: e2-standard-8');
    expect(c.body).toContain('about $66.75 a month');
    expect(c.body).toContain('Cloud Router and NAT');
    expect(c.summary).toContain('$66.75');
  });

  it('asks before an Apply that says it should', () => {
    const step: RunnerSetupStep = { id: 'sleep', title: 'No sleep', why: '', changes: ['changes sleep settings', 'changes sshd'], confirm: 'This changes sleep settings and sshd.', check: [], apply: [], byHand: [], byHandNote: null };
    expect(applyConfirm(step)).toEqual({ title: 'Apply No sleep?', body: 'It changes sleep settings, changes sshd.\n\nThis changes sleep settings and sshd.', confirmLabel: 'Apply' });
    expect(applyConfirm({ ...step, confirm: null })).toBeNull();
  });

  it('never uses the word for the removed onboarding flow', () => {
    const all = [estimateLines(estimate), createConfirm(blankSetupParams('cloud'), estimate).body, setupStateLabel('done')].join(' ');
    expect(all.toLowerCase()).not.toContain('wizard');
  });
});

// M7 (PER-21): naming a runner turns nothing on; What runs there? saves
// only what is ticked.
describe('What runs there?', () => {
  it('offers the four choices, none ticked by default', () => {
    expect(WHAT_RUNS_THERE.map(c => c.id)).toEqual(['unattended', 'builds', 'chats', 'appBuilds']);
    expect(whatRunsTherePatch(new Set())).toEqual({});
  });

  it('maps each tick to its setting and nothing else', () => {
    const one = (id: RunsThereChoice) => whatRunsTherePatch(new Set([id]));
    expect(one('unattended')).toEqual({ unattendedRunsOn: 'runner' });
    expect(one('builds')).toEqual({ buildsOn: 'auto' });
    expect(one('chats')).toEqual({ runsOn: 'runner' });
    expect(one('appBuilds')).toEqual({ appBuildsOn: 'runner' });
    expect(whatRunsTherePatch(new Set(WHAT_RUNS_THERE.map(c => c.id)))).toEqual({ unattendedRunsOn: 'runner', buildsOn: 'auto', runsOn: 'runner', appBuildsOn: 'runner' });
  });

  it('saves the ticked ones over the live settings, keeping every other field', () => {
    const live: AgentSettings = { defaultAgent: 'codex', models: { codex: 'm' }, efforts: { claude: 'high' }, canvases: 'most', whenOut: 'claude', unattendedRunsOn: 'same' };
    const sent = savePayload({ ...draftFromSettings(live), ...whatRunsTherePatch(new Set<RunsThereChoice>(['builds'])) }, live);
    expect(sent).toEqual({ ...agentSettingsPayload(live), buildsOn: 'auto' });
    expect('runsOn' in sent).toBe(false);
    expect('appBuildsOn' in sent).toBe(false);
    expect(sent.unattendedRunsOn).toBe('same');
  });
});
