import { describe, expect, it } from 'vitest';
import { CHECK_TTL_MS, appliesTo, connectorCapability, evidenceLine, turnConnectors, worksOn } from './connectorCapability';
import type { ConnectorView } from './connectorsLibrary';

const NOW = Date.parse('2026-10-01T12:00:00Z');
const conn = (over: Partial<ConnectorView>): ConnectorView => ({ id: 'c1', name: 'Plaud', description: '', transport: 'stdio', command: 'plaud-mcp', args: [], url: '', serverId: '', iconUrl: '', envKeys: [], headerKeys: [], agents: ['claude'], chat: true, automations: false, mode: 'always', autoApprove: false, enabled: true, origin: 'hollow', updatedAt: '', ...over });
const login = { email: 'jo@x.com', organization: 'Acme' };
const ai = conn({ id: 'notion_ai', name: 'Notion', transport: 'claudeAi', url: 'https://mcp.notion.com/mcp' });
const at = '2026-10-01T11:55:00Z';

describe('connector capability (next-level P16)', () => {
  it('configured is not verified: enabled with no result never reads green', () => {
    for (const c of [conn({}), conn({ transport: 'http', url: 'https://mcp.example.com' }), ai]) {
      const cap = connectorCapability(c, { now: NOW, login });
      expect(cap.state).toBe('unchecked');
      expect(cap.tone).not.toBe('ok');
      expect(cap.tested).not.toBe('');
    }
    expect(connectorCapability(ai, { now: NOW, login }).repair?.label).toBe('Check now');
  });

  it('a check under one login does not certify another', () => {
    const ok = { id: 'notion_ai', state: 'connected' as const, detail: null, checkedAt: at, login: 'jo@x.com in Acme' };
    const same = connectorCapability(ai, { status: ok, login, now: NOW });
    expect(same).toMatchObject({ state: 'available', label: 'Available', checkedAt: at, login: 'jo@x.com in Acme' });
    expect(evidenceLine(same, NOW)).toBe('checked 5m ago as jo@x.com in Acme');
    // Same email, another organization: a different claude.ai login.
    const other = connectorCapability(ai, { status: ok, login: { email: 'jo@x.com', organization: 'Beta' }, now: NOW });
    expect(other).toMatchObject({ state: 'unchecked', repair: { kind: 'checkAgain' } });
    expect(other.line).toMatch(/Last checked as jo@x.com in Acme, not jo@x.com in Beta/);
    // A result that never said which login it ran as is not evidence for a known one.
    expect(connectorCapability(ai, { status: { ...ok, login: null }, login, now: NOW }).state).toBe('unchecked');
  });

  it('an old check stops counting', () => {
    const old = new Date(NOW - CHECK_TTL_MS - 60_000).toISOString();
    const cap = connectorCapability(ai, { status: { id: 'notion_ai', state: 'connected', detail: null, checkedAt: old, login: 'jo@x.com in Acme' }, login, now: NOW });
    expect(cap.state).toBe('unchecked');
    expect(cap.line).toMatch(/no longer counts/);
  });

  it('chooses a direct repair by state', () => {
    const needs = connectorCapability(ai, { status: { id: 'notion_ai', state: 'needsSignIn', detail: 'OAuth token has been invalidated.', checkedAt: at, login: 'jo@x.com in Acme' }, login, now: NOW });
    expect(needs).toMatchObject({ state: 'needsSignIn', label: 'Needs sign-in', repair: { kind: 'signInClaudeAi', label: 'Sign in' } });
    expect(needs.line).toBe('Connect it on claude.ai as jo@x.com in Acme, then check again.');
    const err = connectorCapability(ai, { status: { id: 'notion_ai', state: 'error', detail: 'claude.ai proxy returned HTTP 502', checkedAt: at, login: 'jo@x.com in Acme' }, login, now: NOW });
    expect(err).toMatchObject({ state: 'unavailable', line: 'claude.ai proxy returned HTTP 502', repair: { kind: 'checkAgain' } });
    expect(connectorCapability(ai, { checking: true, login, now: NOW }).state).toBe('checking');
  });

  it('reports a missing Microsoft 365 permission and how to grant it', () => {
    const m365 = conn({ id: 'hollow_m365', name: 'Microsoft 365', transport: 'builtin', agents: ['claude', 'codex'], builtin: { ready: true, kind: 'm365', line: 'Signed in to Microsoft as jo@x.com.' } });
    expect(connectorCapability(m365, { now: NOW }).state).toBe('unchecked');
    const calendarOnly = connectorCapability(m365, { m365: { signedIn: true, account: 'jo@x.com', files: false, problem: null, checkedAt: at }, now: NOW });
    expect(calendarOnly).toMatchObject({ state: 'permissionMissing', label: 'Permission missing', missing: 'Files.Read.All and Sites.Read.All', repair: { kind: 'grantM365' }, login: 'jo@x.com' });
    expect(calendarOnly.tested).toMatch(/Calendars.Read: granted.*missing/);
    expect(connectorCapability(m365, { m365: { signedIn: true, account: 'jo@x.com', files: true, problem: null, checkedAt: at }, now: NOW }).state).toBe('available');
    expect(connectorCapability(m365, { m365: { signedIn: true, account: 'jo@x.com', files: true, problem: 'refresh failed', checkedAt: at }, now: NOW })).toMatchObject({ state: 'needsSignIn', repair: { label: 'Sign in again' } });
    const signedOut = conn({ ...m365, builtin: { ready: false, kind: 'm365', line: 'Sign in to Microsoft once, here in Hollow.' } });
    expect(connectorCapability(signedOut, { now: NOW })).toMatchObject({ state: 'needsSignIn', repair: { kind: 'signInM365' } });
  });

  it('says where it applies and turns off what no run uses', () => {
    expect(appliesTo(conn({ agents: ['claude', 'codex'], automations: true, mode: 'relevant' }))).toBe('Claude and Codex · chats and automations · only when relevant');
    expect(connectorCapability(conn({ agents: [] }), { now: NOW }).state).toBe('off');
    expect(connectorCapability(conn({ chat: false }), { now: NOW }).state).toBe('off');
    expect(connectorCapability(conn({ enabled: false }), { now: NOW }).label).toBe('Off');
  });
});

