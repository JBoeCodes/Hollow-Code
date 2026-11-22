// The run review's controller (plan completion L2, item 6): everything the
// Automations run review (`OutputPane` in pages/Workflows.tsx) knows and does,
// behind one interface, so the page renders and this decides. Moved from the
// component unchanged, plus the run outcome and the guarded retry (item 3).
//
// Created inside the component (it owns the resources, effects and listener
// it creates), with the component's props object so every read stays
// reactive.
import { createEffect, createMemo, createResource, createSignal, onCleanup, onMount } from 'solid-js';
import { api } from '../../api';
import { applicableBudgets, todayUsageText } from '../../workflowPreview';
import { failureFix, isAuthFailureText, receiptCheckCounts, type FailureFixAction } from '../../workflowHealth';
import { budgetOfLimit, budgetStateLabel, isBudgetStop, isBudgetStopError, latestRunIsBudgetStop, windowLabel } from '../../budgets';
import { runQueue } from '../../store/runQueue';
import { liveItemForSlug } from '../../runQueue';
import { executionReceiptFromWire, type EvidenceRunContextWire, type ExecutionReceipt as ExecutionReceiptData } from '../../executionReceipt';
import { isExpectedCancellation } from '../../toastPolicy';
import { listen } from '../../nativeEvents';
import { scopedListen } from '../../scopedListen';
import { refreshReviewInbox, requestSuggestions } from '../../reviewInbox';
import { isPermissionRefusal } from '../../workflowPermissions';
import { isBudgetRefusal } from '../../budgets';
import { CHECK_STATE, checkState, type StateWord } from '../../workState';
import { OPS_STATE, automationState, runOutcome, type OpsState, type RunOutcome } from '../../runOutcome';
import { projectWorkspaces as workspaces } from '../../store/workspaces';
import {
  refetchWorkflows, flashFooter, showConfirm, trackTask, AGENT_LABELS, agentsAvailable, agentSettings,
  setActivePage, requestOpenVaultNote, noteIndex, refreshNoteIndex, setActiveBrief,
  budgetsView, enableBudgetsResource, openSettingsFromOutside,
} from '../../store';
import type { RunLogEntry, Workflow, WorkflowEvent, WorkflowOutput } from '../../types';
import { relative } from './workflowText';

/// A run can be refused because the automation's settings changed outside the
/// editor. Refresh the review queue either way, and open Waiting on you on
/// the permission changes when that is why it didn't run.
export function noteRunFailure(error: unknown) {
  refreshReviewInbox();
  if (isPermissionRefusal(error)) requestSuggestions('automations');
  // A usage budget holds it (Paperclip PR 8a): open it on the budget pauses.
  else if (isBudgetRefusal(error)) requestSuggestions('budgets');
}

/** The agent a run of `w` uses: its own, else the default, else the first available. */
const workflowAgentKey = (w: Pick<Workflow, 'agent'>) => w.agent || agentSettings().defaultAgent || agentsAvailable()[0] || '';

/** That agent by name. */
export function workflowAgentName(w: Pick<Workflow, 'agent'>): string {
  const key = workflowAgentKey(w);
  return AGENT_LABELS[key as keyof typeof AGENT_LABELS] ?? (key || 'The agent');
}

/** Where a run of `w` works, in words: its workspace, else its folder, else the vault. */
export function workflowTarget(w: Pick<Workflow, 'cwd'> & Partial<Pick<Workflow, 'agent'>>): string {
  const ws = workspaces().find(x => x.cwd === w.cwd);
  return ws ? `the ${ws.title} workspace` : w.cwd ? w.cwd : 'the vault';
}

/** Run now from the list (the row's bolt and its menu's Run now, P0-13): a
 *  real, costed run one click away from opening the row, so it asks first,
 *  naming the agent and where it works. The review's own Run now says the
 *  same under the button and runs at once. */
export function confirmRunNow(w: Workflow): Promise<boolean> {
  const title = w.title || 'untitled automation';
  return showConfirm({
    title: `Run “${title}” now?`,
    body: `A real run: ${workflowAgentName(w)} works in ${workflowTarget(w)} and uses model allowance.`,
    confirmLabel: 'Run now', focusCancel: true,
  });
}

