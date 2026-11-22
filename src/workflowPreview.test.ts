(globalThis as unknown as { process: { env: Record<string, string> } }).process.env.TZ = 'America/New_York';

import { describe, expect, it } from 'vitest';
import type { ConnectorView } from './connectorsLibrary';
import type { BudgetStatus } from './types';
import { appAccess, applicableBudgets, automationConnectors, buildConfigPreview, taskAccess, todayUsageText, type PreviewContext, type PreviewSettings } from './workflowPreview';
import { stampedChanges } from './workflowPermissions';

const settings = (over: Partial<PreviewSettings> = {}): PreviewSettings => ({
  agent: null, model: null, cwd: '/v/workspaces/launch', permissionMode: 'acceptEdits', allowedTools: [], disallowedTools: [],
  outputGlob: 'workspaces/launch/outputs/*', schedule: '0 0 7 * * * *', triggers: [], skipIfQuiet: false, watch: [],
  misfireGraceMin: null, ...over,
});

function connector(over: Partial<ConnectorView>): ConnectorView {
  return {
    id: 'notion', name: 'Notion', description: '', transport: 'http', command: '', args: [], url: 'https://x', serverId: '',
    iconUrl: '', envKeys: [], headerKeys: [], agents: ['claude', 'codex'], chat: true, automations: true, mode: 'always',
    autoApprove: false, enabled: true, origin: 'hollow', updatedAt: '', ...over,
  };
}

function budget(over: Partial<BudgetStatus>): BudgetStatus {
  return {
    id: 'b', scope: 'global', scopeLabel: 'all usage', metric: 'tokens', window: 'day', limit: 1000, warnPercent: 80, hardStop: true,
    windowKey: 'k', windowStart: '', windowEnd: '', observed: 200, percent: 20, state: 'ok', pause: null, runs: 1, unknownRuns: 0,
    partialRuns: 0, unpricedRuns: 0, cachedInputTokens: 0, estimated: false, enforceability: 'ok', enforceabilityReason: null,
    scopeMissing: false, suggestedLimit: null, ledgerError: null, ...over,
  };
}

const ctx = (over: Partial<PreviewContext> = {}): PreviewContext => ({
  slug: 'brief', defaultAgent: 'claude', agentLabels: { claude: 'Claude', codex: 'Codex', grok: 'Grok' },
  agentsAvailable: ['claude', 'codex'], permissionLabels: { acceptEdits: 'Edit files', plan: 'Plan only' },
  workspaces: [{ cwd: '/v/workspaces/launch', title: 'Launch' }],
  connectors: [], budgets: [], holds: {}, unsaved: null, now: new Date('2026-10-05T08:00:00-04:00').getTime(), ...over,
});

const line = (p: ReturnType<typeof buildConfigPreview>, group: string, label: string) =>
  p.groups.find(g => g.id === group)!.lines.find(l => l.label === label);

describe('buildConfigPreview', () => {
  it('previews the three host window starts and their catch-up policy', () => {
    const times = ['2026-10-24T00:00:00Z', '2026-10-25T00:00:00Z', '2026-10-26T01:00:00Z'];
    const context = {
      ...ctx(), now: Date.parse('2026-10-23T12:00:00Z'), locale: 'en-US',
      windows: { timeZone: 'Europe/Paris', overnightCapUsd: 40, windows: [{ name: 'nightly', label: 'Nightly', days: ['sat', 'sun', 'mon', 'tue'], start: '02:00', end: '05:00', period: 'day', openNow: false, nextStart: times[0], nextStarts: times }] },
    };
    const p = buildConfigPreview(settings({ schedule: 'window:nightly/run-all' }), context);
    const when = p.groups.find(g => g.id === 'when')!.lines;
    expect(when.slice(0, 3).map(l => l.label)).toEqual(['Next run', 'Then', 'Then']);
    for (const entry of when.slice(0, 3)) expect(entry.value).toContain('2:00 AM');
    expect(when.some(l => l.value.includes('at most 5'))).toBe(true);
    expect(when.some(l => l.value.includes('never as a backlog'))).toBe(false);
  });

  it('describes who, where, effects, limits and when without running anything', () => {
    const p = buildConfigPreview(settings(), ctx({
      connectors: [connector({}), connector({ id: 'm365', name: 'Microsoft 365', mode: 'relevant' }), connector({ id: 'off', name: 'Off', automations: false })],
      budgets: [budget({}), budget({ id: 'w', scope: 'workflow:brief', scopeLabel: 'the “brief” automation', state: 'warning', observed: 900 }), budget({ id: 'x', scope: 'workflow:other' })],
    }));
    expect(p.groups.map(g => g.id)).toEqual(['who', 'where', 'effects', 'limits', 'when', 'gate']);
    expect(line(p, 'who', 'Agent')!.value).toBe('Claude (app default)');
    expect(line(p, 'where', 'Works in')!.value).toBe('Launch workspace');
    expect(line(p, 'where', 'Results land in')!.value).toBe('workspaces/launch/outputs/*');
    expect(line(p, 'effects', 'Permission mode')!.value).toBe('Edit files');
    expect(line(p, 'effects', 'Hollow tools')!.value).toBe('tasks (read and write), Hollow app (read and write), teammates');
    expect(line(p, 'effects', 'Connectors')!.value).toBe('Notion, Microsoft 365 (when relevant)');
    const limits = p.groups.find(g => g.id === 'limits')!.lines;
    expect(limits).toHaveLength(2);
    expect(limits[1].tone).toBe('warn');
    const when = p.groups.find(g => g.id === 'when')!.lines;
    expect(when.slice(0, 3).map(l => l.label)).toEqual(['Next run', 'Then', 'Then']);
    expect(when[0].value.replace(/\s/g, ' ')).toBe('Tue, Oct 6 · 7:00 AM EDT');
    expect(when.some(l => /within 2 hours/.test(l.value))).toBe(true);
    expect(line(p, 'gate', 'Approval')!.tone).toBe('ok');
    expect(p.runnable).toBe(true);
  });

  it('says what holds a run and what unsaved changes would approve', () => {
    const before = { permissionMode: 'acceptEdits', allowedTools: ['Read'], disallowedTools: [], cwd: null, agent: null, model: null, triggers: [], schedule: 'manual', prompt: 'x' };
    const unsaved = stampedChanges(before, { ...before, allowedTools: ['Read', 'Bash'] });
    const p = buildConfigPreview(settings({ schedule: 'manual' }), ctx({ holds: { permissionHold: 'review', budgetHold: 'Budget paused: daily tokens' }, unsaved }));
    const gate = p.groups.find(g => g.id === 'gate')!.lines;
    expect(gate.map(l => l.label)).toEqual(['Permission review', 'Budget', 'Unsaved changes']);
    expect(gate[2].value).toBe('Allowed tools: +Bash');
    expect(gate[2].detail).toMatch(/widen/);
    expect(p.next).toBeNull();
    expect(p.groups.find(g => g.id === 'when')!.lines[0].value).toMatch(/only when you start it/);
    expect(p.runnable).toBe(false);
  });

  it('flags an agent that is not installed and agents Hollow cannot attach tools to', () => {
    const p = buildConfigPreview(settings({ agent: 'grok' }), ctx({ connectors: [connector({})] }));
    expect(line(p, 'who', 'Agent')!.tone).toBe('error');
    expect(line(p, 'effects', 'Hollow tools')!.value).toMatch(/can’t attach tools to Grok/);
    expect(line(p, 'effects', 'Connectors')!.value).toBe('None');
    const unread = buildConfigPreview(settings(), ctx({ connectors: null, budgets: null }));
    expect(line(unread, 'effects', 'Connectors')!.tone).toBe('warn');
    expect(line(unread, 'limits', 'Budgets')!.tone).toBe('warn');
  });
});

