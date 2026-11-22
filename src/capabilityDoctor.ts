export type CapabilityState = 'live' | 'broken' | 'declined' | 'stale';

import type { BackupHealth, ContextHealth } from './types';
import type { TrackerStatus } from './generated/ipc';
import { trackerHeadline } from './trackerSettings';

export type CapabilityId =
  | 'vault'
  | 'agent-runtime'
  | 'plaud'
  | 'tracker'
  | 'context'
  | 'backup';

export type DeclineableCapabilityId = 'plaud';
export type VerifiableCapabilityId = 'plaud';

export interface CapabilityCheck {
  id: CapabilityId;
  label: string;
  state: CapabilityState;
  detail: string;
  /** The setting's real home: the section and the row that fixes it (not the section the doctor is shown in). */
  fixSection: string;
  fixControl: string;
  fixLabel: string;
  /** Required or actively configured capabilities cannot be silenced. */
  declinable: boolean;
}

export interface CapabilityDoctorPrefs {
  version: 1;
  declined: Partial<Record<DeclineableCapabilityId, string>>;
  lastVerified: Partial<Record<VerifiableCapabilityId, string>>;
}

export interface CapabilityDoctorInputs {
  vaultExists: boolean | null;
  diagnosticsFailed: boolean;
  /** `null` means the honest `agent_available` probe is still pending. */
  agents: string[] | null;
  agentProbeFailed: boolean;
  plaudStatus: 'unknown' | 'ok' | 'needs-auth' | 'authenticating';
  plaudError: boolean;
  plaudLastSuccess: string | null;
  /** Settings' Task tracker row (A14); absent until it has reported, so the
   *  doctor lists the tracker only once Settings has read it. */
  tracker?: TrackerStatus | null;
  /** The personal context's health (PX7); absent until Settings has read it,
   *  or while no personal clone is set, so the doctor lists the line only then. */
  context?: ContextHealth | null;
  backup: BackupHealth | null;
  backupFailed: boolean;
  prefs: CapabilityDoctorPrefs;
  now?: number;
}

export interface CapabilitySummary {
  live: number;
  broken: number;
  declined: number;
  stale: number;
  state: 'live' | 'broken' | 'stale';
  needsAttention: number;
}

export const CAPABILITY_DOCTOR_KEY = 'capabilityDoctor.v1';
export const CAPABILITY_VERIFY_TTL_MS = 7 * 24 * 60 * 60 * 1000;

type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

export const EMPTY_CAPABILITY_DOCTOR_PREFS: CapabilityDoctorPrefs = {
  version: 1,
  declined: {},
  lastVerified: {},
};

/** Where each check's fix lives: the section id and the row's label, both in settingsSections. */
export const CAPABILITY_HOMES: Readonly<Record<CapabilityId, { section: string; control: string }>> = {
  vault: { section: 'system', control: 'Hollow folder' },
  'agent-runtime': { section: 'advanced', control: 'Runtime' },
  plaud: { section: 'connectors', control: 'Recordings' },
  tracker: { section: 'connectors', control: 'Task tracker' },
  context: { section: 'layers', control: 'Personal' },
  backup: { section: 'system', control: 'Backup management' },
};
/** No agent found: the fix is signing one in, which Accounts holds. */
const SIGN_IN_HOME = { section: 'defaults', control: 'Accounts' } as const;

function check(
  id: CapabilityId,
  label: string,
  state: CapabilityState,
  detail: string,
  fixLabel: string,
  declinable = false,
  home: { section: string; control: string } = CAPABILITY_HOMES[id],
): CapabilityCheck {
  return { id, label, state, detail, fixSection: home.section, fixControl: home.control, fixLabel, declinable };
}

function latestIso(...values: (string | null | undefined)[]): string | null {
  let best: { value: string; time: number } | null = null;
  for (const value of values) {
    if (!value) continue;
    const time = new Date(value).getTime();
    if (Number.isFinite(time) && (!best || time > best.time)) best = { value, time };
  }
  return best?.value ?? null;
}

function isFresh(iso: string | null, now: number): boolean {
  if (!iso) return false;
  const at = new Date(iso).getTime();
  return Number.isFinite(at) && now - at <= CAPABILITY_VERIFY_TTL_MS;
}

