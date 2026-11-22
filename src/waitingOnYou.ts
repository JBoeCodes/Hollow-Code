import type { NeedsYouItem } from './workspaceTypes';
import { SUGGESTION_CONTEXTS, type SuggestionContext } from './workspaceContent';

/**
 * The one "waiting on you" surface (merged plan wave 3, decision 7). Pure,
 * unit-tested in waitingOnYou.test.ts.
 *
 * It replaces Activity's "Waiting on you", Home's "Needs you" and the five
 * review inboxes (Tasks, Settings, Memory, Automations ×3). Its rows come
 * from the exhaustive "needs you" query (`list_needs_you`, every kind, paged)
 * plus this window's unsaved file drafts, which only the client can see. Each
 * row opens by its exact id: a conversation by key, a result by path, a
 * review item by its review id, a failure by its notification id, an
 * ask-canvas (Canvas C4 A3) by its conversation key and `<canvas id>@<revision>`.
 */

export type WaitingGroupId = 'interrupted' | 'asks' | 'results' | 'reviews' | 'failures' | 'conflicts' | 'clashes' | 'copies' | 'memories' | 'drafts';

/** The groups, in the order the surface shows them. Drafts have no query
 *  kind: they are merged in from `listDrafts()`. */
export const WAITING_GROUPS: readonly { id: WaitingGroupId; label: string; kinds: readonly NeedsYouItem['kind'][] }[] = [
  // Recovery precedes routine decisions; preserve query order within groups.
  { id: 'failures', label: 'Failures', kinds: ['failure'] },
  // A restart's offer to resume the chats it cut short (overnight A1).
  { id: 'interrupted', label: 'Interrupted by a restart', kinds: ['interrupted'] },
  { id: 'asks', label: 'Approvals and questions', kinds: ['approval', 'question', 'canvas'] },
  { id: 'results', label: 'Results to review', kinds: ['result'] },
  { id: 'reviews', label: 'Review items and proposals', kinds: ['review'] },
  { id: 'conflicts', label: 'Sync conflicts', kinds: ['conflict'] },
  // Context sync clash cards (CX6c): one per file, every root.
  { id: 'clashes', label: 'Context sync clashes', kinds: ['clash'] },
  // iCloud/Finder conflict copies in the vault (overnight A10).
  { id: 'copies', label: 'Conflict copies', kinds: ['copies'] },
  // "Delete the memory X?" once a lesson's fix is installed (build run L2).
  { id: 'memories', label: 'Memories to delete', kinds: ['lesson_memory'] },
  { id: 'drafts', label: 'Unsaved drafts', kinds: [] },
];

/** Rows the surface loads at a time; "Show more" pages on. */
export const WAITING_PAGE = 30;
/** A page of review items when the surface is filtered to one page's kind
 *  (the query's own maximum page). */
export const WAITING_FILTER_PAGE = 200;

/** An unsaved file draft (`fileDrafts.listDrafts`). */
export interface WaitingDraft { path: string; savedAt: string }

export type WaitingRow =
  | { kind: 'item'; id: string; item: NeedsYouItem }
  | { kind: 'draft'; id: string; draft: WaitingDraft };

export interface WaitingGroup { id: WaitingGroupId; label: string; rows: WaitingRow[] }

/** Whether `item` belongs in the surface filtered to `context` (a page's
 *  review kinds: Tasks, Settings, Memory, the three Automations panels).
 *  Unfiltered, everything does. The query already filters
 *  (`waitingQueryCategories`); this keeps a list loaded for another page's
 *  filter off screen until the new answer lands. */
export function inWaitingFilter(item: NeedsYouItem, context: SuggestionContext | null): boolean {
  if (!context) return true;
  const kinds = SUGGESTION_CONTEXTS[context].kinds as readonly string[];
  return item.kind === 'review' && kinds.includes(item.category ?? '');
}

