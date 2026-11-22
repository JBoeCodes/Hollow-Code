import type { ReviewView } from '../store/destinations';
import { EmptyArt } from './EmptyArt';
import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, onCleanup, onMount } from 'solid-js';
import { Icon, type IconName } from '../icons';
import { keyInView, shortcutsEnabled } from '../shortcuts';
import {
  activateNotification, backgroundChatKeys, confirmDialog, dismissNotificationToast, enableApprovalsResource, enableNotificationsResource, flashFooter, flashWithUndo, markNotificationRead, showChoice,
  openVaultNote, showChecklist, storedChatCanvases, storedChatEffort, storedChatUltracode, AGENT_LABELS, PERM_MODE_LABELS, type ChatAgent, type ChatPermissionMode, refetchNotifications, reloadReviewQueue, requestOpenTaskAtPath, reviewQueueFailed, reviewQueueItems,
} from '../store';
import { openSessionByKey, openWorkspaceResult, selectWorkspace, workspaces as allWorkspaces } from '../store/workspaces';
import { clearCanvasReveal, requestCanvasReveal } from '../store/canvasReveal';
import { discardWaitingDraft, setWaitingFilter, refreshWaitingOnYou, waitingBadge, waitingDrafts, waitingFilter, waitingList, waitingListFor, waitingReviews, waitingItemRequest, setWaitingItemRequest } from '../store/waitingOnYou';
import {
  WAITING_FILTER_PAGE, WAITING_PAGE, groupWaiting, moreCount, openWaitingRow, reviewCardView, reviewEmptyText, unavailableNotice, waitingDisabledReason,
  waitingEmptyState, waitingTarget, waitingWorkspaces, type WaitingOpeners, type WaitingRow,
} from '../waitingOnYou';
import { SUGGESTION_CONTEXTS, isWorkspaceReviewItem, type WorkspaceReviewItem } from '../workspaceContent';
import { relativeWhen } from '../homeReturn';
import { runQueue } from '../store/runQueue';
import { reviewKey, wakeLabel } from '../reviewKeys';
import { ReviewCard, itemTitle } from './ReviewCard';
import { api } from '../api';
import { resumeChecklist, resumeSummary } from '../restartResume';
import { openConflictCopies } from '../store/conflictCopies';
import { openSyncClashes } from '../store/syncClashes';
import { refreshReviewInbox, reviewInboxRevision } from '../reviewInbox';
import { errMsg } from '../utils';
import { Skeleton } from './ui';
import { opensInLearnings } from '../learningBulk';
import { agentPermOption } from '../agentPermModes';
import { openLibraryLearnings } from '../store/libraryMode';

/**
 * The full waiting surface, opened from the bell's See all,
 * Home's summary line and the Tasks / Settings / Automations links.
 * Everything waiting on the person, from `list_needs_you`, grouped by kind
 * (approvals, questions and ask-canvases, Results to review, review items and proposals,
 * failures, sync conflicts) with this window's unsaved drafts merged in.
 * It pages with "Show more", opens every row by its exact id, and names the
 * sources the query couldn't read. A review item opens in place as its card;
 * its buttons are a person's actions (decision 3). The notification history
 * Activity used to show lives once, in Bell → Notifications (U3b).
 *
 * Keys (Review's one keymap, `reviewKeys.ts`): j/k or the arrows move
 * between rows, Enter opens, Escape closes; on a notification row x marks it
 * read (delete is its button); inside a review card e/a/Shift-R/z edit,
 * accept, reject and snooze.
 *
 * A pending learning is listed and counted here, but its row opens Library
 * → Learnings, where it and bulk review/Undo are decided (overhaul J7).
 */
/** Which groups each Review view shows (consolidation D, 2026-10-13). */
const VIEW_GROUPS: Record<Exclude<ReviewView, 'incoming'>, readonly string[] | null> = {
  all: null,
  // Drafts wait in All only, where Discard sits (P1-14, D3).
  decisions: ['interrupted', 'asks', 'reviews', 'conflicts', 'clashes', 'copies', 'memories'],
  results: ['results'],
  failures: ['failures'],
};

