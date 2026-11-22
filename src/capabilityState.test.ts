import { describe, expect, it } from 'vitest';
import { checkedPhrase, forLogin } from './capabilityState';
import { connectorCapability } from './connectorCapability';
import type { ConnectorView } from './connectorsLibrary';

const conn = (over: Partial<ConnectorView>): ConnectorView => ({ id: 'c1', name: 'Plaud', description: '', transport: 'stdio', command: 'plaud-mcp', args: [], url: '', serverId: '', iconUrl: '', envKeys: [], headerKeys: [], agents: ['claude'], chat: true, automations: false, mode: 'always', autoApprove: false, enabled: true, origin: 'hollow', updatedAt: '', ...over });

describe('capability vocabulary (next-level P00/P16)', () => {
  it('a check under one login does not certify another', () => {
    const ran = { state: 'available' as const, login: 'a@example.com', checkedAt: '2026-10-01T12:00:00Z' };
    expect(forLogin(ran, 'A@example.com').state).toBe('available');
    expect(forLogin(ran, 'b@example.com')).toMatchObject({ state: 'unchecked', detail: 'Last checked as a@example.com, not b@example.com.' });
    expect(forLogin(ran, null).state).toBe('unchecked');
    expect(forLogin({ state: 'needsSignIn' }, 'b@example.com').state).toBe('needsSignIn');
  });
  it('says when a check ran', () => {
    const now = Date.parse('2026-10-01T12:00:00Z');
    expect(checkedPhrase(null, now)).toBe('never checked');
    expect(checkedPhrase('2026-10-01T11:55:00Z', now)).toBe('checked 5m ago');
    expect(checkedPhrase('2026-09-29T12:00:00Z', now)).toBe('checked 2d ago');
  });
  it('local and remote servers Hollow cannot probe are configured, not checked', () => {
    expect(connectorCapability(conn({}))).toMatchObject({ label: 'Configured, not checked', tone: 'neutral' });
    expect(connectorCapability(conn({ transport: 'http', url: 'https://mcp.example.com' })).tone).not.toBe('ok');
    expect(connectorCapability(conn({ transport: 'claudeAi' })).label).toBe('Configured, not checked');
    expect(connectorCapability(conn({ transport: 'claudeAi' }), { status: { id: 'c1', state: 'connected', detail: null } })).toMatchObject({ label: 'Available', tone: 'ok' });
    expect(connectorCapability(conn({ enabled: false })).label).toBe('Off');
  });
});