/** The query kinds to ask for: every kind, or only review items when the
 *  surface is filtered to a page's review kinds. */
export function waitingQueryKinds(context: SuggestionContext | null): NeedsYouItem['kind'][] | undefined {
  return context ? ['review'] : undefined;
}

/** The query categories to ask for: none (every category), or that page's
 *  review kinds. The query filters before paging, so a filtered page is
 *  exactly that page's items and its total counts only them: older ones are
 *  never hidden behind a page of other review kinds. */
export function waitingQueryCategories(context: SuggestionContext | null): readonly string[] | undefined {
  return context ? SUGGESTION_CONTEXTS[context].kinds : undefined;
}

/** The non-empty groups, in `WAITING_GROUPS` order, each keeping the query's
 *  order (newest first). Drafts are left out of a filtered view.
 *  `busy`: sessions with a turn running; their asks come first within the
 *  Approvals and questions group, since
 *  answering them unblocks running work (P1-13). `workspace`: only that
 *  workspace's items (drafts have none, so they leave too). */
export function groupWaiting(
  items: readonly NeedsYouItem[], drafts: readonly WaitingDraft[], context: SuggestionContext | null = null,
  opts: { busy?: ReadonlySet<string>; workspace?: string | null } = {},
): WaitingGroup[] {
  const shown = items.filter(item => inWaitingFilter(item, context) && (!opts.workspace || item.workspaceId === opts.workspace));
  const busy = (item: NeedsYouItem) => !!item.sessionKey && !!opts.busy?.has(item.sessionKey);
  const order = (group: WaitingGroupId, rows: NeedsYouItem[]) =>
    group === 'asks' && opts.busy?.size ? [...rows.filter(busy), ...rows.filter(item => !busy(item))] : rows;
  return WAITING_GROUPS.map(group => ({
    id: group.id,
    label: group.label,
    rows: group.id === 'drafts'
      ? (context || opts.workspace ? [] : drafts.map(draft => ({ kind: 'draft' as const, id: `draft:${draft.path}`, draft })))
      : order(group.id, shown.filter(item => group.kinds.includes(item.kind))).map(item => ({ kind: 'item' as const, id: item.id, item })),
  })).filter(group => group.rows.length > 0);
}

/** The workspaces with something waiting, each with its count, largest
 *  first: the workspace filter's choices (P1-13). */
export function waitingWorkspaces(items: readonly NeedsYouItem[]): { id: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const item of items) if (item.workspaceId) counts.set(item.workspaceId, (counts.get(item.workspaceId) ?? 0) + 1);
  return [...counts].map(([id, count]) => ({ id, count })).sort((a, b) => b.count - a.count);
}

/** Where a row opens, exactly, or null when it names no place. */
export type WaitingTarget =
  | { kind: 'session'; key: string }
  | { kind: 'canvas'; key: string; canvasId: string; revision: number }
  | { kind: 'result'; workspaceId: string; path: string }
  | { kind: 'review'; id: string }
  | { kind: 'failure'; id: string; link: string | null }
  | { kind: 'task'; path: string }
  | { kind: 'workspace'; id: string }
  | { kind: 'note'; path: string }
  /** The interrupted-chats card: a checklist, then the person's Resume ticked (P1-40). */
  | { kind: 'resume' }
  /** The conflict-copies dialog: the diff, then the person's keep-one. */
  | { kind: 'copies' }
  /** The context sync clash cards (CX6c): Keep mine, Keep theirs, Keep both, Edit. */
  | { kind: 'clashes' }
  /** "Delete the memory X?": the person's Delete or Keep (build run L2). */
  | { kind: 'lessonMemory'; id: string; title: string; detail: string | null };

/** An ask-canvas item's canvas and revision, from its `sourceId`
 *  (`<canvas id>@<revision>`, the key its answer locks), or null. */
