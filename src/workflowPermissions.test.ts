import { describe, expect, it } from 'vitest';
import type { ReviewInboxItem } from './types';
import {
  isPermissionRefusal, permissionConfirmBody, permissionHoldText, permissionPayload, reviewStatusLabel,
  protectedFlags, runFlagLine, sharedDocFlags, stampedChanges, stampedConfirmBody, WORKFLOW_REVIEW_REFUSAL_PREFIX,
} from './workflowPermissions';
import type { StampedFields } from './workflowPermissions';

function item(overrides: Partial<ReviewInboxItem> = {}): ReviewInboxItem {
  return {
    id: 'review-1', kind: 'workflow_permissions', source: 'workflow', sourcePath: 'workflows/close-my-day.md',
    risk: 'local_write', status: 'pending', createdAt: '2026-09-25', updatedAt: '2026-09-25',
    fingerprint: 'workflow-permissions:close-my-day:a:b', snoozedUntil: null, error: null,
    payload: {
      slug: 'close-my-day', title: 'Close my day', path: 'workflows/close-my-day.md',
      observedDigest: 'b', approvedDigest: 'a', changedFields: ['permission mode', 'allowed tools'],
      escalation: true, summary: 'Permission mode: acceptEdits → bypassPermissions',
      diff: [
        { field: 'permission mode', kind: 'scalar', before: 'acceptEdits', after: 'bypassPermissions' },
        { field: 'allowed tools', kind: 'list', added: ['Read', 'Bash(*)'], removed: ['Glob'], reordered: false },
        { field: 'prompt', kind: 'text', before: 'old', after: 'new', truncated: false },
        { field: 'bogus', kind: 'unknown' },
      ],
    },
    ...overrides,
  };
}

describe('workflow permission reviews', () => {
  it('detects refusals by their prefix, from strings and errors', () => {
    const refusal = `${WORKFLOW_REVIEW_REFUSAL_PREFIX} “Close my day” was changed outside Hollow's automation editor.`;
    expect(isPermissionRefusal(refusal)).toBe(true);
    expect(isPermissionRefusal(new Error(refusal))).toBe(true);
    expect(isPermissionRefusal('workflow close-my-day is already running')).toBe(false);
    expect(isPermissionRefusal('Automation permission records unavailable: …')).toBe(false);
    expect(isPermissionRefusal(undefined)).toBe(false);
  });

  it('labels superseded rejections separately from user rejections', () => {
    expect(reviewStatusLabel({ status: 'rejected', error: 'Superseded: the automation changed again.' })).toBe('superseded');
    expect(reviewStatusLabel({ status: 'rejected', error: null })).toBe('rejected');
    expect(reviewStatusLabel({ status: 'accepted', error: null })).toBe('accepted');
    expect(reviewStatusLabel({ status: 'pending', error: null })).toBe('Suggested');
  });

  it('parses the payload defensively and lists every change in the confirm body', () => {
    const payload = permissionPayload(item())!;
    expect(payload.slug).toBe('close-my-day');
    expect(payload.diff).toHaveLength(3);
    const body = permissionConfirmBody(payload);
    expect(body).toContain('Permission mode: acceptEdits → bypassPermissions');
    expect(body).toContain('+Read');
    expect(body).toContain('+Bash(*)');
    expect(body).toContain('−Glob');
    expect(body).toContain('more access');
    expect(permissionPayload(item({ kind: 'learning' }))).toBeNull();
    expect(permissionPayload(item({ payload: { title: 'no slug' } }))).toBeNull();
  });

  it('marks automation files without a current approval in the confirm body', () => {
    const payload = permissionPayload(item({ payload: { ...item().payload, approvedDigest: null } }))!;
    expect(permissionConfirmBody(payload)).toContain('no current approval');
    expect(permissionConfirmBody(payload)).toContain('recreated after being deleted');
  });

  it('labels a paused automation from the backend hold, not the review queue', () => {
    expect(permissionHoldText('review')).toBe('paused until its settings change is approved');
    expect(permissionHoldText('unavailable')).toContain('permission records are unavailable');
    expect(permissionHoldText(null)).toBeNull();
    expect(permissionHoldText(undefined)).toBeNull();
  });

  it('describes each protected change in a run on one line', () => {
    expect(runFlagLine({ kind: 'workflow', path: 'workflows/x.md', changeType: 'modify', detail: 'permission mode' }))
      .toBe('Automation settings · workflows/x.md · modify · permission mode');
    expect(runFlagLine({ kind: 'stateFile', path: '.review-queue.json', changeType: 'modify', detail: null }))
      .toBe('Hollow state file · .review-queue.json · modify');
    expect(runFlagLine({ kind: 'historyManifest', path: '.hollow-history/runs/r.json', changeType: null, detail: null }))
      .toBe('History record · .hollow-history/runs/r.json');
    expect(runFlagLine({ kind: 'sharedDoc', path: 'workspaces/w/PLAN.md', changeType: 'modify', detail: 'merged back' }))
      .toBe('Shared workspace file · workspaces/w/PLAN.md · modify · merged back');
  });

  it('keeps a shared doc written over out of the protected changes', () => {
    const shared = { kind: 'sharedDoc' as const, path: 'workspaces/w/WORKSPACE.md', changeType: 'modify', detail: null };
    const state = { kind: 'stateFile' as const, path: '.review-queue.json', changeType: 'modify', detail: null };
    const run = { flags: [shared, state] };
    expect(protectedFlags(run)).toEqual([state]);
    expect(sharedDocFlags(run)).toEqual([shared]);
    expect(protectedFlags({})).toEqual([]);
  });
});

