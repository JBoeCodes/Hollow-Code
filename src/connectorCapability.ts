/**
 * Connector capability and repair (next-level P16). Every connector row in
 * Settings and the composer's "connectors for the next turn" list map their
 * evidence through this one function onto the eight capability states
 * (`capabilityState.ts`). Each answer says what was tested, under which
 * login, when, where the connector applies, and the one repair that fits.
 *
 * Rules carried from P00: configuration is never evidence; only a probe that
 * answered (or a built-in sign-in that Hollow itself just checked) may say
 * `available`; a check under one login is stale for another (`forLogin`);
 * a check older than `CHECK_TTL_MS` no longer counts. Pure, unit-tested.
 */
import { CAPABILITY_LABEL, CAPABILITY_TONE, checkedPhrase, forLogin, type CapabilityState, type StatusTone } from './capabilityState';
import {
  AGENT_NAMES, describeLogin, joinNames, hollowEquivalent, hostOf, replacesHost,
  type ClaudeAiLogin, type ConnectorStatus, type ConnectorView, type M365Status, type SharedAgent,
} from './connectorsLibrary';

/** A check older than a day is history, not evidence. */
export const CHECK_TTL_MS = 24 * 60 * 60 * 1000;

/** The direct repair for a state, as the UI offers it. */
export type RepairKind = 'signInClaudeAi' | 'checkAgain' | 'signInM365' | 'grantM365' | 'signInHollow' | 'signInPlaud' | 'checkPlaud';
export interface Repair { kind: RepairKind; label: string; hint: string }

/** Rust `m365_auth::Status` plus when Hollow asked. */
export interface M365Check extends M365Status { checkedAt: string }

export interface ConnectorCapability {
  state: CapabilityState;
  label: string;
  tone: StatusTone;
  /** One sentence: what this means for the user. */
  line: string;
  /** What the check tests (or would test), in words. */
  tested: string;
  checkedAt: string | null;
  /** The account/org the check ran under, when known. */
  login: string | null;
  /** Agents × surfaces: "Claude and Codex · chats · only when relevant". */
  appliesTo: string;
  /** The permission that is missing, when that is the problem. */
  missing: string | null;
  repair: Repair | null;
}

export interface CapabilityInput {
  status?: ConnectorStatus;
  checking?: boolean;
  /** Claude Code's current claude.ai login. */
  login?: ClaudeAiLogin | null;
  /** The last Microsoft 365 sign-in check, for the built-in. */
  m365?: M365Check | null;
  now?: number;
}

const REPAIR: Record<RepairKind, Repair> = {
  signInClaudeAi: { kind: 'signInClaudeAi', label: 'Sign in', hint: 'Opens claude.ai’s connector settings. Sign in there with the login named here, then check again.' },
  checkAgain: { kind: 'checkAgain', label: 'Check again', hint: 'Asks claude.ai whether this connector answers for the current login.' },
  signInM365: { kind: 'signInM365', label: 'Sign in', hint: 'Signs in to Microsoft in your browser, here in Hollow.' },
  grantM365: { kind: 'grantM365', label: 'Grant file access', hint: 'Signs in again asking for Files.Read.All and Sites.Read.All. Your tenant may need an admin to approve them.' },
  signInHollow: { kind: 'signInHollow', label: 'Sign in', hint: 'Signs in to this service in your browser, here in Hollow, once for every Claude account and Codex.' },
  signInPlaud: { kind: 'signInPlaud', label: 'Sign in', hint: 'Asks Plaud’s MCP server to sign in: Plaud’s page opens in your browser. Separate from Recordings in Hollow.' },
  checkPlaud: { kind: 'checkPlaud', label: 'Check again', hint: 'Looks for Plaud’s MCP server and its sign-in again.' },
};

const PLAUD_TESTED = 'Plaud’s MCP server (plaud-mcp) on this Mac, and its own sign-in file (~/.plaud/tokens-mcp.json)';

