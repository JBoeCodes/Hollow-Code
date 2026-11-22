import { describe, expect, it } from 'vitest';
import { clientSecretInput, savedSecretFate, SUPERHUMAN_URL, approvedMail, AUTO_APPROVE_HINT, AUTO_APPROVE_LABEL, alwaysLoginWarning, alwaysSpreadHint, brandDomain, hollowEquivalent, hollowSignInFor, messagingHost, loginForcingConnectors, loginOverrideLine, loginPinFrom, connectorSummary, iconFor, inputFrom, matchesQuery, parseArgs, secretsFrom, shareableAgents, toggleAgent, type ConnectorView } from './connectorsLibrary';
import { connectorCapability } from './connectorCapability';
import { GITHUB_PRESET, SERVICE_SETUP_DEFAULTS, githubConnection, joinNames } from './connectorsLibrary';

const view: ConnectorView = {
  id: 'mail', name: 'Mail', description: 'Read and send email', transport: 'stdio', command: 'npx', args: ['-y', 'mail-mcp'],
  url: '', serverId: '', iconUrl: '', envKeys: ['API_KEY'], headerKeys: [], agents: ['claude', 'codex'], chat: true, automations: false,
  mode: 'relevant', autoApprove: true, enabled: true, origin: 'hollow', updatedAt: '',
};

