// Types and pure helpers for the Connectors library (Rust `connectors.rs`).
// Dependency-free so the node vitest suite can load it. Capability and
// repair (what a row's pill says) live in `connectorCapability.ts`.
export type { StatusTone } from './capabilityState';

/** `claudeAi`: a connector added on claude.ai, reached through Anthropic's
 *  proxy. Claude only; its runs use Claude Code's own login. */
export type ConnectorTransport = 'stdio' | 'http' | 'claudeAi' | 'builtin';
export type ConnectorMode = 'always' | 'relevant';
export type SharedAgent = 'claude' | 'codex' | 'antigravity';

/** Sam's service setup is a draft, never a sign-in or a permission change. */
export const GITHUB_PRESET = {
  name: 'GitHub', url: 'https://api.githubcopilot.com/mcp/readonly', oauth: false,
  description: 'Read GitHub repositories, issues and pull requests.',
  setup: 'The preset uses GitHub’s read-only server. Add an Authorization header with Bearer followed by an appropriately scoped token. Organization policy may require approval. Review any address or access changes before saving.',
} as const;
export const SERVICE_SETUP_DEFAULTS = { agents: ['claude', 'codex'] as SharedAgent[], chat: true, automations: false, mode: 'relevant' as const, autoApprove: false, enabled: true };
export function githubConnection(views: readonly ConnectorView[]): ConnectorView | undefined {
  return views.find(c => {
    if (c.transport !== 'http') return false;
    try { return new URL(c.url).hostname === 'api.githubcopilot.com'; } catch { return false; }
  }) ?? views.find(c => c.transport === 'stdio' && /\bgithub\b/i.test(c.name));
}

/** A library connector as IPC returns it: secrets reduced to their names. */
export interface ConnectorView {
  id: string; name: string; description: string; transport: ConnectorTransport;
  command: string; args: string[]; url: string; serverId: string; iconUrl: string; envKeys: string[]; headerKeys: string[];
  agents: SharedAgent[]; chat: boolean; automations: boolean; mode: ConnectorMode;
  autoApprove: boolean; enabled: boolean; origin: string; updatedAt: string;
  /** A built-in connector's sign-in (Rust `builtin_connectors::BuiltinState`). */
  builtin?: BuiltinState;
  /** A remote connector Hollow signs in to itself (Rust `remote_auth.rs`),
   *  served to Claude and Codex through `hollow remote-mcp`. */
  oauth?: boolean;
  /** The client id of an app the person registered, when the server doesn't
   *  let Hollow register itself. */
  oauthClientId?: string;
  /** A client secret is saved for that app (never the secret; overnight 10-11 Y6). */
  oauthClientSecretSet?: boolean;
  /** Hollow's own sign-in for an `oauth` connector (never a token). */
  remote?: RemoteStatus;
  /** A claude.ai connector kept over Hollow's own for the same service
   *  ("Use Hollow's instead" off, five asks B3). */
  keepClaudeai?: boolean;
}

/** Rust `remote_auth::Status`. */
export interface RemoteStatus { signedIn: boolean; problem: string | null }

/** `builtin`: Microsoft 365 on Hollow's own sign-in, or Plaud's own
 *  MCP server on its own sign-in (overnight S8), so it works on every Claude
 *  account and in Codex. `programMissing`: Plaud's server isn't installed.
 *  `own`: the person's own connector for the same service, which wins; the
 *  built-in then stands aside (`ready` is false). */
export interface BuiltinState { ready: boolean; kind: 'm365' | 'plaud'; line: string; programMissing?: boolean; own?: string }

/** Rust `plaud_mcp_auth::Status`. */
export interface PlaudMcpStatus { signedIn: boolean; installed: boolean; already: boolean }

/** Rust `m365_auth::Status`: the built-in Microsoft 365 sign-in. */
export interface M365Status { signedIn: boolean; account: string | null; files: boolean; problem: string | null }
export interface DiscoveredConnector {
  name: string; source: 'claude' | 'codex' | 'claudeai'; transport: ConnectorTransport;
  command: string; url: string; iconUrl: string; inLibrary: boolean; unsupported: string | null;
  /** Something to do first (a claude.ai connector still needing sign-in). */
  note: string | null;
}
export interface ConnectorsLibraryView {
  connectors: ConnectorView[]; discovered: DiscoveredConnector[]; problems: string[];
  /** Why claude.ai connectors could not be listed, if they couldn't. */
  claudeaiError: string | null;
  /** Whose claude.ai connectors these are: Claude Code's own login. */
  claudeaiLogin?: ClaudeAiLogin | null;
}