describe('access rules mirror the backend', () => {
  it('task and app access follow mode and the stamped allowlist', () => {
    expect(taskAccess('plan', [])).toBe('read');
    expect(taskAccess('acceptEdits', [])).toBe('write');
    expect(taskAccess('acceptEdits', ['Read'])).toBe('read');
    expect(taskAccess('acceptEdits', ['Bash(git *)'])).toBe('write');
    expect(taskAccess('acceptEdits', ['mcp__hollow_tasks__list_tasks'])).toBe('read');
    expect(taskAccess('acceptEdits', ['mcp__hollow_tasks__create_task'])).toBe('write');
    expect(taskAccess('acceptEdits', ['mcp__hollow_tasks__set_project_status'])).toBe('write');
    expect(taskAccess('acceptEdits', ['mcp__hollow_tasks__get_project'])).toBe('read');
    expect(appAccess('plan', [])).toBe('read');
    expect(appAccess('acceptEdits', ['Read'])).toBeNull();
    expect(appAccess('acceptEdits', ['mcp__hollow_app'])).toBe('write');
    expect(appAccess('plan', ['mcp__hollow_app'])).toBe('read');
    expect(appAccess('acceptEdits', ['mcp__hollow_app__list_workspaces'])).toBe('read');
  });

  it('connectors: plan mode, non-owned agents, claude.ai on Codex and a restricted allowlist attach nothing extra', () => {
    const all = [connector({}), connector({ id: 'ai', name: 'AI', transport: 'claudeAi', agents: ['claude'] })];
    expect(automationConnectors(all, 'claude', 'plan', [])).toEqual([]);
    expect(automationConnectors(all, 'grok', 'acceptEdits', [])).toEqual([]);
    expect(automationConnectors(all, 'codex', 'acceptEdits', []).map(c => c.connector.id)).toEqual(['notion']);
    expect(automationConnectors(all, 'claude', 'acceptEdits', ['Read', 'mcp__ai']).map(c => c.connector.id)).toEqual(['ai']);
  });

  it('budgets: all usage, the agent and this automation', () => {
    const list = [budget({ id: 'g' }), budget({ id: 'a', scope: 'agent:claude' }), budget({ id: 'c', scope: 'agent:codex' }), budget({ id: 'w', scope: 'workflow:brief' }), budget({ id: 'h', scope: 'heartbeat' })];
    expect(applicableBudgets(list, 'brief', 'claude').map(b => b.id)).toEqual(['g', 'a', 'w']);
  });
});

describe('todayUsageText', () => {
  it('never reads a missing figure as free', () => {
    expect(todayUsageText(null)).toBe('No usage recorded for it today.');
    expect(todayUsageText({ runs: 3, inputTokens: 30_749, cachedInputTokens: 131_904, outputTokens: 1_905, unknown: 1, reportedCostUsd: null, estimatedCostUsd: null }))
      .toBe('Today: 3 runs · 164.6k tokens (1 without usage figures).');
    expect(todayUsageText({ runs: 1, inputTokens: 10, cachedInputTokens: 0, outputTokens: 5, unknown: 0, reportedCostUsd: 0.42, estimatedCostUsd: null }))
      .toBe('Today: 1 run · 15 tokens · $0.42 reported.');
  });
});
