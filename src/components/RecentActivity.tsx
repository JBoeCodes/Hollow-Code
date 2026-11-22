import { For, Show, createMemo } from 'solid-js';
import { Icon } from '../icons';
import { notifFilter, setNotifFilter, type NotifFilter } from '../store/prefs';
import {
  notifications, notificationTotal, noteIndex, markNotificationRead, markNotificationsRead, deleteNotification, deleteNotifications, activateNotification, refetchNotifications, showConfirm,
} from '../store';
import type { Notification } from '../types';
import { foldRepeats, isWaitingFailure, type ActivityRow } from '../activity';
import { isWorkspaceNotification } from '../workspaceContent';
import { latestUnlessErrored } from '../stableResource';
import { AppMessages } from './AppMessages';
import { ErrorText } from './ui';
import { errMsg } from '../utils';
import { shortcutsEnabled } from '../shortcuts';
import { reviewKey } from '../reviewKeys';
import { formatRelative } from '../formatTime';

/// How many notifications the list receives at most (notifications.rs READ_CAP).
const LIST_CAP = 200;

/// The sources Hollow's own background loops write. Anything else that
/// carries a source is a routine slug — which is what the Routines filter
/// collects, so a new routine needs no change here.
const LOOP_SOURCES = new Set(['nudge', 'heartbeat', 'plaud', 'disk-guard']);

const FILTERS: { id: NotifFilter; label: string; match: (n: Notification) => boolean }[] = [
  { id: 'all', label: 'All', match: () => true },
  { id: 'attention', label: 'Failures', match: n => n.kind === 'error' },
  { id: 'routines', label: 'Routines', match: n => !!n.source && !LOOP_SOURCES.has(n.source) },
  { id: 'nudges', label: 'Nudges', match: n => n.source === 'nudge' },
  { id: 'agent', label: 'Agent', match: n => n.source === 'heartbeat' },
];

interface DayGroup { key: string; label: string; items: ActivityRow[] }

function markHistoryRowRead(ids: string[]) {
  return ids.length > 1 ? markNotificationsRead(ids) : markNotificationRead(ids[0]);
}

/**
 * The one notification history (routine runs, nudges, agent observations and
 * failures): Bell → Notifications (U3b). It was the full waiting view's
 * Recent notifications; that copy is gone. Unread failures are listed here
 * one by one, with their own Open and Mark read, because they still wait in
 * Needs you: opening one never marks it read, and the listed bulk actions
 * skip them (P0-08). Mounted only while the Notifications view is open; it
 * reads the bell's notifications resource and adds no read of its own.
 */