describe('connectors library helpers', () => {
  it('starts GitHub with read-only access and no automatic approval or automation', () => {
    expect(new URL(GITHUB_PRESET.url).pathname).toBe('/mcp/readonly');
    expect(SERVICE_SETUP_DEFAULTS).toMatchObject({ agents: ['claude', 'codex'], chat: true, automations: false, mode: 'relevant', autoApprove: false });
  });
  it('manages existing disabled or broader GitHub connections without changing their settings', () => {
    const existing = { ...view, id: 'github', transport: 'http' as const, url: 'https://api.githubcopilot.com/mcp', enabled: false, automations: true };
    expect(githubConnection([existing])).toBe(existing);
    expect(existing).toMatchObject({ enabled: false, automations: true, url: 'https://api.githubcopilot.com/mcp' });
  });
  it('does not mistake an app proxy, lookalike host or unrelated local name for GitHub', () => {
    const http = { ...view, transport: 'http' as const };
    expect(githubConnection([{ ...http, url: 'https://api.githubcopilot.com.evil.test/mcp' }, { ...http, url: 'not a URL' }, { ...http, transport: 'claudeAi', url: GITHUB_PRESET.url }, { ...view, name: 'notgithub' }])).toBeUndefined();
    expect(githubConnection([{ ...view, name: 'GitHub local' }])?.name).toBe('GitHub local');
  });
  it('says the run-without-asking switch only matters in Edits only (#286)', () => {
    // Rust `connector_preapproved`: the switch decides only outside Auto, Plan
    // and Bypass, so the copy must not promise it anywhere else.
    expect(AUTO_APPROVE_LABEL).toContain('Edits only');
    expect(AUTO_APPROVE_HINT).toMatch(/no effect in other modes/);
    expect(AUTO_APPROVE_HINT).toMatch(/in Auto the agent’s own approval decides/);
    expect(AUTO_APPROVE_HINT).not.toMatch(/Leave on for connectors you trust/);
  });
  it('re-saves unchanged with secrets kept', () => {
    const input = inputFrom(view, { enabled: false });
    expect(input.env).toEqual({ API_KEY: null });
    expect(input.enabled).toBe(false);
    expect(input.id).toBe('mail');
  });
  it('toggles agents in a stable order', () => {
    expect(toggleAgent(['codex'], 'claude')).toEqual(['claude', 'codex']);
    expect(toggleAgent(['claude', 'codex'], 'claude')).toEqual(['codex']);
  });
  it('parses args and secrets', () => {
    expect(parseArgs(' -y \n\n pkg ')).toEqual(['-y', 'pkg']);
    expect(secretsFrom([{ key: 'A', value: '', saved: true }, { key: 'B', value: 'x', saved: false }, { key: ' ', value: 'y', saved: false }]))
      .toEqual({ A: null, B: 'x' });
  });
  it('summarizes where and how a connector runs', () => {
    expect(connectorSummary(view)).toBe('npx -y mail-mcp · Claude and Codex · chats · when relevant');
  });
  it('keeps claude.ai connectors Claude only and carries their id', () => {
    const claudeai: ConnectorView = { ...view, transport: 'claudeAi', command: '', args: [], envKeys: [], url: 'https://mcp.mail.superhuman.com/mcp', serverId: 'mcpsrv_1', agents: ['claude'], mode: 'always' };
    expect(shareableAgents('claudeAi')).toEqual(['claude']);
    expect(shareableAgents('http')).toEqual(['claude', 'codex', 'antigravity']);
    expect(toggleAgent(['claude'], 'antigravity')).toEqual(['claude', 'antigravity']);
    expect(toggleAgent(['antigravity', 'claude'], 'claude')).toEqual(['antigravity']);
    expect(joinNames(['Claude', 'Codex', 'Antigravity'])).toBe('Claude, Codex and Antigravity');
    expect(inputFrom(claudeai).serverId).toBe('mcpsrv_1');
    expect(connectorSummary(claudeai)).toBe('claude.ai · https://mcp.mail.superhuman.com/mcp · Claude · chats');
  });
  it('picks an icon: claude.ai’s own, the service favicon, a known brand, or none', () => {
    expect(iconFor({ name: 'X', url: '', iconUrl: 'https://i/x.png', transport: 'claudeAi' })).toBe('https://i/x.png');
    expect(iconFor({ name: 'Mail', url: 'https://mcp.mail.superhuman.com/mcp', transport: 'http' })).toContain('domain=superhuman.com');
    expect(iconFor({ name: 'Microsoft 365', url: 'https://microsoft365.mcp.claude.com/mcp', transport: 'claudeAi' })).toContain('domain=microsoft.com');
    expect(iconFor({ name: 'plaud', url: '', transport: 'stdio' })).toContain('domain=plaud.ai');
    expect(iconFor({ name: 'Local tool', url: 'http://localhost:3000/mcp', transport: 'http' })).toBeNull();
    expect(brandDomain('api.example.co.uk')).toBe('example.co.uk');
  });
  it('says what a connector needs', () => {
    const claudeai: ConnectorView = { ...view, transport: 'claudeAi', agents: ['claude'] };
    expect(connectorCapability(claudeai, { status: { id: 'mail', state: 'needsSignIn', detail: null } }).label).toBe('Needs sign-in');
    expect(connectorCapability(claudeai, { status: { id: 'mail', state: 'connected', detail: null } }).tone).toBe('ok');
    expect(connectorCapability(claudeai, { checking: true }).label).toBe('Checking…');
    const login = { email: 'jo@x.com', organization: 'Acme' };
    expect(connectorCapability(claudeai, { status: { id: 'mail', state: 'needsSignIn', detail: null }, login }).line).toBe('Connect it on claude.ai as jo@x.com in Acme, then check again.');
    // The check must have run as this login (next-level P16): one that names
    // it counts; one that names no login does not.
    expect(connectorCapability(claudeai, { status: { id: 'mail', state: 'connected', detail: null, login: 'jo@x.com' }, login: { email: 'jo@x.com', organization: null } }).line).toBe('Signed in on claude.ai as jo@x.com.');
    expect(connectorCapability(claudeai, { status: { id: 'mail', state: 'connected', detail: null }, login: { email: 'jo@x.com', organization: null } }).label).toBe('Configured, not checked');
    expect(connectorCapability({ ...view, enabled: false }).label).toBe('Off');
    const found = { name: 'Superhuman Mail', source: 'claudeai' as const, transport: 'claudeAi' as const, command: '', url: '', iconUrl: '', inLibrary: false, unsupported: null, note: null };
    expect(matchesQuery(found, 'super')).toBe(true);
    expect(matchesQuery(found, 'claude.ai')).toBe(true);
    expect(matchesQuery(found, 'linear')).toBe(false);
  });
});