export function askCanvasOf(item: NeedsYouItem): { canvasId: string; revision: number } | null {
  const m = /^([a-z0-9-]{1,64})@([1-9][0-9]{0,15})$/.exec(item.sourceId);
  return m ? { canvasId: m[1], revision: Number(m[2]) } : null;
}

/** A row's exact target. An approval or question opens its conversation by
 *  key (none when no conversation claimed it, or a question can no longer be
 *  answered); an ask-canvas its conversation at that canvas revision; a
 *  result its path in its workspace; a review item its card, by
 *  its review id; a failure its notification, by id, with that
 *  notification's own link; a sync conflict the task (by vault path) or the
 *  workspace that resolves it, else its file; a draft its file. */
export function waitingTarget(row: WaitingRow): WaitingTarget | null {
  if (row.kind === 'draft') return { kind: 'note', path: row.draft.path };
  const item = row.item;
  switch (item.kind) {
    case 'approval':
      return item.sessionKey ? { kind: 'session', key: item.sessionKey } : null;
    case 'question':
      return item.state === 'pending' && item.sessionKey ? { kind: 'session', key: item.sessionKey } : null;
    case 'canvas': {
      const ask = askCanvasOf(item);
      return ask && item.sessionKey ? { kind: 'canvas', key: item.sessionKey, ...ask } : null;
    }
    case 'result':
      return item.workspaceId && item.path ? { kind: 'result', workspaceId: item.workspaceId, path: item.path } : null;
    case 'review':
      return { kind: 'review', id: item.sourceId };
    case 'failure':
      return { kind: 'failure', id: item.sourceId, link: item.link };
    case 'interrupted':
      return { kind: 'resume' };
    case 'copies':
      return { kind: 'copies' };
    case 'clash':
      return { kind: 'clashes' };
    case 'lesson_memory':
      return { kind: 'lessonMemory', id: item.sourceId, title: item.title, detail: item.detail };
    case 'conflict':
      if (item.category === 'task' && item.path) return { kind: 'task', path: item.path };
      if (item.workspaceId) return { kind: 'workspace', id: item.workspaceId };
      return item.path ? { kind: 'note', path: item.path } : null;
  }
  return null;
}

/** What opening each kind of target does; the surface wires these to the
 *  store, tests to spies. */
export interface WaitingOpeners {
  session(key: string): void;
  /** Open conversation `key` scrolled to that canvas revision. */
  canvas(key: string, canvasId: string, revision: number): void;
  result(workspaceId: string, path: string): void;
  review(id: string): void;
  failure(id: string, link: string | null): void;
  task(path: string): void;
  workspace(id: string): void;
  note(path: string): void;
  /** Offer to resume the chats a restart interrupted (never automatic). */
  resume(): void;
  /** Open the conflict-copies dialog. */
  copies(): void;
  /** Open the context sync clash cards (CX6c). */
  clashes(): void;
  /** Ask Delete or Keep for one lesson memory (never automatic). */
  lessonMemory(id: string, title: string, detail: string | null): void;
}

/** Open `row` by its exact id. False when it names no place (nothing runs). */
export function openWaitingRow(row: WaitingRow, open: WaitingOpeners): boolean {
  const target = waitingTarget(row);
  if (!target) return false;
  switch (target.kind) {
    case 'session': open.session(target.key); break;
    case 'canvas': open.canvas(target.key, target.canvasId, target.revision); break;
    case 'result': open.result(target.workspaceId, target.path); break;
    case 'review': open.review(target.id); break;
    case 'failure': open.failure(target.id, target.link); break;
    case 'task': open.task(target.path); break;
    case 'workspace': open.workspace(target.id); break;
    case 'note': open.note(target.path); break;
    case 'resume': open.resume(); break;
    case 'copies': open.copies(); break;
    case 'clashes': open.clashes(); break;
    case 'lessonMemory': open.lessonMemory(target.id, target.title, target.detail); break;
  }
  return true;
}

