import { endsLabel, endsProblem } from '../workflowEnds';
import { formatSeconds } from '../formatTime';
import { TriggerEditor } from './workflows/TriggerEditor';
import { EventWhen } from './workflows/EventWhen';
import { turnOnFor } from '../eventTriggersApi';
import { runDurations } from './workflows/runDurations';
import { newAutomationWindow, setNewAutomationWindow } from './workflows/newRequest';
import { NEW_CHAT, newAutomationChat, setNewAutomationChat, type RunsInChat } from './workflows/runsInChoice';
import { RunsIn } from './workflows/RunsIn';
import { RunsOn } from './workflows/RunsOn';
import { followOwnerMacs, ownerMacs, readOwnerMacs } from './workflows/ownerMacs';
import { runsOnRowNote, vaultShared } from './workflows/runsOnChoice';

import { WindowsAndQueue } from './workflows/WindowsAndQueue';
import { rememberWindowLabels } from '../windowLabels';
import { projectWorkspaces as workspaces, workspaces as allWorkspaces, refreshWorkspaces, workspaceId } from '../store/workspaces';
import { isMainAgent } from '../workspaceTypes';
import { ModelPicker } from '../components/ModelPicker';
import { For, Show, createEffect, createMemo, createResource, createSignal, lazy, onCleanup, onMount, untrack } from 'solid-js';
import { CONTEXT_AUDIT_TEMPLATE } from '../contextLayers';
import { guardNavigation } from '../store/navigationGuard';
import { useAutomationDraftGuard } from './workflows/draftGuard';
import { listen } from '../nativeEvents';
import { scopedListen } from '../scopedListen';
import { RunOutcome } from '../components/RunOutcome';
import { BackgroundWorkExplainer } from '../components/BackgroundWorkExplainer';
import { BackgroundClock } from '../components/BackgroundClock';
import type { AutomationWindows } from '../generated/ipc';
import { confirmRunNow, createRunReview, noteRunFailure, workflowAgentName, type RunReviewProps } from './workflows/runReview';
import { createWorkflowEditor, type WorkflowEditorProps } from './workflows/workflowEditor';
import { createWorkflowStop } from './workflows/workflowStop';
import {
  AUTH_FAILURE_HINT, EDITOR_SECTIONS, SCHEDULE_PROBLEM_DETAIL, conduitState, formatSchedule, nextRunLabel, outputGlobForScope,
  promptPlaceholder, relative, settingsFromWorkflow, workflowGroup, workflowState, workflowSubline, type LogRow,
} from './workflows/workflowText';
import { Button, EmptyState, ErrorDetails, ErrorText, IconButton, Menu, PageHeader, Section, Skeleton, StatePanel, StatusPill } from '../components/ui';
import { resetAutomationsView } from '../store/automationsViews';
import { OutsideSuspense } from '../components/OutsideSuspense';
import { RecoverableView } from '../components/RecoverableView';
import '../styles/destinations.css';
import { Icon } from '../icons';
import { NoteRenderer } from '../components/NoteRenderer';
import { HtmlDoc } from '../components/HtmlDoc';
import { ExecutionReceipt } from '../components/ExecutionReceipt';
import { WaitingLink } from '../components/WaitingLink';
import { refreshReviewInbox, requestSuggestions } from '../reviewInbox';
import { permissionHoldText, type StampedChanges } from '../workflowPermissions';
import { permModeOptions, permModeUnavailable } from '../permModes';
import { NOTIFY_LABELS, NOTIFY_OPTIONS, isAuthFailureText, watchPlaceholder, type NotifyOption } from '../workflowHealth';
import { applicableBudgets, buildConfigPreview, type PreviewSettings } from '../workflowPreview';
import { budgetHoldText, budgetOfLimit, budgetStateLabel, isBudgetStop, isBudgetStopError, windowLabel } from '../budgets';
import '../styles/budget-pauses.css';
import { runQueue } from '../store/runQueue';
import { liveItemForSlug, queueChipText } from '../runQueue';
import { api } from '../api';
import { executionReceiptFromWire } from '../executionReceipt';
import { historyStatusLabel } from '../runOutcome';
import { fmtTime, stripFrontmatter } from '../utils';
import { isExpectedCancellation } from '../toastPolicy';
import {
  workflowsList, refetchWorkflows, enableWorkflowsResource, flashFooter, showConfirm, openContextMenu,
  type ContextMenuItem, PERM_MODE_LABELS, permModeDescFor, PERM_MODE_ORDER,
  workflowOpenRequest, setWorkflowOpenRequest, KNOWN_AGENTS, AGENT_LABELS, agentsAvailable, agentSettings, budgetsView,
  enableBudgetsResource, openSettingsFromOutside, openVaultHistory,
} from '../store';
import type { Workflow, WorkflowDetail, WorkflowEvent, WorkflowInput, RunLogEntry } from '../types';
import type { EvidenceReceiptWire } from '../executionReceipt';
import { Select } from '../components/Select';
import { RefreshNotice } from '../components/RefreshNotice';
import {
  DAYS, describeSchedule, firstDstSkip, formatRun, missedRunSentences, nextRuns, parseScheduleModel,
  renderScheduleModel, scheduleWindow, type ScheduleKind,
} from '../workflowSchedule';


/// An automation the backend's permission check refuses (a pending review, a
/// rejected change, or unreadable permission records) runs neither on
/// schedule nor from Run now, so its next-run text must not say it "starts
/// when Hollow is free". `permissionHold` comes from `list_workflows`, which
/// applies the scheduler's own rule.
///
/// A usage budget that holds it (`budgetHold`, Paperclip PR 8a: a hard-stop
/// pause or a budget at its limit) pauses it too, below a permission hold:
/// precedence permission refusal > budget pause > queued > catch-up.
function permissionPaused(workflow: Workflow): string | null {
  return permissionHoldText(workflow.permissionHold) ?? budgetHoldText(workflow.budgetHold);
}

/// The workflow's live run-queue row (Paperclip PR 7), as chip text: queued
/// behind another writer, starting, or running. A permission hold outranks
/// it (see `nextRunLabel`).
function queuedLabel(workflow: Workflow): string | null {
  const item = liveItemForSlug(runQueue(), workflow.slug);
  return item ? queueChipText(item) : null;
}

// Starter templates surface in "+ New automation". Each is just a
// pre-filled WorkflowInput — schedule + body are the meaningful bits;
// everything else falls back to defaults.
interface Template {
  id: string;
  label: string;
  description: string;
  schedule: string;
  prompt: string;
}
const TEMPLATES: Template[] = [
  { id: 'blank', label: 'Custom procedure', description: 'Repeat a procedure you have already checked.', schedule: 'manual', prompt: '' },
  { id: 'continue-plan', label: 'Continue the plan', description: 'Make the next bounded step toward this workspace’s outcome.', schedule: 'manual', prompt: 'Read WORKSPACE.md and PLAN.md. Use only the attached evidence needed for the next unfinished step. Save the result in outputs/, record what you checked, and update the brief and plan. Report missing evidence or unavailable tools. Do not deliver externally.' },
  { id: 'source-review', label: 'Review attached evidence', description: 'Check sources for changes, gaps and decisions needed.', schedule: 'manual', prompt: 'Review this workspace’s brief, plan and attached source evidence. Distinguish captured facts from stale, partial or unavailable coverage. Write a dated evidence review in outputs/ with citations, unresolved questions and the next useful action. Do not infer remote freshness from a local snapshot or change original source files.' },
  // Leak checks (CX10): a template only; the person adds it, nothing enables it by itself.
  CONTEXT_AUDIT_TEMPLATE,
];

type ViewMode = 'output' | 'detail' | 'new';

const MainAgentActions = lazy(() => import('../components/MainAgentActions'));

/** Automations reuse the same context and output folder as interactive work. */
export function Workflows() { return <WorkflowsView />; }

