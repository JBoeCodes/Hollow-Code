/**
 * Words for the Skills page's portability table (NV3, 2026-11-17): whether
 * each agent loads a skill, loads it degraded or doesn't, from the backend's
 * `skill_portability_table` (`skill_portability.rs`). Pure, so it is
 * unit-tested in node. The backend decides every effect and version status;
 * this file only says them in words, and says "not verified" wherever the
 * backend does rather than turning it into a yes.
 */
import type { PortabilityAgentRow, PortabilityEffect, PortabilityFinding, PortabilityRule, PortabilityVersionStatus } from './types';

/** The effect, in a few words. */
export function effectLabel(effect: PortabilityEffect): string {
  switch (effect) {
    case 'loads': return 'Loads';
    case 'degraded': return 'Loads, degraded';
    case 'noLoad': return 'Does not load';
    case 'unknown': return 'Not verified';
  }
}

/** One rule's evidence, as a short tag. */
export function statusTag(status: PortabilityVersionStatus, verifiedOn: string | null): string {
  switch (status) {
    case 'hollow': return 'Hollow’s own rule';
    case 'verified': return `Checked on ${verifiedOn ?? 'this version'}`;
    case 'notVerifiedOnThisVersion': return `Checked on ${verifiedOn ?? 'another version'}: not verified on this version`;
    case 'installedUnknown': return `Checked on ${verifiedOn ?? 'a version'}: could not be confirmed on this machine`;
    case 'unverified': return 'Not verified';
  }
}

/** The installed version and what the recorded versions say about it, one line. */
export function versionLine(row: Pick<PortabilityAgentRow, 'installed' | 'rules' | 'stale'>): string {
  const installed = row.installed ? `Installed ${row.installed}.` : 'Installed version unknown.';
  const versions = [...new Set(row.rules.map(r => r.verifiedOn).filter((v): v is string => !!v))];
  if (!versions.length) return `${installed} No rule is tied to a CLI version.`;
  const said = versions.map(v => row.installed === v ? `Checked on ${v}.` : row.installed ? `Checked on ${v}: not verified on this version.` : `Checked on ${v}: could not be confirmed on this machine.`);
  return `${installed} ${said.join(' ')}`;
}

/** The findings of one channel, in the backend's order. */
export function findingsOf(row: Pick<PortabilityAgentRow, 'findings'>, channel: 'hollow' | 'native'): PortabilityFinding[] {
  return row.findings.filter(f => f.channel === channel);
}

/** The rules of a profile, Hollow's delivery first, each channel in the backend's order. */
export function rulesInOrder(row: Pick<PortabilityAgentRow, 'rules'>): PortabilityRule[] {
  return [...row.rules.filter(r => r.channel === 'hollow'), ...row.rules.filter(r => r.channel === 'native')];
}

/** The table's one-line summary of an assessed skill: how many agents load it whole. */
export function tableSummary(rows: Pick<PortabilityAgentRow, 'label' | 'summary'>[]): string {
  const assessed = rows.filter(r => r.summary);
  if (!assessed.length) return 'No skill assessed.';
  const whole = assessed.filter(r => r.summary === 'loads').length;
  if (whole === assessed.length) return 'Every agent gets this skill whole.';
  const cut = assessed.filter(r => r.summary !== 'loads').map(r => r.label);
  return `${whole} of ${assessed.length} agents get this skill whole; degraded for ${cut.join(', ')}.`;
}