describe('connectors for the next turn (next-level P16)', () => {
  const ok = { notion_ai: { id: 'notion_ai', state: 'connected' as const, detail: null, checkedAt: at, login: 'jo@x.com in Acme' } };
  const lib = [
    ai,
    conn({ id: 'plaud', name: 'Plaud', agents: ['claude', 'codex'], mode: 'relevant' }),
    conn({ id: 'linear', name: 'Linear', transport: 'http', url: 'https://mcp.linear.app/mcp', agents: ['codex'] }),
    conn({ id: 'off', name: 'Off one', enabled: false }),
    conn({ id: 'auto', name: 'Automations only', chat: false, automations: true }),
  ];
  const by = (rows: ReturnType<typeof turnConnectors>) => Object.fromEntries(rows.map(r => [r.id, `${r.attach}:${r.label}`]));

  it('separates "available in this conversation" from library configuration', () => {
    expect(by(turnConnectors(lib, { agent: 'claude', surface: 'chat', statuses: ok, login, now: NOW }))).toEqual({
      notion_ai: 'attaches:Available in this conversation',
      plaud: 'relevant:Configured, not checked',
      linear: 'off:Off for this agent',
      off: 'off:Off',
      auto: 'off:Off in chats',
    });
  });
  it('a check under another login does not make it available here', () => {
    expect(by(turnConnectors(lib, { agent: 'claude', surface: 'chat', statuses: ok, login: { email: 'sam@x.com', organization: null }, now: NOW })).notion_ai).toBe('attaches:Configured, not checked');
  });
  it('claude.ai connectors are Claude only; plan mode and unsupported agents get nothing', () => {
    expect(by(turnConnectors(lib, { agent: 'codex', surface: 'chat', statuses: ok, login, now: NOW }))).toMatchObject({ notion_ai: 'off:Off for this agent', plaud: 'relevant:Configured, not checked', linear: 'attaches:Configured, not checked' });
    expect(turnConnectors(lib, { agent: 'claude', surface: 'chat', perm: 'plan', statuses: ok, login, now: NOW }).every(r => r.attach === 'off')).toBe(true);
    expect(turnConnectors(lib, { agent: 'grok', surface: 'chat', statuses: ok, login, now: NOW }).every(r => r.attach === 'off')).toBe(true);
  });
  it('a ready built-in replaces claude.ai’s same service, and a signed-out built-in stays off', () => {
    const plaudAi = conn({ ...ai, id: 'plaud_ai', name: 'Plaud (claude.ai)', url: 'https://mcp.plaud.ai/mcp' });
    const builtin = conn({ id: 'hollow_plaud', name: 'Plaud (Hollow)', transport: 'builtin', mode: 'relevant', builtin: { ready: true, kind: 'plaud', line: 'Uses Plaud’s own sign-in.' } });
    const m365 = conn({ id: 'hollow_m365', name: 'Microsoft 365', transport: 'builtin', builtin: { ready: false, kind: 'm365', line: 'Sign in to Microsoft once, here in Hollow.' } });
    const rows = by(turnConnectors([plaudAi, builtin, m365], { agent: 'claude', surface: 'chat', statuses: ok, login, now: NOW }));
    expect(rows).toEqual({ plaud_ai: 'off:Replaced', hollow_plaud: 'relevant:Configured, not checked', hollow_m365: 'off:Needs sign-in' });
  });
  it('a check Hollow couldn’t run is not a verdict on the connector', () => {
    const failed = { notion_ai: { id: 'notion_ai', state: 'unchecked' as const, detail: 'Claude Code’s stored login is expired or unreadable', checkedAt: at, login: 'jo@x.com in Acme' } };
    const row = turnConnectors([ai], { agent: 'claude', surface: 'chat', statuses: failed, login, now: NOW })[0];
    expect(row.label).toBe('Configured, not checked');
    expect(row.tone).not.toBe('error');
    expect(row.reason).toMatch(/couldn’t run the check \(Claude Code’s stored login is expired or unreadable\)/);
  });
  it('names where each connector comes from, and spells out only the every-turn mode', () => {
    const builtin = conn({ id: 'hollow_m365', name: 'Microsoft 365', transport: 'builtin', mode: 'relevant', builtin: { ready: true, kind: 'm365', line: 'Signed in.' } });
    const rows = turnConnectors([ai, builtin, lib[1]], { agent: 'claude', surface: 'chat', statuses: ok, login, now: NOW });
    expect(rows.map(r => r.source)).toEqual(['claude.ai', 'Hollow', 'This Mac']);
    expect(rows[0].reason).toMatch(/^Attaches to every turn\. /);
    expect(rows[2].reason).not.toMatch(/Attaches/);
  });
});