/** Everything waiting on the person, as a Review view (it was the bell's
 *  popover until consolidation D). Each row opens its own card or source;
 *  Allow/Deny, Accept/Reject and recover stay different actions. */
// Kept outside the panel (P1-13): answering a question in its chat and
// pressing Back finds the same cards open, the same scroll and filter.
const [openCards, setOpenCards] = createSignal<ReadonlySet<string>>(new Set());
const [workspaceOnly, setWorkspaceOnly] = createSignal<string | null>(null);
const scrollByView = new Map<string, number>();

export default function WaitingOnYouPanel(props: { view: Exclude<ReviewView, 'incoming'> }) {
  let panelEl: HTMLDivElement | undefined;
  enableApprovalsResource();
  enableNotificationsResource();

  const filter = waitingFilter;
  // A page filter's review items, else the view's own query (P0-09).
  // `?? waitingList`: on the switch to Incoming the view prop changes a
  // tick before this panel is disposed, and a memo reading it then has no
  // list of its own (P0-10's on-screen ids memo hit it).
  const list = () => (filter() ? waitingReviews : waitingListFor(props.view) ?? waitingList);
  // Chats with a turn running: their asks come first (P1-13).
  const busy = createMemo(() => new Set([...backgroundChatKeys(), ...runQueue().filter(r => r.source === 'chat' && (r.status === 'running' || r.status === 'claimed') && r.sourceId).map(r => r.sourceId!)]));
  const workspaceChoices = createMemo(() => waitingWorkspaces(list().items()));
  // What is snoozed, listed in Decisions with Wake now (P1-13); read again
  // whenever the review inbox changes. A failed read shows nothing extra.
  const [snoozed] = createResource(
    () => (props.view === 'decisions' && !filter() ? reviewInboxRevision() + 1 : false),
    async () => { try { return (await api.listReviewInbox('snoozed')).filter(isWorkspaceReviewItem).filter(item => item.kind !== 'learning'); } catch { return []; } },
  );
  const [waking, setWaking] = createSignal<string | null>(null);
  async function wake(id: string) {
    if (waking()) return;
    setWaking(id);
    try { await api.wakeReviewItem(id); refreshReviewInbox(); refreshWaitingOnYou(true); flashFooter('Back in Needs you', 2200); }
    catch (error) { flashFooter(`× Couldn’t wake it: ${errMsg(error)}`, 3500); }
    finally { setWaking(null); }
  }
  // A workspace that has nothing left waiting drops out of the filter.
  const workspace = () => { const id = workspaceOnly(); return id && workspaceChoices().some(w => w.id === id) ? id : null; };
  const groups = createMemo(() => { const only = VIEW_GROUPS[props.view]; return groupWaiting(list().items(), waitingDrafts(), filter(), { busy: busy(), workspace: filter() ? null : workspace() }).filter(g => !only || only.includes(g.id)); });
  // Groups and rows are keyed by id, not by object: every reload builds new
  // objects, and re-creating a row would reset an open review card (and the
  // edit in progress in it) once a minute.
  const sameIds = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((id, i) => id === b[i]);
  const groupIds = createMemo(() => groups().map(g => g.id), [], { equals: sameIds });
  const rowById = createMemo(() => new Map(groups().flatMap(g => g.rows.map(r => [r.id, r] as const))));
  const notice = () => unavailableNotice(list().unavailable());
  const emptyState = () => waitingEmptyState({ shown: groups().length, more: list().next() != null, loaded: list().loaded(), failed: list().failed() });
  const where = (id: string | null) => (id && allWorkspaces().find(w => w.id === id)?.title) || '';

  const leave = (go: () => void) => { go(); };
  const openers: WaitingOpeners = {
    session: key => leave(() => void openSessionByKey(key)),
    // An ask-canvas: its conversation, scrolled to that canvas revision.
    canvas: (key, canvasId, revision) => leave(() => {
      requestCanvasReveal(key, canvasId, revision);
      void openSessionByKey(key).then(ok => { if (!ok) clearCanvasReveal(); });
    }),
    result: (id, path) => leave(() => void openWorkspaceResult(id, path)),
    // A review item opens as its card, here: toggled by its review id.
    review: id => setOpenCards(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    }),
    // By the notification's own id: follows its link and leaves it waiting
    // (the plan's P1-13 call, run 13 FIX-MR); in Review, Mark handled or x
    // marks it read. `read: true` skips activateNotification's mark-read.
    // One with no link has nothing to open, so it says what clears it.
    failure: (id, link) => {
      if (!link) { dismissNotificationToast(id); flashFooter('Nothing to open for this failure: Mark handled clears it.', 4000); return; }
      activateNotification({ id, read: true, actionUrl: link });
    },
    task: path => leave(() => requestOpenTaskAtPath(path)),
    workspace: id => leave(() => void selectWorkspace(id)),
    note: path => leave(() => openVaultNote(path)),
    // The chats a restart interrupted (overnight A1): the confirm lists
    // them and what is sent; only its Resume sends anything.
    resume: () => void offerResume(),
    // Conflict copies (overnight A10): the dialog shows each diff; only its
    // Keep buttons move anything.
    copies: () => leave(() => openConflictCopies()),
    // Context sync clashes (CX6c): only a card's own buttons change a file.
    clashes: () => leave(() => openSyncClashes()),
    // "Delete the memory X?" (build run L2): only the person's Delete
    // removes anything, and its Undo puts it back.
    lessonMemory: (id, title, detail) => void answerLessonMemory(id, title, detail),
  };

  let answering = false;
  async function answerLessonMemory(id: string, title: string, detail: string | null) {
    if (answering) return;
    answering = true;
    try {
      const choice = await showChoice({ title, body: detail ?? undefined, confirmLabel: 'Delete', alternateLabel: 'Keep', danger: true });
      if (choice === 'cancel') return;
      const name = await api.answerLessonMemory(id, choice === 'confirm');
      if (choice === 'confirm') flashWithUndo({ message: `Deleted the memory ${name}`, kind: 'delete', undo: async () => { await api.restoreLessonMemory(id); refreshWaitingOnYou(true); } });
      else flashFooter(`Kept the memory ${name}`, 2200);
    } catch (error) {
      flashFooter(`× Couldn’t answer: ${errMsg(error)}`, 4000);
    } finally {
      answering = false;
      refreshWaitingOnYou(true);
    }
  }

  createEffect(() => {
    const item = waitingItemRequest();
    if (!item) return;
    setWaitingItemRequest(null);
    if (opensInLearnings(item)) { void openLibraryLearnings(); return; }
    const row: WaitingRow = { kind: 'item', id: item.id, item };
    if (!openWaitingRow(row, openers)) flashFooter(waitingDisabledReason(row) ?? 'This item is unavailable.', 5000);
  });

  // One offer at a time: a second click while one is open or sending does
  // nothing (the backend also re-reads the offer under one lock).
  let resuming = false;
  async function offerResume() {
    if (resuming) return;
    resuming = true;
    try {
      const chats = await api.listInterruptedChats();
      if (!chats.some(c => !c.agentStarted)) { flashFooter('Nothing to resume: those chats moved on.'); return; }
      // One ticked row per chat with what its resume runs with (P1-40);
      // agent-started chats only open.
      const { aside, ...ask } = resumeChecklist(chats, {
        where, agent: id => AGENT_LABELS[id as ChatAgent] ?? id, ultracode: storedChatUltracode,
        // In the chat's agent's own words (per-agent menus, 2026-11-12).
        mode: (m, agent) => (m && (agent ? agentPermOption(agent, m)?.label : PERM_MODE_LABELS[m as ChatPermissionMode])) || m || '',
      });
      const ticked = await showChecklist({ ...ask, aside: aside && { ...aside, onAction: key => void openSessionByKey(key) } });
      if (!ticked?.length) return;
      // What each chat's composer would send: its own Canvases choice, its
      // own Ultracode switch (P1-10) and its own effort (Settings PR 7).
      const asks = ticked.map(key => ({ sessionKey: key, canvases: storedChatCanvases(key), ultracode: storedChatUltracode(key), effort: storedChatEffort(key) }));
      const titleOf = (key: string) => chats.find(c => c.sessionKey === key)?.title ?? 'A conversation';
      flashFooter(resumeSummary(await api.resumeInterruptedChats(asks), titleOf, ask.checklist.length - ticked.length), 8000);
    } catch (error) {
      flashFooter(`× Couldn’t resume the interrupted chats: ${error}`);
    } finally {
      resuming = false;
      refreshWaitingOnYou(true);
    }
  }
  // Filtered to one page's review kinds, every card is open: that page's
  // link is how the person came here to act on them.
  const cardOpen = (id: string) => !!filter() || openCards().has(id);

  // Each view's scroll, kept across a visit elsewhere (P1-13): saved as the
  // person scrolls, put back once the list has rows again.
  onMount(() => {
    const save = (e: Event) => { const t = e.target; if (t instanceof HTMLElement && panelEl && t.contains(panelEl)) scrollByView.set(props.view, t.scrollTop); };
    document.addEventListener('scroll', save, true);
    onCleanup(() => document.removeEventListener('scroll', save, true));
  });
  let scrollRestored = false;
  createEffect(() => {
    if (scrollRestored || emptyState() !== 'list') return;
    scrollRestored = true;
    const top = scrollByView.get(props.view);
    if (!top) return;
    queueMicrotask(() => {
      let el = panelEl?.parentElement ?? null;
      while (el && el.scrollHeight <= el.clientHeight) el = el.parentElement;
      if (el) el.scrollTop = top;
    });
  });

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (confirmDialog() || e.metaKey || e.ctrlKey || e.altKey) return;
      // Only keys aimed at Review (its view tabs and this view) or at the
      // page, never a dialog's or pane's beside it (P0-10).
      if (!keyInView(panelEl?.closest('.review-page') ?? panelEl, e.target)) return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) return;
      const take = () => { e.preventDefault(); e.stopImmediatePropagation(); };
      // The letter keys are shortcuts (off with Settings, Keyboard
      // shortcuts); Escape and the arrows always work.
      if (!shortcutsEnabled() && e.key.length === 1) return;
      if (e.key === 'j' || e.key === 'ArrowDown') { take(); moveFocus(1); return; }
      if (e.key === 'k' || e.key === 'ArrowUp') { take(); moveFocus(-1); return; }
      const notif = (document.activeElement as HTMLElement | null)?.closest<HTMLElement>('[data-notif-id]')?.dataset.notifId;
      if (!notif || !panelEl?.contains(document.activeElement)) return;
      // Review's one keymap (P1-13): x dismisses (marks it read), on a
      // failure row (FIX-MR); deleting a notification is the bell's
      // history row button only.
      if (reviewKey('notification', e) === 'dismiss') { take(); void markNotificationRead(notif).then(() => refreshWaitingOnYou(true)); }
    };
    // Captured, so an open surface wins the keystroke over the page's own
    // j/k list nav (Tasks and Vault both listen on document while bubbling).
    document.addEventListener('keydown', onKey, true);
    onCleanup(() => document.removeEventListener('keydown', onKey, true));
    refetchNotifications();
    // The first row takes focus when nothing else has it or the top bar's
    // bell opened it (it keeps focus after the click), so the
    // keys work straight away; a tab click keeps it. Not when coming back to
    // a scrolled list: that would scroll it to the top.
    queueMicrotask(() => {
      const at = document.activeElement;
      const fromEntry = !at || at === document.body || !!at.closest('.top-bar, .rail');
      if (fromEntry && !scrollByView.get(props.view)) moveFocus(1);
    });
  });

  function moveFocus(dir: number) {
    const rows = [...(panelEl?.querySelectorAll<HTMLElement>('[data-waiting-row]') ?? [])];
    if (!rows.length) return;
    const cur = rows.findIndex(el => el === document.activeElement || el.contains(document.activeElement));
    const next = cur < 0 ? (dir > 0 ? 0 : rows.length - 1) : Math.max(0, Math.min(rows.length - 1, cur + dir));
    rows[next].focus();
    rows[next].scrollIntoView({ block: 'nearest' });
  }

  return (
    <div class="waiting-panel waiting-view" ref={panelEl} role="region" aria-label="Waiting on you" data-filter={filter() ?? ''} data-view={props.view}>
      <Show when={filter()}>{context => (
        <div class="waiting-filter" role="status">
          <span>Showing {SUGGESTION_CONTEXTS[context()].title.toLowerCase()}</span>
          <button type="button" class="panel-link" onClick={() => { setWaitingFilter(null); refreshWaitingOnYou(true); }}>Show everything</button>
        </div>
      )}</Show>
      {/* One workspace's items only (P1-13); its counts follow. */}
      <Show when={!filter() && (workspaceChoices().length > 1 || workspace())}>
        <div class="waiting-filter waiting-workspace">
          <label>
            <span>Workspace </span>
            <select value={workspace() ?? ''} onChange={e => setWorkspaceOnly(e.currentTarget.value || null)}>
              <option value="">All workspaces</option>
              <For each={workspaceChoices()}>{w => <option value={w.id}>{where(w.id) || 'Unnamed workspace'} ({w.count})</option>}</For>
            </select>
          </label>
        </div>
      </Show>
      <Show when={list().failed()}><p class="waiting-notice" role="alert">Couldn’t load what is waiting; this list may be out of date. <button type="button" class="panel-link" onClick={() => refreshWaitingOnYou(true)}>Try again</button></p></Show>
      <Show when={notice()}>{text => <p class="waiting-notice" role="status">{text()} <button type="button" class="panel-link" onClick={() => refreshWaitingOnYou(true)}>Try again</button></p>}</Show>
      <div class="notif-center-body waiting-body">
        {/* "None waiting" only once an answer for this filter landed and
            the last load worked: a partial or unknown list never reads as
            complete. */}
        <Show when={emptyState() === 'list'} fallback={
          <Switch>
            <Match when={emptyState() === 'loading'}><Skeleton class="waiting-loading" label="Loading…" rows={3} /></Match>
            <Match when={emptyState() === 'empty'}>
              <div class="empty-state waiting-empty"><EmptyArt kind="waiting" evidence={{ready:list().loaded(), failed:list().failed(), partial:list().unavailable().length > 0, filtered:!!filter()}}/>
                <p>{reviewEmptyText(props.view, filter(), list().unavailable().length > 0)}</p>
              </div>
            </Match>
          </Switch>
        }>
          <For each={groupIds()}>{id => {
            const group = () => groups().find(g => g.id === id);
            const rowIds = createMemo(() => group()?.rows.map(r => r.id) ?? [], [], { equals: sameIds });
            return (
              <section class="waiting-group" data-group={id} aria-label={group()?.label ?? ''}>
                <div class="rail-group-label notif-day-sep"><span>{group()?.label}</span><span class="rail-group-count">{rowIds().length}</span></div>
                <For each={rowIds()}>{rowId => <Show when={rowById().get(rowId)}>{row => <Row row={row()} />}</Show>}</For>
              </section>
            );
          }}</For>
        </Show>
        <Show when={list().next() != null}>
          <button type="button" class="panel-link waiting-more" data-waiting-row aria-disabled={list().reloading() ? 'true' : undefined} onClick={() => void list().load(true)}>
            Show {moreCount(list().total(), list().items().length, filter() ? WAITING_FILTER_PAGE : WAITING_PAGE)} more
          </button>
        </Show>
        <Show when={props.view === 'decisions' && !filter() && (snoozed.latest ?? []).length}>
          <section class="waiting-group waiting-snoozed" aria-label="Snoozed">
            <div class="rail-group-label notif-day-sep"><span>Snoozed</span><span class="rail-group-count">{snoozed.latest!.length}</span></div>
            <For each={snoozed.latest}>{item => (
              <div class="waiting-item">
                <div class="waiting-row-line">
                  <span class="activity-item waiting-row"><Icon name="clock" size={13} /><span><strong>{itemTitle(item)}</strong><small>{item.snoozedUntil ? `Until ${wakeLabel(new Date(item.snoozedUntil))}` : 'Snoozed'}</small></span></span>
                  <button type="button" class="btn ghost sm" aria-label={`Wake now: ${itemTitle(item)}`} aria-disabled={waking() === item.id ? 'true' : undefined}
                    onClick={() => void wake(item.id)}>Wake now</button>
                </div>
              </div>
            )}</For>
          </section>
        </Show>
      </div>
    </div>
  );

  /** One row; `row` is reactive (the same id across reloads). */
  function Row(rowProps: { row: WaitingRow }) {
    const reason = () => waitingDisabledReason(rowProps.row);
    // A learning opens Library → Learnings, never a card here.
    const learning = () => rowProps.row.kind === 'item' && opensInLearnings(rowProps.row.item);
    const reviewId = () => { const target = waitingTarget(rowProps.row); return target?.kind === 'review' && !learning() ? target.id : null; };
    const failureId = () => { const target = waitingTarget(rowProps.row); return target?.kind === 'failure' ? target.id : null; };
    const expanded = () => { const id = reviewId(); return !!id && cardOpen(id); };
    // The card's item, from the review queue snapshot (`latest`: never
    // suspends; the snapshot catches its own read errors).
    const card = () => {
      const id = reviewId();
      return id ? (reviewQueueItems.latest ?? []).find((r): r is WorkspaceReviewItem => r.id === id && isWorkspaceReviewItem(r)) : undefined;
    };
    const [reread, setReread] = createSignal(false);
    const view = () => reviewCardView({ found: !!card(), loading: reviewQueueItems.loading, failed: reviewQueueFailed(), reread: reread() });
    // Missing from the snapshot: re-read it once before saying the item is
    // gone. Closing the card resets that, so opening it again retries.
    createEffect(() => {
      if (!expanded()) { setReread(false); return; }
      if (view() === 'reread') { setReread(true); void reloadReviewQueue(); }
    });
    const text = () => rowText(rowProps.row, where);
    return (
      <div class="waiting-item" data-id={rowProps.row.id} data-notif-id={failureId() ?? undefined}>
        <div class="waiting-row-line">
        <button
          type="button"
          class="activity-item waiting-row"
          data-waiting-row
          aria-disabled={reason() ? 'true' : undefined}
          aria-expanded={reviewId() ? expanded() : undefined}
          title={reason() ?? undefined}
          onClick={() => { if (learning()) void openLibraryLearnings(); else if (!reason()) openWaitingRow(rowProps.row, openers); }}
        >
          <Icon name={text().icon} size={13} class={text().tone} />
          <span><strong>{text().title}</strong><small>{reason() ?? text().meta}</small></span>
          {/* Icon reads its name once, so the two chevrons swap by Show. */}
          <Show when={!reason()}><Show when={expanded()} fallback={<Icon name="chevronRight" size={12} />}><Icon name="chevronDown" size={12} /></Show></Show>
        </button>
        {/* A failure is handled without opening it (P1-13); opening leaves it waiting (FIX-MR). */}
        <Show when={failureId()}>{id => (
          <button type="button" class="btn ghost sm waiting-handled" aria-label={`Mark handled: ${text().title}`} onClick={() => void markNotificationRead(id()).then(() => refreshWaitingOnYou(true))}>Mark handled</button>
        )}</Show>
        {/* A draft is settled without opening it, with Undo (P1-14, D3). */}
        <Show when={rowProps.row.kind === 'draft' ? rowProps.row.draft.path : null}>{path => (
          <button type="button" class="btn ghost sm waiting-handled" aria-label={`Discard: ${text().title}`} onClick={() => discardWaitingDraft(path())}>Discard</button>
        )}</Show>
        </div>
        <Show when={expanded()}>
          <Switch fallback={<p class="settings-note waiting-card-note" role="status">Loading…</p>}>
            <Match when={card()}>{found => <ReviewCard item={found()} onResolved={() => refreshWaitingOnYou(true)} />}</Match>
            <Match when={view() === 'unreadable'}>
              <p class="settings-note waiting-card-note" role="alert">Couldn’t load this item’s card. Close it and open it again to retry.</p>
            </Match>
            <Match when={view() === 'gone'}><p class="settings-note waiting-card-note">This item is no longer waiting.</p></Match>
          </Switch>
        </Show>
      </div>
    );
  }
}