describe('login override (2026-10-09)', () => {
  const claudeai = (patch: Partial<ConnectorView>): ConnectorView => ({
    ...view, id: 'notion-ai', name: 'Notion', transport: 'claudeAi', agents: ['claude'], mode: 'always', automations: true, ...patch,
  });
  it('lists only turned-on claude.ai connectors shared with Claude somewhere', () => {
    const list = [claudeai({}), claudeai({ id: 'off', enabled: false }), claudeai({ id: 'nowhere', chat: false, automations: false }), view];
    expect(loginForcingConnectors(list).map(v => v.id)).toEqual(['notion-ai']);
  });
  it('warns on Always, never on When relevant or other transports', () => {
    expect(alwaysLoginWarning(claudeai({}), true)).toContain('a Claude chat and automation it rides on runs on your Claude Code login');
    expect(alwaysLoginWarning(claudeai({ mode: 'relevant' }), true)).toBeNull();
    expect(alwaysLoginWarning({ ...view, mode: 'always' }, true)).toBeNull();
    // No added account: the login is the only account, nothing to warn about.
    expect(alwaysLoginWarning(claudeai({}), false)).toBeNull();
  });
  it('says Always turns spreading off, before and after it is picked (usage spread)', () => {
    expect(alwaysLoginWarning(claudeai({}), true)).toMatch(/^Always turns account spreading off: /);
    expect(alwaysSpreadHint(claudeai({ mode: 'relevant' }), true)).toContain('Always turns account spreading off for Claude');
    expect(alwaysSpreadHint(claudeai({ mode: 'relevant' }), false)).toBeNull();
    // Picked: the row's warning says it, not a second sentence.
    expect(alwaysSpreadHint(claudeai({}), true)).toBeNull();
    expect(alwaysSpreadHint({ ...view, mode: 'always' }, true)).toBeNull();
  });
  it('pins chats to the login only for an Always claude.ai connector on for chats', () => {
    const mail = claudeai({ id: 'mail', name: 'Superhuman Mail', chat: true });
    const login = { email: 'sam@example.com', organization: 'ExampleCorp' };
    expect(loginPinFrom({ connectors: [mail, claudeai({ mode: 'relevant', chat: true })], claudeaiLogin: login }))
      .toEqual({ connectors: ['Superhuman Mail'], login: 'sam@example.com in ExampleCorp' });
    expect(loginPinFrom({ connectors: [claudeai({ chat: false })], claudeaiLogin: login })).toBeNull();
    expect(loginPinFrom({ connectors: [mail], claudeaiLogin: null })).toEqual({ connectors: ['Superhuman Mail'], login: null });
    expect(loginPinFrom({ connectors: [{ ...view, mode: 'always' }] })).toBeNull();
    // Review M1: Hollow's own Superhuman sign-in takes the claude.ai one over,
    // so chats aren't pinned any more (approved mail: even with keepClaudeai).
    const shMail = claudeai({ id: 'superhuman_mail', name: 'Superhuman Mail', chat: true, url: SUPERHUMAN_URL, keepClaudeai: true });
    const hollows = (signedIn: boolean): ConnectorView => ({ ...view, id: 'superhuman', name: 'Superhuman', transport: 'http', oauth: true, url: SUPERHUMAN_URL, agents: ['claude'], chat: true, remote: { signedIn, problem: null } });
    expect(loginPinFrom({ connectors: [shMail, hollows(true)] })).toBeNull();
    expect(alwaysLoginWarning(shMail, true, [shMail, hollows(true)])).toBeNull();
    expect(alwaysLoginWarning(shMail, true, [shMail, hollows(false)])).toMatch(/^Always turns account spreading off/);
    expect(loginPinFrom({ connectors: [shMail, hollows(false)] })).toEqual({ connectors: ['Superhuman Mail'], login: null });
    expect(loginPinFrom({ connectors: [shMail, { ...hollows(true), enabled: false }] })).toEqual({ connectors: ['Superhuman Mail'], login: null });
  });
  it('names the forcing connectors on Settings → Accounts', () => {
    expect(loginOverrideLine([view])).toBeNull();
    const line = loginOverrideLine([claudeai({}), claudeai({ id: 'mail', name: 'Superhuman Mail', mode: 'relevant' })])!;
    expect(line).toContain('Notion (always), Superhuman Mail.');
    expect(line).toContain('One set to Always rides on every run.');
  });
});

