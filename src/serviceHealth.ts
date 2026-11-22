/**
 * Service health by capability (audit H04). The same service name can mean
 * two independent things: Hollow's own task/project sync (the tracker) or
 * recordings sign-in (Plaud), and an agent-facing MCP connector. Both can be
 * true, false or broken independently, so the summary shows them side by side
 * and never lets one imply the other. Pure, unit-tested.
 */
import type { ConnectorStatus, ConnectorView } from './connectorsLibrary';
import type { TrackerStatus } from './generated/ipc';
import { syncWhen, trackerHeadline, trackerHealth } from './trackerSettings';

/** `unverified`: configured and enabled, but nothing has checked it yet.
 *  Configuration alone is never reported as working (next-level P00). */
export type HealthState = 'ok' | 'attention' | 'unverified' | 'off';
export type Capability = { state: HealthState; text: string };
export type ServiceRow = {
  service: string;
  /** What Hollow itself does with the service, or null when it does nothing. */
  hollow: (Capability & { label: string }) | null;
  /** Whether agents can use it, through which connectors. */
  agents: Capability & { connectors: string[] };
};

const SERVICES: { name: string; match: RegExp }[] = [
  { name: 'Plaud', match: /plaud/i },
];

/** Hollow's own task tracker sync (A14), named for the chosen provider. */
function trackerSync(t: TrackerStatus | null | undefined): Capability & { label: string } {
  const label = 'Task & project sync';
  if (!t) return { label, state: 'off', text: 'Status unavailable.' };
  if (t.state === 'off') return { label, state: 'off', text: 'Not set up. Hollow keeps tasks locally.' };
  const health = trackerHealth(t);
  if (health === 'ok') return { label, state: t.lastSync ? 'ok' : 'unverified', text: t.lastSync ? `Synced ${syncWhen(t.lastSync)}.` : 'Set up; no pass has finished yet.' };
  return { label, state: health === 'busy' ? 'unverified' : 'attention', text: trackerHeadline(t) };
}

function plaudSignIn(p: string | undefined): Capability & { label: string } {
  // The Plaud row's own name in Settings, Connectors (P1-20).
  const label = 'Recordings in Hollow';
  if (p === 'ok') return { label, state: 'ok', text: 'Signed in.' };
  if (p === 'authenticating') return { label, state: 'attention', text: 'Signing in…' };
  if (p === 'needs-auth') return { label, state: 'attention', text: 'Needs sign-in.' };
  return { label, state: 'off', text: 'Not checked yet.' };
}

function agentAccess(service: RegExp, connectors: ConnectorView[], statuses: Record<string, ConnectorStatus>): ServiceRow['agents'] {
  const mine = connectors.filter(c => service.test(`${c.name} ${c.url} ${c.command} ${c.serverId}`));
  if (!mine.length) return { state: 'off', text: 'No agent connector. Agents can’t use it directly.', connectors: [] };
  const on = mine.filter(c => c.enabled && c.agents.length && (c.chat || c.automations));
  if (!on.length) return { state: 'off', text: 'Connector added but turned off for every agent.', connectors: mine.map(c => c.name) };
  // A check Hollow couldn't run (`unchecked`) blocks nothing: it's unverified.
  const blocked = on.filter(c => statuses[c.id] && statuses[c.id].state !== 'connected' && statuses[c.id].state !== 'unchecked');
  const agents = [...new Set(on.flatMap(c => c.agents))].map(a => a === 'claude' ? 'Claude' : 'Codex').join(' and ');
  const where = [...new Set(on.flatMap(c => [c.chat && 'chats', c.automations && 'automations']).filter(Boolean))].join(' and ');
  if (blocked.length) return { state: 'attention', text: `${blocked.map(c => c.name).join(', ')} ${statuses[blocked[0].id].state === 'needsSignIn' ? 'needs sign-in' : 'reports an error'}${statuses[blocked[0].id].detail ? `: ${statuses[blocked[0].id].detail}` : ''}.`, connectors: on.map(c => c.name) };
  // Only a connector whose probe answered `connected` counts as checked. An
  // enabled connector with no result is configured, not verified.
  const verified = on.filter(c => statuses[c.id]?.state === 'connected');
  if (!verified.length) return { state: 'unverified', text: `Set up for ${agents} in ${where}. Not checked yet, so access isn’t confirmed.`, connectors: on.map(c => c.name) };
  const unchecked = on.length - verified.length;
  return { state: 'ok', text: `${agents} can use it in ${where}.${unchecked ? ` ${unchecked} more connector${unchecked === 1 ? '' : 's'} not checked.` : ''}`, connectors: on.map(c => c.name) };
}

export function serviceHealth(input: { plaud?: string; tracker?: TrackerStatus | null; connectors: ConnectorView[]; statuses: Record<string, ConnectorStatus> }): ServiceRow[] {
  const rows = SERVICES.map(s => ({
    service: s.name,
    hollow: s.name === 'Plaud' ? plaudSignIn(input.plaud) : null,
    agents: agentAccess(s.match, input.connectors, input.statuses),
  }));
  // The task tracker's row, once Settings has read it: named for the chosen
  // provider, its agent access matched on the provider's id (A14).
  if (input.tracker !== undefined) {
    const t = input.tracker;
    const id = t?.provider ?? null;
    rows.unshift({
      service: t?.displayName ?? 'Task tracker',
      hollow: trackerSync(t),
      agents: id ? agentAccess(new RegExp(id.replace(/[^a-z0-9]/gi, ''), 'i'), input.connectors, input.statuses) : { state: 'off', text: 'No tracker chosen.', connectors: [] },
    });
  }
  return rows;
}
