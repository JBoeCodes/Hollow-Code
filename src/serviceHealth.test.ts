import { describe, expect, it } from 'vitest';
import type { TrackerStatus } from './generated/ipc';
import { serviceHealth } from './serviceHealth';
import type { ConnectorView } from './connectorsLibrary';

const conn = (over: Partial<ConnectorView>): ConnectorView => ({ id: 'c1', name: 'Plaud', description: '', transport: 'claudeAi', command: '', args: [], url: 'https://mcp.plaud.ai/mcp', serverId: '', iconUrl: '', envKeys: [], headerKeys: [], agents: ['claude'], chat: true, automations: false, mode: 'always', autoApprove: false, enabled: true, origin: 'claudeai', updatedAt: '', ...over });

describe('service health by capability (audit H04)', () => {
  it('shows a working sign-in and a signed-out agent connector side by side', () => {
    const [p] = serviceHealth({ plaud: 'ok', connectors: [conn({})], statuses: { c1: { id: 'c1', state: 'needsSignIn', detail: null } } });
    expect(p.hollow).toMatchObject({ label: 'Recordings in Hollow', state: 'ok', text: 'Signed in.' });
    expect(p.agents).toMatchObject({ state: 'attention', text: 'Plaud needs sign-in.' });
  });
  it('never lets the sign-in imply agent access, or the reverse', () => {
    const [p] = serviceHealth({ plaud: 'ok', connectors: [], statuses: {} });
    expect(p.hollow?.state).toBe('ok');
    expect(p.agents.state).toBe('off');
    const [m] = serviceHealth({ connectors: [conn({ agents: ['claude', 'codex'], transport: 'stdio', automations: true })], statuses: {} });
    expect(m.hollow?.state).toBe('off');
    // Enabled with no probe result: configured, not verified (next-level P00).
    expect(m.agents).toMatchObject({ state: 'unverified', text: 'Set up for Claude and Codex in chats and automations. Not checked yet, so access isn’t confirmed.' });
    const [checked] = serviceHealth({ connectors: [conn({ agents: ['claude', 'codex'], transport: 'stdio', automations: true })], statuses: { c1: { id: 'c1', state: 'connected', detail: null } } });
    expect(checked.agents).toMatchObject({ state: 'ok', text: 'Claude and Codex can use it in chats and automations.' });
  });
  it('reproduces the audit case: an enabled connector with no status is never "can use it"', () => {
    // outputs/hollow-next-level/connector-health-reproduction.json returned
    // {state:'ok', text:'Codex can use it in chats.'} before the fix.
    const [p] = serviceHealth({ connectors: [conn({ id: 'synthetic-plaud', url: '', transport: 'stdio', agents: ['codex'], chat: true, automations: false })], statuses: {} });
    expect(p.agents.state).toBe('unverified');
    expect(p.agents.text).not.toMatch(/can use it/);
  });
  it('one checked connector is not evidence for an unchecked sibling', () => {
    const [p] = serviceHealth({ connectors: [conn({}), conn({ id: 'c2', name: 'Plaud (local)', transport: 'stdio' })], statuses: { c1: { id: 'c1', state: 'connected', detail: null } } });
    expect(p.agents).toMatchObject({ state: 'ok' });
    expect(p.agents.text).toMatch(/1 more connector not checked/);
  });
  it('flags a sign-in that needs attention and a disabled connector', () => {
    const [p] = serviceHealth({ plaud: 'needs-auth', connectors: [conn({ enabled: false })], statuses: {} });
    expect(p.hollow).toMatchObject({ label: 'Recordings in Hollow', state: 'attention' });
    expect(p.agents.text).toMatch(/turned off/);
  });

  it('puts the task tracker first, named for its provider, once Settings has read it', () => {
    expect(serviceHealth({ connectors: [], statuses: {} }).map(r => r.service)).toEqual(['Plaud']);
    const t: TrackerStatus = { state: 'ready', provider: 'linear', displayName: 'Linear', providers: [], teamId: 'x', teamKey: 'ENG', personalTeamId: null, personalTeamKey: null, transport: 'graphql',
      pollSeconds: 120, trackAgentWork: false, fridayStatus: false, postStatusUpdates: false, key: 'saved', missing: null, blockedBy: null, lastSync: null, nextSync: null, lastError: null, rate: null,
      outbox: 0, problems: [], restartNeeded: false, engine: true };
    const [row] = serviceHealth({ tracker: t, connectors: [conn({ id: 'l1', name: 'Linear', url: 'https://mcp.linear.app/mcp' })], statuses: { l1: { id: 'l1', state: 'connected', detail: null } } });
    expect(row.service).toBe('Linear');
    expect(row.hollow).toMatchObject({ state: 'unverified', text: 'Set up; no pass has finished yet.' });
    expect(row.agents).toMatchObject({ state: 'ok', connectors: ['Linear'] });
    const [off] = serviceHealth({ tracker: { ...t, state: 'off', provider: null, displayName: null }, connectors: [], statuses: {} });
    expect(off).toMatchObject({ service: 'Task tracker', hollow: { state: 'off' }, agents: { state: 'off' } });
    const [needs] = serviceHealth({ tracker: { ...t, state: 'needsSetup', missing: 'Save an API key.' }, connectors: [], statuses: {} });
    expect(needs.hollow).toMatchObject({ state: 'attention', text: 'Linear is chosen but not set up: Save an API key.' });
  });
});