/** Why a row can't be opened, for its disabled title, or null when it can. */
export function waitingDisabledReason(row: WaitingRow): string | null {
  if (waitingTarget(row)) return null;
  if (row.kind === 'item' && row.item.kind === 'approval') return 'No conversation asked this: answer it on its card.';
  if (row.kind === 'item' && row.item.kind === 'question') return 'This question can no longer be answered: the file it is about changed.';
  if (row.kind === 'item' && row.item.kind === 'canvas') return 'This canvas names no conversation or version to open.';
  return 'This item names no place to open.';
}

/** The notice for sources the query couldn't read, or null when it read
 *  them all. A partial list never reads as complete. */
export function unavailableNotice(unavailable: readonly string[]): string | null {
  if (!unavailable.length) return null;
  const names = unavailable.length === 1
    ? unavailable[0]
    : `${unavailable.slice(0, -1).join(', ')} and ${unavailable[unavailable.length - 1]}`;
  return `Couldn’t read ${names}, so anything waiting there isn’t listed.`;
}

/** The bell's waiting count: everything the query counts, less the questions
 *  shown that can no longer be answered (one on a page not loaded yet counts
 *  until it is). Unsaved drafts are named apart, never in this number (D3). */
export function waitingCount(total: number, items: readonly NeedsYouItem[]): number {
  const stale = items.filter(item => item.kind === 'question' && item.state !== 'pending').length;
  return Math.max(total - stale, 0);
}

/** How long a draft sits untouched before Review lists it (D3). */
export const DRAFT_IDLE_MS = 10 * 60_000;

/** The drafts Review lists and names (P1-14, D3): none open in an editor,
 *  each idle for `DRAFT_IDLE_MS`, so the file being typed never waits on
 *  the person. A draft with no time (recovered after a restart) is idle. */
export function countedDrafts(drafts: readonly WaitingDraft[], open: ReadonlySet<string>, now: number): WaitingDraft[] {
  return drafts.filter(draft => {
    if (open.has(draft.path)) return false;
    const at = Date.parse(draft.savedAt);
    return Number.isNaN(at) || now - at >= DRAFT_IDLE_MS;
  });
}

/** The waiting count: `null` (unknown) until the list's first answer
 *  lands and after a failed read, so a failure never reads as nothing
 *  waiting; drafts apart. */
export interface WaitingBadge { count: number | null; drafts: number }
export function waitingBadgeOf(
  list: { loaded: boolean; failed: boolean; total: number; items: readonly NeedsYouItem[] }, drafts: readonly WaitingDraft[],
): WaitingBadge {
  return { count: list.loaded && !list.failed ? waitingCount(list.total, list.items) : null, drafts: drafts.length };
}

/** The waiting description: the count, or that it is unavailable, and drafts
 *  in their own words. */
export function reviewRowLabel(badge: WaitingBadge, label = 'Review'): string {
  const parts = [label];
  if (badge.count === null) parts.push('count unavailable');
  else if (badge.count > 0) parts.push(`${badge.count} waiting`);
  if (badge.drafts > 0) parts.push(badge.drafts === 1 ? '1 unsaved draft' : `${badge.drafts} unsaved drafts`);
  return parts.join(', ');
}

/** The bell's visible quiet word for drafts ("1 draft"), or '' (D3:
 *  their own word beside the label, never in the count). */
export function draftsNote(drafts: number): string {
  return drafts > 0 ? (drafts === 1 ? '1 draft' : `${drafts} drafts`) : '';
}

/** Review's waiting views (Incoming aside). */
export type WaitingView = 'all' | 'decisions' | 'results' | 'failures';

/** The query kinds behind each narrower view (P0-09): every kind is in
 *  exactly one, so Decisions + Results + Failures is All. Each view asks the
 *  query for its own kinds, which filters before paging, so older items of
 *  one kind are never hidden behind a page of newer ones of another. */
