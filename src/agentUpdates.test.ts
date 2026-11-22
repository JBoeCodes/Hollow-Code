import { describe, expect, it } from 'vitest';
import type { AgentCliStatus } from './generated/ipc';
import { knownBadLines, primaryAction, stateLine, updatableAgents, versionLine } from './agentUpdates';

const now = new Date('2026-10-13T16:00:00Z');
const row = (patch: Partial<AgentCliStatus> = {}): AgentCliStatus => ({
  agent: 'codex', label: 'Codex', installed: true, version: '0.153.4', latest: '0.162.1', updateAvailable: true,
  method: 'npm (global)', updateCommand: 'npm install -g @openai/codex@0.162.1', canUpdate: true, canCheck: true,
  previousVersion: null, canRestore: false, safeWhileRunning: false, policy: 'automatic',
  lastChecked: '2026-10-13T15:40:00Z', state: 'idle', inUse: 0, message: null, tone: null, knownBad: null, latestKnownBad: null, ...patch,
});

describe('agent update rows', () => {
  it('say the installed version, what is out and how it was installed', () => {
    expect(versionLine(row(), now)).toMatch(/^0\.153\.4 · 0\.162\.1 is out · npm \(global\) · checked /);
    expect(versionLine(row({ updateAvailable: false, latest: '0.153.4' }), now)).toContain('up to date');
    expect(versionLine(row({ latest: null, updateAvailable: false }), now)).toContain('latest unknown');
    expect(versionLine(row({ lastChecked: null }), now)).toContain('never checked');
    expect(versionLine(row({ installed: false }), now)).toBe('Not installed.');
  });

  it('explain that Antigravity checks only when it updates', () => {
    const agy = row({ agent: 'antigravity', label: 'Antigravity', canCheck: false, latest: null, updateAvailable: false, method: "Antigravity's updater" });
    expect(versionLine(agy, now)).toBe("0.153.4 · Antigravity checks when it updates · Antigravity's updater");
    expect(primaryAction(agy)).toEqual({ kind: 'update', label: 'Update' });
  });

  it('offer Update only when there is something to do', () => {
    expect(primaryAction(row())).toEqual({ kind: 'update', label: 'Update to 0.162.1' });
    expect(primaryAction(row({ updateAvailable: false }))).toBeNull();
    expect(primaryAction(row({ canUpdate: false }))).toBeNull();
    expect(primaryAction(row({ state: 'updating' }))).toBeNull();
    expect(primaryAction(row({ state: 'waiting' }))).toEqual({ kind: 'cancel', label: 'Cancel update' });
  });

  it('describe a check or update in progress', () => {
    expect(stateLine(row({ state: 'updating' }))).toBe('Updating Codex… New runs are unavailable. Try again when the update finishes.');
    expect(stateLine(row({ state: 'checking' }))).toBe('Checking for a newer Codex…');
    expect(stateLine(row())).toBeNull();
  });
});

describe('known-bad versions (repo ideas 2026-10-27 #9)', () => {
  it('says nothing for a version that is not listed', () => {
    expect(knownBadLines(row())).toEqual([]);
  });
  it('flags an installed version and warns before a listed update, with each reason', () => {
    const listed = row({ knownBad: 'Its exec mode ignores --output-schema.', latestKnownBad: 'It hangs on a closed stdin.' });
    expect(knownBadLines(listed)).toEqual([
      'Codex 0.153.4 is known to break Hollow: Its exec mode ignores --output-schema.',
      "0.162.1 is known to break Hollow: It hangs on a closed stdin. Automatic won't install it; Update asks first.",
    ]);
  });
});

describe('Update all', () => {
  it('lists the agents with an update to take, and leaves out known-bad, busy, current and uninstalled ones', () => {
    const a = row({ agent: 'a', label: 'A' });
    const bad = row({ agent: 'b', label: 'B', latestKnownBad: 'breaks tools' });
    const busy = row({ agent: 'c', label: 'C', state: 'updating' });
    const current = row({ agent: 'd', label: 'D', updateAvailable: false });
    const missing = row({ agent: 'e', label: 'E', installed: false });
    const cannot = row({ agent: 'f', label: 'F', canUpdate: false });
    expect(updatableAgents([a, bad, busy, current, missing, cannot]).map(r => r.agent)).toEqual(['a']);
    expect(updatableAgents([])).toEqual([]);
  });
});