export function NotificationHistory(props: { onClose: () => void }) {
  let historyEl: HTMLDivElement | undefined;
  const list = createMemo(() => [...(latestUnlessErrored(notifications) ?? [])].filter(isWorkspaceNotification)
    .sort((a, b) => b.created.localeCompare(a.created)));
  const active = () => FILTERS.find(x => x.id === notifFilter()) ?? FILTERS[0];
  const shown = createMemo(() => list().filter(active().match));
  /// Only what this list shows, never an unread failure (P0-08): that keeps
  /// its own Mark read, a deliberate choice per row.
  const readable = () => shown().filter(n => !n.read && !isWaitingFailure(n)).map(n => n.id);
  const clearable = () => shown().filter(n => !isWaitingFailure(n)).map(n => n.id);

  /// Clear listed: asks first, with the count, and removes only the rows
  /// listed under this filter. The confirm is its own dialog, and the bell
  /// closes when focus leaves it, so close first.
  async function clearListed() {
    const ids = clearable();
    if (!ids.length) return;
    props.onClose();
    if (await showConfirm({ title: `Clear ${ids.length} notification${ids.length === 1 ? '' : 's'}?`, body: 'Only the listed history is removed. Unread failures stay in Needs you. This cannot be undone.', confirmLabel: 'Clear', danger: true, focusCancel: true })) await deleteNotifications(ids);
  }

  const groups = createMemo<DayGroup[]>(() => {
    const out: DayGroup[] = [];
    const byKey = new Map<string, DayGroup>();
    // Repeated notices fold into one row (next-level P19), per day; an
    // unread failure never folds, so it keeps its own row and actions.
    for (const row of foldRepeats(shown(), undefined, isWaitingFailure)) {
      const key = dayKey(row.head.created);
      let g = byKey.get(key);
      if (!g) {
        g = { key, label: dayLabel(key), items: [] };
        byKey.set(key, g);
        out.push(g);
      }
      g.items.push(row);
    }
    return out;
  });

  /// A `vault:` target can outlive its file. The note index is already in
  /// the store, so the check costs no IPC; an empty index means it hasn't
  /// loaded yet, and saying nothing beats striking every row.
  function targetMissing(n: Notification): boolean {
    const url = n.actionUrl;
    if (!url?.startsWith('vault:')) return false;
    const idx = noteIndex();
    if (!idx.length) return false;
    const rel = url.slice('vault:'.length);
    return !idx.some(r => r.path === rel);
  }

  // Keep the history's existing list keys local to its new bell home.
  function onKey(event: KeyboardEvent) {
    if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest('input, textarea, select, [contenteditable="true"]')) return;
    if (!shortcutsEnabled() && event.key.length === 1) return;
    const row = target?.closest<HTMLElement>('[data-notif-id]');
    if (event.key === 'j' || event.key === 'k' || event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      const buttons = Array.from(historyEl?.querySelectorAll<HTMLButtonElement>('.notif-row-body') ?? []);
      if (!buttons.length) return;
      const index = buttons.findIndex(button => button.closest('[data-notif-id]') === row);
      const forward = event.key === 'j' || event.key === 'ArrowDown';
      const next = index < 0 ? (forward ? 0 : buttons.length - 1) : Math.max(0, Math.min(buttons.length - 1, index + (forward ? 1 : -1)));
      event.preventDefault(); event.stopPropagation(); buttons[next].focus();
    } else if (row?.dataset.notifId && reviewKey('notification', event) === 'dismiss') {
      const item = groups().flatMap(group => group.items).find(item => item.head.id === row.dataset.notifId);
      if (item) { event.preventDefault(); event.stopPropagation(); void markHistoryRowRead(item.ids); }
    }
  }

  return <>
    <div class="notification-center-actions">
      <button type="button" class="btn small ghost" disabled={notifications.loading} onClick={() => void refetchNotifications()}>Refresh</button>
      <Show when={readable().length}><button type="button" class="btn small ghost" title="Mark the listed notifications read; unread failures keep their own Mark read" onClick={() => void markNotificationsRead(readable())}>Mark listed read</button></Show>
      <Show when={clearable().length}><button type="button" class="btn small ghost" title="Remove the listed notifications; unread failures stay" onClick={() => void clearListed()}>Clear listed</button></Show>
    </div>
    <div ref={historyEl} class="notification-center-scroll" role="region" aria-label="Notification history" onKeyDown={onKey}>
      <div class="notif-center-filters notification-history-filters" role="group" aria-label="Filter notifications">
        <For each={FILTERS}>{f => (
          <button type="button" class="chip notif-center-chip" classList={{ active: active().id === f.id }} aria-pressed={active().id === f.id} onClick={() => setNotifFilter(f.id)}>
            {f.label}
            <Show when={f.id !== 'all'}><span class="notif-center-chip-count">{list().filter(f.match).length}</span></Show>
          </button>
        )}</For>
      </div>
      <Show when={notifications.error}><ErrorText class="notification-history-error" text={errMsg(notifications.error)} /><button type="button" class="btn small ghost" disabled={notifications.loading} onClick={() => void refetchNotifications()}>Retry notifications</button></Show>
      <Show when={notifications.loading}><p class="hint" role="status">Loading notifications…</p></Show>
      {/* Only beside rows: the total also counts other folders' notifications, which this list never shows. */}
      <Show when={list().length > 0 && notificationTotal() > LIST_CAP}>
        <p class="hint notification-history-capped">Newest {LIST_CAP} of {notificationTotal()}.</p>
      </Show>
      <Show when={groups().length > 0} fallback={
        // A failed or unfinished read is never "nothing": the error and
        // loading lines above say which.
        <Show when={notifications.state === 'ready'}>
          <div class="empty-state">
            <p>{list().length ? 'Nothing in this view.' : 'Nothing since you last looked.'}</p>
            <p class="hint">{list().length ? 'Pick another filter to see the rest.' : 'Routine runs, nudges, agent observations and failures land here.'}</p>
          </div>
        </Show>
      }>
        <For each={groups().map(group => group.key)}>{key => {
          const group = () => groups().find(item => item.key === key)!;
          return (
          <section class="notif-day-group">
            <div class="rail-group-label notif-day-sep"><span>{group().label}</span><span class="rail-group-count">{group().items.length}</span></div>
            <For each={group().items.map(item => item.head.id)}>{id => {
              const row = () => group().items.find(item => item.head.id === id)!;
              return <NotificationRow notification={row().head} ids={row().ids} repeats={row().repeats} firstAt={row().firstAt} missing={targetMissing(row().head)} onClose={props.onClose} />;
            }}</For>
          </section>
          );
        }}</For>
      </Show>
      <details class="notification-app-messages"><summary>App messages</summary><AppMessages/></details>
    </div>
  </>;
}

