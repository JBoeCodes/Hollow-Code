// Automations page: the pure text and shape helpers its views share (plan
// completion L2, item 6: moved out of pages/Workflows.tsx unchanged, so the
// page keeps only components). No store or Tauri import.
import type { Workflow } from '../../types';
import type { PreviewSettings } from '../../workflowPreview';
import { isBudgetStopError } from '../../budgets';
import { describeSchedule, nextRunState } from '../../workflowSchedule';

/// Prefix of an editor save refused because the automation changed on disk
/// since it was opened (Rust `vault_workflows::EDIT_CONFLICT_ERROR`).
export const WORKFLOW_EDIT_CONFLICT = 'WORKFLOW_EDIT_CONFLICT';

/// Authentication failures belong to the selected agent runtime.
export const AUTH_FAILURE_HINT =
  'The selected agent could not authenticate. Sign in through that agent’s app or CLI, then retry. Scheduled automations retry on their next run.';

/// The results glob after the editor's workspace changes to `newPath`
/// (P1-04, AUTO-22): a default (none, or any known workspace's
/// `<workspace>/outputs/*`) follows the workspace; a glob the person typed stays.
export function outputGlobForScope(current: string, workspacePaths: readonly string[], newPath: string): string {
  const glob = current.trim();
  return !glob || workspacePaths.some(p => glob === `${p}/outputs/*`) ? `${newPath}/outputs/*` : current;
}

/// The prompt box's placeholder, naming where the automation runs (P1-04,
/// AUTO-22: it used to say the vault was the working directory): its
/// workspace, the Hollow folder when the working folder is empty, or its
/// working folder.
export function promptPlaceholder(workspace: string | null | undefined, cwd = ''): string {
  const where = workspace ? `It runs in ${workspace}` : cwd.trim() ? 'It runs in its working folder' : 'It runs in the Hollow folder';
  return `What the agent does each run, in plain words. ${where}, so name files there like \`outputs/\` freely.`;
}

// Live run-event row shown in the right-side run panel. Keyed by id so
// SolidJS can do efficient list updates as the stream comes in.
export interface LogRow {
  id: string;
  kind: 'started' | 'switched' | 'tool' | 'text' | 'done' | 'stopped' | 'error';
  text: string;
  ts: number;
}

export const EDITOR_SECTIONS = [
  { id: 'purpose', label: 'Purpose' },
  { id: 'scope', label: 'Inputs & scope' },
  { id: 'schedule', label: 'Schedule' },
  { id: 'destination', label: 'Destination' },
  { id: 'effects', label: 'Allowed effects' },
  { id: 'budget', label: 'Budget' },
] as const;

/// What a run of a saved automation would use, for the preview.
export function settingsFromWorkflow(w: Workflow): PreviewSettings {
  return {
    agent: w.agent, model: w.model, cwd: w.cwd, permissionMode: w.permissionMode,
    allowedTools: w.allowedTools, disallowedTools: w.disallowedTools, outputGlob: w.outputGlob,
    schedule: w.schedule, triggers: w.triggers, skipIfQuiet: w.skipIfQuiet, watch: w.watch,
    misfireGraceMin: w.misfireGraceMin,
  };
}

/// Maps a workflow's run state to the shared .status-conduit vocabulary:
/// success→ok, running→busy, cancelled→off, error→error, never→never. A run
/// a usage budget stopped (Paperclip PR 8b) keeps status `error` but is not a
/// failure: it reads as a warning.
export function conduitState(lastStatus: string | null, running: boolean, lastError?: string | null): string {
  if (running) return 'busy';
  switch (lastStatus) {
    case 'success':   return 'ok';
    case 'running':   return 'busy';
    case 'cancelled': return 'off';
    case 'paused': return 'warn';
    case 'error':     return isBudgetStopError(lastError) ? 'warn' : 'error';
    default:          return 'never';
  }
}

export function splitCsv(s: string): string[] {
  return s.split(',').map(t => t.trim()).filter(t => t.length > 0);
}

export function slugifyName(s: string): string {
  return s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 60);
}

export function shortPath(p: string): string {
  if (p.length <= 60) return p;
  return '…' + p.slice(-60);
}

export function formatSchedule(s: string): string {
  return describeSchedule(s ?? '');
}

