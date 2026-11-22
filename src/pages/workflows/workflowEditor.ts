// The automation editor's controller (plan completion L2, item 6): the form
// state, its hydrate rule, the live run log, save (with the widening confirm
// and the edit-base conflict), Run now, Stop, Delete, Save as skill and the
// permission-mode pick, moved out of `WorkflowDetailPane` in
// pages/Workflows.tsx unchanged. Created inside the component with its props.
import { createEffect, createMemo, createResource, createSignal, onCleanup, onMount, untrack } from 'solid-js';
import { api } from '../../api';
import { formatSeconds } from '../../formatTime';
import { turnOnFor } from '../../eventTriggersApi';
import { listen } from '../../nativeEvents';
import { scopedListen } from '../../scopedListen';
import { stampedChanges, stampedConfirmBody, type StampedChanges, type StampedFields } from '../../workflowPermissions';
import { permModeUnavailable } from '../../permModes';
import { endsForSave, endsProblem } from '../../workflowEnds';
import { notifyForSave, parseWatch, precheckForSave, quietForSave, startedLogText, switchedLogText, type NotifyOption } from '../../workflowHealth';
import type { PreviewSettings } from '../../workflowPreview';
import { shouldHydrateWorkflowForm, workflowFormDirty, workflowFormFields, type WorkflowFormFields } from '../../workflowForm';
import { executionReceiptFromWire, type EvidenceRunContextWire, type ExecutionReceipt as ExecutionReceiptData } from '../../executionReceipt';
import { isExpectedCancellation } from '../../toastPolicy';
import {
  flashFooter, showConfirm, trackTask, PERM_MODE_ORDER, PERM_MODE_DANGER, confirmPermissionModeChange,
  agentsAvailable, agentSettings, agentDescriptors,
} from '../../store';
import type { Workflow, WorkflowEvent, WorkflowInput } from '../../types';
import { errMsg } from '../../utils';
import { noteRunFailure } from './runReview';
import { useAutomationDraftGuard } from './draftGuard';
import { createSkillWithChoice } from '../../store/writeSkill';
import { createWorkflowStop } from './workflowStop';
import { runQueue } from '../../store/runQueue';
import { liveItemForSlug } from '../../runQueue';
import { WORKFLOW_EDIT_CONFLICT, shortPath, slugifyName, splitCsv, type LogRow } from './workflowText';
import { inChatForSave, type RunsInChat } from './runsInChoice';
import { ownerMacs } from './ownerMacs';
import { ownerLabel, runsOnForSave, runsOnStamp, runsOnValue } from './runsOnChoice';

export interface WorkflowEditorProps {
  workflow: Workflow;
  onSaved: (w: Workflow) => void | Promise<void>;
  onDeleted: () => void | Promise<void>;
}

