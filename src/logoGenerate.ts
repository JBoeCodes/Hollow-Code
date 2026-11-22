/** Generate a logo (overhaul H2, 2026-11-12): which agent draws, and the
 *  request. Codex first when it has room left, else Claude, else any installed
 *  agent with room; when none has room, the first installed in that order
 *  still gets the request. Room left is Settings, Usage's own reading
 *  (`cfo_view` accounts): an agent with no readings counts as having room.
 *  The backend makes one tool-free call (`generate_logo_options`) and returns
 *  only SVGs that pass the icon check. */
import type { IconTarget } from './customIcons';

export interface LogoOptions { agent: string; options: string[] }
export interface LogoAccount { agent: string; room: boolean }
export interface LogoDeps {
  agentAvailable: () => Promise<string[]>;
  cfoView: () => Promise<{ accounts: LogoAccount[] }>;
  generateLogoOptions: (agent: string, target: IconTarget, request: string, requestId?: string) => Promise<LogoOptions>;
}

const PREFERRED = ['codex', 'claude'];

/** Whether `agent` has room: no readings, or one account with room. */
export function hasRoom(agent: string, accounts: readonly LogoAccount[] | null): boolean {
  const mine = (accounts ?? []).filter(a => a.agent === agent);
  return mine.length === 0 || mine.some(a => a.room);
}

/** The agent to ask, or null when no agent is installed. */
export function chooseLogoAgent(installed: readonly string[], accounts: readonly LogoAccount[] | null): string | null {
  const order = [...PREFERRED.filter(a => installed.includes(a)), ...installed.filter(a => !PREFERRED.includes(a))];
  return order.find(a => hasRoom(a, accounts)) ?? order[0] ?? null;
}

const AGENT_NAMES: Record<string, string> = { codex: 'Codex', claude: 'Claude', antigravity: 'Antigravity', grok: 'Grok' };
export const agentName = (agent: string) => AGENT_NAMES[agent] ?? agent;

/** A fresh id for one request, so closing the dialog can stop it (`cancel_logo_options`).
 *  Safari 15.4 (macOS 11 is supported) has no randomUUID: random words instead. */
export const logoRequestId = () =>
  (globalThis as { crypto?: { randomUUID?: () => string } }).crypto?.randomUUID?.()
  ?? `logo-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}${Math.random().toString(36).slice(2)}`;

/** Pick the agent and ask it for four options, under `requestId` when given. A room-left read that fails
 *  counts as no readings. */
export async function generateLogo(deps: LogoDeps, target: IconTarget, request: string, requestId?: string): Promise<LogoOptions> {
  const installed = await deps.agentAvailable();
  const accounts = await deps.cfoView().then(v => v.accounts, () => null);
  const agent = chooseLogoAgent(installed, accounts);
  if (!agent) throw new Error('No agent is installed on this Mac. Upload an icon instead.');
  return deps.generateLogoOptions(agent, target, request.trim(), requestId);
}