/** Who gets a connector and where: agents × surfaces × mode. */
export function appliesTo(view: Pick<ConnectorView, 'agents' | 'chat' | 'automations' | 'mode'>): string {
  const agents = joinNames(view.agents.map(a => AGENT_NAMES[a])) || 'No agent';
  const use = [view.chat && 'chats', view.automations && 'automations'].filter(Boolean).join(' and ') || 'nowhere yet';
  return `${agents} · ${use}${view.mode === 'relevant' ? ' · only when relevant' : ''}`;
}

const M365_SCOPES = 'Microsoft sign-in with Calendars.Read';
const M365_FILES = 'Files.Read.All and Sites.Read.All';

function result(state: CapabilityState, view: ConnectorView, rest: Partial<ConnectorCapability> & { line: string; tested: string }): ConnectorCapability {
  return {
    state, label: state === 'checking' ? `${CAPABILITY_LABEL.checking}…` : CAPABILITY_LABEL[state], tone: CAPABILITY_TONE[state],
    checkedAt: null, login: null, missing: null, repair: null, appliesTo: appliesTo(view), ...rest,
  };
}

function stale(checkedAt: string | null | undefined, now: number): boolean {
  if (!checkedAt) return false;
  const t = Date.parse(checkedAt);
  return !Number.isNaN(t) && now - t > CHECK_TTL_MS;
}

