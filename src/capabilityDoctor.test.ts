import { describe, expect, it } from 'vitest';
import type { TrackerStatus } from './generated/ipc';
import type { ContextHealth } from './types';
import {
  CAPABILITY_VERIFY_TTL_MS,
  buildCapabilityChecks,
  markCapabilityVerified,
  readCapabilityDoctorPrefs,
  setCapabilityDeclined,
  summarizeCapabilities,
  type CapabilityDoctorInputs,
} from './capabilityDoctor';

const NOW = Date.parse('2026-07-13T12:00:00Z');
const RECENT = new Date(NOW - CAPABILITY_VERIFY_TTL_MS + 1).toISOString();

function input(over: Partial<CapabilityDoctorInputs> = {}): CapabilityDoctorInputs {
  return {
    vaultExists: true,
    diagnosticsFailed: false,
    agents: ['claude', 'codex'],
    agentProbeFailed: false,
    plaudStatus: 'ok',
    plaudError: false,
    plaudLastSuccess: RECENT,
    backup: { status: 'protected', summary: 'Configured.', signals: [] },
    backupFailed: false,
    prefs: { version: 1, declined: {}, lastVerified: {} },
    now: NOW,
    ...over,
  };
}

function state(inputs: Partial<CapabilityDoctorInputs>, id: string) {
  return buildCapabilityChecks(input(inputs)).find(c => c.id === id)?.state;
}

