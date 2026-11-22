// Settings, Accounts, Agent updates: the wording for one agent
// CLI's row (Rust `agent_cli::AgentCliStatus`).
import type { AgentCliStatus } from './generated/ipc';

export const UPDATE_POLICIES = [
  { value: 'automatic', label: 'Automatic' },
  { value: 'ask', label: 'Ask first' },
  { value: 'off', label: 'Off' },
] as const;

function when(at: string | null, now: Date): string {
  if (!at) return 'never checked';
  const t = new Date(at);
  if (Number.isNaN(t.getTime())) return 'never checked';
  return t.toDateString() === now.toDateString()
    ? `checked ${t.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
    : `checked ${t.toLocaleDateString([], { month: 'short', day: 'numeric' })}`;
}

/** "0.153.4 · 0.162.1 is out · npm (global) · checked 12:40". */
export function versionLine(row: AgentCliStatus, now = new Date()): string {
  if (!row.installed) return 'Not installed.';
  const installed = row.version ?? 'version unknown';
  const latest = !row.canCheck
    ? `${row.label} checks when it updates`
    : row.latest == null ? 'latest unknown'
    : row.updateAvailable ? `${row.latest} is out` : 'up to date';
  const checked = row.canCheck ? ` · ${when(row.lastChecked, now)}` : '';
  return `${installed} · ${latest} · ${row.method}${checked}`;
}

/** What the row's main button does now, or null for none. */
export function primaryAction(row: AgentCliStatus): { kind: 'update' | 'cancel'; label: string } | null {
  if (!row.installed || !row.canUpdate) return null;
  if (row.state === 'waiting') return { kind: 'cancel', label: 'Cancel update' };
  if (row.state !== 'idle') return null;
  if (row.updateAvailable) return { kind: 'update', label: row.latest ? `Update to ${row.latest}` : 'Update' };
  // Antigravity can't check without updating; its updater checks itself.
  if (!row.canCheck) return { kind: 'update', label: 'Update' };
  return null;
}

/** The agents Update all would update: installed, with an update to take,
 *  not busy, and not on the known-bad list (those ask first, one by one). */
export function updatableAgents(rows: readonly AgentCliStatus[]): AgentCliStatus[] {
  return rows.filter(r => primaryAction(r)?.kind === 'update' && !r.latestKnownBad);
}

/** Repo ideas 2026-10-27 #9: the row's warnings from Hollow's shipped list of
 *  versions known to break it (Rust `agent_cli::known_bad`). Warn, never
 *  block: Update still installs a listed version once the person confirms. */
export function knownBadLines(row: AgentCliStatus): string[] {
  const lines: string[] = [];
  if (row.installed && row.version && row.knownBad) lines.push(`${row.label} ${row.version} is known to break Hollow: ${row.knownBad}`);
  if (row.latest && row.latestKnownBad) lines.push(`${row.latest} is known to break Hollow: ${row.latestKnownBad} Automatic won't install it; Update asks first.`);
  return lines;
}

/** The running state in words, for the status line. */
export function stateLine(row: AgentCliStatus): string | null {
  if (row.state === 'checking') return `Checking for a newer ${row.label}…`;
  if (row.state === 'updating') return `Updating ${row.label}… New runs are unavailable. Try again when the update finishes.`;
  return null;
}