// Shared with Rust `remote_auth::tests::MESSAGING_CASES` (it checks this file).
const MESSAGING_CASES: ReadonlyArray<[string, boolean]> = [
  // 10-11 B4: Superhuman Mail's MCP server is an approved mail connector.
  ['https://mcp.mail.superhuman.com/mcp', false],
  ['https://api.superhuman.com/mcp', true],
  ['https://mcp.slack.com/mcp', true],
  ['https://teams-mcp.example.com/x', true],
  ['https://gmail.example.com/mcp', true],
  ['https://outlook-mcp.example.com/mcp', true],
  ['https://mcp.zoom.us/mcp/zoom/streamable', false],
  ['https://api.wisprflow.ai/connect/mcp', false],
];

describe('the approved mail connector (10-11 B4)', () => {
  it('is Superhuman Mail’s MCP server, by exact host only', () => {
    expect(approvedMail(SUPERHUMAN_URL)).toBe(true);
    expect(approvedMail('https://MCP.mail.superhuman.com/other')).toBe(true);
    expect(approvedMail('https://api.superhuman.com/mcp')).toBe(false);
    expect(approvedMail('https://mcp.mail.superhuman.com.evil.example/mcp')).toBe(false);
    expect(messagingHost('https://mcp.mail.superhuman.com.evil.example/mcp')).toBe(true);
  });
});

describe('hollowSignInFor (five asks B2)', () => {
  it('guards mail, Slack and Teams exactly as Rust does', () => {
    for (const [url, want] of MESSAGING_CASES) expect(messagingHost(url), url).toBe(want);
  });
  const row = (over: Partial<ConnectorView>): ConnectorView => ({ id: 'x', name: 'X', description: '', transport: 'claudeAi', command: '', args: [], url: '', serverId: 'mcpsrv_1', iconUrl: '', envKeys: [], headerKeys: [], agents: ['claude'], chat: true, automations: false, mode: 'relevant', autoApprove: true, enabled: true, origin: 'claudeai', updatedAt: '', ...over });
  it('offers it for ordinary claude.ai connectors only, never mail or Anthropic-only addresses', () => {
    const zoom = row({ id: 'zoom_for_claude', url: 'https://mcp.zoom.us/mcp/zoom/streamable' });
    expect(hollowSignInFor(zoom, [zoom])).toEqual({ kind: 'add' });
    expect(hollowSignInFor(row({ url: 'https://mcp.mail.superhuman.com/mcp' }), [])).toEqual({ kind: 'add' });
    expect(hollowSignInFor(row({ url: 'https://api.superhuman.com/mcp' }), [])).toBeNull();
    expect(hollowSignInFor(row({ url: 'https://mcp.slack.com/mcp' }), [])).toBeNull();
    expect(hollowSignInFor(row({ url: 'https://microsoft365.mcp.claude.com/mcp' }), [])).toBeNull();
    expect(hollowSignInFor(row({ transport: 'http', url: 'https://mcp.zoom.us/mcp' }), [])).toBeNull();
    const mine = row({ id: 'zoom', transport: 'http', url: 'https://MCP.zoom.us/other', oauth: true });
    expect(hollowSignInFor(zoom, [zoom, mine])).toEqual({ kind: 'existing', id: 'zoom' });
  });
  it('re-saving keeps the sign-in fields', () => {
    const v = row({ transport: 'http', url: 'https://mcp.zoom.us/mcp', oauth: true, oauthClientId: 'app-1' });
    expect(inputFrom(v)).toMatchObject({ oauth: true, oauthClientId: 'app-1' });
  });
});