/** Rust `connectors::ClaudeAiLogin`. */
export interface ClaudeAiLogin { email: string; organization: string | null }

/** `jo@x.com in Acme`, or just the email. */
export function describeLogin(login: ClaudeAiLogin): string {
  return login.organization ? `${login.email} in ${login.organization}` : login.email;
}

/** Save input. A null secret value keeps the stored one. */
export interface ConnectorInput {
  id?: string; name: string; description: string; transport: ConnectorTransport;
  command: string; args: string[]; url: string; serverId?: string;
  env: Record<string, string | null>; headers: Record<string, string | null>;
  agents: SharedAgent[]; chat: boolean; automations: boolean; mode: ConnectorMode;
  autoApprove: boolean; enabled: boolean;
  oauth?: boolean; oauthClientId?: string; keepClaudeai?: boolean;
  /** Left out keeps the saved secret (unless the client id changes); `''` clears it. */
  oauthClientSecret?: string;
}

/** The secret part of a save: a typed secret replaces, Remove clears, and
 *  otherwise the field is left out so the saved one is kept (Rust drops it
 *  when the client id changes). Overnight 10-11 Y6. */
export function clientSecretInput(d: { transport: ConnectorTransport; oauth: boolean; oauthClientId: string; oauthClientSecret: string; clearSecret: boolean }): { oauthClientSecret?: string } {
  if (d.transport !== 'http' || !d.oauth || !d.oauthClientId.trim()) return {};
  const typed = d.oauthClientSecret.trim();
  if (typed) return { oauthClientSecret: typed };
  return d.clearSecret ? { oauthClientSecret: '' } : {};
}

/** What saving does to a saved client secret (B2, #537 review): the backend
 *  keeps it only with the same client id and the same address, on Hollow's
 *  sign-in, so the form must stop saying "Saved" once an edit will drop it.
 *  `replaced`: a new one is typed; `cleared`: Remove secret; `dropped`: the
 *  id or address changed, or the connector no longer signs in with an app.
 *  Every outcome but `kept` and `none` signs the connector out. */
export type SavedSecretFate = 'none' | 'kept' | 'replaced' | 'cleared' | 'dropped';
export function savedSecretFate(d: { transport: ConnectorTransport; oauth: boolean; oauthClientId: string; oauthClientSecret: string; secretSaved: boolean; clearSecret: boolean; url: string }, saved: { clientId: string; url: string }): SavedSecretFate {
  if (!d.secretSaved) return 'none';
  const withApp = d.transport === 'http' && d.oauth && !!d.oauthClientId.trim();
  if (withApp && d.oauthClientSecret.trim()) return 'replaced';
  if (withApp && d.clearSecret) return 'cleared';
  return withApp && d.oauthClientId.trim() === saved.clientId.trim() && d.url.trim() === saved.url.trim() ? 'kept' : 'dropped';
}

/** "Claude", "Claude and Codex", "Claude, Codex and Antigravity". */
export function joinNames(names: string[]): string {
  return names.length > 2 ? `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}` : names.join(' and ');
}

export const AGENT_NAMES: Record<SharedAgent, string> = { claude: 'Claude', codex: 'Codex', antigravity: 'Antigravity' };
export const SOURCE_NAMES: Record<string, string> = { claude: 'Claude Code', codex: 'Codex', claudeai: 'claude.ai', hollow: 'Hollow' };

/** The agents a connector can be shared with: claude.ai ones are Claude only. */
export function shareableAgents(transport: ConnectorTransport): SharedAgent[] {
  return transport === 'claudeAi' ? ['claude'] : ['claude', 'codex', 'antigravity'];
}

/** The input that re-saves a connector unchanged, with `patch` applied.
 *  Secret names map to null so stored values are kept. */
export function inputFrom(view: ConnectorView, patch: Partial<ConnectorInput> = {}): ConnectorInput {
  return {
    id: view.id, name: view.name, description: view.description, transport: view.transport,
    command: view.command, args: [...view.args], url: view.url, serverId: view.serverId,
    env: Object.fromEntries(view.envKeys.map(k => [k, null])),
    headers: Object.fromEntries(view.headerKeys.map(k => [k, null])),
    agents: [...view.agents], chat: view.chat, automations: view.automations, mode: view.mode,
    autoApprove: view.autoApprove, enabled: view.enabled,
    oauth: view.oauth ?? false, oauthClientId: view.oauthClientId ?? '', keepClaudeai: view.keepClaudeai ?? false,
    ...patch,
  };
}

