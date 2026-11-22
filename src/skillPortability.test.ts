import { describe, expect, it } from 'vitest';
import { effectLabel, findingsOf, rulesInOrder, statusTag, tableSummary, versionLine } from './skillPortability';
import type { PortabilityAgentRow, PortabilityRule } from './types';

const rule = (over: Partial<PortabilityRule>): PortabilityRule => ({
  id: 'r', channel: 'hollow', effect: 'degraded', what: 'w', sentence: 's', verifiedOn: null, unverifiedReason: null, status: 'hollow', ...over,
});
const row = (over: Partial<PortabilityAgentRow>): PortabilityAgentRow => ({
  agent: 'grok', label: 'Grok', installed: '1.0.79', rules: [], summary: null, findings: [], stale: false, ...over,
});

describe('skill portability words', () => {
  it('names every effect and never turns unknown into a yes', () => {
    expect(effectLabel('loads')).toBe('Loads');
    expect(effectLabel('degraded')).toBe('Loads, degraded');
    expect(effectLabel('noLoad')).toBe('Does not load');
    expect(effectLabel('unknown')).toBe('Not verified');
  });

  it('says "not verified on this version" when the recorded version differs', () => {
    expect(statusTag('notVerifiedOnThisVersion', '1.0.46')).toBe('Checked on 1.0.46: not verified on this version');
    expect(statusTag('verified', '1.0.46')).toBe('Checked on 1.0.46');
    expect(statusTag('hollow', null)).toBe('Hollow’s own rule');
    expect(statusTag('unverified', null)).toBe('Not verified');
    expect(statusTag('installedUnknown', '1.0.46')).toContain('could not be confirmed on this machine');
  });

  it('writes the installed version beside each recorded one', () => {
    const rules = [rule({ verifiedOn: '1.0.46', status: 'notVerifiedOnThisVersion' }), rule({ id: 'q', verifiedOn: '1.0.46' })];
    expect(versionLine(row({ rules, stale: true }))).toBe('Installed 1.0.79. Checked on 1.0.46: not verified on this version.');
    expect(versionLine(row({ rules, installed: '1.0.46' }))).toBe('Installed 1.0.46. Checked on 1.0.46.');
    expect(versionLine(row({ rules, installed: null }))).toBe('Installed version unknown. Checked on 1.0.46: could not be confirmed on this machine.');
    expect(versionLine(row({ rules: [rule({})] }))).toBe('Installed 1.0.79. No rule is tied to a CLI version.');
  });

  it('keeps each channel apart and Hollow’s rules first', () => {
    const rules = [rule({ id: 'n', channel: 'native' }), rule({ id: 'h' })];
    expect(rulesInOrder({ rules }).map(r => r.id)).toEqual(['h', 'n']);
    const findings = [
      { rule: 'n', channel: 'native' as const, effect: 'unknown' as const, sentence: '', verifiedOn: null, status: 'unverified' as const },
      { rule: 'h', channel: 'hollow' as const, effect: 'degraded' as const, sentence: '', verifiedOn: null, status: 'hollow' as const },
    ];
    expect(findingsOf({ findings }, 'hollow').map(f => f.rule)).toEqual(['h']);
    expect(findingsOf({ findings }, 'native').map(f => f.rule)).toEqual(['n']);
  });

  it('sums up how many agents get the skill whole', () => {
    expect(tableSummary([row({})])).toBe('No skill assessed.');
    expect(tableSummary([row({ summary: 'loads' }), row({ label: 'Codex', summary: 'loads' })])).toBe('Every agent gets this skill whole.');
    expect(tableSummary([row({ label: 'Claude', summary: 'loads' }), row({ summary: 'degraded' }), row({ label: 'Antigravity', summary: 'degraded' })]))
      .toBe('1 of 3 agents get this skill whole; degraded for Grok, Antigravity.');
  });
});
