import { describe, expect, it } from 'vitest';
import type { Workflow, WorkflowDetail } from './types';
import { shouldHydrateWorkflowForm, workflowFormDirty, workflowFormFields } from './workflowForm';
// The editor's hydrate lives in its controller since plan completion L2 (item 6).
import workflowsSource from './pages/workflows/workflowEditor.ts?raw';

// Merged plan slice 4 (A·D3): a finished run refetches the automation's
// detail, and that refetch must not overwrite the editor's unsaved edits or
// clear the run log. The behaviour itself is covered end to end by
// tests/smoke/workflow-hygiene.spec.ts.

const workflow = {
  slug: 'morning-brief', title: 'Morning Brief', description: 'Daily operating brief.', schedule: '',
  agent: null, model: null, cwd: null, outputGlob: 'briefings/*.md', routine: null, permissionMode: '',
  allowedTools: ['Read', 'Write'], disallowedTools: [], triggers: [], created: null, lastRunAt: null,
  lastStatus: null, lastError: null, lastDurationSec: null, nextRunAt: null, misfireGraceMin: null,
  skipIfQuiet: false, watch: ['notes/*.md'], notify: 'FAILURES', lastStartedAt: null, path: 'workflows/morning-brief.md',
} as unknown as Workflow;
const detail: WorkflowDetail = { workflow, prompt: 'Write the brief.', runLog: [], editBase: 'base-0' };

describe('automation form hydration (merged plan slice 4, A·D3)', () => {
  it('hydrates triggers and protects trigger-only edits across a detail refresh', () => {
    const hydrated = workflowFormFields({ ...detail, workflow: { ...workflow, triggers: ['task-overdue'] } });
    expect(hydrated).toMatchObject({ triggers: ['task-overdue'] });
    const unchanged = { ...hydrated, triggers: ['task-overdue'] };
    expect(workflowFormDirty(hydrated, unchanged)).toBe(false);
    const edited = { ...hydrated, triggers: ['task-unblocked'] };
    const dirty = workflowFormDirty(hydrated, edited);
    expect(dirty).toBe(true);
    expect(shouldHydrateWorkflowForm({ hydratedSlug: workflow.slug, nextSlug: workflow.slug, dirty })).toBe(false);
  });

  it('hydrates the fields the editor shows, with its defaults', () => {
    expect(workflowFormFields(detail)).toEqual({
      title: 'Morning Brief', description: 'Daily operating brief.', schedule: 'manual', agent: '', model: '', cwd: '',
      permissionMode: 'acceptEdits', allowedTools: 'Read, Write', disallowedTools: '', outputGlob: 'briefings/*.md',
      prompt: 'Write the brief.', notify: 'failures', skipIfQuiet: false, watchText: 'notes/*.md', precheck: '', triggers: [], inChat: '',
      ends: '', afterRuns: '', runsOn: '',
    });
  });

  it('is dirty when any field differs from what it hydrated with', () => {
    const hydrated = workflowFormFields(detail);
    expect(workflowFormDirty(null, hydrated)).toBe(false);
    expect(workflowFormDirty(hydrated, { ...hydrated })).toBe(false);
    for (const [key, value] of Object.entries({
      title: 'Draft', description: 'Edited.', prompt: 'Changed.', notify: 'never', skipIfQuiet: true, watchText: '',
      outputGlob: '', permissionMode: 'plan', allowedTools: 'Read', inChat: 's-20261112-0000aaaa',
      ends: '2026-11-20T18:00', afterRuns: '3', runsOn: 'm-8d21c4be90f17a3e', precheck: 'exit 1',
    })) {
      expect(workflowFormDirty(hydrated, { ...hydrated, [key]: value }), key).toBe(true);
    }
  });

  it('keeps a dirty form when the same automation reloads (a run finished)', () => {
    expect(shouldHydrateWorkflowForm({ hydratedSlug: 'morning-brief', nextSlug: 'morning-brief', dirty: true })).toBe(false);
  });

  it('hydrates a clean form, a different automation, and the reload after its own save', () => {
    expect(shouldHydrateWorkflowForm({ hydratedSlug: 'morning-brief', nextSlug: 'morning-brief', dirty: false })).toBe(true);
    expect(shouldHydrateWorkflowForm({ hydratedSlug: null, nextSlug: 'morning-brief', dirty: false })).toBe(true);
    expect(shouldHydrateWorkflowForm({ hydratedSlug: 'morning-brief', nextSlug: 'close-my-day', dirty: true })).toBe(true);
    expect(shouldHydrateWorkflowForm({ hydratedSlug: 'morning-brief', nextSlug: 'morning-brief', dirty: true, afterSave: true })).toBe(true);
  });

  it('the editor gates its hydrate effect and clears the run log only on a slug change', () => {
    const start = workflowsSource.indexOf('function createWorkflowEditor(');
    expect(start).toBeGreaterThan(-1);
    const pane = workflowsSource.slice(start);
    expect(pane).toMatch(/if\s*\(\s*!shouldHydrateWorkflowForm\(/);
    // One setLog([]) belongs to runNow; the hydrate's sits inside the
    // slug-change branch, so a reload of the same automation keeps the log.
    const hydrate = pane.slice(pane.indexOf('shouldHydrateWorkflowForm('), pane.indexOf('function handleEvent('));
    const slugBranch = hydrate.match(/if\s*\(\s*slugChanged\s*\)\s*\{([^}]*)\}/);
    expect(slugBranch?.[1]).toMatch(/setLog\(\[\]\)/);
    expect(hydrate.replace(slugBranch?.[0] ?? '', '')).not.toMatch(/setLog\(\[\]\)/);
  });
});