/** The lower-cased host of a URL, or null. */
export function hostOf(url: string): string | null {
  try { return new URL(url).hostname.toLowerCase(); } catch { return null; }
}

/** Mail servers Alex approved as Hollow connectors, by exact host (Rust
 *  `remote_auth::APPROVED_MAIL_HOSTS`, 10-11 B4). */
export const APPROVED_MAIL_HOSTS = ['mcp.mail.superhuman.com'];
/** Superhuman Mail's MCP server (the library's one-click tile). */
export const SUPERHUMAN_URL = 'https://mcp.mail.superhuman.com/mcp';

/** An approved mail connector: signed in by Hollow, its writes ask on a card. */
export function approvedMail(url: string): boolean {
  return /^https:\/\//i.test(url.trim()) && APPROVED_MAIL_HOSTS.includes(hostOf(url) ?? '');
}

/** Mail, Slack or Teams: Hollow never signs in to one, except an approved
 *  mail connector (Rust `remote_auth::messaging_host`). */
export function messagingHost(url: string): boolean {
  if (approvedMail(url)) return false;
  const words = ['mail', 'gmail', 'outlook', 'slack', 'teams', 'superhuman'];
  return (hostOf(url) ?? '').split(/[.-]/).some(label => words.includes(label));
}

/** Served by Anthropic for claude.ai only (Rust `remote_auth::anthropic_hosted`). */
export function anthropicHosted(url: string): boolean {
  const h = hostOf(url) ?? '';
  return h === 'claude.ai' || h.endsWith('.claude.ai') || h === 'claude.com' || h.endsWith('.claude.com') || h.endsWith('.anthropic.com');
}

/** The claude.ai host each built-in replaces (Rust `Builtin::supersedes_host`). */
export const BUILTIN_HOST: Partial<Record<BuiltinState['kind'], string>> = { m365: 'microsoft365.mcp.claude.com', plaud: 'mcp.plaud.ai' };

/** The host a Hollow connector takes over from claude.ai (Rust
 *  `connectors::supersedes`), whether or not it is signed in yet. */
export function replacesHost(c: ConnectorView): string | null {
  if (c.transport === 'builtin' && c.builtin) return BUILTIN_HOST[c.builtin.kind] ?? null;
  if (c.transport === 'http' && c.oauth) return hostOf(c.url);
  return null;
}

/** Hollow's own connector for a claude.ai connector's service, if the
 *  library has one (five asks B3: the "Use Hollow's instead" switch). */
export function hollowEquivalent(c: ConnectorView, all: readonly ConnectorView[]): ConnectorView | null {
  if (c.transport !== 'claudeAi') return null;
  const host = hostOf(c.url);
  return all.find(x => x.id !== c.id && host !== null && replacesHost(x) === host) ?? null;
}

/**
 * Whether a claude.ai connector can move to Hollow's own sign-in (Rust
 * `connectors::use_hollow_sign_in_in`): `existing` when the library already
 * has one for its host, `add` when it can, null when it can't (not claude.ai,
 * a mail/Slack/Teams server, or one only Anthropic serves).
 */
export function hollowSignInFor(c: ConnectorView, all: readonly ConnectorView[]): { kind: 'existing'; id: string } | { kind: 'add' } | null {
  if (c.transport !== 'claudeAi' || messagingHost(c.url) || anthropicHosted(c.url)) return null;
  const host = hostOf(c.url);
  const existing = all.find(x => x.transport === 'http' && x.oauth && hostOf(x.url) === host);
  return existing ? { kind: 'existing', id: existing.id } : { kind: 'add' };
}

/** Toggle one agent in a sharing list, keeping the claude, codex, antigravity order. */
export function toggleAgent(agents: SharedAgent[], agent: SharedAgent): SharedAgent[] {
  const next = agents.includes(agent) ? agents.filter(a => a !== agent) : [...agents, agent];
  return (['claude', 'codex', 'antigravity'] as SharedAgent[]).filter(a => next.includes(a));
}

/** One argument per non-empty line. */
export function parseArgs(text: string): string[] {
  return text.split('\n').map(s => s.trim()).filter(Boolean);
}

/** Secret rows → save map: an empty value on an existing key keeps it (null). */
export function secretsFrom(rows: { key: string; value: string; saved: boolean }[]): Record<string, string | null> {
  const out: Record<string, string | null> = {};
  for (const row of rows) {
    const key = row.key.trim();
    if (!key) continue;
    out[key] = row.value === '' && row.saved ? null : row.value;
  }
  return out;
}


