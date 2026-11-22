/** Remote runners 3 (2026-11-03): Settings, Devices, Remote
 *  runner → Set up, in words. The wire types of `runner_setup_*` and
 *  `runner_cloud_*` (src-tauri/src/runner_setup.rs) and the pure helpers
 *  settings/RunnerSetupWizard.tsx shows them with: state labels, who runs a
 *  command, the estimate's lines and the confirm dialogs' text. */

import type { AgentSettings } from './types';

export type RunnerSetupPath = 'mac' | 'cloud';

export interface RunnerSetupParams {
  path: RunnerSetupPath;
  admin: string;
  alias: string;
  account: string;
  keyFile: string;
  hostName: string;
  project: string;
  zone: string;
  machineType: string;
  diskGb: number;
  idleMinutes: number;
  hoursPerMonth: number;
  billingAccount: string;
  budgetUsd: number;
  instance: string;
  serviceAccount: string;
}

export type RunnerSetupOn = 'local' | 'admin' | 'runner' | 'gcloud' | 'hollow';

export interface RunnerSetupCmd {
  on: RunnerSetupOn;
  who: string;
  /** Exactly what runs. */
  shown: string;
  sudo: boolean;
}

export interface RunnerSetupStep {
  id: string;
  title: string;
  why: string;
  changes: string[];
  confirm: string | null;
  check: RunnerSetupCmd[];
  apply: RunnerSetupCmd[];
  byHand: RunnerSetupCmd[];
  byHandNote: string | null;
}

export interface RunnerSetupRan {
  shown: string;
  code: number | null;
  output: string;
}

export type RunnerSetupState = 'done' | 'missing' | 'unknown' | 'failed';

export interface RunnerSetupStepState {
  id: string;
  state: RunnerSetupState;
  summary: string;
  details: string[];
  needsByHand: boolean;
  ran: RunnerSetupRan[];
}

export interface RunnerMachineType {
  id: string;
  label: string;
  hourlyUsd: number;
  arm: boolean;
}

export interface RunnerCloudEstimate {
  machineType: string;
  machineLabel: string;
  hourlyUsd: number;
  hours: number;
  computeUsd: number;
  diskGb: number;
  diskUsd: number;
  /** Cloud NAT (the VM's way out): its gateway at the hours and its address. */
  natUsd: number;
  totalUsd: number;
  /** What Create must be handed back. */
  totalCents: number;
  region: string;
  pricesRegion: string;
  pricesTaken: string;
}

export interface RunnerSetupPlan {
  params: RunnerSetupParams;
  problems: string[];
  /** Worth a look; stops nothing (an admin login that goes elsewhere than the runner's address). */
  notes: string[];
  steps: RunnerSetupStep[];
  estimate: RunnerCloudEstimate | null;
  machineTypes: RunnerMachineType[];
}

/** Blank fields for `path` (the backend fills the defaults). */
export function blankSetupParams(path: RunnerSetupPath): RunnerSetupParams {
  return { path, admin: '', alias: '', account: '', keyFile: '', hostName: '', project: '', zone: '', machineType: '', diskGb: 0, idleMinutes: 0, hoursPerMonth: 0, billingAccount: '', budgetUsd: 0, instance: '', serviceAccount: '' };
}

/** `current` with its blank fields (empty text, 0) taken from `filled`. */
export function fillBlanks(current: RunnerSetupParams, filled: RunnerSetupParams): RunnerSetupParams {
  const out = { ...current } as Record<string, string | number>;
  for (const [key, value] of Object.entries(filled)) {
    if (key === 'path') continue;
    const have = out[key];
    if (have === '' || have === 0 || have === undefined) out[key] = value;
  }
  return out as unknown as RunnerSetupParams;
}

export function setupStateLabel(state: RunnerSetupState | undefined): string {
  switch (state) {
    case 'done': return 'Done';
    case 'missing': return 'Not yet';
    case 'unknown': return "Couldn't tell";
    case 'failed': return 'Failed';
    default: return 'Not checked';
  }
}

/** The StatusPill tone for a step's state. */
export function setupStateTone(state: RunnerSetupState | undefined): string {
  switch (state) {
    case 'done': return 'ok';
    case 'missing': return 'warn';
    case 'failed': return 'error';
    case 'unknown': return 'neutral';
    default: return 'off';
  }
}

/** Who runs a command, as the label above it says. */
export function onLabel(cmd: RunnerSetupCmd): string {
  switch (cmd.on) {
    case 'local': return 'On this Mac';
    case 'gcloud': return 'gcloud, on this Mac';
    case 'hollow': return 'Hollow itself';
    default: return `On ${cmd.who}`;
  }
}