/// Honest next-run text: a past-due timestamp means this occurrence has not
/// started yet, either because Hollow was closed or because another automated
/// run holds the vault (the scheduler starts one run per minute, oldest
/// waiting first). Within grace the scheduler starts it once the vault is
/// free; past grace it is skipped (its run-log row says whether it was missed
/// or blocked) and rescheduled, so the user has to start it. Both render
/// beside a "Run now" control, which is what the copy points at. An
/// automation the permission check holds (`paused`: the text from
/// `permissionHoldText`, Paperclip PR 5) runs neither on schedule nor from Run
/// now until that is resolved, whatever the timestamp says, so it outranks
/// both. A live run-queue row (`queued`, Paperclip PR 7: waiting behind
/// another writer, starting or running) is the next most specific fact, so it
/// outranks the timestamp but not the hold.
export function nextRunLabel(
  iso: string,
  graceMin: number | null,
  paused: string | null = null,
  queued: string | null = null,
  now: number = Date.now(),
): { text: string; warn: boolean } {
  if (paused) return { text: paused, warn: true };
  if (queued) return { text: queued, warn: true };
  const state = nextRunState(iso, graceMin, now);
  if (state === 'missed') return { text: 'missed. Run now to catch up.', warn: true };
  if (state === 'catchup') return { text: 'due now; starts when Hollow is free', warn: true };
  const date = new Date(iso);
  const absolute = Number.isFinite(date.getTime()) ? date.toLocaleString([], { weekday: 'short', hour: 'numeric', minute: '2-digit', timeZoneName: 'short' }) : '';
  return { text: absolute ? `${absolute} · ${relative(iso, now)}` : relative(iso, now), warn: false };
}

export function workflowGroup(w: Workflow): 'Needs attention' | 'Scheduled or triggered' | 'Manual' {
  if (w.permissionHold || w.budgetHold || w.scheduleProblem || w.lastStatus === 'error') return 'Needs attention';
  return w.schedule !== 'manual' || w.triggers.length ? 'Scheduled or triggered' : 'Manual';
}

export function workflowState(w: Workflow, running: boolean): string {
  if (running) return 'Running';
  if (w.permissionHold) return 'Access review needed';
  if (w.budgetHold || isBudgetStopError(w.lastError)) return 'Budget paused';
  if (w.scheduleProblem) return 'Schedule needs attention';
  if (w.lastStatus === 'error') return 'Last run failed';
  if (w.lastStatus === 'cancelled') return 'Last run stopped';
  if (w.lastStatus === 'paused') return 'Paused for requested work';
  return w.lastStatus === 'success' ? 'Last run passed' : w.lastStatus ? `Last run: ${w.lastStatus}` : 'Not run yet';
}

/// The row and detail-pane words for an unreadable schedule (B04).
export const SCHEDULE_PROBLEM_ROW = 'Schedule can’t be read, so this won’t run on schedule';
export const SCHEDULE_PROBLEM_DETAIL =
  'This schedule can’t be read, so the automation won’t run on schedule. Use Edit to set it again.';

/// What an automation's row says under its title. `paused` is the permission
/// or budget hold text and `queued` the live run-queue chip text (the page
/// reads both from stores, so they come in as arguments). A live queue row
/// wins (it is happening now), unless a hold refuses the automation. Next, a
/// schedule the backend can't read (`scheduleProblem`, B04): the scheduler
/// never fires it, so the row says so rather than "—". Then the manual,
/// next-run and last-run text as before.
export function workflowSubline(
  w: Pick<Workflow, 'schedule' | 'nextRunAt' | 'lastRunAt' | 'misfireGraceMin' | 'scheduleProblem'>,
  paused: string | null = null,
  queued: string | null = null,
  now: number = Date.now(),
): { text: string; warn: boolean } {
  const live = paused ? null : queued;
  if (live) return { text: live, warn: false };
  if (w.scheduleProblem) return { text: SCHEDULE_PROBLEM_ROW, warn: true };
  const last = w.lastRunAt;
  if (w.schedule === 'manual') {
    return { text: last ? `last run ${relative(last, now)}` : 'never run', warn: false };
  }
  if (w.nextRunAt) {
    const l = nextRunLabel(w.nextRunAt, w.misfireGraceMin, paused, null, now);
    return l.warn ? l : { text: `next ${l.text}`, warn: false };
  }
  return { text: last ? `last run ${relative(last, now)}` : '—', warn: false };
}

export function relative(iso: string, now: number = Date.now()): string {
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return iso;
  const diff = t - now;
  const abs = Math.abs(diff);
  const m = Math.floor(abs / 60_000);
  const h = Math.floor(m / 60);
  const d = Math.floor(h / 24);
  let label: string;
  if (m < 1) label = 'just now';
  else if (m < 60) label = `${m}m`;
  else if (h < 24) label = `${h}h`;
  else label = `${d}d`;
  if (label === 'just now') return label;
  return diff < 0 ? `${label} ago` : `in ${label}`;
}