describe('remote connectors on Hollow’s own sign-in (five asks B2)', () => {
  const zoomAi = conn({ id: 'zoom_for_claude', name: 'Zoom for Claude', transport: 'claudeAi', url: 'https://mcp.zoom.us/mcp/zoom/streamable' });
  const zoom = (remote: { signedIn: boolean; problem: string | null }) => conn({ id: 'zoom', name: 'Zoom', transport: 'http', url: 'https://mcp.zoom.us/mcp/zoom/streamable', agents: ['claude', 'codex'], oauth: true, remote });

  it('says whether Hollow is signed in, and offers the sign-in', () => {
    expect(connectorCapability(zoom({ signedIn: false, problem: null }), { now: NOW })).toMatchObject({ state: 'needsSignIn', repair: { kind: 'signInHollow', label: 'Sign in' } });
    const broken = connectorCapability(zoom({ signedIn: false, problem: 'invalid_grant: revoked' }), { now: NOW });
    expect(broken).toMatchObject({ state: 'needsSignIn', repair: { kind: 'signInHollow', label: 'Sign in again' } });
    expect(broken.line).toContain('invalid_grant: revoked');
    // Signed in is Hollow's sign-in, not a probe of the server: never green.
    const ok = connectorCapability(zoom({ signedIn: true, problem: null }), { now: NOW });
    expect(ok.state).toBe('unchecked');
    expect(ok.line).toMatch(/every Claude account and in Codex/);
  });

  it('a signed-in one replaces claude.ai’s for the same host, for Claude and Codex', () => {
    const by = (rows: ReturnType<typeof turnConnectors>) => Object.fromEntries(rows.map(r => [r.id, `${r.attach}:${r.label}`]));
    const signedIn = [zoomAi, zoom({ signedIn: true, problem: null })];
    expect(by(turnConnectors(signedIn, { agent: 'claude', surface: 'chat', now: NOW }))).toEqual({ zoom_for_claude: 'off:Replaced', zoom: 'attaches:Configured, not checked' });
    expect(by(turnConnectors(signedIn, { agent: 'codex', surface: 'chat', now: NOW })).zoom).toBe('attaches:Configured, not checked');
    const signedOut = [zoomAi, zoom({ signedIn: false, problem: null })];
    expect(by(turnConnectors(signedOut, { agent: 'claude', surface: 'chat', now: NOW }))).toEqual({ zoom_for_claude: 'attaches:Configured, not checked', zoom: 'off:Needs sign-in' });
  });

  it('a ready built-in Microsoft 365 replaces claude.ai’s Microsoft 365', () => {
    const m365Ai = conn({ id: 'microsoft_365', name: 'Microsoft 365', transport: 'claudeAi', url: 'https://microsoft365.mcp.claude.com/mcp' });
    const builtin = conn({ id: 'hollow_m365', name: 'Microsoft 365', transport: 'builtin', agents: ['claude', 'codex'], builtin: { ready: true, kind: 'm365', line: 'Signed in.' } });
    const row = turnConnectors([m365Ai, builtin], { agent: 'claude', surface: 'chat', now: NOW }).find(r => r.id === 'microsoft_365')!;
    expect(row).toMatchObject({ attach: 'off', label: 'Replaced' });
    expect(row.reason).toBe('Hollow’s own Microsoft 365 connector is used instead.');
  });
});