/** A row's glyph, headline and second line. */
function rowText(row: WaitingRow, where: (workspaceId: string | null) => string): { icon: IconName; tone: string; title: string; meta: string } {
  if (row.kind === 'draft') {
    const name = row.draft.path.split('/').pop() ?? row.draft.path;
    return { icon: 'history', tone: 'tone-accent', title: `Unsaved draft: ${name}`, meta: row.draft.savedAt ? `Last changed ${relativeWhen(row.draft.savedAt)}` : 'Recovered after a restart' };
  }
  const item = row.item;
  const when = item.at ? relativeWhen(item.at) : '';
  const join = (...parts: (string | null | undefined)[]) => parts.filter(Boolean).join(' · ');
  switch (item.kind) {
    case 'approval':
      return { icon: 'alert', tone: 'tone-warn', title: `${item.detail || 'A conversation'} is waiting for your answer`, meta: join(item.title, when && `asked ${when}`, where(item.workspaceId)) };
    case 'question':
      return { icon: 'message', tone: 'tone-accent', title: item.title, meta: join(when && `Asked ${when}`, where(item.workspaceId)) };
    case 'canvas':
      return { icon: 'message', tone: 'tone-accent', title: item.title, meta: join('Canvas waiting for your answer', when && `asked ${when}`, where(item.workspaceId)) };
    case 'result':
      return { icon: 'eye', tone: 'tone-accent', title: item.title, meta: join(item.state === 'changed' ? 'Changed since accepted' : 'Ready for review', when, item.detail || where(item.workspaceId)) };
    case 'review':
      return { icon: 'bot', tone: 'tone-accent', title: item.title, meta: join(item.detail, when, opensInLearnings(item) ? 'Decide in Library Learnings' : null) };
    case 'failure':
      return { icon: 'alert', tone: 'tone-warn', title: item.title, meta: join(item.detail && trim(item.detail, 120), when) };
    case 'conflict':
      return { icon: 'refresh', tone: 'tone-warn', title: item.title, meta: join(item.detail, when, where(item.workspaceId)) };
    case 'interrupted':
      return { icon: 'refresh', tone: 'tone-accent', title: item.title, meta: join(item.detail && trim(item.detail, 120), when) };
    case 'copies':
      return { icon: 'copy', tone: 'tone-warn', title: item.title, meta: join(item.detail && trim(item.detail, 120), when) };
    case 'clash':
      return { icon: 'copy', tone: 'tone-warn', title: item.title, meta: join(item.state === 'editing' ? 'Editing' : null, item.detail && trim(item.detail, 120), when) };
    case 'lesson_memory':
      return { icon: 'bot', tone: 'tone-accent', title: item.title, meta: join(item.category, 'Lesson fixed in this build', when) };
  }
  return { icon: 'bell', tone: '', title: item.title, meta: '' };
}

function trim(s: string, n: number): string {
  return s.length <= n ? s : s.slice(0, n - 1).trimEnd() + '…';
}
