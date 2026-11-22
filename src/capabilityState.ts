/**
 * One vocabulary for "can this be used?" (next-level P00/P16, 2026-10-01).
 *
 * Settings, the composer and service health used to mix "On", "Connected",
 * "Unknown" and "can use it" for different evidence. The audit reproduced an
 * enabled connector with no probe result being reported as usable. Every
 * surface now maps its evidence onto these eight states, and only a positive
 * probe result (or a built-in whose own check passed) may say `available`.
 * Configuration is never evidence of access. Pure, unit-tested.
 */
/** The pill tone a status renders with (shared with the Connectors library). */
export type StatusTone = 'ok' | 'warn' | 'error' | 'off' | 'neutral' | 'busy';

export type CapabilityState =
  | 'notConfigured'
  | 'unchecked'
  | 'checking'
  | 'available'
  | 'needsSignIn'
  | 'permissionMissing'
  | 'unavailable'
  | 'off';

export const CAPABILITY_LABEL: Record<CapabilityState, string> = {
  notConfigured: 'Not configured',
  unchecked: 'Configured, not checked',
  checking: 'Checking',
  available: 'Available',
  needsSignIn: 'Needs sign-in',
  permissionMissing: 'Permission missing',
  unavailable: 'Unavailable',
  off: 'Off',
};

export const CAPABILITY_TONE: Record<CapabilityState, StatusTone> = {
  notConfigured: 'off',
  unchecked: 'neutral',
  checking: 'busy',
  available: 'ok',
  needsSignIn: 'warn',
  permissionMissing: 'warn',
  unavailable: 'error',
  off: 'off',
};

/** Evidence that a probe produced: when, under which login, what was tested. */
export interface CapabilityEvidence {
  state: CapabilityState;
  /** ISO time of the probe, when one ran. */
  checkedAt?: string | null;
  /** The account/org the probe ran as, when known. A check under one login
   *  never certifies another. */
  login?: string | null;
  /** What the probe actually tested, in words ("claude.ai sign-in"). */
  tested?: string | null;
  detail?: string | null;
}

/** The probe result applies only to the login it ran under. When the current
 *  login differs (or either is unknown while the other is known), the check is
 *  stale for this user: report `unchecked`, never `available`. */
export function forLogin(evidence: CapabilityEvidence, currentLogin: string | null | undefined): CapabilityEvidence {
  if (evidence.state !== 'available') return evidence;
  const ran = evidence.login?.trim().toLowerCase() || null;
  const now = currentLogin?.trim().toLowerCase() || null;
  if (ran === now) return evidence;
  return { ...evidence, state: 'unchecked', detail: ran && now ? `Last checked as ${evidence.login}, not ${currentLogin}.` : 'Checked under a different sign-in.' };
}

/** A short "checked 5m ago" phrase, or "never checked". */
export function checkedPhrase(checkedAt: string | null | undefined, now = Date.now()): string {
  if (!checkedAt) return 'never checked';
  const t = Date.parse(checkedAt);
  if (Number.isNaN(t)) return 'never checked';
  const m = Math.max(0, Math.round((now - t) / 60_000));
  if (m < 1) return 'checked just now';
  if (m < 60) return `checked ${m}m ago`;
  if (m < 60 * 24) return `checked ${Math.round(m / 60)}h ago`;
  return `checked ${Math.round(m / 1440)}d ago`;
}
