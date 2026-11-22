// The configuration preview (next-level P17): what a run of this automation
// would do, computed locally from data Hollow already has. Pure, no store or
// Tauri import (the node vitest suite loads it), and it never starts anything:
// the caller passes in the connector library and budget statuses it read.
// It is a description, not a dry run, so every line says "would".
import type { ConnectorView } from './connectorsLibrary';
import type { AutomationWindows } from './generated/ipc';
import type { BudgetStatus, UsageBucket, Workflow } from './types';
import { bucketCost, formatTokens } from './usageFormat';
import type { Tone } from './workState';
import { budgetOfLimit, budgetStateLabel, windowLabel } from './budgets';
import { permissionHoldText, type StampedChanges } from './workflowPermissions';
import { firstDstSkip, formatRun, missedRunSentences, nextRuns, scheduleWindow, type NextRuns } from './workflowSchedule';

/** The editor's current values (saved or not). */
export interface PreviewSettings {
  agent: string | null;
  model: string | null;
  cwd: string | null;
  permissionMode: string;
  allowedTools: string[];
  disallowedTools: string[];
  outputGlob: string | null;
  schedule: string;
  triggers: string[];
  skipIfQuiet: boolean;
  watch: string[];
  misfireGraceMin: number | null;
}

export interface PreviewContext {
  slug: string;
  /** The app-wide default agent, used when `agent` is null. */
  defaultAgent: string | null;
  agentLabels: Record<string, string>;
  agentsAvailable: readonly string[];
  permissionLabels: Record<string, string>;
  workspaces: readonly { cwd: string; title: string }[];
  /** The connector library, or null when it could not be read. */
  connectors: readonly ConnectorView[] | null;
  /** Budget statuses, or null when they could not be read. */
  budgets: readonly BudgetStatus[] | null;
  holds: Pick<Workflow, 'permissionHold' | 'budgetHold'>;
  /** Unsaved changes to approved settings ({@link stampedChanges}). */
  unsaved: StampedChanges | null;
  now: number;
  windows?: AutomationWindows | null;
  locale?: string;
}

export interface PreviewLine {
  label: string;
  value: string;
  detail?: string;
  tone?: Tone;
}

export interface PreviewGroup {
  id: 'who' | 'where' | 'effects' | 'limits' | 'when' | 'gate';
  title: string;
  lines: PreviewLine[];
}

export interface ConfigPreview {
  groups: PreviewGroup[];
  next: NextRuns | null;
  /** Whether a real run would start now if asked (no hold, no unsaved approval). */
  runnable: boolean;
}

/** Agents Hollow attaches its own tools and library connectors to
 *  (Rust `agent::capabilities(..).owned_mcp`). */
const OWNED_MCP = new Set(['claude', 'codex']);
/** Rust `tasks_mcp::WRITE_TOOLS`, same order. */
const TASK_WRITE_TOOLS = ['create_task', 'update_task', 'set_status', 'claim_task', 'release_task', 'add_comment', 'propose_tasks', 'create_project', 'update_project', 'set_project_status'];

/** Task tools for an automation (Rust `TasksAccess::for_workflow`). */
export function taskAccess(mode: string, allowed: readonly string[]): 'read' | 'write' {
  if (mode.trim() === 'plan') return 'read';
  const tools = allowed.map(t => t.trim()).filter(Boolean);
  if (tools.length === 0) return 'write';
  const writes = tools.some(t => ['Write', 'Edit', 'MultiEdit', 'NotebookEdit', 'Bash'].includes(t)
    || t.startsWith('Bash(') || t === 'mcp__hollow_tasks'
    || (t.startsWith('mcp__hollow_tasks__') && TASK_WRITE_TOOLS.includes(t.slice('mcp__hollow_tasks__'.length))));
  return writes ? 'write' : 'read';
}

/** Hollow app tools for an automation (Rust `AppAccess::for_workflow`). */
export function appAccess(mode: string, allowed: readonly string[]): 'read' | 'write' | null {
  const plan = mode.trim() === 'plan';
  const tools = allowed.map(t => t.trim()).filter(Boolean);
  if (tools.length === 0) return plan ? 'read' : 'write';
  const whole = tools.includes('mcp__hollow_app');
  const perTool = tools.some(t => t.startsWith('mcp__hollow_app__'));
  if (whole && !plan) return 'write';
  if (whole || perTool) return 'read';
  return null;
}

/** Library connectors an automation run could receive (Rust
 *  `connectors::eligible_with` for automations, then the stamped-allowlist
 *  filter in `workflows::prepare_spawn`). `always` ones attach to every run;
 *  `relevant` ones attach when Hollow judges the prompt needs them. */