describe('capability doctor', () => {
  it('classifies recently verified capabilities as live', () => {
    const checks = buildCapabilityChecks(input());
    expect(checks).toHaveLength(4);
    expect(checks.every(c => c.state === 'live')).toBe(true);
    expect(summarizeCapabilities(checks)).toMatchObject({ live: 4, needsAttention: 0, state: 'live' });
  });

  it('lists the task tracker once Settings has read it', () => {
    const t: TrackerStatus = { state: 'ready', provider: 'linear', displayName: 'Linear', providers: [], teamId: 'x', teamKey: 'ENG', personalTeamId: null, personalTeamKey: null, transport: 'graphql',
      pollSeconds: 120, trackAgentWork: false, fridayStatus: false, postStatusUpdates: false, key: 'saved', missing: null, blockedBy: null, lastSync: new Date(NOW - 60_000).toISOString(), nextSync: null,
      lastError: null, rate: null, outbox: 0, problems: [], restartNeeded: false, engine: true };
    expect(buildCapabilityChecks(input()).some(c => c.id === 'tracker')).toBe(false);
    expect(state({ tracker: null }, 'tracker')).toBe('stale');
    expect(state({ tracker: t }, 'tracker')).toBe('live');
    expect(state({ tracker: { ...t, lastSync: new Date(NOW - 3_600_000).toISOString() } }, 'tracker')).toBe('stale');
    expect(state({ tracker: { ...t, state: 'off' } }, 'tracker')).toBe('declined');
    expect(state({ tracker: { ...t, state: 'needsSetup', missing: 'Save an API key.' } }, 'tracker')).toBe('broken');
    expect(state({ tracker: { ...t, state: 'error', lastError: 'refused' } }, 'tracker')).toBe('broken');
  });

  it('lists the personal context only when a clone reported, and flags drift as stale', () => {
    const h: ContextHealth = { meCompared: true, meDiffers: [], homeChecked: '2026-11-14', homeAgeDays: 2, homeStale: null, memoryFiles: 32, memoryDated: 0, memoryExpired: [], memoryUnverified: [] };
    expect(buildCapabilityChecks(input()).some(c => c.id === 'context')).toBe(false);
    expect(buildCapabilityChecks(input({ context: null })).some(c => c.id === 'context')).toBe(false);
    expect(state({ context: h }, 'context')).toBe('live');
    const planted = buildCapabilityChecks(input({ context: { ...h, meDiffers: ['Guardrails.md (differs)'] } })).find(c => c.id === 'context')!;
    expect(planted.state).toBe('stale');
    expect(planted.detail).toContain('me/ differs between the vault and the clone: Guardrails.md (differs)');
    // The backend's one home rule decides (ME2), not the age here.
    expect(state({ context: { ...h, homeAgeDays: 31, homeChecked: '2026-10-16' } }, 'context')).toBe('live');
    const home = buildCapabilityChecks(input({ context: { ...h, homeAgeDays: 1, homeStale: 'stale: past valid_until 2026-11-01' } })).find(c => c.id === 'context')!;
    expect(home.state).toBe('stale');
    expect(home.detail).toContain('The home file is stale: past valid_until 2026-11-01.');
    const lapsed = buildCapabilityChecks(input({ context: { ...h, memoryExpired: ['a', 'b', 'c', 'd'], memoryUnverified: ['e'] } })).find(c => c.id === 'context')!;
    expect(lapsed.state).toBe('stale');
    expect(lapsed.detail).toContain('Past valid_until: a, b, c and 1 more.');
    expect(lapsed.detail).toContain('Not verified in over 180 days: e.');
    expect(lapsed).toMatchObject({ fixSection: 'layers', fixControl: 'Personal' });
  });

  it('accepts any detected supported agent without requiring Claude', () => {
    expect(state({ agents: ['codex'] }, 'agent-runtime')).toBe('live');
    expect(state({ agents: ['antigravity'] }, 'agent-runtime')).toBe('live');
    expect(state({ agents: [] }, 'agent-runtime')).toBe('broken');
    expect(state({ agents: null }, 'agent-runtime')).toBe('stale');
    expect(state({ agents: ['codex'], agentProbeFailed: true }, 'agent-runtime')).toBe('broken');
  });

  it('uses a seven-day verification boundary, not the page cache TTL', () => {
    expect(state({ plaudLastSuccess: new Date(NOW - CAPABILITY_VERIFY_TTL_MS).toISOString() }, 'plaud')).toBe('live');
    expect(state({ plaudLastSuccess: new Date(NOW - CAPABILITY_VERIFY_TTL_MS - 1).toISOString() }, 'plaud')).toBe('stale');
  });

  it('lets a current error outrank recent success', () => {
    expect(state({ plaudError: true }, 'plaud')).toBe('broken');
    expect(state({ plaudStatus: 'needs-auth' }, 'plaud')).toBe('broken');
  });

  it('routes agent repairs to System and vault repairs to the vault path surface', () => {
    const checks = buildCapabilityChecks(input({ vaultExists: false, agents: [] }));
    expect(checks.find(c => c.id === 'vault')?.fixSection).toBe('system');
    expect(checks.find(c => c.id === 'agent-runtime')?.fixSection).toBe('defaults');
  });

  it('never treats a cloud-looking backup path alone as protected', () => {
    expect(state({ backup: { status: 'partial', summary: 'Cloud path only.', signals: [] } }, 'backup')).toBe('stale');
    expect(state({ backup: { status: 'unavailable', summary: 'Cannot inspect.', signals: [] } }, 'backup')).toBe('broken');
  });

  it('keeps required capabilities non-declinable', () => {
    const checks = buildCapabilityChecks(input({
      vaultExists: false,
      agents: [],
      prefs: { version: 1, declined: { plaud: RECENT }, lastVerified: {} },
    }));
    expect(checks.find(c => c.id === 'vault')).toMatchObject({ state: 'broken', declinable: false });
    expect(checks.find(c => c.id === 'agent-runtime')).toMatchObject({ state: 'broken', declinable: false });
    expect(checks.find(c => c.id === 'plaud')?.state).toBe('declined');
  });

  it('persists declines and successful verification timestamps', () => {
    const data = new Map<string, string>();
    const storage = {
      getItem: (key: string) => data.get(key) ?? null,
      setItem: (key: string, value: string) => { data.set(key, value); },
    };

    setCapabilityDeclined(storage, 'plaud', true, RECENT);
    expect(readCapabilityDoctorPrefs(storage).declined.plaud).toBe(RECENT);

    markCapabilityVerified(storage, 'plaud', '2026-07-13T12:00:00Z');
    const prefs = readCapabilityDoctorPrefs(storage);
    expect(prefs.declined.plaud).toBeUndefined();
    expect(prefs.lastVerified.plaud).toBe('2026-07-13T12:00:00Z');
  });
});