/** A connector's capability from the evidence we have. */
export function connectorCapability(view: ConnectorView, input: CapabilityInput = {}): ConnectorCapability {
  const now = input.now ?? Date.now();
  if (!view.enabled) return result('off', view, { line: 'Turned off. Agents don’t get it.', tested: 'Nothing: it is off.' });
  if (!view.agents.length || !(view.chat || view.automations)) {
    return result('off', view, { line: !view.agents.length ? 'Shared with no agent, so no run gets it.' : 'On for neither chats nor automations, so no run gets it.', tested: 'Nothing: no run uses it.' });
  }

  if (view.transport === 'builtin') {
    // The person's own connector for the same service wins (overnight S8).
    if (view.builtin?.own) return result('off', view, { line: view.builtin.line, tested: `Nothing: your own connector “${view.builtin.own}” is used instead.` });
    if (view.builtin?.kind === 'plaud') {
      if (view.builtin.programMissing) return result('notConfigured', view, { line: view.builtin.line, tested: `${PLAUD_TESTED}: the server is missing.`, repair: REPAIR.checkPlaud });
      return view.builtin.ready
        ? result('unchecked', view, { line: view.builtin.line, tested: `${PLAUD_TESTED}: both present. Hollow hasn’t called Plaud with it; an agent’s run does.` })
        : result('needsSignIn', view, { line: view.builtin.line, tested: `${PLAUD_TESTED}: no sign-in yet.`, repair: REPAIR.signInPlaud });
    }
    if (view.builtin?.kind === 'm365') {
      const m = input.m365;
      if (input.checking && !m) return result('checking', view, { line: 'Checking the Microsoft sign-in.', tested: M365_SCOPES });
      if (!m) {
        return view.builtin.ready
          ? result('unchecked', view, { line: `${view.builtin.line} Not checked in this session.`, tested: M365_SCOPES, repair: { ...REPAIR.checkAgain, label: 'Check', hint: 'Checks Hollow’s Microsoft sign-in and which permissions it has.' } })
          : result('needsSignIn', view, { line: view.builtin.line, tested: M365_SCOPES, repair: REPAIR.signInM365 });
      }
      const at = { checkedAt: m.checkedAt, login: m.account };
      if (!m.signedIn) return result('needsSignIn', view, { ...at, line: 'Sign in to Microsoft once, here in Hollow.', tested: M365_SCOPES, repair: REPAIR.signInM365 });
      if (m.problem) return result('needsSignIn', view, { ...at, line: `The Microsoft sign-in stopped working: ${m.problem}`, tested: M365_SCOPES, repair: { ...REPAIR.signInM365, label: 'Sign in again' } });
      if (stale(m.checkedAt, now)) return result('unchecked', view, { ...at, line: `Last checked ${checkedPhrase(m.checkedAt, now).replace(/^checked /, '')}; that no longer counts.`, tested: M365_SCOPES, repair: { ...REPAIR.checkAgain, label: 'Check' } });
      if (!m.files) {
        return result('permissionMissing', view, {
          ...at, missing: M365_FILES,
          line: `Calendar works${m.account ? ` as ${m.account}` : ''}. OneDrive and SharePoint files need ${M365_FILES}, which this sign-in didn’t grant.`,
          tested: `${M365_SCOPES}: granted. ${M365_FILES}: missing.`, repair: REPAIR.grantM365,
        });
      }
      return result('available', view, { ...at, line: view.builtin.ready ? view.builtin.line : `Signed in to Microsoft${m.account ? ` as ${m.account}` : ''}.`, tested: `${M365_SCOPES}, ${M365_FILES}` });
    }
    return result('unchecked', view, { line: 'Built into Hollow.', tested: 'Nothing yet.' });
  }

  if (view.transport === 'claudeAi') {
    const current = input.login ? describeLogin(input.login) : null;
    const tested = 'claude.ai sign-in (an MCP handshake through Anthropic’s proxy)';
    const s = input.status;
    if (!s) {
      return input.checking
        ? result('checking', view, { line: 'Asking claude.ai whether it’s signed in.', tested, login: current })
        : result('unchecked', view, { line: 'Its claude.ai sign-in hasn’t been checked yet.', tested, login: current, repair: { ...REPAIR.checkAgain, label: 'Check now' } });
    }
    const at = { checkedAt: s.checkedAt ?? null, login: s.login ?? current };
    if (s.state === 'connected') {
      const ev = forLogin({ state: 'available', login: s.login ?? null }, current);
      if (ev.state !== 'available') return result('unchecked', view, { ...at, line: `${ev.detail} Check again to confirm it for ${current ?? 'this login'}.`, tested, repair: REPAIR.checkAgain });
      if (stale(s.checkedAt, now)) return result('unchecked', view, { ...at, line: `Last checked ${checkedPhrase(s.checkedAt, now).replace(/^checked /, '')}; that no longer counts.`, tested, repair: REPAIR.checkAgain });
      return result('available', view, { ...at, line: current ? `Signed in on claude.ai as ${current}.` : 'Signed in on claude.ai.', tested });
    }
    if (s.state === 'needsSignIn') {
      return result('needsSignIn', view, { ...at, line: current ? `Connect it on claude.ai as ${current}, then check again.` : 'Sign in to it on claude.ai, then check again.', tested, repair: REPAIR.signInClaudeAi });
    }
    if (s.state === 'unchecked') {
      const why = s.detail ? ` (${s.detail.replace(/[.\s]+$/, '')})` : '';
      return result('unchecked', view, { ...at, line: `Hollow couldn’t run the check${why}. That says nothing about the connector. Check again after a Claude chat has run.`, tested, repair: REPAIR.checkAgain });
    }
    return result('unavailable', view, { ...at, line: s.detail ?? 'claude.ai didn’t answer.', tested, repair: REPAIR.checkAgain });
  }

  // A remote server on Hollow's own sign-in (five asks B2): the sign-in is
  // Hollow's to report; the server itself is still unprobed (P00).
  if (view.transport === 'http' && view.oauth) {
    const tested = 'Hollow’s own sign-in (stored on this Mac). The server itself is only called inside an agent’s run.';
    const r = view.remote;
    if (r?.problem) return result('needsSignIn', view, { line: `Hollow’s sign-in stopped working: ${r.problem}`, tested, repair: { ...REPAIR.signInHollow, label: 'Sign in again' } });
    if (!r?.signedIn) return result('needsSignIn', view, { line: 'Sign in once, here in Hollow. Then it works on every Claude account and in Codex.', tested, repair: REPAIR.signInHollow });
    return result('unchecked', view, { line: 'Signed in with Hollow. Works on every Claude account and in Codex.', tested });
  }
  // Hollow has no probe for local or remote servers it doesn't host: they are
  // configured, not checked (P00). Never a green "On".
  return view.transport === 'http'
    ? result('unchecked', view, { line: 'Remote server, not checked by Hollow. If it needs a sign-in, each agent asks for it once.', tested: 'Nothing: Hollow can’t probe a remote server without an agent starting it.' })
    : result('unchecked', view, { line: 'Runs on this Mac when an agent needs it. Hollow hasn’t tested it.', tested: 'Nothing: the command only starts inside an agent’s run.' });
}