export function formatUsd(amount: number): string {
  return `$${amount.toFixed(2)}`;
}

/** The estimate panel's lines, in order. */
export function estimateLines(e: RunnerCloudEstimate): string[] {
  const hourly = e.hourlyUsd < 1 ? `$${e.hourlyUsd.toFixed(3)}` : formatUsd(e.hourlyUsd);
  const lines = [
    `${e.machineType} (${e.machineLabel}): ${hourly} an hour × ${e.hours} hours = ${formatUsd(e.computeUsd)}`,
    `${e.diskGb} GB disk, all month (stopped too): ${formatUsd(e.diskUsd)}`,
    `Cloud NAT, its way out to the internet (no public address): ${formatUsd(e.natUsd)}, plus $0.045 per GB it carries`,
    `About ${formatUsd(e.totalUsd)} a month.`,
    `Approximate Google Cloud list prices for ${e.pricesRegion}, taken ${e.pricesTaken}; other network use and taxes are extra.`,
  ];
  if (e.region && e.region !== e.pricesRegion) lines.push(`Prices in ${e.region} differ; check Google Cloud's pricing page.`);
  return lines;
}

/** The confirm dialog before Create: the project, the machine and the estimate. */
export function createConfirm(params: RunnerSetupParams, e: RunnerCloudEstimate): { title: string; summary: string; body: string; confirmLabel: string } {
  return {
    title: `Create ${params.instance} in ${params.project}?`,
    summary: `A billed ${e.machineType} VM, about ${formatUsd(e.totalUsd)} a month at ${e.hours} hours.`,
    body: `Project: ${params.project}\nMachine: ${e.machineType} (${e.machineLabel}), ${e.diskGb} GB disk, zone ${params.zone}\nEstimate: about ${formatUsd(e.totalUsd)} a month (approximate list prices)\n\nIt also creates a service account with no roles, a firewall rule that admits SSH only from IAP, a Cloud Router and NAT for its way out to the internet, and a budget alert at ${formatUsd(params.budgetUsd)} a month. The VM powers itself off after ${params.idleMinutes} idle minutes.`,
    confirmLabel: 'Create',
  };
}

/** The confirm dialog before a step's Apply, when it asks first. */
export function applyConfirm(step: RunnerSetupStep): { title: string; body: string; confirmLabel: string } | null {
  if (!step.confirm) return null;
  const what = step.changes.length ? `It ${step.changes.join(', ')}.\n\n` : '';
  return { title: `Apply ${step.title}?`, body: `${what}${step.confirm}`, confirmLabel: 'Apply' };
}

/** M7 (PER-21, 2026-11-15): Set up's last step, "What runs there?". Naming
 *  a runner turns nothing on: each of these is unticked, and only what the
 *  person ticks is saved, through the normal agent settings save. The same
 *  four are Settings, Devices (and Advanced, App builds), and an agent sets
 *  them with change_setting (agents.unattendedRunsOn, agents.buildsOn,
 *  agents.runsOn, agents.appBuildsOn). */
export type RunsThereChoice = 'unattended' | 'builds' | 'chats' | 'appBuilds';

export const WHAT_RUNS_THERE: readonly { id: RunsThereChoice; label: string; detail: string }[] = [
  { id: 'unattended', label: 'Overnight and worker chats', detail: 'Night-queue items and unattended chats in build workspaces, when the runner answers.' },
  { id: 'builds', label: 'Build and test commands', detail: 'Auto: gates, Rust builds and whole test suites go there; quick checks stay here.' },
  { id: 'chats', label: 'Chats in build workspaces', detail: "Claude and Codex turns in build workspaces that follow Settings' Runs on." },
  { id: 'appBuilds', label: 'App builds', detail: 'Build app compiles Hollow there and signs it here.' },
];

/** The settings the ticked choices save: only those, each as its runner
 *  value (Builds and tests as Auto). Nothing ticked saves nothing. */
export function whatRunsTherePatch(ticked: ReadonlySet<RunsThereChoice>): Partial<Pick<AgentSettings, 'unattendedRunsOn' | 'buildsOn' | 'runsOn' | 'appBuildsOn'>> {
  const patch: Partial<Pick<AgentSettings, 'unattendedRunsOn' | 'buildsOn' | 'runsOn' | 'appBuildsOn'>> = {};
  if (ticked.has('unattended')) patch.unattendedRunsOn = 'runner';
  if (ticked.has('builds')) patch.buildsOn = 'auto';
  if (ticked.has('chats')) patch.runsOn = 'runner';
  if (ticked.has('appBuilds')) patch.appBuildsOn = 'runner';
  return patch;
}