function WorkflowsView() {
  onMount(()=>{enableWorkflowsResource();void refreshWorkspaces({ join: true });void readOwnerMacs();onCleanup(followOwnerMacs());});
  // Leaving Automations: the next visit starts at the automations list.
  onCleanup(resetAutomationsView);
  // Saved window labels for `describeSchedule` (overnight S10): read when the
  // page opens, never at launch; a failure leaves the shipped names.
  const [windows, setWindows] = createSignal<AutomationWindows | null>(null);
  const [windowsLoading, setWindowsLoading] = createSignal(true);
  const [windowsError, setWindowsError] = createSignal('');
  let windowRead = 0;
  onCleanup(() => { windowRead++; });
  const refreshWindows = async () => {
    const read = ++windowRead;
    setWindowsLoading(!windows());
    try {
      const value = await api.automationWindows();
      if (read !== windowRead) return;
      setWindows(value); setWindowsError(''); rememberWindowLabels(value.windows);
    } catch (error) {
      if (read === windowRead) setWindowsError(String(error));
    } finally { if (read === windowRead) setWindowsLoading(false); }
  };
  onMount(() => {
    void refreshWindows();
    const refreshVisible = () => { if (!document.hidden) void refreshWindows(); };
    const timer = setInterval(() => {
      if (document.hidden) return;
      const expired = windows()?.windows.some(w => w.nextStarts?.some(at => new Date(at).getTime() <= Date.now()));
      if (expired) void refreshWindows();
    }, 60_000);
    window.addEventListener('focus', refreshVisible);
    document.addEventListener('visibilitychange', refreshVisible);
    onCleanup(() => {
      clearInterval(timer);
      window.removeEventListener('focus', refreshVisible);
      document.removeEventListener('visibilitychange', refreshVisible);
    });
  });

  const [active, setActive] = createSignal<Workflow | null>(null);
  const [view, setView] = createSignal<ViewMode>('output');
  const [newSchedule, setNewSchedule] = createSignal('manual');
  const [newChat, setNewChat] = createSignal<RunsInChat | null>(null);
  createEffect(() => {
    const name = newAutomationWindow();
    if (!name) return;
    setNewAutomationWindow(null);
    setNewSchedule(`window:${name}/run-once-on-wake`);
    setNewChat(null);
    setShowList(false); setView('new'); setActive(null);
  });
  // A chat's Schedule in this chat… (overhaul F1): the form opens bound to it.
  createEffect(() => {
    const chat = newAutomationChat();
    if (!chat) return;
    setNewAutomationChat(null);
    setNewSchedule('manual');
    setNewChat(chat);
    setShowList(false); setView('new'); setActive(null);
  });
  // Owner Mac (M4): whether another Mac shares the vault, by the backend's
  // rule over this list and the paired Macs (false until they are read).
  const ownersShared = createMemo(() => {
    const macs = ownerMacs();
    return !!macs && vaultShared(macs, (workflowsList() ?? []).map(w => w.runsOn));
  });
  const [filter, setFilter] = createSignal('');
  const [showList, setShowList] = createSignal(true);
  const initialListError = () => !workflowsList() && !!workflowsList.error;
  const listFailure = () => <StatePanel kind="failed" compact title="Couldn't read your automations" error={workflowsList.error}
    actions={<Button size="sm" disabled={workflowsList.loading} onClick={() => refetchWorkflows()}>Retry</Button>} />;

  // Page-level live-run tracking. The detail pane keeps its slug-scoped
  // listener for the streaming log; this one only maintains the set of
  // slugs currently running so the aside conduits + header pill light up
  // for scheduler-fired runs too. Refetch on 'done' only — not 'started'.
  const [eventRunningSlugs, setRunningSlugs] = createSignal<Set<string>>(new Set());
  const runningSlugs = createMemo(() => {
    const slugs = new Set(eventRunningSlugs());
    const queue = runQueue();
    for (const slug of new Set(queue.map(row => row.sourceId))) {
      if (!slug) continue;
      const live = liveItemForSlug(queue, slug);
      if (live && live.status !== 'queued') slugs.add(slug);
    }
    return slugs;
  });
  onMount(() => onCleanup(scopedListen(() =>
    listen<WorkflowEvent>('workflow-event', (e) => {
      const p = e.payload;
      if (p.type === 'started') {
        setRunningSlugs(prev => new Set(prev).add(p.slug));
      } else if (p.type === 'done') {
        setRunningSlugs(prev => {
          const n = new Set(prev);
          n.delete(p.slug);
          return n;
        });
        refetchWorkflows();
        // The run's failure streak was recorded (opened or closed) before
        // this event, in the unwatched review queue (Paperclip PR 9).
        refreshReviewInbox();
      }
    }))));

  const filtered = createMemo(() => {
    const all = workflowsList() ?? [];
    const q = filter().toLowerCase().trim();
    if (!q) return all;
    return all.filter(w =>
      w.title.toLowerCase().includes(q) ||
      w.description.toLowerCase().includes(q)
    );
  });

  // Recovery first; trigger-only workflows are automatic too.
  const groupLabels = ['Needs attention', 'Scheduled or triggered', 'Manual'];
  const groupItems = (label: string) => filtered().filter(w => workflowGroup(w) === label);

  // Restore the previously-active workflow when the list refreshes.
  createEffect(() => {
    const list = workflowsList();
    if (!list) return;
    if (view() === 'new') return;
    const current = active();
    if (current) {
      const match = list.find(w => w.slug === current.slug);
      if (match) {
        // Compare by content, not identity: `workflowsList` is an IPC
        // resource, so a refetch always deserializes fresh objects and
        // `match !== current` was unconditionally true. Include lastRunAt /
        // lastStatus because the cover conduit and the failure panels render
        // from them — dropping those would freeze the status light. The
        // schedule fields too, so an outside edit that breaks (or fixes) the
        // schedule reaches the open pane as well as the row (B04).
        if (match.slug !== current.slug
          || match.lastRunAt !== current.lastRunAt
          || match.lastStatus !== current.lastStatus
          || match.lastError !== current.lastError
          || match.schedule !== current.schedule
          || match.nextRunAt !== current.nextRunAt
          || match.scheduleProblem !== current.scheduleProblem
          || match.permissionHold !== current.permissionHold
          || match.budgetHold !== current.budgetHold
          || match.runsOn !== current.runsOn
          || match.endedNote !== current.endedNote
          || match.title !== current.title) setActive(match);
        return;
      }
    }
    if (!current && list.length > 0) setActive(list[0]);
  });

  // Native banners retain their foreground workflow scope. In-app toasts
  // use the bell policy. Keyed on the slug so refetch never re-sends it.
  const foregroundSlug = createMemo(() => active()?.slug ?? null);
  createEffect(() => {
    const slug = foregroundSlug();
    void api.setForegroundNotificationScope(slug ? `workflow:${slug}` : null).catch(() => { /* not in tauri */ });
  });
  onCleanup(() => {
    void api.setForegroundNotificationScope(null).catch(() => { /* not in tauri */ });
  });

  // Cross-page request — a notification click sets this signal so we
  // jump to the right workflow's detail pane. Honoured even when the
  // page was unmounted at the moment of the click.
  createEffect(() => {
    const req = workflowOpenRequest();
    if (!req) return;
    const list = workflowsList();
    if (!list) return;
    const match = list.find(w => w.slug === req.slug);
    if (match) {
      // Consume before asking: list refreshes must not replay an accepted
      // or cancelled request. An unmatched request still waits for its row.
      setWorkflowOpenRequest(null);
      // A run-completion notification → show what it produced (output view).
      untrack(() => selectWorkflow(match));
    }
  });

  function openNew() {
    if (view() === 'new') { setShowList(false); return; }
    void guardNavigation(() => {
      setShowList(false);
      setNewSchedule('manual');
      setNewChat(null);
      setView('new');
      setActive(null);
    });
  }

  function selectWorkflow(w: Workflow, next: ViewMode = 'output') {
    if (active()?.slug === w.slug && view() === next) { setShowList(false); return; }
    void guardNavigation(() => {
      setShowList(false);
      setView(next);
      setActive(w);
    });
  }

  function workflowMenu(e: MouseEvent, w: Workflow) {
    const items: ContextMenuItem[] = [
      { label: 'Run now', action: () => void runFromList(w) },
      { label: 'Edit',    action: () => selectWorkflow(w, 'detail') },
      { separator: true },
      { label: 'Delete',  action: () => deleteWorkflow(w), danger: true },
    ];
    openContextMenu(e, items);
  }

  /** The row's bolt and its menu's Run now: one confirm, then the run (P0-13). */
  async function runFromList(w: Workflow) {
    if (await confirmRunNow(w)) await triggerRun(w.slug);
  }

  async function triggerRun(slug: string) {
    try {
      const result = await api.runWorkflow(slug);
      refetchWorkflows();
      if (result.status === 'cancelled') flashFooter('Stopped.', 1500);
    } catch (e: any) {
      if (isExpectedCancellation(e)) {
        flashFooter('Stopped.', 1500);
        return;
      }
      noteRunFailure(e);
      flashFooter(`× run failed: ${e?.message || e}`, 4000);
    }
  }

  async function deleteWorkflow(w: Workflow) {
    const ok = await showConfirm({
      title: `Delete automation "${w.title}"?`,
      body: 'This removes the automation’s file from your Hollow folder, including its run history.',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteWorkflow(w.slug);
      if (active()?.slug === w.slug) setActive(null);
      await refetchWorkflows();
      flashFooter('Automation deleted', 1500);
    } catch (e: any) {
      flashFooter(`× delete failed: ${e?.message || e}`, 3000);
    }
  }

  return (
    <>
      <PageHeader
        title="Automations"
        meta={
          <>
            <span>{initialListError() ? 'Automations unavailable' : `${(workflowsList() ?? []).length} automation${workflowsList()?.length === 1 ? '' : 's'}`}</span>
            <Show when={scheduledCount() > 0}>
              <span class="sep" /><span>{scheduledCount()} scheduled</span>
            </Show>
            <Show when={runningSlugs().size > 0}>
              <span class="sep" /><span class="workflow-running-pill">{runningSlugs().size} running</span>
            </Show>
          </>
        }
        actions={<Button variant="primary" size="sm" icon="plus" disabled={initialListError()} onClick={openNew}>New automation</Button>}
      />
      {/* Scheduled automations, the heartbeat and Main agent check-ins
          explained together (plan completion L2). Tonight's queue screen is gone. */}
      <div class="workflow-support">
        <BackgroundClock windows={windows()} loading={windowsLoading()} error={windowsError()} refresh={() => void refreshWindows()} />
        <WindowsAndQueue />
        <BackgroundWorkExplainer />
      </div>
      <Show when={allWorkspaces().some(isMainAgent)}>
        <OutsideSuspense>{() => <RecoverableView name="COO actions" compact suspends={false}><MainAgentActions /></RecoverableView>}</OutsideSuspense>
      </Show>
      {/* Above the split so it is visible at every width (the viewer is
          hidden while browsing the list on narrow windows). */}
      <div class="workflow-permission-reviews">
        {/* Each opens Waiting on you filtered to its kind (decision 7):
            permission changes, automations that failed three times in a
            row (Paperclip PR 9) and budget pauses. */}
        <WaitingLink context="automations" hideWhenEmpty />
        <WaitingLink context="automationHealth" hideWhenEmpty />
        <WaitingLink context="budgets" hideWhenEmpty />
      </div>
      {/* X21: a failed refresh keeps the last list; say so, above the split
          so it shows at every width. */}
      <RefreshNotice resource={workflowsList} onRetry={refetchWorkflows} what="automations" />
      <div class="split workflows-split" classList={{'workflow-browsing':showList(),'workflow-empty-collection':!initialListError()&&workflowsList()?.length===0&&view()!=='new'&&!active()}}>
        <aside class="split-aside">
          <div class="split-search">
            <input
              aria-label="Find an automation"
              placeholder="Find an automation…"
              value={filter()}
              onInput={e => setFilter(e.currentTarget.value)}
            />
          </div>
          <div class="split-list" data-key-list data-key-default>
            <Show when={!initialListError()} fallback={listFailure()}>
            <Show when={filtered().length > 0} fallback={<Show when={filter().trim()} fallback={<EmptyState class="workflow-empty workflow-empty-list" icon="zap" title="No automations yet"
              art={view()!=='new'?{kind:'automations',evidence:{ready:workflowsList.state==='ready',failed:!!workflowsList.error,filtered:false}}:undefined}>
              An automation reruns a checked procedure inside a workspace, with its current brief, plan, sources and Results. Start one with New automation once the steps have worked by hand.
            </EmptyState>}><EmptyState class="workflow-empty-list" title="No matching automations" actions={<Button size="sm" onClick={()=>setFilter('')}>Clear search</Button>}>Try another name or clear your search.</EmptyState></Show>}>
              <For each={groupLabels}>{group => <Show when={groupItems(group).length > 0}>
                <div class="rail-group-label workflow-group-label">
                  {group} <span class="rail-group-count">{groupItems(group).length}</span>
                </div>
                <For each={groupItems(group)}>
                  {(w, i) => (
                    <WorkflowRow
                      workflow={w}
                      index={i()}
                      running={runningSlugs().has(w.slug)}
                      ownerNote={runsOnRowNote(w.runsOn, ownerMacs(), ownersShared())}
                      active={active()?.slug === w.slug && view() !== 'new'}
                      onClick={() => selectWorkflow(w)}
                      onRun={() => void runFromList(w)}
                      onContextMenu={(e: MouseEvent) => workflowMenu(e, w)}
                    />
                  )}
                </For>
              </Show>}</For>
            </Show>
            </Show>
          </div>
        </aside>

        <div class="viewer workflow-viewer">
          <Show when={!initialListError()} fallback={listFailure()}>
          <button class="btn small workflow-browse-back" onClick={()=>setShowList(true)}>← All automations</button>
          <Show when={view() === 'new'}>
            <NewWorkflowForm windows={windows()} initialSchedule={newSchedule()} initialChat={newChat()}
              onCreated={async (w) => {
                await refetchWorkflows();
                setView('output');
                setActive(w);
                flashFooter(`Automation ${w.title} added`, 1800);
              }}
              onCancel={() => { void guardNavigation(() => {setView('output');setShowList(true);}); }}
            />
          </Show>
          <Show when={view() === 'output' && active()}>
            <OutputPane windows={windows()}
              workflow={active()!}
              running={runningSlugs().has(active()!.slug)}
              onEdit={() => setView('detail')}
            />
          </Show>
          <Show when={view() === 'detail' && active()}>
            <div class="workflow-editor-wrap">
              <button class="panel-link workflow-back" onClick={() => { void guardNavigation(() => setView('output')); }}>← Back to latest result</button>
              <WorkflowDetailPane windows={windows()} ownerShared={ownersShared()}
                workflow={active()!}
                onSaved={async (w) => {
                  await refetchWorkflows();
                  setActive(w);
                }}
                onDeleted={async () => {
                  setActive(null);
                  await refetchWorkflows();
                  setView('output');
                }}
              />
            </div>
          </Show>
          <Show when={view() !== 'new' && !active()}>
            {/* One creation entry per surface (audit H08): New automation lives
                in the header; this pane explains instead of repeating it. */}
            <Show when={(workflowsList() ?? []).length}>
              <EmptyState class="workflow-empty" icon="zap" title="No automation selected">Choose an automation to see its last result, schedule and access.</EmptyState>
            </Show>
          </Show>
          </Show>
        </div>
      </div>
    </>
  );

  function scheduledCount(): number {
    return (workflowsList() ?? []).filter(w => w.schedule !== 'manual').length;
  }
}

function WorkflowRow(props: {
  workflow: Workflow;
  index: number;
  running: boolean;
  active: boolean;
  /** Where it runs when not here (M4): another Mac, or no Mac. */
  ownerNote?: { text: string; warn: boolean } | null;
  onClick: () => void;
  onRun: () => void;
  onContextMenu: (e: MouseEvent) => void;
}) {
  const scheduleLabel = () => formatSchedule(props.workflow.schedule);
  const subline = createMemo(() =>
    workflowSubline(props.workflow, permissionPaused(props.workflow), queuedLabel(props.workflow)),
  );
  return (
    <div
      class={`list-row workflow-row ${props.active ? 'active' : ''} ${props.running ? 'running' : ''}`}
      style={{ '--i': Math.min(props.index, 12) }}
      onClick={props.onClick}
      onContextMenu={props.onContextMenu}
    >
      <span
        class="status-conduit workflow-conduit"
        data-state={conduitState(props.workflow.lastStatus, props.running, props.workflow.lastError)}
        aria-hidden="true"
      />
      <div class="workflow-row-head">
        {/* The row's key item (five asks A4): j / k reach it; `.` bubbles to the row's menu. */}
        <button class="title workflow-open" data-key-item aria-current={props.active ? 'true' : undefined} aria-label={`Open ${props.workflow.title || 'untitled automation'}`} onClick={e=>{e.stopPropagation();props.onClick();}}>{props.workflow.title || '(untitled)'}</button>
        <button
          class="row-quick-btn workflow-row-run"
          title="Run now"
          aria-label={`Run ${props.workflow.title || 'untitled automation'} now`}
          disabled={props.running}
          onClick={e => { e.stopPropagation(); props.onRun(); }}
        ><Icon name="zap" size={12} /></button>
      </div>
      <div class="meta">
        <span>{workflowState(props.workflow, props.running)}</span>
        <span class="workflow-schedule-tag">{scheduleLabel()}</span>
        <Show when={endsLabel(props.workflow)}>
          {text => <span class="dim workflow-ends-tag" title={props.workflow.endedNote ?? 'This automation switches to manual by itself.'}>{text()}</span>}
        </Show>
        <Show when={props.workflow.routine}>
          <span class="workflow-routine-chip" title="A built-in routine">Routine</span>
        </Show>
        <Show when={!props.workflow.permissionHold && props.workflow.budgetHold}>
          <span class="workflow-budget-chip" title={props.workflow.budgetHold ?? ''}>Budget paused</span>
        </Show>
        <Show when={props.workflow.lastDurationSec}>
          {(d) => <span class="dim">{formatSeconds(d())}</span>}
        </Show>
      </div>
      <div class={`workflow-row-foot ${subline().warn ? 'workflow-next-warn' : 'dim'}`}>{subline().text}</div>
      <Show when={props.ownerNote}>
        {note => <div class={`workflow-row-foot workflow-row-owner ${note().warn ? 'workflow-next-warn' : 'dim'}`}>{note().text}</div>}
      </Show>
      <Show when={!props.running && props.workflow.lastStatus === 'error' && props.workflow.lastError}>
        <div class="workflow-row-err" classList={{ 'is-budget': isBudgetStopError(props.workflow.lastError) }}>{props.workflow.lastError!.slice(0, 90)}</div>
      </Show>
    </div>
  );
}

/// The run review (next-level P17, one of the programme's representative
/// compositions). One page per automation, in reading order: the header with
/// its single primary action (Run now, a real and costed run) and a status
/// line in the shared operational words; the one actionable reason it is
/// paused or failed, with its fix; what the newest run did and which retry is
/// safe (plan completion L2); the latest useful output (glob-discovered
/// files, else the files recent runs wrote, from their receipts); the latest
/// receipt; run history; and logs (stderr, prompt, settings) behind a
/// disclosure. "Preview configuration" opens a local description of what a
/// run would do that starts nothing. This subsumes the old Morning Briefs
/// tab: a `briefings/*.html` output simply renders here. Its state and
/// actions live in `workflows/runReview.ts`.
function OutputPane(props: RunReviewProps & { windows: AutomationWindows | null }) {
  const review = createRunReview(props);
  const { stop, stopping } = createWorkflowStop(() => props.workflow.slug);
  const { detail, loading, error, content, selected, current, effectiveOutputs, usingDerived,
    previewOpen, setPreviewOpen, isRunning, lastBudgetStop, coverConduitState, agentName, target, state, checks,
    fix, budgetNote, usageNote, outputCaption, latestRun, outcome, select, runNow, applyFix, openInVault, showOutput } = review;
  const outcomeWhen = () => {
    const run = latestRun();
    if (!run) return '';
    const how = run.status === 'error' ? (isBudgetStop(run) ? 'stopped by a budget' : 'failed')
      : run.status === 'cancelled' ? 'stopped' : run.status === 'paused' ? 'paused for requested work' : run.status === 'interrupted' ? 'interrupted' : 'finished';
    return `Latest run · ${relative(run.at)} · ${how}`;
  };
  const openHistory = () => {
    const wire = review.latestRunWire();
    const first = wire?.files[0]?.path;
    if (wire && first) openVaultHistory(first, wire.historyRunId);
  };

  return (
    <div class="workflow-output-pane workflow-review">
      <header class="doc-cover workflow-cover" data-state={coverConduitState()}>
        <div class="workflow-cover-top">
          <div class="workflow-cover-title">
            <div class="workflow-title-line"><h2 class="doc-cover-title">{props.workflow.title}</h2>
              <Show when={props.workflow.routine}><span class="workflow-routine-chip">Routine</span></Show>
            </div>
            <Show when={props.workflow.description}>
              <p class="dim">{props.workflow.description}</p>
            </Show>
          </div>
        </div>
        <div class="workflow-output-actions workflow-status-line" role="group" aria-label="Run and status">
          <Show when={isRunning()} fallback={<button class="btn primary" onClick={() => runNow()} aria-describedby="workflow-run-note">Run now</button>}>
            <button class="btn danger" title="Stop this run and cancel its queued follow-ups" disabled={stopping()} onClick={() => void stop()}>{stopping() ? 'Stopping…' : 'Stop run'}</button>
          </Show>
          <StatusPill label={state().word.label} tone={state().word.tone} hint={state().word.hint} />
          <Show when={checks()}>{c => <StatusPill label={c().label} tone={c().tone} hint={c().hint} quiet />}</Show>
          <Menu label="Automation options" items={[
            { label: 'Preview configuration', action: () => setPreviewOpen(true) },
            { label: 'Stop pending work', action: () => { void stop(); } },
            { label: 'Edit', action: props.onEdit },
          ]} />
        </div>
        <p class="workflow-run-note" id="workflow-run-note">
          Run now is a real run: {agentName()} works in {target()} and uses model allowance.
          <Show when={usageNote()}>{n => <> {n()}</>}</Show>
          <Show when={budgetNote()}>{n => <> {n()}</>}</Show>
        </p>
        <dl class="workflow-cover-stats">
          <div class="workflow-cover-stat">
            <dt>Schedule</dt>
            <dd>
              {formatSchedule(props.workflow.schedule)}
              {/* B04: the scheduler never fires a schedule it can't read; say so, with its words under Details. */}
              <Show when={props.workflow.scheduleProblem}>{problem =>
                <ErrorText class="workflow-next-warn workflow-schedule-problem" plain={false} text={SCHEDULE_PROBLEM_DETAIL}>
                  <ErrorDetails detail={problem()} />
                </ErrorText>}
              </Show>
            </dd>
          </div>
          {/* An unreadable schedule never runs, so a stored next-run time would mislead (B04). */}
          <Show when={props.workflow.nextRunAt && props.workflow.schedule !== 'manual' && !props.workflow.scheduleProblem}>
            <div class="workflow-cover-stat workflow-cover-next">
              <dt>Next run</dt>
              <dd class={nextRunLabel(props.workflow.nextRunAt!, props.workflow.misfireGraceMin, permissionPaused(props.workflow), queuedLabel(props.workflow)).warn ? 'workflow-next-warn' : ''}>
                {nextRunLabel(props.workflow.nextRunAt!, props.workflow.misfireGraceMin, permissionPaused(props.workflow), queuedLabel(props.workflow)).text}
              </dd>
            </div>
          </Show>
          <div class="workflow-cover-stat">
            <dt>Last run</dt>
            <dd>{props.workflow.lastRunAt
              ? `${relative(props.workflow.lastRunAt)}${props.workflow.lastDurationSec ? ` · ${formatSeconds(props.workflow.lastDurationSec)}` : ''}`
              : 'never'}</dd>
          </div>
        </dl>
      </header>
      <Show when={previewOpen()}>
        <ConfigPreviewPanel windows={props.windows}
          slug={props.workflow.slug}
          settings={() => settingsFromWorkflow(props.workflow)}
          holds={props.workflow}
          unsaved={() => null}
          onClose={() => setPreviewOpen(false)}
        />
      </Show>
      <Show when={lastBudgetStop()}>
        <section class="workflow-budget-stop">
          <div class="workflow-debugger-title"><Icon name="alert" size={14} /> Stopped at a budget limit</div>
          <div class="workflow-debugger-sub">{props.workflow.lastError}</div>
          <button type="button" class="btn" onClick={() => requestSuggestions('budgets')}>Review budget pauses</button>
        </section>
      </Show>
      <Show when={!isRunning() && fix()}>
        {f => (
          <section class="workflow-fix" data-action={f().action} role="status" aria-label="What to fix">
            <div class="workflow-fix-title">
              <Icon name="alert" size={14} />
              {props.workflow.permissionHold || props.workflow.budgetHold ? 'Paused' : 'The last run failed'}
            </div>
            <p class="workflow-fix-reason">{f().reason}</p>
            <p class="workflow-fix-step">{f().fix}</p>
            <Show when={f().actionLabel}>
              <Button size="sm" onClick={() => applyFix(f().action)}>{f().actionLabel}</Button>
            </Show>
          </section>
        )}
      </Show>
      <Show when={outcome()}>
        {o => (
          <Show when={review.latestRunReceipt()} fallback={<RunOutcome outcome={o()} when={outcomeWhen()} running={isRunning() || review.queuedOrLive()} onRetry={retryOf => void runNow(retryOf)} />}>
            {receipt => <ExecutionReceipt receipt={receipt()} onOpenOutput={showOutput} head={{
              outcome: o(), when: outcomeWhen(), running: isRunning() || review.queuedOrLive(),
              onRetry: retryOf => void runNow(retryOf),
              onOpenHistory: review.latestRunWire()?.files.length ? openHistory : undefined,
            }} />}
          </Show>
        )}
      </Show>
      <section class="workflow-latest" aria-label="Latest result">
        <header class="workflow-latest-head">
          <h3>Latest result</h3>
          <Show when={outputCaption()}><span class="workflow-latest-caption">{outputCaption()}</span></Show>
        </header>
        <Show when={!loading()} fallback={<Skeleton rows={4} label="Reading this automation's results" />}>
          <Show when={!error()} fallback={<StatePanel kind="failed" compact title="Couldn’t read its results" error={error()} />}>
            <Show
              when={effectiveOutputs().length > 0}
              fallback={<OutputEmpty workflow={props.workflow} onEdit={props.onEdit} />}
            >
              <Show when={usingDerived()}>
                <details class="workflow-derived-note">
                  <summary>About these results</summary>
                  <p>It doesn’t name the files it produces, so these are the files its recent runs wrote (from their run records). Name them under Edit → Destination → Files it produces.</p>
                </details>
              </Show>
              <Show when={effectiveOutputs().length > 1}>
                <div class="briefs-bar" role="group" aria-label="Result editions">
                  <For each={effectiveOutputs()}>
                    {(o, i) => (
                      <button
                        class={`chip ${o.path === selected() ? 'active' : ''}`}
                        style={{ '--i': Math.min(i(), 12) }}
                        aria-pressed={o.path === selected()}
                        onClick={() => select(o)}
                      >{o.name}</button>
                    )}
                  </For>
                </div>
              </Show>
              <div class="viewer">
                <Show
                  when={current()?.format === 'html'}
                  fallback={<NoteRenderer content={content()} onWikilinkClick={openInVault} />}
                >
                  <HtmlDoc code={stripFrontmatter(content())} onNavigate={openInVault} />
                </Show>
              </div>
            </Show>
          </Show>
        </Show>
      </section>
      <Section title="Run history" count={(detail()?.runLog ?? []).length || undefined} class="workflow-history-section">
        <RunHistory entries={detail()?.runLog ?? []} sourceId={props.workflow.slug} wires={review.receiptWires.latest ?? []} />
      </Section>
      <RunLogs workflow={props.workflow} detail={detail()} />
    </div>
  );
}

/// "Preview · nothing runs" (next-level P17): what a run would do with these
/// settings, computed locally by `buildConfigPreview`. It reads the
/// connector library and the budget statuses and nothing else; it never
/// calls `run_workflow` or any other spawn path, and the smoke spec counts
/// the calls to prove it.
function ConfigPreviewPanel(props: {
  slug: string;
  windows: AutomationWindows | null;
  settings: () => PreviewSettings;
  holds: Pick<Workflow, 'permissionHold' | 'budgetHold'>;
  unsaved: () => StampedChanges | null;
  onClose: () => void;
}) {
  onMount(() => enableBudgetsResource());
  const [library] = createResource(() => api.listConnectors(false).then(v => v.connectors).catch(() => null));
  const preview = createMemo(() => {
    const settings = props.settings();
    const result = buildConfigPreview(settings, {
    slug: props.slug,
    defaultAgent: agentSettings().defaultAgent || null,
    agentLabels: AGENT_LABELS as Record<string, string>,
    agentsAvailable: agentsAvailable(),
    permissionLabels: PERM_MODE_LABELS as Record<string, string>,
    workspaces: workspaces().map(w => ({ cwd: w.cwd, title: w.title })),
    connectors: library.loading ? [] : (library() ?? null),
    budgets: budgetsView.error ? null : (budgetsView()?.budgets ?? []),
    holds: props.holds,
    unsaved: props.unsaved(),
    now: Date.now(), windows: props.windows,
    });
    return result;
  });
  return (
    <section class="workflow-preview" aria-label="Configuration preview">
      <header class="workflow-preview-head">
        <div>
          <h3><Icon name="eye" size={14} /> Preview · nothing runs</h3>
          <p>What a run would do with these settings. Showing this starts nothing, calls no model and spends nothing. Run now is the real run, and it uses model allowance.</p>
        </div>
        <IconButton icon="x" label="Close preview" onClick={props.onClose} />
      </header>
      <div class="workflow-preview-groups">
        <For each={preview().groups}>
          {group => (
            <div class="workflow-preview-group" data-group={group.id}>
              <h4>{group.title}</h4>
              <dl>
                <For each={group.lines}>
                  {line => (
                    <div class="workflow-preview-line" data-tone={line.tone} classList={{ 'is-sentence': !line.label }}>
                      <Show when={line.label}><dt>{line.label}</dt></Show>
                      <dd>
                        {line.value}
                        <Show when={line.detail}><span class="workflow-preview-detail">{line.detail}</span></Show>
                      </dd>
                    </div>
                  )}
                </For>
              </dl>
            </div>
          )}
        </For>
      </div>
    </section>
  );
}

/// Logs on demand: the stderr of recent failures, the prompt and the
/// settings a failure is usually read against. Closed by default; budget
/// stops are not failures and stay out.
function RunLogs(props: { workflow: Workflow; detail?: WorkflowDetail }) {
  const failures = createMemo(() => [...(props.detail?.runLog ?? [])]
    .sort((a, b) => (a.at < b.at ? 1 : -1))
    .filter(e => e.status === 'error' && !isBudgetStop(e))
    .slice(0, 5));
  return (
    <Show when={props.detail}>
      <details class="workflow-logs">
        <summary>
          <Icon name="terminal" size={12} /> Logs and run details
          <Show when={failures().length > 0}><span class="workflow-logs-count">{failures().length} failed run{failures().length === 1 ? '' : 's'}</span></Show>
        </summary>
        <div class="workflow-debugger-grid">
          <div>
            <span class="workflow-debugger-k">Schedule</span>
            <span class="workflow-debugger-v">{formatSchedule(props.workflow.schedule)}</span>
          </div>
          <div>
            <span class="workflow-debugger-k">Permissions</span>
            <span class="workflow-debugger-v">{PERM_MODE_LABELS[props.workflow.permissionMode as keyof typeof PERM_MODE_LABELS] ?? props.workflow.permissionMode}</span>
          </div>
          <div>
            <span class="workflow-debugger-k">Files it produces</span>
            <span class="workflow-debugger-v">{props.workflow.outputGlob || 'not named: shows what its runs wrote'}</span>
          </div>
          <div>
            <span class="workflow-debugger-k">Agent</span>
            <span class="workflow-debugger-v">{props.workflow.agent ? workflowAgentName(props.workflow) : 'App default'}</span>
          </div>
        </div>
        <For each={failures()}>
          {(e, i) => (
            <div class="workflow-recent-failure">
              <div class="workflow-debugger-run" data-status={e.status}>
                <span>{fmtTime(e.at)}</span>
                <b>{e.status}</b>
                <span>{formatSeconds(e.durationSec)}</span>
                <span>{e.note}</span>
              </div>
              <Show when={isAuthFailureText(e.note, e.errorTail)}>
                <div class="workflow-auth-hint">{AUTH_FAILURE_HINT}</div>
              </Show>
              <Show when={e.errorTail}>
                <div class="workflow-stderr-label">{i() === 0 ? 'Error output (latest failure)' : 'Error output'}</div>
                <pre class="workflow-stderr-tail">{e.errorTail}</pre>
              </Show>
            </div>
          )}
        </For>
        <Show when={props.detail!.prompt}>
          <details class="workflow-debugger-prompt">
            <summary>Prompt</summary>
            <pre>{props.detail!.prompt.slice(0, 2400)}</pre>
          </details>
        </Show>
      </details>
    </Show>
  );
}

/// Shown when a workflow has produced no viewable outputs: prompts for an
/// `output_glob` if none is declared, otherwise explains files will appear
/// after a run. Run history follows on the page either way.
function OutputEmpty(props: { workflow: Workflow; onEdit: () => void }) {
  return (
    <div class="workflow-output-empty">
      <Show
        when={props.workflow.outputGlob}
        fallback={
          <StatePanel kind="empty" compact title="It doesn’t name the files it produces" actions={<Button size="sm" onClick={props.onEdit}>Edit destination</Button>}>
            Name them (for example <code>briefings/*.html</code>) to show them here. Until then, its run history is below.
          </StatePanel>
        }
      >
        <StatePanel kind="empty" compact title="No results yet">The files it produces (<code>{props.workflow.outputGlob}</code>) appear here after a run.</StatePanel>
      </Show>
    </div>
  );
}

/// The automation editor: the form and its sections. State and actions live
/// in `workflows/workflowEditor.ts`.
function WorkflowDetailPane(props: WorkflowEditorProps & { windows: AutomationWindows | null; ownerShared: boolean }) {
  const {
    runsOn, setRunsOn, savedRunsOn,
    detail, title, setTitle, description, setDescription, schedule, setSchedule, triggers, setTriggers, agent, setAgent, model, setModel,
    cwd, setCwd, permissionMode, allowedTools, setAllowedTools, disallowedTools, setDisallowedTools,
    outputGlob, setOutputGlob, prompt, setPrompt, notify, setNotify, skipIfQuiet, setSkipIfQuiet, watchText, setWatchText, precheck, setPrecheck,
    inChat, setInChat, runsInChat, ends, setEnds, afterRuns, setAfterRuns,
    saving, running, previewOpen, setPreviewOpen, log, unsaved, hasUnsaved, previewSettings, runReceipt,
    save, runNow, cancel, stopping, deleteThis, promoteToSkill, skillUnsaved, skillBusy, workflowAgentCaps, onPermissionChange,
  } = createWorkflowEditor(props);

  return (
    <div class="workflow-detail">
      <Show when={detail.loading}>
        <Skeleton rows={5} label="Opening automation" />
      </Show>
      <Show when={detail()}>
        <h2 class="sr-only">Edit automation</h2>
        <div class="workflow-detail-head">
          <input
            class="workflow-title-input"
            placeholder="Automation name"
            aria-label="Automation name"
            value={title()}
            onInput={e => setTitle(e.currentTarget.value)}
          />
          <div class="workflow-detail-actions">
            <button class="btn primary" onClick={save} disabled={saving()}>
              {saving() ? 'Saving…' : 'Save'}
            </button>
            <button class="btn" aria-pressed={previewOpen()} onClick={() => setPreviewOpen(v => !v)}>
              <Icon name="eye" size={13} /> Preview configuration
            </button>
            <Show
              when={running()}
              fallback={
                <button
                  class="btn"
                  onClick={runNow}
                  disabled={!prompt().trim() || hasUnsaved()}
                  title={hasUnsaved() ? 'Save first: a run uses the saved settings, not these edits.' : 'A real run: it uses model allowance.'}
                >
                  Run now
                </button>
              }
            >
              <button class="btn danger" title="Stop this run and cancel its queued follow-ups" onClick={cancel} disabled={stopping()}>{stopping() ? 'Stopping…' : 'Stop run'}</button>
            </Show>
            <button class="btn ghost" onClick={promoteToSkill} disabled={skillUnsaved() || skillBusy()} title={skillUnsaved() ? 'Save first: a skill uses the saved automation, not these edits.' : 'Write this prompt out as a /skill so you can run it from chat'}>
              Save as skill
            </button>
            <button class="btn ghost danger-text" onClick={deleteThis}>Delete</button>
          </div>
        </div>

        <Show when={previewOpen()}>
          <ConfigPreviewPanel windows={props.windows}
            slug={props.workflow.slug}
            settings={previewSettings}
            holds={props.workflow}
            unsaved={unsaved}
            onClose={() => setPreviewOpen(false)}
          />
        </Show>

        <nav class="workflow-editor-index" aria-label="Automation settings sections">
          <For each={EDITOR_SECTIONS}>{s => <a href={`#workflow-edit-${s.id}`} onClick={e => { e.preventDefault(); document.getElementById(`workflow-edit-${s.id}`)?.scrollIntoView({ block: 'start' }); }}>{s.label}</a>}</For>
        </nav>

        {/* 1 · Purpose: what it is for and what the agent is asked to do. */}
        <section class="workflow-section" id="workflow-edit-purpose" aria-labelledby="workflow-edit-purpose-h">
          <header class="workflow-section-head">
            <h3 id="workflow-edit-purpose-h">Purpose</h3>
            <span class="dim">{prompt().length} chars</span>
          </header>
          <textarea
            class="workflow-description-input"
            placeholder="The outcome in one line: what it produces or does (for example “A morning brief in briefings/”)."
            aria-label="Description"
            rows={2}
            value={description()}
            onInput={e => setDescription(e.currentTarget.value)}
          />
          <textarea
            class="workflow-prompt-input"
            rows={14}
            aria-label="Prompt"
            placeholder={promptPlaceholder(workspaces().find(w => w.cwd === cwd())?.title, cwd())}
            value={prompt()}
            onInput={e => setPrompt(e.currentTarget.value)}
          />
        </section>

        {/* 2 · Inputs & scope: where it works and what it watches. */}
        <section class="workflow-section" id="workflow-edit-scope" aria-labelledby="workflow-edit-scope-h">
          <header class="workflow-section-head"><h3 id="workflow-edit-scope-h">Inputs &amp; scope</h3></header>
          <div class="workflow-grid">
            <RunsIn value={inChat()} chat={runsInChat()} onChange={setInChat} />
            <RunsOn value={runsOn()} saved={savedRunsOn()} shared={props.ownerShared} onChange={setRunsOn} />
            <label class="workflow-field span-2">
              <span class="workflow-label">Workspace</span>
              <Select label="Automation workspace scope" value={workspaces().some(w=>w.cwd===cwd())?cwd():''} onChange={v=>{const w=workspaces().find(w=>w.cwd===v);if(w){setCwd(w.cwd);setOutputGlob(outputGlobForScope(outputGlob(),workspaces().map(o=>o.path),w.path));}}} emptyText="No workspace matches" options={[{value:'',label:'Existing custom / storage-root scope'},...workspaces().map(w=>({value:w.cwd,label:w.title}))]}/>
              <span class="workflow-label">Working folder (advanced)</span>
              <input
                type="text"
                aria-label="Working folder"
                placeholder="(empty: runs in the Hollow folder)"
                value={cwd()}
                onInput={e => setCwd(e.currentTarget.value)}
              />
            </label>
            <label class="workflow-field workflow-field-check span-2">
              <input
                type="checkbox"
                aria-label="Skip when nothing changed"
                checked={skipIfQuiet()}
                onChange={e => setSkipIfQuiet(e.currentTarget.checked)}
              />
              <span class="workflow-label">Skip when nothing changed</span>
              <span class="workflow-hint">
                Scheduled runs only. Skips when no watched file changed since the last successful run started.
                Deletions and renames are not noticed; files the last run wrote with a write tool are ignored,
                but shell- or Codex-written files count as changes. Paused workspaces skip on their own.
              </span>
            </label>
            <Show when={skipIfQuiet()}>
              <label class="workflow-field span-2">
                <span class="workflow-label">Files to watch (patterns, one per line or comma-separated)</span>
                <input
                  type="text"
                  aria-label="Watch globs"
                  placeholder={watchPlaceholder(cwd())}
                  value={watchText()}
                  onInput={e => setWatchText(e.currentTarget.value)}
                />
              </label>
            </Show>
            <label class="workflow-field span-2">
              <span class="workflow-label">Check before each scheduled run (optional command)</span>
              <input
                type="text"
                aria-label="Precheck command"
                placeholder="e.g. test -n &quot;$(git log --since=yesterday)&quot;"
                value={precheck()}
                onInput={e => setPrecheck(e.currentTarget.value)}
              />
              <span class="workflow-hint">
                Runs on this Mac as you, in the working folder, before each scheduled run, with only PATH and HOME set,
                no input, and 20 seconds to finish. Exit 0 runs the automation; anything else skips that run and no agent
                turn starts (the run log says skipped_precheck). If it cannot start or times out the run is skipped too.
                One line, up to 300 characters. Setting or changing it asks you to approve.
              </span>
            </label>
          </div>
        </section>

        {/* 3 · Schedule, with the next three runs and missed-run behaviour. */}
        <section class="workflow-section" id="workflow-edit-schedule" aria-labelledby="workflow-edit-schedule-h">
          <header class="workflow-section-head">
            <h3 id="workflow-edit-schedule-h">Schedule</h3>
            <Show when={props.workflow.nextRunAt && !props.workflow.scheduleProblem && schedule() !== 'manual' && schedule() === (detail()?.workflow.schedule ?? '')}>
              {(() => {
                const l = nextRunLabel(props.workflow.nextRunAt!, props.workflow.misfireGraceMin, permissionPaused(props.workflow), queuedLabel(props.workflow));
                // A missed, paused or queued run reads as its own sentence, so it takes no "next:" prefix.
                return <span class={l.warn ? 'workflow-next-warn' : 'dim'}>{l.warn ? l.text : `next: ${l.text}`}</span>;
              })()}
            </Show>
          </header>
          <EventWhen triggers={triggers()} onTriggers={setTriggers} onSchedule={setSchedule}
            workspaceId={workspaces().find(w => w.cwd === cwd().trim())?.id ?? null}>
            <SchedulePicker value={schedule()} onChange={setSchedule} windows={props.windows} />
            <TriggerEditor value={triggers()} onChange={setTriggers} />
          </EventWhen>
          <NextRunsList windows={props.windows} schedule={schedule()} misfireGraceMin={detail()?.workflow.misfireGraceMin ?? null} skipIfQuiet={skipIfQuiet()} triggers={triggers()} />
          {/* End rule (G2): the scheduler sets the automation to manual when this passes. */}
          <div class="workflow-grid workflow-end-rule">
            <label class="workflow-field">
              <span class="workflow-label">Ends at</span>
              <input type="datetime-local" aria-label="Ends at" value={ends()} onInput={e => setEnds(e.currentTarget.value)} />
            </label>
            <label class="workflow-field">
              <span class="workflow-label">Or after this many runs</span>
              <input type="number" min="1" step="1" inputmode="numeric" aria-label="Ends after runs" placeholder="(no limit)" value={afterRuns()} onInput={e => setAfterRuns(e.currentTarget.value)} />
            </label>
            <span class="workflow-hint span-2">
              {endsProblem(ends(), afterRuns()) ?? 'When the time passes or the last run finishes, this automation switches to manual and never starts on its own again. Pick one.'}
            </span>
            <Show when={detail()?.workflow.endedNote}>
              {note => <span class="workflow-hint span-2">{note()}</span>}
            </Show>
          </div>
        </section>

        {/* 4 · Destination: where results land and who hears about them. */}
        <section class="workflow-section" id="workflow-edit-destination" aria-labelledby="workflow-edit-destination-h">
          <header class="workflow-section-head"><h3 id="workflow-edit-destination-h">Destination</h3></header>
          <div class="workflow-grid">
            <label class="workflow-field span-2">
              <span class="workflow-label">Files it produces (a pattern)</span>
              <input
                type="text"
                aria-label="Result files"
                placeholder="e.g. briefings/*.html — vault-relative; powers Latest result"
                value={outputGlob()}
                onInput={e => setOutputGlob(e.currentTarget.value)}
              />
              <span class="workflow-hint">Files matching this show as its latest result. Leave it empty and Hollow shows the files its recent runs wrote. Results stay in the vault: an automation publishes or sends nothing unless its prompt asks for exactly that.</span>
            </label>
            <label class="workflow-field">
              <span class="workflow-label">Notifications</span>
              <Select label="Automation notifications" value={notify()} searchable={false} onChange={v => setNotify(v as NotifyOption)}
                options={NOTIFY_OPTIONS.map(o => ({ value: o, label: NOTIFY_LABELS[o] }))} />
            </label>
          </div>
        </section>

        {/* 5 · Allowed effects: who runs it and what it may do. Every field
            here is part of the approved settings. */}
        <section class="workflow-section" id="workflow-edit-effects" aria-labelledby="workflow-edit-effects-h">
          <header class="workflow-section-head">
            <h3 id="workflow-edit-effects-h">Allowed effects</h3>
            <span class="dim">Approved settings</span>
          </header>
          <div class="workflow-grid">
            <label class="workflow-field">
              <span class="workflow-label">Agent</span>
              <Select label="Automation agent" value={agent()} searchable={false} onChange={v => { setAgent(v); setModel(''); }}
                options={[{ value: '', label: 'App default' }, ...KNOWN_AGENTS.map(a => ({ value: a, label: AGENT_LABELS[a], detail: agentsAvailable().includes(a) ? undefined : 'Not installed' }))]} />
            </label>
            <div class="workflow-field">
              <span class="workflow-label">Model override</span>
              <ModelPicker label="Automation model override" agent={agent() || agentSettings().defaultAgent || agentsAvailable()[0] || ''}
                value={model()} onChange={setModel} defaultLabel="Inherit agent’s model setting" />
            </div>
            <label class="workflow-field">
              <span class="workflow-label">Permission mode</span>
              <Select label="Permission mode" value={permissionMode()} searchable={false} onChange={v => onPermissionChange(v as typeof PERM_MODE_ORDER[number])}
                options={permModeOptions(PERM_MODE_ORDER, PERM_MODE_LABELS, permModeDescFor(agent() || agentSettings().defaultAgent || agentsAvailable()[0]), workflowAgentCaps())} />
              <Show when={permModeUnavailable(workflowAgentCaps(), permissionMode())}>{reason => <span class="workflow-hint">{reason()}</span>}</Show>
            </label>
            {/* Tool lists are advanced (consolidation H, review F12): the
                permission mode above is the everyday control. Open when set. */}
            <details class="workflow-advanced span-2" open={!!allowedTools().trim() || !!disallowedTools().trim()}>
            <summary>Advanced: exactly which tools it may use</summary>
            <div class="workflow-grid">
            <label class="workflow-field span-2">
              <span class="workflow-label">Allowed tools (comma-separated)</span>
              <input
                type="text"
                aria-label="Allowed tools"
                placeholder="(empty — let permission mode decide)"
                value={allowedTools()}
                onInput={e => setAllowedTools(e.currentTarget.value)}
              />
            </label>
            <label class="workflow-field span-2">
              <span class="workflow-label">Disallowed tools (comma-separated)</span>
              <input
                type="text"
                aria-label="Disallowed tools"
                placeholder="e.g. Bash, WebFetch"
                value={disallowedTools()}
                onInput={e => setDisallowedTools(e.currentTarget.value)}
              />
            </label>
            </div>
            </details>
          </div>
          <div class="workflow-approval" data-state={props.workflow.permissionHold ? 'held' : hasUnsaved() ? (unsaved()!.widens ? 'widens' : 'changed') : 'approved'} aria-live="polite">
            <Show when={props.workflow.permissionHold === 'review'}>
              <p class="workflow-approval-hold">A change made outside this editor is waiting in Permission changes. Saving here does not approve that change; review it there.</p>
            </Show>
            <Show
              when={hasUnsaved()}
              fallback={<p class="workflow-approval-ok"><Icon name="shieldCheck" size={13} /> No unsaved changes to its approved settings.</p>}
            >
              <p class="workflow-approval-title">
                {unsaved()!.widens ? 'Saving approves wider access:' : 'Saving approves these changes:'}
              </p>
              <ul class="workflow-approval-list">
                <For each={unsaved()!.changes}>{c => <li data-field={c.field}>{c.text}</li>}</For>
              </ul>
              <p class="workflow-hint">
                {unsaved()!.widens ? 'You’ll be asked to confirm when you save. ' : ''}Until you save, Run now and scheduled runs use the saved settings.
              </p>
            </Show>
          </div>
        </section>

        {/* 6 · Budget: the usage limits that apply (managed in Settings → Usage). */}
        <section class="workflow-section" id="workflow-edit-budget" aria-labelledby="workflow-edit-budget-h">
          <header class="workflow-section-head">
            <h3 id="workflow-edit-budget-h">Budget</h3>
            <button type="button" class="panel-link" onClick={() => { openSettingsFromOutside('usage'); }}>Manage budgets</button>
          </header>
          <WorkflowBudgets slug={props.workflow.slug} agent={agent() || agentSettings().defaultAgent || agentsAvailable()[0] || ''} />
        </section>

        <section class="workflow-section" id="workflow-edit-log" aria-labelledby="workflow-edit-log-h">
          <header class="workflow-section-head">
            <h3 id="workflow-edit-log-h">Run log</h3>
            <Show when={running()}>
              <span class="workflow-running-pill">running…</span>
            </Show>
          </header>
          <div class="workflow-run-panel">
            <Show
              when={log().length > 0 || running()}
              fallback={<RunHistory entries={detail()?.runLog ?? []} sourceId={props.workflow.slug} />}
            >
              <For each={log()}>
                {row => (
                  <div class={`workflow-run-row kind-${row.kind}`}>
                    <LogIcon kind={row.kind} />
                    <span>{row.text}</span>
                  </div>
                )}
              </For>
            </Show>
            <Show when={runReceipt()}>
              {(receipt) => <ExecutionReceipt receipt={receipt()} />}
            </Show>
          </div>
        </section>
      </Show>
    </div>
  );
}

/// The next three runs of a (possibly unsaved) schedule in this Mac's zone,
/// each with its zone name, and how a missed run is handled (next-level P17).
function NextRunsList(props: { windows: AutomationWindows | null; schedule: string; misfireGraceMin: number | null; skipIfQuiet: boolean; triggers: string[] }) {
  // Recomputed each minute so "next" never trails the clock.
  const [now, setNow] = createSignal(Date.now());
  const timer = setInterval(() => setNow(Date.now()), 60_000);
  onCleanup(() => clearInterval(timer));
  const next = createMemo(() => nextRuns(props.schedule, now(), 3, 1500, props.windows));
  const sentences = createMemo(() => missedRunSentences({
    schedule: props.schedule, misfireGraceMin: props.misfireGraceMin, skipIfQuiet: props.skipIfQuiet, triggers: props.triggers,
    dstSkip: next() ? firstDstSkip(props.schedule, now()) : null, timeZone: next()?.timeZone,
  }));
  return (
    <div class="workflow-next-runs" aria-label="Next runs">
      <Show when={next()?.runs.length}>
        <ol class="workflow-next-list">
          <For each={next()!.runs}>{(run, index) => <li><span class="workflow-next-label">{index() === 0 ? 'Next' : 'Then'}</span><time dateTime={run.at.toISOString()}>{formatRun(run)}</time></li>}</For>
        </ol>
      </Show>
      <Show when={scheduleWindow(props.schedule) && !next()?.runs.length}><p class="workflow-hint">Window start times are unavailable. Refresh windows to try again.</p></Show>
      <For each={sentences()}>{line => <p class="workflow-hint">{line}</p>}</For>
    </div>
  );
}

/// The usage budgets that apply to this automation's runs, read-only.
function WorkflowBudgets(props: { slug: string; agent: string }) {
  onMount(() => enableBudgetsResource());
  const list = createMemo(() => (budgetsView.error ? null : applicableBudgets(budgetsView()?.budgets ?? [], props.slug, props.agent)));
  return (
    <Show when={list() !== null} fallback={<p class="workflow-hint">Usage budgets couldn’t be read, so automations are held until they can.</p>}>
      <Show when={list()!.length} fallback={<p class="workflow-hint">No budget applies. Each run uses model allowance with no cap from Hollow. Add one in Settings → Usage.</p>}>
        <ul class="workflow-budget-list">
          <For each={list()!}>
            {b => (
              <li>
                <StatusPill label={budgetStateLabel(b.state, b.pause?.held)} tone={b.state === 'ok' ? 'ok' : b.state === 'warning' || b.state === 'unknown' ? 'warn' : 'error'} quiet />
                <span>{windowLabel(b.window)} · {b.scopeLabel}</span>
                <span class="dim">{budgetOfLimit(b)}{b.hardStop ? ' · hard stop' : ' · warns only'}</span>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </Show>
  );
}

function RunHistory(props: { entries: RunLogEntry[]; sourceId: string; wires?: EvidenceReceiptWire[] }) {
  const sorted = createMemo(() => [...props.entries].sort((a, b) => (a.at < b.at ? 1 : -1)));
  const durations = createMemo(() => runDurations(sorted()));
  // The run review passes the receipts it already read; elsewhere this reads them.
  const [fetched] = createResource(
    () => (props.wires ? false : props.sourceId),
    sourceId => api.listEvidenceReceipts('workflow', sourceId, 100),
  );
  const receiptsByRun = createMemo(() => new Map(
    ((props.wires ?? fetched()) ?? []).map(wire => [wire.runId, executionReceiptFromWire(wire)]),
  ));
  return (
    <Show
      when={sorted().length > 0}
      fallback={<p class="hint">No runs yet. Press <b>Run now</b> to run this automation.</p>}
    >
      <figure class="workflow-durations">
        <figcaption>Run durations · last {durations().points.length} listed · oldest to newest</figcaption>
        <Show when={durations().path} fallback={<p class="hint">Durations unavailable.</p>}>
          <svg viewBox="0 0 300 52" aria-hidden="true">
            <path d={durations().path} />
            <For each={durations().points}>{p => <Show when={p.seconds !== null}><circle cx={p.x} cy={p.y} r="2.5" /></Show>}</For>
          </svg>
          <span class="dim">0 to {formatSeconds(durations().max)} · exact dates, durations and outcomes below; gaps mean unavailable</span>
        </Show>
      </figure>
      <ul class="workflow-history">
        <For each={sorted().slice(0, 25)}>
          {entry => (
            <li class={`workflow-history-row kind-${entry.status}`}>
              <div class="workflow-history-summary">
                <span class={`workflow-status-dot ${entry.status}`} aria-hidden="true" />
                <time class="workflow-history-at" dateTime={entry.at} title={entry.at} aria-label={`${relative(entry.at)} (${new Date(entry.at).toLocaleString()})`}>{relative(entry.at)}</time>
                <span class="dim">{historyStatusLabel(entry.status, isBudgetStop(entry))} · {Number.isFinite(entry.durationSec) && entry.durationSec >= 0 ? formatSeconds(entry.durationSec) : 'duration unavailable'}</span>
                <Show when={entry.note}>
                  <span class="workflow-history-note">{entry.note}</span>
                </Show>
              </div>
              <Show when={entry.runId ? receiptsByRun().get(entry.runId) : undefined}>
                {receipt => <ExecutionReceipt receipt={receipt()} compact />}
              </Show>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}

function NewWorkflowForm(props: {
  windows: AutomationWindows | null;
  initialSchedule: string;
  /** Schedule in this chat… (overhaul F1): the chat it opens bound to. */
  initialChat: RunsInChat | null;
  onCreated: (w: Workflow) => void | Promise<void>;
  onCancel: () => void;
}) {
  const [templateId, setTemplateId] = createSignal(TEMPLATES[0].id);

  const [target, setTarget] = createSignal(workspaceId() ?? '');
  const selectedWorkspace = () => workspaces().find(w=>w.id===target());
  const [title, setTitle] = createSignal('');
  const [description, setDescription] = createSignal(TEMPLATES[0].description);
  const [schedule, setSchedule] = createSignal(props.initialSchedule);
  const [triggers, setTriggers] = createSignal<string[]>([]);
  const [prompt, setPrompt] = createSignal('');
  const [saving, setSaving] = createSignal(false);
  // Runs in (overhaul F1): a chat-bound automation needs no workspace; its
  // runs use the chat's own folder.
  const [inChat, setInChat] = createSignal(props.initialChat?.key ?? NEW_CHAT);
  const placed = () => !!selectedWorkspace() || inChat() !== NEW_CHAT;

  const draftRevision = () => JSON.stringify([target(), title(), description(), schedule(), triggers(), prompt(), templateId(), inChat()]);
  const pristine = draftRevision();
  useAutomationDraftGuard(() => draftRevision() !== pristine
    ? { title: title(), revision: draftRevision() } : null, saving);

  async function submit(e: Event) {
    e.preventDefault();
    if (!title().trim() || !prompt().trim() || !placed() || saving()) return;
    setSaving(true);
    try {
      const ws = selectedWorkspace();
      const input: WorkflowInput = {
        title: title().trim(),
        description: description().trim(),
        schedule: schedule().trim(),
        triggers: triggers(),
        agent: null,
        model: null,
        cwd: ws ? ws.cwd : null,
        outputGlob: ws ? `${ws.path}/outputs/*` : null,
        permissionMode: 'acceptEdits',
        allowedTools: [],
        disallowedTools: [],
        prompt: prompt(),
        ...(inChat() !== NEW_CHAT ? { inChat: inChat() } : {}),
      };
      const w = await api.writeWorkflow(input, null);
      // An event automation's trigger turns on with the save (F2).
      await turnOnFor(input.triggers ?? []).catch(err => flashFooter(`Saved, but its event trigger didn't turn on: ${err?.message || err}`, 5000));
      await props.onCreated(w);
    } catch (err: any) {
      flashFooter(`× create failed: ${err?.message || err}`, 4000);
    } finally {
      setSaving(false);
    }
  }

  return (
    <form class="workflow-new" onSubmit={submit}>
      <h2>New automation</h2>
      <label><span class="form-label">Workspace</span><Select label="Automation workspace" placeholder="Choose the work this belongs to…" value={target()} onChange={setTarget} emptyText="No workspace matches" options={workspaces().map(w=>({value:w.id,label:w.title}))}/></label>
      <p class="hint">Every run reads this workspace’s current context. File-capable agents save Results in outputs/; tool-free responses remain in run history. Attached sources remain snapshots.</p>
      {/* Outcome first (matrix O7): where its results land, who runs it and
          with what access, said before it is created. */}
      <Show when={selectedWorkspace()}>
        {w => <p class="hint workflow-new-defaults">Its results land in <code>{w().path}/outputs/</code>. It runs with the app’s default agent at Edits only: it edits files without asking, and anything that would need your permission is refused, since nobody is there to answer. Change the agent, permission, result files and schedule later under Edit.</p>}
      </Show>
      <Show when={!workspaces().length && inChat() === NEW_CHAT}><p class="hint">Create a project on Projects before adding an automation.</p></Show>
      <RunsIn value={inChat()} chat={props.initialChat} onChange={setInChat} />
      <div class="workflow-templates">
        <For each={TEMPLATES}>
          {t => (
            <button
              type="button"
              class={`workflow-template-card ${templateId() === t.id ? 'active' : ''}`}
              onClick={() => {
                setTemplateId(t.id);
                // Reset fields to template defaults explicitly on click,
                // so later Name edits never reset the schedule or refill fields.
                setTitle(t.id === 'blank' ? '' : t.label);
                setDescription(t.description);
                if (!scheduleWindow(props.initialSchedule)) setSchedule(t.schedule);
                setPrompt(t.prompt);
              }}
            >
              <div class="workflow-template-title">{t.label}</div>
              <div class="workflow-template-desc">{t.description}</div>
            </button>
          )}
        </For>
      </div>

      <label>
        <span class="form-label">Name</span>
        <input
          autofocus
          placeholder="e.g. Weekly evidence review"
          value={title()}
          onInput={e => setTitle(e.currentTarget.value)}
        />
      </label>
      <label>
        <span class="form-label">Outcome</span>
        <input
          placeholder="What a good run produces, in one line (for example “A dated evidence review in outputs/”)"
          value={description()}
          onInput={e => setDescription(e.currentTarget.value)}
        />
      </label>
      {/* div, not label — the picker holds several buttons/selects and a
          label would forward clicks on the caption to the first chip. */}
      <div class="schedule-field">
        <span class="form-label">Schedule</span>
        <EventWhen triggers={triggers()} onTriggers={setTriggers} onSchedule={setSchedule} workspaceId={target() || null}>
          <SchedulePicker value={schedule()} onChange={setSchedule} windows={props.windows} />
          <TriggerEditor value={triggers()} onChange={setTriggers} />
        </EventWhen>
        <NextRunsList windows={props.windows} schedule={schedule()} misfireGraceMin={null} skipIfQuiet={false} triggers={triggers()} />
      </div>
      <label>
        <span class="form-label">Prompt</span>
        <textarea
          rows={14}
          placeholder="Describe the checked procedure, the expected result, and how to verify it. Sources come only from the selected workspace."
          value={prompt()}
          onInput={e => setPrompt(e.currentTarget.value)}
        />
      </label>

      <div class="workflow-new-actions">
        <button type="submit" class="btn primary" disabled={!title().trim() || !prompt().trim() || !placed() || saving()}>
          {saving() ? 'Creating…' : 'Create automation'}
        </button>
        <span class="hint">{describeSchedule(schedule())}</span>
        <button type="button" class="btn ghost" onClick={props.onCancel}>Cancel</button>
      </div>
    </form>
  );
}

/// Structured schedule editor over the stored 7-field cron format. Presets
/// cover the shapes Hollow writes (Manual / Every weekday / Daily / Weekly at
/// a local time-of-day); the "Custom cron" escape hatch reveals the raw
/// field. Round-trips existing strings — the stored format never changes.
function SchedulePicker(props: { value: string; onChange: (v: string) => void; windows: AutomationWindows | null }) {
  // Sticky custom mode: while the user is explicitly in Custom, a raw string
  // that happens to parse structured must not collapse the UI mid-typing.
  const [forceCustom, setForceCustom] = createSignal(false);
  const model = createMemo(() => parseScheduleModel(props.value));
  const kind = createMemo<ScheduleKind>(() => (forceCustom() ? 'custom' : model().kind));
  const hasTime = () => kind() === 'daily' || kind() === 'weekday' || kind() === 'weekly';

  const KINDS: { id: ScheduleKind; label: string; desc: string }[] = [
    { id: 'manual',  label: 'Manual',        desc: 'No automatic runs — fire on demand.' },
    { id: 'weekday', label: 'Every weekday', desc: 'Mon–Fri at the chosen time (local).' },
    { id: 'daily',   label: 'Daily',         desc: 'Every day at the chosen time (local).' },
    { id: 'weekly',  label: 'Weekly',        desc: 'Once a week on the chosen day.' },
    { id: 'window', label: 'Window', desc: 'A saved background-work window.' },
    { id: 'custom',  label: 'Advanced (cron)', desc: 'Any schedule, written as a 7-field cron: seconds, minutes, hours, day, month, weekday, year (local time).' },
  ];

  function setKind(k: ScheduleKind) {
    if (k === 'custom') {
      setForceCustom(true);
      return; // keep the current raw string as the starting point
    }
    setForceCustom(false);
    props.onChange(renderScheduleModel({ ...model(), kind: k, ...(k === 'window' ? { window: model().window ?? props.windows?.windows[0]?.name ?? '', catchUp: model().catchUp ?? 'run-once-on-wake' } : {}) }));
  }
  function patch(p: Partial<ReturnType<typeof parseScheduleModel>>) {
    props.onChange(renderScheduleModel({ ...model(), kind: kind(), ...p }));
  }

  const minuteOptions = createMemo(() => {
    const base = [0, 15, 30, 45];
    const m = model().minute;
    return base.includes(m) ? base : [...base, m].sort((a, b) => a - b);
  });
  const hourLabel = (h: number) =>
    h === 0 ? '12 AM' : h < 12 ? `${h} AM` : h === 12 ? '12 PM' : `${h - 12} PM`;

  return (
    <div class="schedule-picker">
      <div class="schedule-presets">
        <For each={KINDS}>
          {k => (
            <button
              type="button"
              class={`schedule-chip ${kind() === k.id ? 'active' : ''}`}
              title={k.desc}
              disabled={k.id === 'window' && !model().window && !props.windows?.windows.length}
              onClick={() => setKind(k.id)}
            >
              {k.label}
            </button>
          )}
        </For>
      </div>
      <Show when={kind() === 'window'}>
        <div class="schedule-picker-row">
          <Select size="sm" label="Saved window" value={model().window ?? ''} searchable={false}
            onChange={window => patch({ window })} options={[
              ...(model().window && !props.windows?.windows.some(w => w.name === model().window)
                ? [{ value: model().window!, label: `Unavailable: ${model().window}` }] : []),
              ...(props.windows?.windows ?? []).map(w => ({ value: w.name, label: w.label })),
            ]} />
          <Select size="sm" label="Catch-up" value={model().catchUp ?? 'run-once-on-wake'} searchable={false}
            onChange={catchUp => patch({ catchUp: catchUp as 'skip' | 'run-once-on-wake' | 'run-all' })}
            options={[{ value: 'skip', label: 'Skip missed runs' }, { value: 'run-once-on-wake', label: 'Once on wake' }, { value: 'run-all', label: 'Every missed run' }]} />
        </div>
        <Show when={!props.windows}><p class="workflow-hint">Saved windows are unavailable. Refresh windows to try again.</p></Show>
      </Show>
      <Show when={hasTime()}>
        <div class="schedule-picker-row">
          <Show when={kind() === 'weekly'}>
            <Select size="sm" label="Day of week" value={model().day} searchable={false} onChange={v => patch({ day: v as typeof DAYS[number] })} options={[...DAYS].map(d => ({ value: d, label: d }))} />
          </Show>
          <span class="dim">at</span>
          <Select size="sm" label="Hour" value={String(model().hour)} searchable={false} onChange={v => patch({ hour: Number(v) })} options={Array.from({ length: 24 }, (_, h) => ({ value: String(h), label: hourLabel(h) }))} />
          <span class="dim">:</span>
          <Select size="sm" label="Minute" value={String(model().minute)} searchable={false} onChange={v => patch({ minute: Number(v) })} options={minuteOptions().map(m => ({ value: String(m), label: String(m).padStart(2, '0') }))} />
          <span class="dim">local time</span>
        </div>
      </Show>
      <Show when={kind() === 'custom'}>
        <input
          class="schedule-cron-input"
          placeholder="manual  —  or  0 0 7 * * * *  —  cron expression (7 fields: sec min hr day mon dow year)"
          value={props.value}
          onInput={e => props.onChange(e.currentTarget.value)}
        />
        {/* The cron read back in words (consolidation H). */}
        <p class="schedule-cron-reading" aria-live="polite">Runs: {formatSchedule(props.value)}</p>
      </Show>
    </div>
  );
}

function LogIcon(props: { kind: LogRow['kind'] }) {
  const name = () => {
    switch (props.kind) {
      case 'started': return 'play';
      case 'switched': return 'arrowRight';
      case 'tool':    return 'terminal';
      case 'text':    return 'message';
      case 'done':    return 'checkSquare';
      case 'stopped': return 'flag';
      case 'error':   return 'alert';
    }
  };
  return <span class={`workflow-log-icon icon-${props.kind}`}><Icon name={name()} size={13} /></span>;
}