/**
 * Where a connector works (five asks B3): Claude Code's login, added Claude
 * accounts, Codex. A claude.ai connector only on the login; one on Hollow's
 * own sign-in nowhere until signed in, then everywhere it is shared.
 */
export function worksOn(view: ConnectorView, all: readonly ConnectorView[] = []): string {
  if (!view.enabled) return 'Works on: nothing while it is off';
  const signedIn = (c: ConnectorView) => c.transport === 'builtin' ? !!c.builtin?.ready : c.transport === 'http' && c.oauth ? !!c.remote?.signedIn : true;
  if (view.transport === 'claudeAi') {
    // Replaced by Hollow's own connector (review B3 #4).
    const eq = hollowEquivalent(view, all);
    if (eq && eq.enabled && signedIn(eq) && !view.keepClaudeai) return `Works on: nothing now. Hollow’s ${eq.name} is used instead`;
    return 'Works on: Claude Code’s login only. Not on added Claude accounts or in Codex';
  }
  if (view.builtin?.own) return `Works on: nothing. Your own ${view.name} connector “${view.builtin.own}” is used instead`;
  if (!signedIn(view)) return 'Works on: nothing until it is signed in';
  const host = replacesHost(view);
  const kept = !!host && all.some(x => x.transport === 'claudeAi' && x.enabled && x.keepClaudeai && hostOf(x.url) === host);
  if (kept) return view.agents.includes('codex') ? 'Works on: Codex only. The claude.ai one is kept for Claude' : 'Works on: nothing. The claude.ai one is kept for Claude';
  const parts = [
    ...(view.agents.includes('claude') ? ['Claude Code’s login', 'added Claude accounts'] : []),
    ...(view.agents.includes('codex') ? ['Codex'] : []),
  ];
  return parts.length ? `Works on: ${parts.join(', ')}` : 'Works on: no agent yet (share it with Claude or Codex)';
}

/** "checked 5m ago as jo@x.com in Acme", or "never checked". */
export function evidenceLine(c: Pick<ConnectorCapability, 'checkedAt' | 'login'>, now = Date.now()): string {
  const when = checkedPhrase(c.checkedAt, now);
  return c.checkedAt && c.login ? `${when} as ${c.login}` : when;
}

// ── The next turn's connectors (the composer) ──────────────────────────────

/** Agents Hollow can hand connectors to (Rust `agent::capabilities(..).owned_mcp`). */
export const CONNECTOR_AGENTS: readonly string[] = ['claude', 'codex', 'antigravity'];

export type Attachment = 'attaches' | 'relevant' | 'off';
export interface TurnConnector {
  id: string;
  name: string;
  /** Where it comes from, so two connectors with one name (Hollow's own
   *  Microsoft 365 and claude.ai's) can be told apart: "Hollow", "claude.ai", … */
  source: string;
  attach: Attachment;
  /** "Available in this conversation", "Configured, not checked", "Off for this agent", … */
  label: string;
  tone: StatusTone;
  /** Why it does or doesn't attach, and what its capability is. */
  reason: string;
  capability: ConnectorCapability;
}

export interface TurnInput extends Omit<CapabilityInput, 'status'> {
  agent: string;
  surface: 'chat' | 'automation';
  /** The permission mode; `plan` attaches nothing. */
  perm?: string;
  statuses?: Record<string, ConnectorStatus>;
}

const SOURCE: Record<ConnectorView['transport'], string> = { builtin: 'Hollow', claudeAi: 'claude.ai', stdio: 'This Mac', http: 'Remote' };

function host(url: string): string | null {
  try { return new URL(url).hostname.toLowerCase(); } catch { return null; }
}



