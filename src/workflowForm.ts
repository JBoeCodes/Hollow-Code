// The automation editor's form state (merged plan slice 4, review A·D3).
//
// A finished run refetches the automation's detail. The editor used to
// re-hydrate every field (and clear the run log) on each detail change, so a
// run ending wiped the user's unsaved edits. It now hydrates only when the
// selected automation changes, when the form is clean, or right after its own
// save. The run log is not form state and is cleared only on a slug change.
import { notifySelectValue } from './workflowHealth';
import type { WorkflowDetail } from './types';
import { toLocalInput } from './workflowEnds';

/** Every editable field of the automation form, as the inputs hold it. */
export interface WorkflowFormFields {
  title: string;
  description: string;
  schedule: string;
  agent: string;
  model: string;
  cwd: string;
  permissionMode: string;
  allowedTools: string;
  disallowedTools: string;
  outputGlob: string;
  prompt: string;
  notify: string;
  skipIfQuiet: boolean;
  watchText: string;
  /** OC13: the precheck command as typed ('' for none). */
  precheck?: string;
  triggers?: string[];
  /** Runs in (overhaul F1): '' for a new chat each run, else the chat's key. */
  inChat?: string;
  /** End time (G2) as the datetime-local input holds it ('' for none). */
  ends?: string;
  /** End after this many runs (G2), as typed ('' for none). */
  afterRuns?: string;
  /** Owner Mac (M4): the saved `runs_on`, or what the Runs on choice will
   *  write ('' for no owner, `this` for this Mac before it is read). */
  runsOn?: string;
}

/** The form fields a loaded automation hydrates to. */
export function workflowFormFields(d: WorkflowDetail): WorkflowFormFields {
  const w = d.workflow;
  return {
    title: w.title,
    description: w.description,
    schedule: w.schedule || 'manual',
    triggers: [...w.triggers],
    agent: w.agent ?? '',
    model: w.model ?? '',
    cwd: w.cwd ?? '',
    permissionMode: w.permissionMode || 'acceptEdits',
    allowedTools: w.allowedTools.join(', '),
    disallowedTools: w.disallowedTools.join(', '),
    outputGlob: w.outputGlob ?? '',
    prompt: d.prompt,
    notify: notifySelectValue(w.notify),
    skipIfQuiet: !!w.skipIfQuiet,
    watchText: (w.watch ?? []).join(', '),
    precheck: w.precheck ?? '',
    inChat: w.inChat ?? '',
    ends: toLocalInput(w.ends),
    afterRuns: w.afterRuns ? String(w.afterRuns) : '',
    runsOn: (w.runsOn ?? '').trim(),
  };
}

/** True when any field differs from what the form was last hydrated with.
 *  A form that has not hydrated yet is clean. */
export function workflowFormDirty(hydrated: WorkflowFormFields | null, current: WorkflowFormFields): boolean {
  if (!hydrated) return false;
  return (Object.keys(hydrated) as (keyof WorkflowFormFields)[]).some(key => key === 'triggers' ? JSON.stringify(hydrated.triggers ?? []) !== JSON.stringify(current.triggers ?? []) : hydrated[key] !== current[key]);
}

/** Whether a detail (re)load should overwrite the form: a different
 *  automation, a clean form, or the reload after the form's own save. */
export function shouldHydrateWorkflowForm(input: {
  hydratedSlug: string | null;
  nextSlug: string;
  dirty: boolean;
  afterSave?: boolean;
}): boolean {
  return input.hydratedSlug !== input.nextSlug || !input.dirty || !!input.afterSave;
}