describe('connector reliability (five asks B3)', () => {
  const zoomAi = (keep = false) => conn({ id: 'zoom_for_claude', name: 'Zoom for Claude', transport: 'claudeAi', url: 'https://mcp.zoom.us/mcp', keepClaudeai: keep });
  const zoom = (signedIn: boolean) => conn({ id: 'zoom', name: 'Zoom', transport: 'http', url: 'https://mcp.zoom.us/mcp', agents: ['claude', 'codex'], oauth: true, remote: { signedIn, problem: null } });
  const by = (rows: ReturnType<typeof turnConnectors>) => Object.fromEntries(rows.map(r => [r.id, `${r.attach}:${r.label}`]));

  it('keeping the claude.ai connector leaves Hollow’s out for Claude, not for Codex', () => {
    expect(by(turnConnectors([zoomAi(true), zoom(true)], { agent: 'claude', surface: 'chat', now: NOW }))).toEqual({ zoom_for_claude: 'attaches:Configured, not checked', zoom: 'off:Not used' });
    expect(by(turnConnectors([zoomAi(true), zoom(true)], { agent: 'codex', surface: 'chat', now: NOW })).zoom).toBe('attaches:Configured, not checked');
    expect(by(turnConnectors([zoomAi(false), zoom(true)], { agent: 'claude', surface: 'chat', now: NOW })).zoom_for_claude).toBe('off:Replaced');
  });

  it('says when a claude.ai connector is replaced, or Hollow’s is left out because the claude.ai one is kept', () => {
    expect(worksOn(zoomAi(false), [zoomAi(false), zoom(true)])).toBe('Works on: nothing now. Hollow’s Zoom is used instead');
    expect(worksOn(zoomAi(false), [zoomAi(false), zoom(false)])).toMatch(/login only/);
    expect(worksOn(zoom(true), [zoomAi(true), zoom(true)])).toBe('Works on: Codex only. The claude.ai one is kept for Claude');
    expect(worksOn(zoomAi(true), [zoomAi(true), zoom(true)])).toMatch(/login only/);
  });

  it('a kept row that is off for this surface keeps nothing there', () => {
    const keptChatsOnly = conn({ ...zoomAi(true), chat: true, automations: false });
    const hollow = conn({ ...zoom(true), chat: true, automations: true });
    const rows = turnConnectors([keptChatsOnly, hollow], { agent: 'claude', surface: 'automation', now: NOW });
    expect(rows.find(r => r.id === 'zoom')).toMatchObject({ attach: 'attaches' });
  });

  it('says where each connector works', () => {
    expect(worksOn(zoomAi())).toBe('Works on: Claude Code’s login only. Not on added Claude accounts or in Codex');
    expect(worksOn(zoom(false))).toBe('Works on: nothing until it is signed in');
    expect(worksOn(zoom(true))).toBe('Works on: Claude Code’s login, added Claude accounts, Codex');
    expect(worksOn(conn({ agents: ['codex'] }))).toBe('Works on: Codex');
    expect(worksOn(conn({ enabled: false }))).toBe('Works on: nothing while it is off');
    const m365 = conn({ transport: 'builtin', agents: ['claude', 'codex'], builtin: { ready: false, kind: 'm365', line: 'The Microsoft sign-in stopped working: AADSTS50173' } });
    expect(worksOn(m365)).toBe('Works on: nothing until it is signed in');
  });
});