describe('hollowEquivalent (five asks B3)', () => {
  const row = (over: Partial<ConnectorView>): ConnectorView => ({ id: 'x', name: 'X', description: '', transport: 'claudeAi', command: '', args: [], url: '', serverId: 'mcpsrv_1', iconUrl: '', envKeys: [], headerKeys: [], agents: ['claude'], chat: true, automations: false, mode: 'relevant', autoApprove: true, enabled: true, origin: 'claudeai', updatedAt: '', ...over });
  it('finds Hollow’s own connector for a claude.ai service, signed in or not', () => {
    const plaudAi = row({ id: 'plaud', url: 'https://mcp.plaud.ai/mcp' });
    const builtin = row({ id: 'hollow_plaud', transport: 'builtin', builtin: { ready: false, kind: 'plaud', line: '' } });
    expect(hollowEquivalent(plaudAi, [plaudAi, builtin])?.id).toBe('hollow_plaud');
    const zoomAi = row({ id: 'zoom_for_claude', url: 'https://mcp.zoom.us/mcp' });
    const zoom = row({ id: 'zoom', transport: 'http', url: 'https://mcp.zoom.us/other', oauth: true });
    expect(hollowEquivalent(zoomAi, [zoomAi, zoom])?.id).toBe('zoom');
    expect(hollowEquivalent(zoomAi, [zoomAi])).toBeNull();
    expect(hollowEquivalent(zoom, [zoomAi, zoom])).toBeNull();
    expect(inputFrom({ ...zoomAi, keepClaudeai: true }).keepClaudeai).toBe(true);
  });
});

describe('client secret on a save (overnight 10-11 Y6)', () => {
  const d = { transport: 'http' as const, oauth: true, oauthClientId: 'app-1', oauthClientSecret: '', clearSecret: false };
  it('a typed secret replaces, Remove clears, otherwise the saved one is kept', () => {
    expect(clientSecretInput({ ...d, oauthClientSecret: ' s3cret ' })).toEqual({ oauthClientSecret: 's3cret' });
    expect(clientSecretInput({ ...d, clearSecret: true })).toEqual({ oauthClientSecret: '' });
    expect(clientSecretInput(d), 'left out keeps it').toEqual({});
  });
  it('the form says what saving does to a saved secret: kept, replaced, cleared, or dropped with a new id or server (B2)', () => {
    const f = { transport: 'http' as const, oauth: true, oauthClientId: 'app-1', oauthClientSecret: '', secretSaved: true, clearSecret: false, url: 'https://zoom.example.com/mcp' };
    const saved = { clientId: 'app-1', url: 'https://zoom.example.com/mcp' };
    expect(savedSecretFate(f, saved)).toBe('kept');
    expect(savedSecretFate({ ...f, url: ' https://zoom.example.com/mcp ' }, saved), 'whitespace aside').toBe('kept');
    expect(savedSecretFate({ ...f, url: 'https://zoom.example.com/v2/mcp' }, saved), 'another path is another address').toBe('dropped');
    expect(savedSecretFate({ ...f, oauthClientId: 'app-2' }, saved)).toBe('dropped');
    expect(savedSecretFate({ ...f, url: 'https://other.example.net/mcp' }, saved)).toBe('dropped');
    expect(savedSecretFate({ ...f, oauth: false }, saved), 'Sign in with Hollow off').toBe('dropped');
    expect(savedSecretFate({ ...f, oauthClientId: ' ' }, saved), 'no client id').toBe('dropped');
    expect(savedSecretFate({ ...f, transport: 'stdio' }, saved), 'a local command').toBe('dropped');
    expect(savedSecretFate({ ...f, oauthClientSecret: 'new' }, saved)).toBe('replaced');
    expect(savedSecretFate({ ...f, clearSecret: true }, saved)).toBe('cleared');
    expect(savedSecretFate({ ...f, secretSaved: false }, saved)).toBe('none');
  });
  it('no client id, no Hollow sign-in or a local command sends no secret', () => {
    expect(clientSecretInput({ ...d, oauthClientId: ' ', oauthClientSecret: 's' })).toEqual({});
    expect(clientSecretInput({ ...d, oauth: false, oauthClientSecret: 's' })).toEqual({});
    expect(clientSecretInput({ ...d, transport: 'stdio', oauthClientSecret: 's' })).toEqual({});
  });
});