export function automationConnectors(
  connectors: readonly ConnectorView[],
  agent: string,
  mode: string,
  allowed: readonly string[],
): { connector: ConnectorView; when: 'always' | 'relevant' }[] {
  if (mode.trim() === 'plan' || !OWNED_MCP.has(agent)) return [];
  const restricted = allowed.map(t => t.trim()).filter(Boolean);
  return connectors
    .filter(c => c.enabled && c.automations && (c.agents as string[]).includes(agent))
    .filter(c => c.transport !== 'claudeAi' || agent === 'claude')
    .filter(c => c.transport !== 'builtin' || c.builtin?.ready !== false)
    .filter(c => restricted.length === 0 || restricted.includes(`mcp__${c.id}`))
    .map(c => ({ connector: c, when: c.mode === 'always' ? 'always' as const : 'relevant' as const }));
}

/** Budgets that apply to a run of this automation: all usage, its agent, and
 *  the automation itself. */
export function applicableBudgets(budgets: readonly BudgetStatus[], slug: string, agent: string): BudgetStatus[] {
  return budgets.filter(b => b.scope === 'global' || b.scope === `agent:${agent}` || b.scope === `workflow:${slug}`);
}

function budgetTone(b: BudgetStatus): Tone {
  if (b.state === 'paused' || b.state === 'over') return 'error';
  if (b.state === 'warning' || b.state === 'unknown') return 'warn';
  return 'ok';
}

/** Build the preview. Never executes anything; every value comes from `s`
 *  and `ctx`. */