function applyExplicitDecline(item: CapabilityCheck, prefs: CapabilityDoctorPrefs): CapabilityCheck {
  if (!item.declinable || !prefs.declined[item.id as DeclineableCapabilityId]) return item;
  return {
    ...item,
    state: 'declined',
    detail: 'Declined by you. Hollow will keep this capability quiet until you enable it again.',
  };
}

/**
 * Turn Hollow's existing probes into one deterministic capability doctor.
 * Seven-day verification freshness is deliberately separate from the short
 * data-cache TTLs used by the individual pages.
 */
export function buildCapabilityChecks(input: CapabilityDoctorInputs): CapabilityCheck[] {
  const now = input.now ?? Date.now();
  const prefs = input.prefs;

  const vault = input.diagnosticsFailed
    ? check('vault', 'Vault', 'broken', 'Diagnostics failed, so Hollow cannot verify the configured vault.', 'Open Paths')
    : input.vaultExists === null
      ? check('vault', 'Vault', 'stale', 'Vault diagnostics are still loading.', 'Open Paths')
      : input.vaultExists
        ? check('vault', 'Vault', 'live', 'The configured vault directory is reachable.', 'Open Paths')
        : check('vault', 'Vault', 'broken', 'The configured vault directory does not exist.', 'Repair vault path');

  const runtime = input.agentProbeFailed
    ? check('agent-runtime', 'Agent runtime', 'broken', 'Agent runtime detection failed.', 'Refresh detection')
    : input.agents === null
      ? check('agent-runtime', 'Agent runtime', 'stale', 'Agent runtime detection is still loading.', 'Refresh detection')
      : input.agents.length > 0
        ? check('agent-runtime', 'Agent runtime', 'live', `Available to Hollow: ${input.agents.join(', ')}. Availability does not verify sign-in.`, 'Open System')
        : check('agent-runtime', 'Agent runtime', 'broken', 'No supported agent runtime was found. Install and sign in to an agent, then refresh detection.', 'Set up an agent', false, SIGN_IN_HOME);

  const plaudSuccess = latestIso(input.plaudLastSuccess, prefs.lastVerified.plaud);
  const plaudBase = input.plaudStatus === 'needs-auth'
    ? check('plaud', 'Plaud', 'broken', 'Not signed in. Sign in to Plaud.', 'Connect Plaud', true)
    : input.plaudError
      ? check('plaud', 'Plaud', 'broken', 'The last Plaud sync failed.', 'Connect Plaud', true)
      : input.plaudStatus !== 'ok' || !isFresh(plaudSuccess, now)
        ? check('plaud', 'Plaud', 'stale', 'Last checked more than a week ago.', 'Test Plaud', true)
        : check('plaud', 'Plaud', 'live', 'Checked within the last week.', 'Open Connectors', true);

  const backup = input.backupFailed
    ? check('backup', 'Vault backup', 'broken', 'Could not check your backup.', 'Retry backup check')
    : !input.backup
      ? check('backup', 'Vault backup', 'stale', 'Checking your backup…', 'Check backup')
      : input.backup.status === 'protected'
        ? check('backup', 'Vault backup', 'live', input.backup.summary, 'Review signals')
        : input.backup.status === 'unavailable'
          ? check('backup', 'Vault backup', 'broken', input.backup.summary, 'Retry backup check')
          : check('backup', 'Vault backup', 'stale', input.backup.summary, 'Review backup signals');

  const checks = [
    vault,
    backup,
    runtime,
    applyExplicitDecline(plaudBase, prefs),
  ];
  if (input.tracker !== undefined) checks.push(trackerCheck(input.tracker, now));
  if (input.context) checks.push(contextCheck(input.context));
  return checks;
}

/** "a, b and 3 more": a short list of names for a one-line detail. */
function nameList(names: string[], shown = 3): string {
  const head = names.slice(0, shown).join(', ');
  return names.length > shown ? `${head} and ${names.length - shown} more` : head;
}

/** The personal context (PX7): the vault's and the clone's `me/` must agree
 *  until CX13 retires the copy, and the home file's `Last checked` date and
 *  each memory's `verified_at` / `valid_until` must not have lapsed. Drift is
 *  `stale` (a prompt to look), never `broken`: nothing has stopped working. */