describe('stampedChanges (next-level P17)', () => {
  const base: StampedFields = { permissionMode: 'acceptEdits', allowedTools: ['Read', 'Glob'], disallowedTools: ['WebFetch'], cwd: '/v/w', agent: null, model: null, triggers: [], schedule: 'manual', prompt: 'Do it.' };

  it('lists nothing for an unchanged form and ignores prompt whitespace', () => {
    expect(stampedChanges(base, { ...base, prompt: '  Do it.\n' })).toEqual({ changes: [], widens: false });
  });

  it('follows the backend escalation rule', () => {
    const esc = (after: Partial<typeof base>) => stampedChanges(base, { ...base, ...after }).widens;
    expect(esc({ permissionMode: 'bypassPermissions' })).toBe(true);
    expect(esc({ permissionMode: 'plan' })).toBe(false);
    expect(esc({ allowedTools: ['Read'] })).toBe(false);
    expect(esc({ allowedTools: ['Read', 'Glob', 'Bash'] })).toBe(true);
    expect(esc({ allowedTools: [] })).toBe(true);
    expect(esc({ disallowedTools: [] })).toBe(true);
    expect(esc({ disallowedTools: ['WebFetch', 'Bash'] })).toBe(false);
    expect(esc({ cwd: '/v/other' })).toBe(true);
    expect(esc({ agent: 'codex' })).toBe(true);
    expect(esc({ model: 'opus' })).toBe(false);
    expect(esc({ schedule: '0 0 7 * * * *' })).toBe(false);
    expect(esc({ prompt: 'Do more.' })).toBe(false);
    // Adding the first entries to an empty allowlist restricts.
    expect(stampedChanges({ ...base, allowedTools: [] }, { ...base, allowedTools: ['Read'] }).widens).toBe(false);
  });

  it('treats a new or changed precheck as widening and clearing as not (OC13)', () => {
    const set = stampedChanges(base, { ...base, precheck: 'exit 1' });
    expect(set.widens).toBe(true);
    expect(set.changes).toEqual([{ field: 'precheck', text: 'Precheck: none → exit 1' }]);
    const withCheck = { ...base, precheck: 'exit 1' };
    expect(stampedChanges(withCheck, { ...withCheck, precheck: 'exit 0' }).widens).toBe(true);
    expect(stampedChanges(withCheck, { ...withCheck, precheck: '' }).widens).toBe(false);
    expect(stampedChanges(withCheck, { ...withCheck, precheck: ' exit 1 ' })).toEqual({ changes: [], widens: false });
    expect(stampedChanges(base, { ...base, precheck: null })).toEqual({ changes: [], widens: false });
  });

  it('describes each change on one line', () => {
    const c = stampedChanges(base, { ...base, permissionMode: '', allowedTools: ['Read', 'Bash'], cwd: null, model: 'opus', prompt: 'x' });
    expect(c.changes.map(x => x.field)).toEqual(['allowed tools', 'working folder', 'model', 'prompt']);
    expect(c.changes[0].text).toBe('Allowed tools: +Bash, −Glob');
    expect(c.changes[1].text).toBe('Working folder: /v/w → the vault');
    expect(stampedChanges(base, { ...base, allowedTools: [] }).changes[0].text).toMatch(/cleared/);
    expect(stampedConfirmBody(c)).toMatch(/Saving approves these settings/);
  });
});