/**
 * Which library connectors the next turn gets, mirroring Rust
 * `connectors::eligible_with` (enabled, shared with this agent, on for this
 * surface, claude.ai ones Claude only, built-ins only while ready, a ready
 * built-in leaves out claude.ai's same service, nothing in plan mode). Only
 * connectors the user shared through the library are listed; nothing is
 * discovered here.
 */
export function turnConnectors(connectors: readonly ConnectorView[], input: TurnInput): TurnConnector[] {
  const supported = CONNECTOR_AGENTS.includes(input.agent);
  const agentName = AGENT_NAMES[input.agent as SharedAgent] ?? input.agent;
  const surfaceOn = (c: ConnectorView) => input.surface === 'automation' ? c.automations : c.chat;
  // Hosts a signed-in Hollow connector takes over from claude.ai (Rust
  // `connectors::supersedes`): built-in Microsoft 365 and Plaud, and every
  // remote connector on Hollow's own sign-in, unless the claude.ai one is
  // kept ("Use Hollow's instead" off, B3), which leaves Hollow's out instead.
  const usable = (c: ConnectorView) => c.enabled && c.agents.includes(input.agent as SharedAgent) && surfaceOn(c);
  const kept = new Set(connectors.filter(c => c.transport === 'claudeAi' && c.keepClaudeai && usable(c)
    && input.agent === 'claude').map(c => host(c.url)).filter((h): h is string => !!h));
  const replaced = new Map<string, string>();
  for (const c of connectors) {
    if (!usable(c)) continue;
    const ready = c.transport === 'builtin' ? !!c.builtin?.ready : c.transport === 'http' && c.oauth ? !!c.remote?.signedIn : false;
    const h = ready ? replacesHost(c) : null;
    if (h && !kept.has(h)) replaced.set(h, c.name);
  }
  return connectors.map(c => {
    const capability = connectorCapability(c, { ...input, status: input.statuses?.[c.id] });
    const source = SOURCE[c.transport] ?? c.transport;
    const off = (reason: string, label = 'Off for this agent'): TurnConnector => ({ id: c.id, name: c.name, source, attach: 'off', label, tone: 'off', reason, capability });
    if (!c.enabled) return off('Turned off in Settings → Connectors.', 'Off');
    if (!supported) return off(`${agentName} can’t take connectors from Hollow.`);
    if (input.perm === 'plan') return off('Plan mode attaches no connectors: their tools can act outside Hollow.', 'Off in plan mode');
    if (!c.agents.includes(input.agent as SharedAgent)) return off(`Not shared with ${agentName}.`);
    if (c.transport === 'claudeAi' && input.agent !== 'claude') return off('claude.ai connectors work with Claude only.');
    if (!surfaceOn(c)) return off(input.surface === 'chat' ? 'Off in chats.' : 'Off in automations.', input.surface === 'chat' ? 'Off in chats' : 'Off in automations');
    if (c.transport === 'builtin' && !c.builtin?.ready) return off(`${capability.line}`, capability.label);
    if (c.transport === 'http' && c.oauth && !c.remote?.signedIn) return off(`${capability.line}`, capability.label);
    const keptHost = replacesHost(c);
    if (keptHost && kept.has(keptHost)) return off('You keep the claude.ai connector for this service (Settings → Connectors → Use Hollow’s instead).', 'Not used');
    const by = c.transport === 'claudeAi' ? replaced.get(host(c.url) ?? '') : undefined;
    if (by) return off(`Hollow’s own ${by} connector is used instead.`, 'Replaced');
    const attach: Attachment = c.mode === 'relevant' ? 'relevant' : 'attaches';
    // "When the request needs it" is the usual mode, so only the exception
    // is spelled out on the row.
    const reason = attach === 'attaches' ? `Attaches to every turn. ${capability.line}` : capability.line;
    if (capability.state === 'available') return { id: c.id, name: c.name, source, attach, label: 'Available in this conversation', tone: 'ok', reason, capability };
    return { id: c.id, name: c.name, source, attach, label: capability.label, tone: capability.tone, reason, capability };
  });
}
