import { Show, Suspense, createEffect, createSignal, createUniqueId, lazy, on, onCleanup, onMount } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Icon } from '../icons';
import { chatBusy, chatTurns, enableNotificationsResource, notifications, pendingApprovals, refetchNotifications, unreadNotificationCount } from '../store';
import { workspaceSessions } from '../store/workspaces';
import { refreshWaitingOnYou, waitingBadgeState, waitingList } from '../store/waitingOnYou';
import { reviewRowLabel } from '../waitingOnYou';
import { reviewInboxRevision } from '../reviewInbox';
import { RecoverableView } from './RecoverableView';
import '../styles/notification-bell.css';

const NotificationCenter = lazy(() => import('./NotificationCenter'));

/** The bell stays mounted; its history loads only on opening. */
export function NotificationBell() {
  const [open, setOpen] = createSignal(false);
  const [needsYou, setNeedsYou] = createSignal(false);
  const badge = () => waitingBadgeState();
  // Waiting and unread counts overlap for failures. Name them separately,
  // never add them into a misleading total.
  const count = () => badge().count || unreadNotificationCount();
  const checking = () => !waitingList.loaded() && !waitingList.failed();
  const unavailable = () => notifications.error || waitingList.failed() || waitingList.unavailable().length > 0;
  const id = `notification-center-${createUniqueId()}`;
  let trigger!: HTMLButtonElement, panel: HTMLElement | undefined;
  const close = (returnFocus = false) => { setOpen(false); if (returnFocus) trigger.focus(); };
  function toggle() {
    if (open()) { close(); return; }
    setNeedsYou(badge().count !== 0 || badge().drafts > 0 || waitingList.unavailable().length > 0);
    refreshWaitingOnYou(true);
    enableNotificationsResource();
    if (notifications.state !== 'pending') void refetchNotifications();
    setOpen(true);
    queueMicrotask(() => panel?.querySelector<HTMLButtonElement>('button')?.focus());
  }
  onMount(() => {
    refreshWaitingOnYou(true);
    const timer = window.setInterval(() => refreshWaitingOnYou(), 60_000);
    const outside = (event: Event) => {
      if (open() && !trigger.contains(event.target as Node) && !panel?.contains(event.target as Node)) close();
    };
    document.addEventListener('pointerdown', outside);
    document.addEventListener('focusin', outside);
    onCleanup(() => { window.clearInterval(timer); document.removeEventListener('pointerdown', outside); document.removeEventListener('focusin', outside); });
  });
  createEffect(on([pendingApprovals, workspaceSessions, reviewInboxRevision, unreadNotificationCount, () => chatTurns.length, chatBusy], () => refreshWaitingOnYou(), { defer: true }));
  return <>
    <button ref={trigger} type="button" class="top-bar-iconbtn notification-bell" aria-label="Notifications"
      title="Notifications" aria-haspopup="dialog" aria-expanded={open()} aria-controls={open() ? id : undefined}
      aria-description={[checking() ? 'Checking what needs you' : reviewRowLabel(badge(), 'Needs you'), waitingList.unavailable().length ? 'Some waiting sources unavailable' : '', notifications.error ? 'Notification history unavailable' : `${unreadNotificationCount()} unread notifications`].filter(Boolean).join('. ')}
      onClick={toggle}>
      <Icon name="bell" size={16}/>
      <Show when={unavailable() || count() > 0 || badge().drafts > 0}><span class="notification-bell-count notification-bell-dot" aria-hidden="true">{unavailable() ? '!' : '•'}</span></Show>
    </button>
    <Show when={open()}><Portal>
      <section ref={panel} id={id} class="notification-center" role="dialog" aria-label="Notifications" tabindex="-1"
        onKeyDown={event => { if (event.key === 'Escape' && !event.defaultPrevented) { event.preventDefault(); event.stopPropagation(); close(true); } }}>
        <header class="notification-center-head"><strong>Notifications</strong><button type="button" class="icon-btn" aria-label="Close notifications" onClick={() => close(true)}><Icon name="x" size={16}/></button></header>
        <RecoverableView name="Notifications" compact fallback={<p role="status">Loading notifications…</p>}>
          <Suspense fallback={<p role="status">Loading notifications…</p>}><NotificationCenter initialNeedsYou={needsYou()} onClose={() => close(true)}/></Suspense>
        </RecoverableView>
      </section>
    </Portal></Show>
  </>;
}
