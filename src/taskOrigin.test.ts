import { describe, expect, it } from 'vitest';
import { commitmentTaskOrigin, describeTaskOrigin, normalizePlanText, planItemState } from './taskOrigin';

const titles: Record<string, string> = { 'local-q3': 'Q3 review' };
const t = (id: string) => titles[id] ?? null;

describe('task origin', () => {
  it('names the plan a task came from and links back to it', () => {
    expect(describeTaskOrigin('plan:workspaces/local-q3/PLAN.md', t)).toMatchObject({ text: 'An item in “Q3 review”’s plan', link: { kind: 'plan', workspaceId: 'local-q3' }, linkLabel: 'Open the plan' });
    expect(describeTaskOrigin('plan:workspaces/gone/PLAN.md', t)?.text).toBe('An item in a workspace’s plan');
  });
  it('tells recordings, Incoming and agents apart', () => {
    expect(describeTaskOrigin('meeting:rec-1', t)?.link).toEqual({ kind: 'recording', id: 'rec-1' });
    expect(describeTaskOrigin('triage:abc', t)).toMatchObject({ text: 'Filed from an incoming item' });
    expect(describeTaskOrigin('triage:abc', t)?.link).toBeUndefined();
    expect(describeTaskOrigin('run:run-123', t)?.text).toMatch(/^Proposed by an agent run you approved/);
    expect(describeTaskOrigin('agent', t)?.link).toBeUndefined();
    expect(describeTaskOrigin('commitment:commitments/send-deck.md', t)?.link).toBeUndefined();
    expect(describeTaskOrigin('commitment:commitments/send-deck.md', t)?.text).toBe('A promise you made');
  });
  it('says nothing for a task with no origin', () => {
    expect(describeTaskOrigin(null, t)).toBeNull();
    expect(describeTaskOrigin('  ', t)).toBeNull();
  });
});

describe('exact origin navigation (plan completion item 4)', () => {
  it('marks the plan item the task was copied from, and says the copy is not synced', () => {
    const o = describeTaskOrigin('plan:workspaces/local-q3/PLAN.md', t, { taskTitle: 'Revise section 3' })!;
    expect(o.link).toEqual({ kind: 'plan', workspaceId: 'local-q3', item: 'Revise section 3' });
    expect(o.linkLabel).toBe('Open the plan item');
    expect(o.relation).toMatch(/aren’t kept in sync/);
  });
  it('opens one recording, one promise and one run by their ids', () => {
    expect(describeTaskOrigin('recording:rec-9\naction', t)?.link).toEqual({ kind: 'recording', id: 'rec-9' });
    expect(describeTaskOrigin('run:run-20260926-1', t)).toMatchObject({ link: { kind: 'run', runId: 'run-20260926-1' }, linkLabel: 'Open the run in History' });
    expect(describeTaskOrigin('commitment:commitments/a.md', t)?.link).toBeUndefined();
  });
  it('names the promise’s counterparty when known and keeps the promise distinct from the task', () => {
    const o = describeTaskOrigin('commitment:commitments/a.md', t, { promiseOf: p => p === 'commitments/a.md' ? { counterparty: 'Dana', direction: 'owed_by_me' } : null })!;
    expect(o.text).toBe('A promise you made to Dana');
    expect(o.relation).toMatch(/does not close it/);
    expect(describeTaskOrigin('commitment:commitments/b.md', t, { promiseOf: () => null })!.text).toBe('A promise you made');
  });
  it('is honest where no exact item survives', () => {
    expect(describeTaskOrigin('triage:abc', t)?.relation).toMatch(/cannot be reopened/);
    expect(describeTaskOrigin('meeting:', t)?.link).toBeUndefined();
    expect(describeTaskOrigin('companion:dev-1', t)).toEqual({ text: 'Added from a companion device' });
    // `meeting:<path>` is a meeting note, not a recording id.
    expect(describeTaskOrigin('meeting:calendar/x.md', t)).toMatchObject({ link: { kind: 'note', path: 'calendar/x.md' }, linkLabel: 'Open the meeting note' });
  });
  it('shares the origin a promise’s task carries, capped like the backend', () => {
    expect(commitmentTaskOrigin('commitments/a.md')).toBe('commitment:commitments/a.md');
    expect(commitmentTaskOrigin('x'.repeat(300))).toHaveLength(200);
  });
});

describe('plan item state for a plan-made task', () => {
  const items = [{ text: 'Revise  Section 3', done: false }, { text: 'Book the room', done: true }];
  it('matches the copied text the way the backend keys it', () => {
    expect(normalizePlanText('  Revise   section 3 ')).toBe('revise section 3');
    expect(planItemState('revise section 3', items)).toBe('open');
    expect(planItemState('Book the room', items)).toBe('done');
    expect(planItemState('Renamed task', items)).toBe('missing');
  });
});