export function contextCheck(h: ContextHealth): CapabilityCheck {
  const problems: string[] = [];
  if (h.meDiffers.length > 0) {
    problems.push(`me/ differs between the vault and the clone: ${nameList(h.meDiffers)}. Hollow reads the vault's copy.`);
  }
  // One home rule for the doctor and recall (ME2): `context_stale::home_staleness`'s label.
  if (h.homeStale) {
    problems.push(`The home file is ${h.homeStale}.`);
  }
  if (h.memoryExpired.length > 0) {
    problems.push(`Past valid_until: ${nameList(h.memoryExpired)}.`);
  }
  if (h.memoryUnverified.length > 0) {
    problems.push(`Not verified in over 180 days: ${nameList(h.memoryUnverified)}.`);
  }
  if (problems.length > 0) {
    return check('context', 'Personal context', 'stale', problems.join(' '), 'Open Context');
  }
  const parts = [h.meCompared ? 'me/ matches in the vault and the clone.' : 'me/ is not in the clone yet.'];
  if (h.homeChecked) parts.push(`Home file checked ${h.homeChecked}.`);
  parts.push(`${h.memoryDated} of ${h.memoryFiles} memories carry a date.`);
  return check('context', 'Personal context', 'live', parts.join(' '), 'Open Context');
}

/** The task tracker (A14): off is a choice, not a fault; a pass within the
 *  last two intervals is live. */
function trackerCheck(t: TrackerStatus | null, now: number): CapabilityCheck {
  const label = t?.displayName ? `Task tracker (${t.displayName})` : 'Task tracker';
  if (!t) return check('tracker', label, 'stale', 'Task tracker settings are still loading.', 'Open Task tracker');
  if (t.state === 'off') return check('tracker', label, 'declined', 'Not set up. Tasks stay local.', 'Set up a task tracker');
  if (t.state === 'needsSetup' || t.state === 'blocked' || t.state === 'error') return check('tracker', label, 'broken', trackerHeadline(t), 'Fix Task tracker');
  const last = t.lastSync ? Date.parse(t.lastSync) : NaN;
  if (t.state === 'ready' && Number.isFinite(last) && now - last <= Math.max(2 * t.pollSeconds, 300) * 1000) {
    return check('tracker', label, 'live', `Synced ${Math.max(0, Math.round((now - last) / 60_000))} min ago.`, 'Open Task tracker');
  }
  return check('tracker', label, 'stale', t.lastSync ? 'No pass in the last two intervals. Sync now to check it.' : 'Set up; waiting for the first pass.', 'Open Task tracker');
}

export function readCapabilityDoctorPrefs(storage: StorageLike): CapabilityDoctorPrefs {
  try {
    const parsed = JSON.parse(storage.getItem(CAPABILITY_DOCTOR_KEY) ?? '') as CapabilityDoctorPrefs;
    if (parsed?.version !== 1 || !parsed.declined || !parsed.lastVerified) return { ...EMPTY_CAPABILITY_DOCTOR_PREFS };
    return { version: 1, declined: { ...parsed.declined }, lastVerified: { ...parsed.lastVerified } };
  } catch {
    return { version: 1, declined: {}, lastVerified: {} };
  }
}

function writePrefs(storage: StorageLike, prefs: CapabilityDoctorPrefs): CapabilityDoctorPrefs {
  storage.setItem(CAPABILITY_DOCTOR_KEY, JSON.stringify(prefs));
  return prefs;
}

export function setCapabilityDeclined(
  storage: StorageLike,
  id: DeclineableCapabilityId,
  declined: boolean,
  at = new Date().toISOString(),
): CapabilityDoctorPrefs {
  const prefs = readCapabilityDoctorPrefs(storage);
  if (declined) prefs.declined[id] = at;
  else delete prefs.declined[id];
  return writePrefs(storage, prefs);
}

export function markCapabilityVerified(
  storage: StorageLike,
  id: VerifiableCapabilityId,
  at = new Date().toISOString(),
): CapabilityDoctorPrefs {
  const prefs = readCapabilityDoctorPrefs(storage);
  prefs.lastVerified[id] = at;
  delete prefs.declined[id];
  return writePrefs(storage, prefs);
}

export function summarizeCapabilities(checks: CapabilityCheck[]): CapabilitySummary {
  const out: CapabilitySummary = {
    live: 0,
    broken: 0,
    declined: 0,
    stale: 0,
    state: 'live',
    needsAttention: 0,
  };
  for (const item of checks) out[item.state] += 1;
  out.needsAttention = out.broken + out.stale;
  out.state = out.broken > 0 ? 'broken' : out.stale > 0 ? 'stale' : 'live';
  return out;
}