export function createWorkflowEditor(props: WorkflowEditorProps) {
  // Re-fetch the detail (including prompt body + run log) whenever the
  // selected workflow changes. The list-row resource only has the summary.
  const [detail, { refetch: refetchDetail }] = createResource(
    () => props.workflow.slug,
    (slug) => api.readWorkflow(slug),
  );

  const [title, setTitle] = createSignal('');
  const [description, setDescription] = createSignal('');
  const [schedule, setSchedule] = createSignal('manual');
  const [triggers, setTriggers] = createSignal<string[]>([]);
  const [agent, setAgent] = createSignal<string>('');
  const [model, setModel] = createSignal<string>('');
  const [cwd, setCwd] = createSignal('');
  const [permissionMode, setPermissionMode] = createSignal<typeof PERM_MODE_ORDER[number]>('acceptEdits');
  const [allowedTools, setAllowedTools] = createSignal('');
  const [disallowedTools, setDisallowedTools] = createSignal('');
  const [outputGlob, setOutputGlob] = createSignal('');
  const [prompt, setPrompt] = createSignal('');
  // `notify` (Paperclip PR 9) is sent only when the user changes it, so the
  // value it was hydrated from is kept alongside.
  const [notify, setNotify] = createSignal<NotifyOption>('always');
  let hydratedNotify: NotifyOption = 'always';
  // Skip when nothing changed (Paperclip PR 9b), also sent only when changed.
  const [skipIfQuiet, setSkipIfQuiet] = createSignal(false);
  const [watchText, setWatchText] = createSignal('');
  // Precheck command (OC13): sent only when changed, like the quiet fields.
  const [precheck, setPrecheck] = createSignal('');
  let hydratedPrecheck = '';
  // Runs in (overhaul F1): '' for a new chat each run, or the bound chat's
  // key; sent only when changed.
  const [inChat, setInChat] = createSignal('');
  let hydratedInChat = '';
  // Runs on (M4 follow-up): the saved owner, and the choice once the user
  // picks one (null: none picked, so the choice follows the saved owner as
  // the Macs load). Sent only when it changed.
  const [savedRunsOn, setSavedRunsOn] = createSignal('');
  const [pickedRunsOn, setPickedRunsOn] = createSignal<string | null>(null);
  const runsOn = () => pickedRunsOn() ?? runsOnValue(savedRunsOn(), ownerMacs());
  const runsOnOwner = () => runsOnStamp(runsOn(), savedRunsOn(), ownerMacs());
  // End rule (G2): the end time (datetime-local text) and an optional run
  // count, each sent only when changed.
  const [ends, setEnds] = createSignal('');
  const [afterRuns, setAfterRuns] = createSignal('');
  let hydratedEnds = '';
  let hydratedAfterRuns = '';
  const [boundChat] = createResource(() => detail()?.workflow.inChat || null, async (key): Promise<RunsInChat> => {
    const title = await api.readAgentSession(key).then(d => d.summary.title ?? '', () => '');
    return { key, title: title || key };
  });
  const runsInChat = (): RunsInChat | null => {
    const key = detail()?.workflow.inChat;
    return key ? boundChat() ?? { key, title: key } : null;
  };
  let hydratedSkip = false;
  let hydratedWatch: string[] = [];

  const [saving, setSaving] = createSignal(false);
  const [localRunning, setRunning] = createSignal(false);
  const running = () => {
    const live = liveItemForSlug(runQueue(), props.workflow.slug);
    return localRunning() || !!(live && live.status !== 'queued');
  };
  const { stop: cancel, stopping } = createWorkflowStop(() => props.workflow.slug);
  const [previewOpen, setPreviewOpen] = createSignal(false);
  const [log, setLog] = createSignal<LogRow[]>([]);
  // The approved-settings fields as loaded (next-level P17): what an editor
  // save would change in them, and whether that widens access, is listed in
  // Allowed effects and confirmed at save. A permission mode the user already
  // confirmed when picking it is not asked about twice.
  const [baseline, setBaseline] = createSignal<StampedFields | null>(null);
  const [confirmedMode, setConfirmedMode] = createSignal<string | null>(null);
  const currentStamp = createMemo<StampedFields>(() => ({
    permissionMode: permissionMode(),
    allowedTools: splitCsv(allowedTools()),
    disallowedTools: splitCsv(disallowedTools()),
    cwd: cwd().trim() || null,
    agent: agent() || null,
    model: model().trim() || null,
    triggers: triggers(),
    schedule: schedule().trim(),
    prompt: prompt(),
    runsOn: runsOnOwner(),
    precheck: precheck(),
  }));
  const macName = (owner: string | null) => ownerLabel(owner, ownerMacs());
  const unsaved = createMemo<StampedChanges | null>(() => {
    const base = baseline();
    return base ? stampedChanges(base, currentStamp(), macName) : null;
  });
  const hasUnsaved = () => (unsaved()?.changes.length ?? 0) > 0;
  const previewSettings = (): PreviewSettings => ({
    agent: agent() || null, model: model().trim() || null, cwd: cwd().trim() || null,
    permissionMode: permissionMode(), allowedTools: splitCsv(allowedTools()), disallowedTools: splitCsv(disallowedTools()),
    outputGlob: outputGlob().trim() || null, schedule: schedule().trim(), triggers: triggers(),
    skipIfQuiet: skipIfQuiet(), watch: parseWatch(watchText()), misfireGraceMin: detail()?.workflow.misfireGraceMin ?? null,
  });
  const [runReceipt, setRunReceipt] = createSignal<ExecutionReceiptData | null>(null);
  let runContext: EvidenceRunContextWire | null = null;
  let hydratedWorkflowSlug: string | null = null;
  // What the form last hydrated from (merged plan slice 4, A·D3). A finished
  // run refetches the detail; that reload only overwrites the form when it is
  // clean, so unsaved edits survive it. The edit base is pinned with the
  // fields, so a kept edit still saves against the file it was made from.
  let hydratedFields: WorkflowFormFields | null = null;
  const [skillBaseline, setSkillBaseline] = createSignal<WorkflowFormFields | null>(null);
  const [skillBusy, setSkillBusy] = createSignal(false);
  let disposed = false;
  onCleanup(() => { disposed = true; });
  let hydratedEditBase: string | undefined;
  // Set by save() for the reload that follows its own write.
  let hydrateAfterSave = false;
  const formFields = (): WorkflowFormFields => ({
    title: title(), description: description(), schedule: schedule(), agent: agent(), model: model(), cwd: cwd(),
    permissionMode: permissionMode(), allowedTools: allowedTools(), disallowedTools: disallowedTools(),
    outputGlob: outputGlob(), prompt: prompt(), notify: notify(), skipIfQuiet: skipIfQuiet(), watchText: watchText(), precheck: precheck(), triggers: triggers(),
    inChat: inChat(), ends: ends(), afterRuns: afterRuns(), runsOn: runsOnOwner() ?? '',
  });
  const skillUnsaved = () => !skillBaseline() || workflowFormDirty(skillBaseline(), formFields());
  useAutomationDraftGuard(() => workflowFormDirty(skillBaseline(), formFields())
    ? { title: title(), revision: JSON.stringify([props.workflow.slug, formFields()]) } : null, saving);

  // Hydrate form state when the detail loads: on a slug change, when the form
  // is clean, or after this form's own save. Only `detail` is tracked.
  createEffect(() => {
    const d = detail();
    if (!d) return;
    const slugChanged = hydratedWorkflowSlug !== d.workflow.slug;
    const dirty = untrack(() => workflowFormDirty(hydratedFields, formFields()));
    if (!shouldHydrateWorkflowForm({
      hydratedSlug: hydratedWorkflowSlug, nextSlug: d.workflow.slug, dirty, afterSave: hydrateAfterSave,
    })) return;
    hydrateAfterSave = false;
    if (slugChanged) {
      hydratedWorkflowSlug = d.workflow.slug;
      setRunReceipt(null);
      runContext = null;
      // The run log belongs to the automation, not to the form.
      setLog([]);
    }
    const fields = workflowFormFields(d);
    hydratedFields = fields;
    setSkillBaseline(fields);
    hydratedEditBase = d.editBase;
    setTitle(fields.title);
    setDescription(fields.description);
    setSchedule(fields.schedule);
    setTriggers(fields.triggers ?? []);
    setAgent(fields.agent);
    setModel(fields.model);
    setCwd(fields.cwd);
    setPermissionMode(fields.permissionMode as typeof PERM_MODE_ORDER[number]);
    setAllowedTools(fields.allowedTools);
    setDisallowedTools(fields.disallowedTools);
    setOutputGlob(fields.outputGlob);
    hydratedNotify = fields.notify as NotifyOption;
    setNotify(hydratedNotify);
    hydratedSkip = fields.skipIfQuiet;
    hydratedWatch = d.workflow.watch ?? [];
    setSkipIfQuiet(hydratedSkip);
    setWatchText(fields.watchText);
    hydratedPrecheck = fields.precheck ?? '';
    setPrecheck(hydratedPrecheck);
    hydratedInChat = fields.inChat ?? '';
    setInChat(hydratedInChat);
    hydratedEnds = fields.ends ?? '';
    hydratedAfterRuns = fields.afterRuns ?? '';
    setEnds(hydratedEnds);
    setAfterRuns(hydratedAfterRuns);
    setPrompt(fields.prompt);
    setSavedRunsOn(fields.runsOn ?? '');
    setPickedRunsOn(null);
    setBaseline({
      permissionMode: (d.workflow.permissionMode as string) || 'acceptEdits',
      allowedTools: [...d.workflow.allowedTools],
      disallowedTools: [...d.workflow.disallowedTools],
      cwd: d.workflow.cwd ?? null,
      agent: d.workflow.agent ?? null,
      model: d.workflow.model ?? null,
      triggers: [...d.workflow.triggers],
      schedule: (d.workflow.schedule || 'manual').trim(),
      prompt: d.prompt,
      runsOn: (d.workflow.runsOn ?? '').trim() || null,
      precheck: (d.workflow.precheck ?? '').trim() || null,
    });
    setConfirmedMode(null);
  });

  // Subscribe to workflow-event for *this* slug so the run log streams
  // live updates while the workflow runs. The listener is scoped to
  // this pane, also when it closes before the listener registers (P6).
  onMount(() => onCleanup(scopedListen(() => listen<WorkflowEvent>('workflow-event', (e) => {
    const p = e.payload;
    if (p.slug !== props.workflow.slug) return;
    handleEvent(p);
  }))));

  function handleEvent(p: WorkflowEvent) {
    switch (p.type) {
      case 'started':
        setRunning(true);
        runContext = { runId: p.runId, recall: p.recall };
        setRunReceipt(null);
        appendLog({ kind: 'started', text: startedLogText(p.source, p.reason) });
        {
          // Merged plan decision 6: a move off the agent's budget ceiling
          // shows while the run is live, not only on the receipt.
          const moved = switchedLogText(p.switched);
          if (moved) appendLog({ kind: 'switched', text: moved });
        }
        break;
      case 'toolUse':
        if (p.name && p.name !== 'Write') {
          appendLog({ kind: 'tool', text: `tool: ${p.name}` });
        } else if (p.name === 'Write') {
          const path = (p.input as any)?.file_path ?? '(file)';
          appendLog({ kind: 'tool', text: `wrote ${shortPath(path)}` });
        }
        break;
      case 'assistantText': {
        const t = p.text.trim().slice(0, 220);
        if (t) appendLog({ kind: 'text', text: t });
        break;
      }
      case 'done':
        setRunning(false);
        if (p.receipt) setRunReceipt(executionReceiptFromWire(p.receipt, runContext));
        appendLog({
          kind: p.status === 'success' ? 'done' : (p.status === 'cancelled' || p.status === 'paused') ? 'stopped' : 'error',
          text: p.status === 'success'
            ? `Done: ${formatSeconds(p.durationSec)}`
            : p.status === 'paused'
              ? `Paused for requested work (${formatSeconds(p.durationSec)})`
            : p.status === 'cancelled'
              ? `Stopped (${formatSeconds(p.durationSec)})`
              : `${p.status}: ${p.error || 'unknown error'} (${formatSeconds(p.durationSec)})`,
        });
        if (p.evidenceError) appendLog({ kind: 'error', text: `Evidence finalization failed: ${p.evidenceError}` });
        refetchDetail();
        break;
    }
  }

  function appendLog(row: Omit<LogRow, 'id' | 'ts'>) {
    const id = `r_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`;
    setLog(prev => [...prev, { ...row, id, ts: Date.now() }]);
  }

  async function save() {
    if (!title().trim()) {
      flashFooter('× title is required', 2400);
      return;
    }
    const endProblem = endsProblem(ends(), afterRuns());
    if (endProblem) {
      flashFooter(`× ${endProblem}`, 4000);
      return;
    }
    // An editor save approves the settings it writes (when the file still
    // matched its approval), so a change that widens access is confirmed
    // here, with every changed field listed. A permission mode already
    // confirmed when it was picked doesn't count twice.
    const base = baseline();
    const changes = unsaved();
    if (base && changes?.widens) {
      const after = currentStamp();
      const rest = stampedChanges(base, confirmedMode() === after.permissionMode ? { ...after, permissionMode: base.permissionMode } : after, macName);
      if (rest.widens) {
        const ok = await showConfirm({
          title: `Approve wider access for “${title().trim()}”?`,
          body: stampedConfirmBody(changes),
          confirmLabel: 'Save and approve',
          danger: true,
        });
        if (!ok) return;
      }
    }
    setSaving(true);
    const savedFrom = props.workflow.slug;
    try {
      const input: WorkflowInput = {
        title: title().trim(),
        description: description().trim(),
        schedule: schedule().trim(),
        triggers: triggers(),
        // An absent agent inherits the app-wide default; explicit choices persist.
        agent: agent() || null,
        model: model().trim() || null,
        cwd: cwd().trim() || null,
        permissionMode: permissionMode(),
        allowedTools: splitCsv(allowedTools()),
        disallowedTools: splitCsv(disallowedTools()),
        outputGlob: outputGlob().trim() || null,
        prompt: prompt(),
        // The baseline this form was loaded from: the save is refused if the
        // file's editor-owned fields changed on disk since. Pinned at hydrate,
        // so a reload that kept unsaved edits doesn't move it.
        editBase: hydratedEditBase,
      };
      const changedNotify = notifyForSave(notify(), hydratedNotify);
      if (changedNotify) input.notify = changedNotify;
      Object.assign(input, quietForSave(skipIfQuiet(), hydratedSkip, watchText(), hydratedWatch));
      const changedPrecheck = precheckForSave(precheck(), hydratedPrecheck);
      if (changedPrecheck !== undefined) input.precheck = changedPrecheck;
      const changedInChat = inChatForSave(inChat(), hydratedInChat);
      if (changedInChat !== undefined) input.inChat = changedInChat;
      const changedRunsOn = runsOnForSave(runsOn(), savedRunsOn(), ownerMacs());
      if (changedRunsOn !== undefined) input.runsOn = changedRunsOn;
      Object.assign(input, endsForSave(ends(), afterRuns(), hydratedEnds, hydratedAfterRuns));
      const updated = await api.writeWorkflow(input, savedFrom);
      // An event automation's trigger turns on with the save (F2).
      await turnOnFor(input.triggers ?? []).catch(err => flashFooter(`Saved, but its event trigger didn't turn on: ${errMsg(err)}`, 5000));
      await props.onSaved(updated);
      // An in-place save keeps the slug, so the detail resource does not
      // refetch by itself; without this the next save would send the
      // pre-save baseline and be refused as a conflict.
      if (updated.slug === savedFrom) {
        // The reload takes what was just saved, even though the form still
        // differs from the pre-save hydrate.
        hydrateAfterSave = true;
        await refetchDetail();
      }
      flashFooter('Saved', 1500);
    } catch (e) {
      // Tauri rejects with a plain string, so `e.message` would be undefined.
      const message = errMsg(e);
      if (message.startsWith(WORKFLOW_EDIT_CONFLICT)) {
        // Keep the draft in the form; the message says to reopen and save.
        flashFooter(`× ${message.replace(/^WORKFLOW_EDIT_CONFLICT:\s*/, '')}`, 8000);
      } else {
        flashFooter(`× save failed: ${message}`, 6000);
      }
    } finally {
      setSaving(false);
    }
  }

  async function runNow() {
    // A run uses the saved file, so it never runs edits the user hasn't
    // saved (and so hasn't approved) under the name of this form.
    if (hasUnsaved()) return;
    setLog([]);
    setRunReceipt(null);
    setRunning(true);
    try {
      const result = await trackTask(
        `Run ${title()}`,
        () => api.runWorkflow(props.workflow.slug),
        {
          onSuccessResult: r => `${r.status} (${formatSeconds(r.durationSec)})`,
          onSuccessStatus: r => r.status === 'success' ? 'success' : r.status === 'cancelled' ? 'cancelled' : 'warning',
        },
      );
      if (result.receipt) setRunReceipt(executionReceiptFromWire(result.receipt, runContext));
      if (result.evidenceError) appendLog({ kind: 'error', text: `Evidence finalization failed: ${result.evidenceError}` });
    } catch (e: any) {
      if (isExpectedCancellation(e)) {
        appendLog({ kind: 'stopped', text: 'Stopped before it started.' });
      } else {
        noteRunFailure(e);
        appendLog({ kind: 'error', text: e?.message || String(e) });
      }
    } finally {
      setRunning(false);
    }
  }

  async function deleteThis() {
    const ok = await showConfirm({
      title: `Delete automation "${title()}"?`,
      body: 'This removes the automation’s file from your Hollow folder, including its run history.',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteWorkflow(props.workflow.slug);
      await props.onDeleted();
      flashFooter('Automation deleted', 1500);
    } catch (e: any) {
      flashFooter(`× delete failed: ${e?.message || e}`, 3000);
    }
  }

  async function promoteToSkill() {
    if (skillBusy() || skillUnsaved()) return;
    const slug = props.workflow.slug;
    const skillName = slugifyName(title());
    if (!skillName) {
      flashFooter('× pick a title first', 2400);
      return;
    }
    setSkillBusy(true);
    try {
      const skill = await createSkillWithChoice(
        skillName,
        description() || `Made from the automation ${title()}`,
        prompt(),
        () => !disposed && props.workflow.slug === slug && !skillUnsaved(),
      );
      if (skill && !disposed) flashFooter(`Skill /${skill.name} added`, 2400);
    } catch (e: any) {
      flashFooter(`× promote failed: ${e?.message || e}`, 3500);
    } finally { if (!disposed) setSkillBusy(false); }
  }

  // Merged plan slice 2: the run's agent decides which modes it can run
  // (Antigravity has no Auto); a saved one it can't run is explained here
  // and refused, with the same reason, when the automation runs.
  const workflowAgentCaps = () => {
    const id = agent() || agentSettings().defaultAgent || agentsAvailable()[0] || '';
    return agentDescriptors().find(d => d.id === id)?.capabilities;
  };

  async function onPermissionChange(target: typeof PERM_MODE_ORDER[number]) {
    if (permModeUnavailable(workflowAgentCaps(), target)) return;
    if (PERM_MODE_DANGER[target] && target !== permissionMode()) {
      const ok = await confirmPermissionModeChange(target);
      if (!ok) return;
      setConfirmedMode(target);
    }
    setPermissionMode(target);
  }

  return {
    detail, refetchDetail,
    title, setTitle, description, setDescription, schedule, setSchedule, triggers, setTriggers, agent, setAgent, model, setModel,
    cwd, setCwd, permissionMode, allowedTools, setAllowedTools, disallowedTools, setDisallowedTools,
    outputGlob, setOutputGlob, prompt, setPrompt, notify, setNotify, skipIfQuiet, setSkipIfQuiet, watchText, setWatchText, precheck, setPrecheck,
    inChat, setInChat, runsInChat, ends, setEnds, afterRuns, setAfterRuns,
    runsOn, setRunsOn: setPickedRunsOn, savedRunsOn,
    saving, running, previewOpen, setPreviewOpen, log, unsaved, hasUnsaved, previewSettings, runReceipt,
    save, runNow, cancel, stopping, deleteThis, promoteToSkill, skillUnsaved, skillBusy, workflowAgentCaps, onPermissionChange,
  };
}

export type WorkflowEditor = ReturnType<typeof createWorkflowEditor>;