export const REVIEW_VIEW_KINDS: Record<Exclude<WaitingView, 'all'>, readonly NeedsYouItem['kind'][]> = {
  decisions: ['interrupted', 'approval', 'question', 'canvas', 'review', 'conflict', 'copies', 'lesson_memory'],
  results: ['result'],
  failures: ['failure'],
};

/** The kinds a view asks for: a page filter's review items, everything for
 *  All, else the view's own kinds. */
export function reviewViewKinds(view: WaitingView, context: SuggestionContext | null): NeedsYouItem['kind'][] | undefined {
  if (context) return waitingQueryKinds(context);
  return view === 'all' ? undefined : [...REVIEW_VIEW_KINDS[view]];
}

/** A view's tab count, by the one rule (P0-09): its query's total, less the
 *  questions that can't be answered, plus unsaved drafts in All only, the
 *  one view that lists them (P1-14, D3). */
export function reviewViewCount(view: WaitingView, page: { total: number; items: readonly NeedsYouItem[] }, drafts: readonly WaitingDraft[]): number {
  return waitingCount(page.total, page.items) + (view === 'all' ? drafts.length : 0);
}

/** The tab counts on screen: a narrower view shows none until its list has
 *  loaded (never a false 0, as Incoming), and All is their sum plus drafts
 *  once all three have, so the tabs always add up (an unanswerable question
 *  past All's first page would otherwise count there); until then, the
 *  waiting count plus drafts, or none while that is unknown (`null`). */
export function reviewTabCounts(badge: number | null, views: Record<Exclude<WaitingView, 'all'>, number | undefined>, drafts: number): Record<WaitingView, number | undefined> {
  const parts = [views.decisions, views.results, views.failures];
  const known = parts.every((n): n is number => n !== undefined) ? parts.reduce((a, b) => a + b, 0) : badge;
  return { all: known === null ? undefined : known + drafts, ...views };
}

/** What an empty view says: the view by name, a page filter's kind, or that
 *  some sources couldn't be read. */
export function reviewEmptyText(view: WaitingView, context: SuggestionContext | null, partial: boolean): string {
  if (partial) return 'Nothing waiting in the sources that loaded.';
  if (context) return `No ${SUGGESTION_CONTEXTS[context].title.toLowerCase()} are waiting.`;
  return view === 'all' ? 'Nothing is waiting on you.' : `No ${view} waiting.`;
}

/** What an open review card shows. A review row comes from the query, but
 *  its card's item from the review queue snapshot, which can lag the query
 *  (a chat learning proposal bumps nothing the snapshot watches). So a card
 *  whose item is missing re-reads the snapshot once before it gives up, and
 *  only an item still missing after a read that worked is "no longer
 *  waiting"; after a failed read the card says it couldn't be loaded.
 *  `loading`: a snapshot read is in flight. `reread`: this card has already
 *  asked for its one re-read since it opened. */
export type ReviewCardView = 'card' | 'loading' | 'reread' | 'unreadable' | 'gone';
export function reviewCardView(state: { found: boolean; loading: boolean; failed: boolean; reread: boolean }): ReviewCardView {
  if (state.found) return 'card';
  if (state.loading) return 'loading';
  if (!state.reread) return 'reread';
  return state.failed ? 'unreadable' : 'gone';
}

/** Whether the surface may say nothing is waiting: only once an answer for
 *  the current question landed and the last load didn't fail. Before that
 *  an empty list is unknown, not empty. */
export function waitingEmptyState(state: { shown: number; more: boolean; loaded: boolean; failed: boolean }): 'list' | 'loading' | 'empty' | 'none' {
  if (state.shown > 0 || state.more) return 'list';
  if (state.failed) return 'none';
  return state.loaded ? 'empty' : 'loading';
}

/** "Show N more": what the next page adds, at most a page. */
export function moreCount(total: number, loaded: number, page: number): number {
  return Math.max(0, Math.min(page, total - loaded));
}
