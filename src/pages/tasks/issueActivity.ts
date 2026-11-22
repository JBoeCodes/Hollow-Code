// The issue detail's activity feed: who changed what and when, read from the
// tracker on demand (`tracker_issue_history`, never mirrored), merged with
// the comments in time order. When the history can't be read (the tracker
// is off, offline, or refused) the feed is the synthetic one the mirror
// alone can give: created, the comments, last updated. Pure; part of the
// lazily loaded Tasks chunk.
import type { TrackerComment, TrackerHistoryEvent, TrackerIssue } from '../../generated/ipc';
import { priorityLabel } from '../../trackerTasks';

export type FeedItem =
  | { kind: 'created'; at: string; who: string | null }
  | { kind: 'comment'; at: string; comment: TrackerComment }
  | { kind: 'change'; at: string; event: TrackerHistoryEvent; text: string }
  | { kind: 'gap'; at: string }
  | { kind: 'updated'; at: string };

const quoted = (text: string | null) => `“${text ?? ''}”`;
const list = (names: readonly string[]) => names.join(', ');
const priorityOf = (value: string | null) => priorityLabel(Number(value ?? 0));

/** What one change did, after the person's name: "moved from Todo to In
 *  Progress", "assigned Sam Lee", "added the label Bug". */
export function changeText(e: TrackerHistoryEvent): string {
  const { from, to } = e;
  switch (e.kind) {
    case 'state':
      return from && to ? `moved from ${from} to ${to}` : `set the status to ${to ?? from ?? 'none'}`;
    case 'assignee':
      if (from && to) return `reassigned from ${from} to ${to}`;
      return to ? `assigned ${to}` : `unassigned ${from ?? 'the issue'}`;
    case 'priority':
      if (!Number(to ?? 0)) return 'removed the priority';
      return Number(from ?? 0) ? `changed the priority from ${priorityOf(from)} to ${priorityOf(to)}` : `set the priority to ${priorityOf(to)}`;
    case 'labels': {
      const added = e.added.length ? `added ${e.added.length > 1 ? 'the labels' : 'the label'} ${list(e.added)}` : '';
      const removed = e.removed.length ? `removed ${e.removed.length > 1 ? 'the labels' : 'the label'} ${list(e.removed)}` : '';
      return [added, removed].filter(Boolean).join(' and ') || 'changed the labels';
    }
    case 'project':
      if (from && to) return `moved it from ${from} to ${to}`;
      return to ? `added it to ${to}` : `removed it from ${from ?? 'its project'}`;
    case 'title':
      return `renamed it from ${quoted(from)} to ${quoted(to)}`;
    case 'description':
      return 'updated the description';
  }
}

/** The feed, oldest first. `history` null means it couldn't be read: the
 *  synthetic feed (created, comments, last updated) stands in. `more` (the
 *  tracker held older changes than it sent, only the newest page is read)
 *  puts one `gap` just before the oldest change shown, so "created" never
 *  reads as directly followed by those changes. The comments are complete,
 *  so older ones keep their place above it. */
export function activityFeed(issue: TrackerIssue, history: readonly TrackerHistoryEvent[] | null, more = false): FeedItem[] {
  const created: FeedItem = { kind: 'created', at: issue.createdAt, who: issue.creator?.name ?? null };
  const comments: FeedItem[] = issue.comments.map(c => ({ kind: 'comment', at: c.createdAt, comment: c }));
  if (!history) return [created, ...comments, { kind: 'updated', at: issue.updatedAt }];
  const changes: FeedItem[] = history.map(e => ({ kind: 'change', at: e.at, event: e, text: changeText(e) }));
  // Stable: a change and a comment at the same moment keep history first.
  const timed = [...changes, ...comments]
    .map((item, i) => ({ item, i, t: Date.parse(item.at) || 0 }))
    .sort((a, b) => a.t - b.t || a.i - b.i)
    .map(x => x.item);
  if (more) {
    const first = timed.findIndex(item => item.kind === 'change');
    const at = first < 0 ? 0 : first;
    timed.splice(at, 0, { kind: 'gap', at: timed[at]?.at ?? issue.createdAt });
  }
  return [created, ...timed];
}

/** "just now", "5m ago", "2h ago", "3d ago", else the date. */
export function ago(iso: string, now = Date.now()): string {
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return '';
  const minutes = Math.floor((now - t) / 60_000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  const d = new Date(t);
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', ...(d.getFullYear() !== new Date(now).getFullYear() ? { year: 'numeric' } : {}) });
}