export function buildConfigPreview(s: PreviewSettings, ctx: PreviewContext): ConfigPreview {
  const agent = s.agent || ctx.defaultAgent || ctx.agentsAvailable[0] || '';
  const agentName = ctx.agentLabels[agent] ?? (agent || 'No agent available');
  const mode = s.permissionMode.trim() || 'acceptEdits';
  const groups: PreviewGroup[] = [];

  // Who would run it.
  const who: PreviewLine[] = [{
    label: 'Agent',
    value: s.agent ? agentName : `${agentName} (app default)`,
    ...(agent && !ctx.agentsAvailable.includes(agent) ? { detail: 'Not installed on this Mac: a run would fail before it starts.', tone: 'error' as Tone } : {}),
  }, {
    label: 'Model',
    value: s.model?.trim() || 'The agent’s own model setting',
  }];
  groups.push({ id: 'who', title: 'Who would run it', lines: who });

  // Where it would work and write.
  const workspace = ctx.workspaces.find(w => w.cwd === s.cwd);
  const where: PreviewLine[] = [{
    label: 'Works in',
    value: workspace ? `${workspace.title} workspace` : s.cwd?.trim() ? s.cwd : 'The vault root',
    detail: workspace ? s.cwd! : undefined,
  }, {
    label: 'Results land in',
    value: s.outputGlob?.trim() || 'Not declared',
    detail: s.outputGlob?.trim() ? 'Files matching this appear as its latest result.' : 'Without a result location, files a run writes are found from its run record.',
    tone: s.outputGlob?.trim() ? undefined : 'warn',
  }];
  groups.push({ id: 'where', title: 'Where it would work', lines: where });

  // What it would be allowed to do.
  const effects: PreviewLine[] = [{
    label: 'Permission mode',
    value: ctx.permissionLabels[mode] ?? mode,
    tone: mode === 'bypassPermissions' || mode === 'auto' ? 'warn' : undefined,
  }, {
    label: 'Allowed tools',
    value: s.allowedTools.length ? s.allowedTools.join(', ') : 'Any tool the permission mode allows',
  }, {
    label: 'Disallowed tools',
    value: s.disallowedTools.length ? s.disallowedTools.join(', ') : 'None',
  }];
  if (OWNED_MCP.has(agent)) {
    const tasks = taskAccess(mode, s.allowedTools);
    const app = appAccess(mode, s.allowedTools);
    effects.push({
      label: 'Hollow tools',
      value: [
        `tasks (${tasks === 'write' ? 'read and write' : 'read only'})`,
        app ? `Hollow app (${app === 'write' ? 'read and write' : 'read only'})` : null,
        mode !== 'plan' ? 'teammates' : null,
      ].filter(Boolean).join(', '),
    });
  } else {
    effects.push({ label: 'Hollow tools', value: `None: Hollow can’t attach tools to ${agentName}` });
  }
  if (ctx.connectors === null) {
    effects.push({ label: 'Connectors', value: 'Couldn’t read the connector library', tone: 'warn' });
  } else {
    const list = automationConnectors(ctx.connectors, agent, mode, s.allowedTools);
    effects.push({
      label: 'Connectors',
      value: list.length
        ? list.map(({ connector, when }) => `${connector.name}${when === 'relevant' ? ' (when relevant)' : ''}`).join(', ')
        : 'None',
      detail: list.length
        ? 'Turned on for automations in Settings → Connectors. An automation is told never to send, publish or delete through one unless its prompt asks for exactly that.'
        : undefined,
    });
  }
  groups.push({ id: 'effects', title: 'What it would be allowed to do', lines: effects });

  // Limits.
  const limits: PreviewLine[] = [];
  if (ctx.budgets === null) {
    limits.push({ label: 'Budgets', value: 'Couldn’t read usage budgets', tone: 'warn' });
  } else {
    const list = applicableBudgets(ctx.budgets, ctx.slug, agent);
    if (!list.length) limits.push({ label: 'Budgets', value: 'None apply. Each run uses model allowance with no cap from Hollow.' });
    for (const b of list) {
      limits.push({
        label: `${windowLabel(b.window)} · ${b.scopeLabel}`,
        value: `${budgetOfLimit(b)} · ${budgetStateLabel(b.state, b.pause?.held)}`,
        detail: b.hardStop ? 'Hard stop: a run is refused at the limit.' : 'Warns only.',
        tone: budgetTone(b),
      });
    }
  }
  groups.push({ id: 'limits', title: 'Limits', lines: limits });

  // When.
  const next = nextRuns(s.schedule, ctx.now, 3, 1500, ctx.windows);
  const when: PreviewLine[] = [];
  if (next?.runs.length) {
    next.runs.forEach((run, i) => when.push({ label: i === 0 ? 'Next run' : 'Then', value: formatRun(run, ctx.locale) }));
  } else if (scheduleWindow(s.schedule)) {
    when.push({ label: 'Next run', value: 'Window start times are unavailable. Refresh windows to try again.', tone: 'warn' });
  } else if (next) {
    when.push({ label: 'Next run', value: 'None within the next four years', tone: 'warn' });
  }
  for (const sentence of missedRunSentences({
    schedule: s.schedule, misfireGraceMin: s.misfireGraceMin, skipIfQuiet: s.skipIfQuiet, triggers: s.triggers,
    dstSkip: next ? firstDstSkip(s.schedule, ctx.now) : null, timeZone: next?.timeZone, locale: ctx.locale,
  })) when.push({ label: '', value: sentence });
  groups.push({ id: 'when', title: 'When it would run', lines: when });

  // Before it can run.
  const gate: PreviewLine[] = [];
  const hold = permissionHoldText(ctx.holds.permissionHold);
  if (hold) gate.push({ label: 'Permission review', value: hold.charAt(0).toUpperCase() + hold.slice(1), detail: 'Approve or reject it in Permission changes above.', tone: 'error' });
  if (ctx.holds.budgetHold) gate.push({ label: 'Budget', value: ctx.holds.budgetHold, tone: 'error' });
  if (ctx.unsaved && ctx.unsaved.changes.length) {
    gate.push({
      label: 'Unsaved changes',
      value: ctx.unsaved.changes.map(c => c.text).join('; '),
      detail: ctx.unsaved.widens
        ? 'These widen what it may do. Saving asks you to approve them; until then a run uses the saved settings.'
        : 'Saving approves them; until then a run uses the saved settings.',
      tone: ctx.unsaved.widens ? 'warn' : 'neutral',
    });
  }
  if (!gate.length) gate.push({ label: 'Approval', value: 'Settings approved: nothing waits for review.', tone: 'ok' });
  groups.push({ id: 'gate', title: 'Before it can run', lines: gate });

  const runnable = !hold && !ctx.holds.budgetHold && !(ctx.unsaved && ctx.unsaved.changes.length);
  return { groups, next, runnable };
}

/** What this automation's runs used today (Settings, Usage's ledger,
 *  `usage_summary('day').byWorkflow`), for the real-run note beside Run now.
 *  No figure is never "free": a missing bucket says nothing was recorded. */
export function todayUsageText(bucket: Pick<UsageBucket, 'runs' | 'inputTokens' | 'cachedInputTokens' | 'outputTokens' | 'unknown' | 'reportedCostUsd' | 'estimatedCostUsd'> | null | undefined): string {
  if (!bucket || bucket.runs === 0) return 'No usage recorded for it today.';
  const tokens = bucket.inputTokens + bucket.cachedInputTokens + bucket.outputTokens;
  const parts = [`${bucket.runs} run${bucket.runs === 1 ? '' : 's'}`, `${formatTokens(tokens)} tokens`];
  const cost = bucketCost(bucket);
  if (cost !== '—') parts.push(cost);
  const unknown = bucket.unknown > 0 ? ` (${bucket.unknown} without usage figures)` : '';
  return `Today: ${parts.join(' · ')}${unknown}.`;
}