export interface RunReviewProps {
  workflow: Workflow;
  running?: boolean;
  onEdit: () => void;
}

/** Run-log statuses of a run that started (skipped and deferred rows
 *  describe an occurrence that never ran). */
const RAN = new Set(['success', 'error', 'cancelled', 'interrupted', 'paused']);

/** The newest run that actually ran: the run the review explains. */
export function newestRun(log: readonly RunLogEntry[] | undefined): RunLogEntry | null {
  return [...(log ?? [])].sort((a, b) => (a.at < b.at ? 1 : -1)).find(r => RAN.has(r.status)) ?? null;
}

export function createRunReview(props: RunReviewProps) {
  const [outputs, setOutputs] = createSignal<WorkflowOutput[]>([]);
  const [selected, setSelected] = createSignal<string | null>(null);
  const [content, setContent] = createSignal('');
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [running, setRunning] = createSignal(false);
  const [previewOpen, setPreviewOpen] = createSignal(false);
  const [latestReceipt, setLatestReceipt] = createSignal<ExecutionReceiptData | null>(null);
  const [detail, { refetch: refetchDetail }] = createResource(
    () => props.workflow.slug,
    (slug) => api.readWorkflow(slug),
  );
  // Durable receipts double as an output discovery path: when a workflow
  // declares no output_glob (or the glob matches nothing), the files its
  // recent runs actually wrote — filesystem-diff-derived — stand in.
  const [receiptWires, { refetch: refetchReceipts }] = createResource(
    () => props.workflow.slug,
    (slug) => api.listEvidenceReceipts('workflow', slug, 100),
  );
  // Today's usage of this automation, for the real-run note (a local read of
  // the usage ledger; null when it can't be read).
  const [usage, { refetch: refetchUsage }] = createResource(
    () => props.workflow.slug,
    () => api.usageSummary('day').catch(() => null),
  );
  onMount(() => enableBudgetsResource());
  const [runContext, setRunContext] = createSignal<EvidenceRunContextWire | null>(null);

  const derivedOutputs = createMemo<WorkflowOutput[]>(() => {
    if (loading() || outputs().length > 0) return [];
    if (detail.loading || receiptWires.loading) return [];
    const byRun = new Map((receiptWires() ?? []).map(w => [w.runId, w] as const));
    const runs = [...(detail()?.runLog ?? [])]
      .sort((a, b) => (a.at < b.at ? 1 : -1))
      .filter(r => r.runId && byRun.has(r.runId))
      .slice(0, 5);
    const seen = new Set<string>();
    const out: WorkflowOutput[] = [];
    for (const run of runs) {
      for (const f of byRun.get(run.runId!)!.files) {
        if (f.changeType === 'delete' || seen.has(f.path)) continue;
        // The run's own bookkeeping write to workflows/*.md isn't an output,
        // and dot-prefixed state files aren't viewable.
        if (f.path.startsWith('workflows/')) continue;
        if (f.path.split('/').some(seg => seg.startsWith('.'))) continue;
        seen.add(f.path);
        const base = f.path.split('/').pop() ?? f.path;
        const dot = base.lastIndexOf('.');
        const ext = dot > 0 ? base.slice(dot + 1).toLowerCase() : '';
        // Text formats only — the viewer reads through vaultNote, so a
        // binary write (image, db) has no sane rendering here.
        if (!['md', 'markdown', 'html', 'htm', 'txt', 'json', 'csv', 'log'].includes(ext)) continue;
        out.push({
          path: f.path,
          name: dot > 0 ? base.slice(0, dot) : base,
          format: ext === 'html' || ext === 'htm' ? 'html'
            : ext === 'md' || ext === 'markdown' ? 'md'
            : ext,
          modified: run.at,
        });
      }
    }
    return out;
  });
  const effectiveOutputs = createMemo(() => (outputs().length > 0 ? outputs() : derivedOutputs()));
  const usingDerived = createMemo(() => outputs().length === 0 && derivedOutputs().length > 0);

  // Mount-time evidence: the newest receipted run's receipt renders without
  // waiting for a live run event (run recall context unknown → omitted). A
  // live run resets this and repopulates from its own `done` payload.
  createEffect(() => {
    if (latestReceipt() || running() || props.running) return;
    if (detail.loading || receiptWires.loading) return;
    const byRun = new Map((receiptWires() ?? []).map(w => [w.runId, w] as const));
    const newest = [...(detail()?.runLog ?? [])]
      .sort((a, b) => (a.at < b.at ? 1 : -1))
      .find(r => r.runId && byRun.has(r.runId));
    if (newest) setLatestReceipt(executionReceiptFromWire(byRun.get(newest.runId!)!, null));
  });

  const current = createMemo(() => effectiveOutputs().find(o => o.path === selected()) ?? null);

  // Local `running` covers the cover-button path; `props.running` carries
  // runs fired elsewhere (row bolt, scheduler) via the page-level listener.
  const isRunning = () => running() || !!props.running;
  // The newest run a usage budget ended (Paperclip PR 8b): not a failure,
  // so no failure reason; it points at Budget pauses instead.
  const lastBudgetStop = createMemo(() =>
    props.workflow.lastStatus === 'error'
    && (latestRunIsBudgetStop(detail()?.runLog) || isBudgetStopError(props.workflow.lastError)));
  const coverConduitState = (): string | undefined => {
    if (isRunning()) return 'busy';
    if (props.workflow.lastStatus === 'error') return lastBudgetStop() ? 'warn' : 'error';
    if (props.workflow.lastStatus === 'success') return 'ok';
    if (props.workflow.lastStatus === 'paused') return 'warn';
    return undefined; // never run / cancelled — resting hairline
  };

  const agentKey = () => workflowAgentKey(props.workflow);
  const agentName = () => workflowAgentName(props.workflow);
  const target = () => workflowTarget(props.workflow);

  // ── The newest run and what it did (plan completion L2) ─────────────────
  const latestRun = createMemo(() => newestRun(detail.latest?.runLog));
  const latestRunWire = createMemo(() => {
    const run = latestRun();
    if (!run?.runId) return null;
    return (receiptWires.latest ?? []).find(w => w.historyRunId === run.runId || w.runId === run.runId) ?? null;
  });
  const latestRunReceipt = createMemo(() => {
    const wire = latestRunWire();
    return wire ? executionReceiptFromWire(wire, runContext()) : null;
  });
  const [cost] = createResource(
    () => latestRun()?.runId ?? undefined,
    (runId) => api.runCost(runId).catch(() => null),
  );
  const blocked = () => !!(props.workflow.permissionHold || props.workflow.budgetHold);
  /** A row of this automation is queued or running: a retry pressed now
   *  would join it (the queue coalesces manual rows), unguarded. */
  const queuedOrLive = () => !!liveItemForSlug(runQueue(), props.workflow.slug);
  /** What the newest run did; null before the run log and receipts first
   *  load, or while a run is live. A refetch keeps the last answer (`latest`)
   *  so the section doesn't blink. */
  const outcome = createMemo<RunOutcome | null>(() => {
    if (isRunning() || detail.latest === undefined || receiptWires.latest === undefined) return null;
    const run = latestRun();
    if (!run) return null;
    return runOutcome({
      status: run.status,
      receipt: latestRunWire(),
      outputGlob: props.workflow.outputGlob,
      cost: !run.runId ? null : cost.loading ? undefined : (cost() ?? null),
      // The receipt's own classification first (`result_kind` auth), then the text.
      authFailure: run.status === 'error' && (latestRunWire()?.resultKind === 'auth' || isAuthFailureText(run.note, run.errorTail)),
      blocked: blocked(),
      budgetStop: isBudgetStop(run),
    });
  });

  // The status line, in the shared operational vocabulary.
  const state = createMemo<{ key: OpsState; word: StateWord }>(() => {
    const live = liveItemForSlug(runQueue(), props.workflow.slug);
    const key = automationState({
      running: isRunning() || (!!live && live.status !== 'queued') || props.workflow.lastStatus === 'running',
      queued: live?.status === 'queued',
      blocked: blocked(),
      lastStatus: props.workflow.lastStatus,
      budgetStop: lastBudgetStop(),
      produced: (outcome()?.outputs.length ?? 0) > 0,
    });
    return { key, word: OPS_STATE[key] };
  });
  const checks = createMemo(() => {
    const r = latestRunReceipt();
    if (!r || isRunning()) return null;
    return CHECK_STATE[checkState(receiptCheckCounts(r.checks))];
  });

  const latestFailure = createMemo(() => [...(detail()?.runLog ?? [])]
    .sort((a, b) => (a.at < b.at ? 1 : -1))
    .find(r => r.status === 'error' && !isBudgetStop(r)) ?? null);
  const fix = createMemo(() => {
    if (lastBudgetStop() && !props.workflow.permissionHold) return null;
    const failedCheck = latestRunReceipt()?.checks.find(c => c.outcome === 'failed')?.command ?? null;
    return failureFix({
      permissionHold: props.workflow.permissionHold,
      budgetHold: props.workflow.budgetHold,
      lastStatus: props.workflow.lastStatus,
      lastError: props.workflow.lastError,
      errorTail: latestFailure()?.errorTail ?? null,
      agentName: agentName(),
      failedCheck,
      // Only the newest run's receipt can say whether it changed anything.
      changedAnything: latestRun()?.status === 'error' ? outcome()?.changedAnything : undefined,
      outcomeShown: !!outcome(),
    });
  });
  function applyFix(action: FailureFixAction) {
    if (action === 'permissions') requestSuggestions('automations');
    else if (action === 'budgets') requestSuggestions('budgets');
    else if (action === 'accounts') { openSettingsFromOutside('accounts'); }
    else if (action === 'edit') props.onEdit();
  }

  // The real-run note: what Run now costs, from the ledger and budgets.
  const budgetNote = createMemo(() => {
    if (budgetsView.error) return null;
    const list = applicableBudgets(budgetsView()?.budgets ?? [], props.workflow.slug, agentKey());
    if (!list.length) return null;
    const tightest = [...list].sort((a, b) => (b.percent ?? -1) - (a.percent ?? -1))[0];
    return `Budget for ${tightest.scopeLabel}: ${budgetOfLimit(tightest)} ${windowLabel(tightest.window).toLowerCase()} (${budgetStateLabel(tightest.state, tightest.pause?.held)}).`;
  });
  const usageNote = createMemo(() => {
    const u = usage();
    if (usage.loading || !u) return null;
    return todayUsageText(u.byWorkflow.find(b => b.key === props.workflow.slug));
  });

  async function loadOutputs() {
    setLoading(true);
    setError(null);
    try {
      const list = await api.listWorkflowOutputs(props.workflow.slug); // newest-first
      setOutputs(list);
    } catch (e: any) {
      setError(e?.message || String(e));
    } finally {
      setLoading(false);
    }
  }

  // Selection follows whichever output list is in effect (glob-discovered or
  // receipt-derived, both resolved asynchronously): auto-open the newest,
  // clear the pane when nothing renders.
  createEffect(() => {
    if (loading()) return;
    const list = effectiveOutputs();
    if (list.length === 0) {
      if (selected()) {
        setSelected(null);
        setContent('');
        setActiveBrief(null);
      }
      return;
    }
    if (!selected() || !list.some(o => o.path === selected())) {
      void select(list[0]);
    }
  });

  async function select(o: WorkflowOutput) {
    setSelected(o.path);
    try {
      const note = await api.vaultNote(o.path);
      setContent(note.content);
      // Expose the viewed output to chat page-context (HTML is text-stripped there).
      setActiveBrief({ date: o.name, content: note.content, format: o.format === 'html' ? 'html' : 'md' });
    } catch (e: any) {
      setError(e?.message || String(e));
      setContent('');
    }
  }

  /** Run now, or (plan completion L2) the guarded retry of `retryOf`: the
   *  failed run whose completed outside effects the new run must not repeat. */
  async function runNow(retryOf?: string | null) {
    if (running()) return;
    setRunning(true);
    try {
      const result = await trackTask(
        retryOf ? `Run ${props.workflow.title} again without repeating` : `Run ${props.workflow.title}`,
        () => api.runWorkflow(props.workflow.slug, retryOf ?? null),
        { onSuccessStatus: r => r.status === 'cancelled' ? 'cancelled' : 'success' },
      );
      if (result.receipt) setLatestReceipt(executionReceiptFromWire(result.receipt, runContext()));
      await loadOutputs();
      refetchDetail();
      refetchReceipts();
      refetchUsage();
      refetchWorkflows();
      // A stopped run is not a completed one (Paperclip PR 22: this used to
      // say "✓ run complete" for a cancelled run).
      if (result.status === 'cancelled') flashFooter('Stopped.', 1500);
      else flashFooter(result.evidenceError
        ? `⚠ run finished, but evidence finalization failed: ${result.evidenceError}`
        : '✓ run complete', result.evidenceError ? 5000 : 1500);
    } catch (e: any) {
      if (isExpectedCancellation(e)) {
        flashFooter('Stopped.', 1500);
      } else {
        noteRunFailure(e);
        flashFooter(`× run failed: ${e?.message || e}`, 4000);
      }
    } finally {
      setRunning(false);
    }
  }

  // Clicking a vault link inside an output — `[[wikilink]]` in markdown, or
  // `<a data-vault="Name">` in HTML (via HtmlDoc's postMessage bridge) —
  // jumps to that note in the Vault tab.
  function openInVault(name: string) {
    const match = noteIndex().find(n => n.basename.toLowerCase() === name.toLowerCase());
    if (match) {
      setActivePage('vault');
      requestOpenVaultNote(match.path);
    } else {
      flashFooter(`No vault note named "${name}"`, 2500);
    }
  }

  /** Show an output the outcome names: in Latest result when it is one of
   *  the results shown there, else in the vault. */
  function showOutput(path: string) {
    const listed = effectiveOutputs().find(o => o.path === path);
    if (listed) { void select(listed); return; }
    setActivePage('vault');
    requestOpenVaultNote(path);
  }

  // Reload outputs whenever the selected workflow ACTUALLY changes.
  // `props.workflow` is a JSX getter over a store resource, so every
  // `refetchWorkflows()` — which fires on every completed run, of any
  // workflow — handed this effect a fresh object with the same slug and it
  // tore down the artifact the user was reading. The guard is a plain `let`,
  // not a signal: it is a guard, not state. Read the slug first so the
  // effect stays subscribed.
  let lastSlug: string | null = null;
  createEffect(() => {
    const slug = props.workflow.slug;
    if (slug === lastSlug) return;
    lastSlug = slug;
    setSelected(null);
    setContent('');
    setActiveBrief(null);
    setLatestReceipt(null);
    setPreviewOpen(false);
    setRunContext(null);
    loadOutputs();
  });
  // Falling edge of an external run (row bolt, scheduler) → a fresh
  // edition may exist; reload through the same loadOutputs path. The run
  // log + receipts moved too (they feed the derived-output fallback).
  let wasRunning = false;
  createEffect(() => {
    const r = !!props.running;
    if (wasRunning && !r) {
      loadOutputs();
      refetchDetail();
      refetchReceipts();
      refetchUsage();
    }
    wasRunning = r;
  });
  onMount(() => {
    if (noteIndex().length === 0) refreshNoteIndex();
    onCleanup(scopedListen(() => listen<WorkflowEvent>('workflow-event', (event) => {
      const payload = event.payload;
      if (payload.slug !== props.workflow.slug) return;
      if (payload.type === 'started') {
        setRunContext({ runId: payload.runId, recall: payload.recall });
        setLatestReceipt(null);
      } else if (payload.type === 'done') {
        if (payload.receipt) setLatestReceipt(executionReceiptFromWire(payload.receipt, runContext()));
        if (payload.evidenceError) flashFooter(`⚠ evidence finalization failed: ${payload.evidenceError}`, 5000);
      }
    })));
  });

  const outputCaption = createMemo(() => {
    const o = current();
    if (!o) return '';
    const parts = [o.path, relative(o.modified)];
    const last = props.workflow.lastRunAt;
    if (props.workflow.lastStatus === 'error' && last && o.modified < last) parts.push('from an earlier run');
    return parts.join(' · ');
  });

  return {
    // reads
    detail, receiptWires, loading, error, content, selected, current, effectiveOutputs, usingDerived,
    latestReceipt, previewOpen, isRunning, lastBudgetStop, coverConduitState,
    agentName, target, state, checks, fix, budgetNote, usageNote, outputCaption,
    latestRun, latestRunWire, latestRunReceipt, outcome, queuedOrLive,
    // actions
    setPreviewOpen, select, runNow, applyFix, openInVault, showOutput,
  };
}

export type RunReview = ReturnType<typeof createRunReview>;