/** A short "where it runs" line for a connector row. */
export function connectorSummary(view: ConnectorView): string {
  const where = view.transport === 'claudeAi' ? `claude.ai · ${view.url}`
    : view.transport === 'http' ? view.url : [view.command, ...view.args].join(' ');
  const use = [view.chat && 'chats', view.automations && 'automations'].filter(Boolean).join(' and ') || 'nothing yet';
  const agents = joinNames(view.agents.map(a => AGENT_NAMES[a])) || 'no agent';
  return `${where} · ${agents} · ${use}${view.mode === 'relevant' ? ' · when relevant' : ''}`;
}

/** The "run without asking" switch's copy. Rust `mode_contract::connector_preapproved`
 *  preapproves every connector in Bypass, none in Auto (the agent's own
 *  classifier or reviewer decides each call, #286) or Plan, and the
 *  `autoApprove` ones otherwise. So the switch only changes anything in Edits
 *  only, and the copy says so, and what the other modes do. */
export const AUTO_APPROVE_LABEL = 'Run its tools without asking in Edits only';
export const AUTO_APPROVE_HINT =
  'Hollow’s agents run without a terminal, so in Edits only a tool that needs approval is refused unless this is on. '
  + 'It has no effect in other modes: in Auto the agent’s own approval decides each call (an automation whose allowed tools name the connector runs it), in Bypass every connector runs, and Plan preapproves none.';

/** Whether a library connector is ready (Rust `connectors::ConnectorStatus`).
 *  Only claude.ai connectors have one: Hollow asks Anthropic's proxy. */
export interface ConnectorStatus {
  /** `unchecked`: Hollow couldn't run the check (no readable Claude Code
   *  login, no network); it says nothing about the connector itself. */
  id: string; state: 'connected' | 'needsSignIn' | 'error' | 'unchecked'; detail: string | null;
  /** When the probe ran (Rust stamps it; older builds and the cache may not). */
  checkedAt?: string | null;
  /** The claude.ai login it ran as (`describeLogin` form). A check under one
   *  login never certifies another (next-level P16). */
  login?: string | null;
}

/** Where claude.ai connectors are signed in to. */
export const CLAUDE_AI_CONNECTORS_URL = 'https://claude.ai/customize/connectors';

/** Brand domains for connectors whose URL says nothing (local commands),
 *  matched against words in the name. */
const KNOWN_DOMAINS: Record<string, string> = {
  plaud: 'plaud.ai', notion: 'notion.so', linear: 'linear.app', github: 'github.com', gitlab: 'gitlab.com',
  slack: 'slack.com', superhuman: 'superhuman.com', gmail: 'mail.google.com', google: 'google.com',
  figma: 'figma.com', blender: 'blender.org', stripe: 'stripe.com', airtable: 'airtable.com', asana: 'asana.com',
  jira: 'atlassian.com', atlassian: 'atlassian.com', zoom: 'zoom.us', dropbox: 'dropbox.com', canva: 'canva.com',
  outlook: 'outlook.com', microsoft: 'microsoft.com', teams: 'microsoft.com', sentry: 'sentry.io',
  postgres: 'postgresql.org', supabase: 'supabase.com', vercel: 'vercel.com', granola: 'granola.ai',
};

/** The registrable part of a host, roughly: `mcp.mail.superhuman.com` → `superhuman.com`. */
export function brandDomain(host: string): string {
  const parts = host.toLowerCase().split('.').filter(Boolean);
  if (parts.length <= 2) return parts.join('.');
  const tail2 = parts.slice(-2).join('.');
  // Two-part public suffixes keep one more label.
  return /^(co|com|org|net|ac|gov)\.[a-z]{2}$/.test(tail2) ? parts.slice(-3).join('.') : tail2;
}

export function faviconFor(domain: string): string {
  return `https://www.google.com/s2/favicons?domain=${encodeURIComponent(domain)}&sz=64`;
}

/** An icon URL for a connector: claude.ai's own, else the service's favicon
 *  from its URL (skipping localhost and Anthropic's hosts), else a known
 *  brand in its name. `null` means show a lettered tile. */
export function iconFor(c: { name: string; url: string; iconUrl?: string; transport: ConnectorTransport }): string | null {
  if (c.iconUrl) return c.iconUrl;
  if (c.transport !== 'stdio' && c.url) {
    try {
      const host = new URL(c.url).hostname;
      if (host && !/^(localhost|127\.0\.0\.1)$/.test(host) && !/(^|\.)(anthropic|claude)\.com$/.test(host)) return faviconFor(brandDomain(host));
    } catch { /* fall through to the name */ }
  }
  for (const word of c.name.toLowerCase().split(/[^a-z0-9]+/)) {
    if (KNOWN_DOMAINS[word]) return faviconFor(KNOWN_DOMAINS[word]);
  }
  return null;
}

