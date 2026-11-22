import { describe, expect, it } from 'vitest';
import type { AgentDescriptor } from './types';
import { permModeConfirmName, permModeDescriptions, permModeNote, permModeOptions, permModeUnavailable } from './permModes';
// Settings' permission row moved to settings/AgentDefaults.tsx (L4, X18).
import settingsSource from './settings/AgentDefaults.tsx?raw';
// The composer's Permissions moved into the model menu (2026-11-01).
import chatModelMenuSource from './components/ChatModelMenu.tsx?raw';
import workflowsSource from './pages/Workflows.tsx?raw';

// A sample of what `agent::capabilities` sends for a mode an agent can't run.
// No agent has one since Antigravity's Auto came back (2026-11-20, Rust side
// `antigravity_runs_auto_natively_and_only_bypass_skips_permissions`), but the
// pickers keep the mechanism, so this fixture stays.
const REASON = "Auto isn't available for Antigravity: its sandbox holds only shell commands, and its file tools wrote outside the working folder in Hollow's probe (agy 1.2.15). Choose Accept edits or Plan, or Skip permissions if it may run unrestricted.";
const base: AgentDescriptor['capabilities'] = {
  modelSelection: true, sessionResume: true, toolRestrictions: false, ownedMcp: false, structuredOutput: true, effortLevels: [],
};
const antigravity = { ...base, unavailableModes: { auto: REASON } };
const claude = { ...base, toolRestrictions: true, ownedMcp: true, unavailableModes: {} };

const ORDER = ['acceptEdits', 'auto', 'plan', 'bypassPermissions'] as const;
type Mode = typeof ORDER[number];
const LABELS: Record<Mode, string> = { acceptEdits: 'Edits only', auto: 'Auto', plan: 'Plan', bypassPermissions: 'Bypass all (danger)' };
const DESC: Record<Mode, string> = { acceptEdits: 'edits', auto: 'auto desc', plan: 'plan desc', bypassPermissions: 'bypass desc' };

describe('permission modes per agent (merged plan slice 2)', () => {
  it('shows Auto unavailable for Antigravity, with the reason, and leaves the rest', () => {
    const options = permModeOptions(ORDER, LABELS, DESC, antigravity);
    expect(options.map((o) => o.value)).toEqual([...ORDER]);
    const auto = options.find((o) => o.value === 'auto')!;
    expect(auto).toEqual({ value: 'auto', label: 'Auto (unavailable)', detail: REASON, disabled: true });
    for (const mode of ['acceptEdits', 'plan', 'bypassPermissions'] as const) {
      const option = options.find((o) => o.value === mode)!;
      expect(option.disabled).toBeFalsy();
      expect(option.detail).toBe(DESC[mode]);
    }
  });

  it('keeps every mode for agents that can run them, and before descriptors load', () => {
    for (const caps of [claude, base, undefined]) {
      const options = permModeOptions(ORDER, LABELS, DESC, caps);
      expect(options.every((o) => !o.disabled)).toBe(true);
      expect(options.find((o) => o.value === 'auto')!.label).toBe('Auto');
    }
  });

  it('explains a saved Auto that the current agent cannot run', () => {
    expect(permModeUnavailable(antigravity, 'auto')).toBe(REASON);
    expect(permModeUnavailable(antigravity, 'bypassPermissions')).toBeNull();
    expect(permModeUnavailable({ ...base, unavailableModes: { auto: '  ' } }, 'auto')).toBeNull();
    expect(permModeNote('Auto', 'auto desc', antigravity, 'auto')).toBe(`Auto · unavailable: ${REASON}`);
    expect(permModeNote('Auto', 'auto desc', claude, 'auto')).toBe('Auto · auto desc');
  });

  // Every picker for a permission mode builds its options through
  // permModeOptions, so none offers Auto to Antigravity as a normal choice
  // (review: the Settings "Chat permission mode" row once mapped
  // PERM_MODE_ORDER by hand and did).
  it.each([
    ['Settings', settingsSource],
    ['ChatModelMenu', chatModelMenuSource],
    ['Workflows', workflowsSource],
  ])('the %s mode picker uses permModeOptions and refuses an unavailable pick', (_name, source) => {
    // A Select's options={…}, or the model menu's <For each={…}> rows.
    // The chat's pickers list the agent's own modes (per-agent menus, 2026-11-12).
    expect(source).toMatch(/(options|each)=\{permModeOptions\((PERM_MODE_ORDER|agentModeOrder\((agent|chatAgent\(\))\)),/);
    expect(source).not.toMatch(/PERM_MODE_ORDER\.map\(\s*m\s*=>\s*\(\{\s*value/);
    expect(source).toMatch(/permModeUnavailable\(/);
  });
});

describe('per-agent mode descriptions (provider copy, PROVIDER_MATRIX overclaim 3)', () => {
  it('says Antigravity’s Edits only can’t edit, and leaves other agents and modes alone', () => {
    const anti = permModeDescriptions(DESC, 'antigravity');
    expect(anti.acceptEdits).toMatch(/can’t edit files or run commands/);
    expect(anti.acceptEdits).toMatch(/Bypass all/);
    expect(anti.plan).toBe(DESC.plan);
    expect(permModeOptions(ORDER, LABELS, anti, antigravity).find(o => o.value === 'acceptEdits')!.detail).toBe(anti.acceptEdits);
    for (const agent of ['claude', 'codex', 'grok', '', null, undefined]) expect(permModeDescriptions(DESC, agent)).toEqual(DESC);
  });
});

describe('the danger confirm names the mode as both surfaces do (lane S re-review of #1304)', () => {
  it('gives the chip’s name and the pickers’ name where they differ', () => {
    expect(permModeConfirmName('Bypass all (danger)', 'Full access')).toBe('Full access (Bypass all)');
    expect(permModeConfirmName('Auto', 'Auto')).toBe('Auto');
  });
});
