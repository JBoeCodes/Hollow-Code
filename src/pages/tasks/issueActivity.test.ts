import { describe, expect, it } from 'vitest';
import type { TrackerHistoryEvent, TrackerIssue } from '../../generated/ipc';
import { activityFeed, ago, changeText } from './issueActivity';

const event = (over: Partial<TrackerHistoryEvent>): TrackerHistoryEvent => ({
  id: 'h1:state', at: '2026-11-08T05:00:00Z', actor: 'Ana', kind: 'state', from: null, to: null,
  fromKind: null, toKind: null, added: [], removed: [], ...over,
});

const issue = {
  createdAt: '2026-11-07T21:27:00Z', updatedAt: '2026-11-08T06:00:00Z', creator: { id: 'u-1', name: 'Sam Lee', email: null },
  comments: [{ id: 'c-1', author: 'Sam Lee', body: 'Hi', createdAt: '2026-11-08T05:30:00Z', editedAt: null, parentId: null }],
} as unknown as TrackerIssue;

describe('changeText', () => {
  it('says what each kind of change did', () => {
    expect(changeText(event({ from: 'Todo', to: 'In Progress' }))).toBe('moved from Todo to In Progress');
    expect(changeText(event({ to: 'Todo' }))).toBe('set the status to Todo');
    expect(changeText(event({ kind: 'assignee', to: 'Sam Lee' }))).toBe('assigned Sam Lee');
    expect(changeText(event({ kind: 'assignee', from: 'Sam Lee' }))).toBe('unassigned Sam Lee');
    expect(changeText(event({ kind: 'assignee', from: 'Sam Lee', to: 'Ana' }))).toBe('reassigned from Sam Lee to Ana');
    expect(changeText(event({ kind: 'priority', from: '0', to: '2' }))).toBe('set the priority to High');
    expect(changeText(event({ kind: 'priority', from: '3', to: '1' }))).toBe('changed the priority from Medium to Urgent');
    expect(changeText(event({ kind: 'priority', from: '2', to: '0' }))).toBe('removed the priority');
    expect(changeText(event({ kind: 'labels', added: ['Bug'], removed: ['Idea', 'Chore'] }))).toBe('added the label Bug and removed the labels Idea, Chore');
    expect(changeText(event({ kind: 'project', to: 'Receipts rebuild' }))).toBe('added it to Receipts rebuild');
    expect(changeText(event({ kind: 'project', from: 'A', to: 'B' }))).toBe('moved it from A to B');
    expect(changeText(event({ kind: 'title', from: 'Old', to: 'New' }))).toBe('renamed it from “Old” to “New”');
    expect(changeText(event({ kind: 'description' }))).toBe('updated the description');
  });
});

describe('activityFeed', () => {
  it('falls back to the synthetic feed without a history', () => {
    expect(activityFeed(issue, null).map(i => i.kind)).toEqual(['created', 'comment', 'updated']);
  });

  it('merges changes and comments in time order after the creation', () => {
    const feed = activityFeed(issue, [
      event({ id: 'late', at: '2026-11-08T06:00:00Z', from: 'In Progress', to: 'Done' }),
      event({ id: 'early', at: '2026-11-08T05:00:00Z', from: 'Todo', to: 'In Progress' }),
    ]);
    expect(feed.map(i => i.kind === 'change' ? i.event.id : i.kind)).toEqual(['created', 'early', 'comment', 'late']);
    expect(feed[1]).toMatchObject({ text: 'moved from Todo to In Progress' });
  });

  it('a history with older entries left out says so just before the oldest change shown', () => {
    const older = { id: 'c-0', author: 'Ana', body: 'Early', createdAt: '2026-11-07T22:00:00Z', editedAt: null, parentId: null };
    const withOlder = { ...issue, comments: [older, ...issue.comments] } as TrackerIssue;
    const feed = activityFeed(withOlder, [event({ id: 'late', at: '2026-11-08T06:00:00Z' }), event({ id: 'early' })], true);
    expect(feed.map(i => i.kind === 'change' ? i.event.id : i.kind === 'comment' ? i.comment.id : i.kind))
      .toEqual(['created', 'c-0', 'gap', 'early', 'c-1', 'late']);
    expect(feed[2]).toMatchObject({ at: '2026-11-08T05:00:00Z' });
    // Not cut: no gap. Cut with no change shown: the gap follows "created".
    expect(activityFeed(issue, [event({})], false).some(i => i.kind === 'gap')).toBe(false);
    expect(activityFeed(issue, [], true).map(i => i.kind)).toEqual(['created', 'gap', 'comment']);
  });

  it('an empty history shows created and the comments, no synthetic Last updated', () => {
    expect(activityFeed(issue, []).map(i => i.kind)).toEqual(['created', 'comment']);
  });
});

describe('ago', () => {
  const now = Date.parse('2026-11-08T08:00:00Z');
  it('reads as a short relative time', () => {
    expect(ago('2026-11-08T07:59:40Z', now)).toBe('just now');
    expect(ago('2026-11-08T07:55:00Z', now)).toBe('5m ago');
    expect(ago('2026-11-08T06:00:00Z', now)).toBe('2h ago');
    expect(ago('2026-11-05T08:00:00Z', now)).toBe('3d ago');
    expect(ago('2026-10-01T12:00:00Z', now)).toBe('Oct 1');
    expect(ago('not a time', now)).toBe('');
  });
});