/** Case-insensitive name/source filter for the "Add" gallery. */
export function matchesQuery(d: DiscoveredConnector, query: string): boolean {
  const q = query.trim().toLowerCase();
  return !q || d.name.toLowerCase().includes(q) || (SOURCE_NAMES[d.source] ?? d.source).toLowerCase().includes(q);
}

/** Login override (2026-10-09): the library connectors that put a Claude
 *  run on Claude Code's own login, never on an added account
 *  (`agent.rs` → `claude_accounts::apply_login`): every turned-on claude.ai
 *  connector shared with Claude for chats or automations. */
export function loginForcingConnectors(views: ConnectorView[]): ConnectorView[] {
  return views.filter(v => v.transport === 'claudeAi' && v.enabled && v.agents.includes('claude') && (v.chat || v.automations) && !takenOver(v, views));
}

/** A claude.ai connector a signed-in Hollow connector for its host takes
 *  over (Rust `connectors::eligible_with`): the run gets Hollow's and never
 *  goes on the login for it. "Use Hollow's instead" off keeps the claude.ai
 *  one, except for approved mail, where Hollow's always wins once signed in. */
export function takenOver(c: ConnectorView, all: readonly ConnectorView[]): boolean {
  if (c.transport !== 'claudeAi' || (c.keepClaudeai && !approvedMail(c.url))) return false;
  const host = hostOf(c.url);
  const ready = (x: ConnectorView) => (x.transport === 'builtin' ? !!x.builtin?.ready : !!x.remote?.signedIn);
  return host !== null && all.some(x => x.id !== c.id && x.enabled && x.agents.includes('claude') && x.chat && replacesHost(x) === host && ready(x));
}

/** The warning on a claude.ai connector set to "Always": it rides on every
 *  Claude run it applies to, so each of them goes on the login. Null for
 *  any other connector, and while no Claude account is added (`hasAdded`):
 *  then the login is the only account anyway. */
export function alwaysLoginWarning(view: ConnectorView, hasAdded: boolean, all: readonly ConnectorView[] = [view]): string | null {
  if (!hasAdded || !loginForcingConnectors([view]).length || view.mode !== 'always' || takenOver(view, all)) return null;
  const where = [view.chat && 'chat', view.automations && 'automation'].filter(Boolean).join(' and ');
  return `Always turns account spreading off: a Claude ${where} it rides on runs on your Claude Code login, not on your added accounts, and never moves to another account when the login runs low.`;
}

/** Under a claude.ai connector's Attach choice, before Always is picked
 *  (Spread usage across accounts, 2026-10-21). Null without added accounts. */
export function alwaysSpreadHint(view: ConnectorView, hasAdded: boolean, all: readonly ConnectorView[] = [view]): string | null {
  // Once Always is picked, the row's warning (`alwaysLoginWarning`) says it.
  if (!hasAdded || view.mode === 'always' || !loginForcingConnectors([view]).length || takenOver(view, all)) return null;
  return 'Always turns account spreading off for Claude: every run it rides on uses Claude Code’s login.';
}

/** What pins every Claude chat to the login (`claudeAccounts.LoginPin`):
 *  the claude.ai connectors set to Always for chats, and whose login that
 *  is. Null when none is. */
export function loginPinFrom(library: Pick<ConnectorsLibraryView, 'connectors' | 'claudeaiLogin'>): { connectors: string[]; login: string | null } | null {
  const always = loginForcingConnectors(library.connectors).filter(v => v.mode === 'always' && v.chat);
  if (!always.length) return null;
  return { connectors: always.map(v => v.name), login: library.claudeaiLogin ? describeLogin(library.claudeaiLogin) : null };
}

/** Settings, Accounts' line naming the connectors that keep runs on the
 *  login. Null when there are none. */
export function loginOverrideLine(views: ConnectorView[]): string | null {
  const forcing = loginForcingConnectors(views);
  if (!forcing.length) return null;
  const names = forcing.map(v => `${v.name}${v.mode === 'always' ? ' (always)' : ''}`).join(', ');
  const always = forcing.some(v => v.mode === 'always');
  return `Runs that carry a claude.ai connector use Claude Code's own login and never move between accounts: ${names}.${always ? ' One set to Always rides on every run.' : ''}`;
}