function NotificationRow(props: {
  notification: Notification;
  ids: string[];
  /** Folded repeats of the same notice (next-level P19). */
  repeats?: number;
  firstAt?: string;
  missing: boolean;
  onClose: () => void;
}) {
  const n = () => props.notification;
  const waiting = () => isWaitingFailure(n());
  // The glyph carries the source, so the text label is kept only for
  // routine slugs — there the name itself is the information.
  const slug = () => (n().source && !LOOP_SOURCES.has(n().source!) ? n().source : null);
  return (
    <div class={`notif-row notif-kind-${n().kind} ${n().read ? 'read' : 'unread'}`} data-notif-id={n().id}>
      <button
        type="button"
        class="notif-row-body"
        onClick={() => {
          const notification = n(), ids = [...props.ids], failure = waiting();
          // A dead target still gets acknowledged; it just doesn't send the
          // user to a note that isn't there any more. An unread failure
          // waits in Needs you, so only its own Mark read clears it.
          if (props.missing) { if (!failure) void markHistoryRowRead(ids); return; }
          // Close first: a review link reopens Waiting on you filtered to
          // its page, and closing after it would shut that again.
          if (notification.actionUrl) props.onClose();
          if (!notification.read && !failure) void markHistoryRowRead(ids);
          // Opening a failure follows its link and leaves it waiting (P1-13,
          // run 13 FIX-MR): `read: true` skips activateNotification's mark-read.
          activateNotification({ id: notification.id, read: true, actionUrl: notification.actionUrl });
        }}
        title={props.missing ? 'The file this points at has moved' : n().actionUrl ? 'Open' : waiting() ? 'Mark read clears this failure' : undefined}
      >
        <Icon name={sourceGlyph(n().source)} size={14} class={`notif-row-glyph kind-${n().kind}`} />
        <div class="notif-row-text">
          <div class="notif-row-head">
            <span class="notif-row-title">{n().title}<Show when={(props.repeats ?? 1) > 1}><span class="activity-repeats" title={`The same notice ${props.repeats} times since ${props.firstAt ? new Date(props.firstAt).toLocaleString() : ''}`}> · {props.repeats} times</span></Show></span>
            <span class="notif-row-time" title={absolute(n().created)}>{relative(n().created)}</span>
          </div>
          <Show when={n().body}><div class="notif-row-snippet">{trim(n().body, 220)}</div></Show>
          <Show when={slug() || n().actionUrl || waiting()}>
            <div class="notif-row-meta dim">
              <Show when={waiting()}><span class="notif-row-source">Failure · Unread</span></Show>
              <Show when={slug()}><span class="notif-row-source">{slug()}</span></Show>
              <Show when={n().actionUrl}><span class="notif-row-cta" classList={{ missing: props.missing }}>{props.missing ? 'Target moved' : n().actionLabel || 'Open'}</span></Show>
            </div>
          </Show>
        </div>
      </button>
      <div class="notif-row-actions">
        <Show when={!n().read}>
          <button type="button" class="notif-row-icon-btn" onClick={() => void markHistoryRowRead(props.ids)} title="Mark read (x)" aria-label="Mark read">
            <Icon name="checkSquare" size={12} />
          </button>
        </Show>
        <button type="button" class="notif-row-icon-btn" onClick={() => void (props.ids.length > 1 ? deleteNotifications(props.ids) : deleteNotification(n().id))} title="Delete" aria-label={props.ids.length > 1 ? 'Delete notifications' : 'Delete notification'}>×</button>
      </div>
      <details class="notif-row-details">
        <summary>Details</summary>
        <strong>{n().title}</strong>
        <time dateTime={n().created}>{absolute(n().created)}</time>
        <Show when={n().body}><p>{n().body}</p></Show>
      </details>
    </div>
  );
}

/// One glyph per producer, so a routine's output reads differently from a
/// nudge or an agent observation without reading a word.
function sourceGlyph(source: string | null): import('../icons').IconName {
  switch (source) {
    case 'nudge': return 'zap';
    case 'heartbeat': return 'bot';
    case 'plaud': return 'message';
    case null: case '': return 'bell';
    default: return 'fileText';
  }
}

function trim(s: string, n: number): string {
  if (s.length <= n) return s;
  return s.slice(0, n - 1).trimEnd() + '…';
}

/// Local calendar day of a notification, as a sortable key.
function dayKey(iso: string): string {
  const d = new Date(iso);
  if (!Number.isFinite(d.getTime())) return '';
  const mo = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${d.getFullYear()}-${mo}-${day}`;
}

function dayLabel(key: string): string {
  if (!key) return 'Undated';
  const [y, m, d] = key.split('-').map(Number);
  const day = new Date(y, m - 1, d);
  const today = new Date(); today.setHours(0, 0, 0, 0);
  const diff = Math.round((today.getTime() - day.getTime()) / 86400000);
  if (diff === 0) return 'Today';
  if (diff === 1) return 'Yesterday';
  return day.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' });
}

/// Full local date and time for the hover title and expanded row: the only way to date an old row.
function absolute(iso: string): string {
  return new Date(iso).toLocaleString();
}

/// Shared relative label, then an absolute date after the first week.
function relative(iso: string): string {
  return formatRelative(Date.parse(iso), Date.now());
}