describe('built-in Plaud (overnight S8)', () => {
  const plaud = (builtin: Partial<NonNullable<ConnectorView['builtin']>>) => conn({ id: 'hollow_plaud', name: 'Plaud', transport: 'builtin', agents: ['claude', 'codex'], mode: 'relevant', builtin: { ready: false, kind: 'plaud', line: 'Sign in to Plaud once, here in Hollow.', ...builtin } });
  it('signed in is configuration, not a check; signed out offers Plaud’s own sign-in', () => {
    expect(connectorCapability(plaud({ ready: true, line: 'Runs Plaud’s MCP server on its own Plaud sign-in.' }), { now: NOW })).toMatchObject({ state: 'unchecked', line: 'Runs Plaud’s MCP server on its own Plaud sign-in.', repair: null });
    const out = connectorCapability(plaud({}), { now: NOW });
    expect(out).toMatchObject({ state: 'needsSignIn', repair: { kind: 'signInPlaud', label: 'Sign in' } });
    expect(out.tested).toMatch(/tokens-mcp\.json/);
  });
  it('without the program it says what to install and offers to look again', () => {
    const missing = connectorCapability(plaud({ programMissing: true, line: 'Needs Plaud’s MCP server on this Mac.' }), { now: NOW });
    expect(missing).toMatchObject({ state: 'notConfigured', line: 'Needs Plaud’s MCP server on this Mac.', repair: { kind: 'checkPlaud', label: 'Check again' } });
  });
  it('your own Plaud wins: the built-in stands aside, is off for every turn, and says so', () => {
    const aside = plaud({ own: 'plaud', line: 'Stands aside: your own Plaud connector “plaud” is in use.' });
    expect(connectorCapability(aside, { now: NOW })).toMatchObject({ state: 'off', line: 'Stands aside: your own Plaud connector “plaud” is in use.', repair: null });
    expect(worksOn(aside)).toBe('Works on: nothing. Your own Plaud connector “plaud” is used instead');
    const mine = conn({ id: 'plaud', name: 'plaud', command: '/opt/homebrew/bin/plaud-mcp', agents: ['claude', 'codex'] });
    const rows = turnConnectors([mine, aside], { agent: 'codex', surface: 'chat', now: NOW });
    expect(rows.map(r => `${r.id}:${r.attach}`)).toEqual(['plaud:attaches', 'hollow_plaud:off']);
    expect(rows[1].reason).toMatch(/your own Plaud connector/);
  });
});

describe('built-in Plaud replaces claude.ai’s Plaud (overnight S8 review)', () => {
  it('a ready built-in Plaud leaves the claude.ai Plaud at mcp.plaud.ai out', () => {
    const plaudAi = conn({ id: 'plaud_ai', name: 'Plaud', transport: 'claudeAi', url: 'https://mcp.plaud.ai/mcp' });
    const builtin = conn({ id: 'hollow_plaud', name: 'Plaud', transport: 'builtin', agents: ['claude', 'codex'], mode: 'relevant', builtin: { ready: true, kind: 'plaud', line: 'Runs Plaud’s MCP server.' } });
    const rows = turnConnectors([plaudAi, builtin], { agent: 'claude', surface: 'chat', now: NOW });
    expect(rows.map(r => `${r.id}:${r.label}`)).toEqual(['plaud_ai:Replaced', 'hollow_plaud:Configured, not checked']);
    expect(worksOn(plaudAi, [plaudAi, builtin])).toBe('Works on: nothing now. Hollow’s Plaud is used instead');
  });
});
