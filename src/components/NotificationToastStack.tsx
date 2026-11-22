import { Portal } from 'solid-js/web';
import { toastMount } from '../toastPlacement';
import { For, Show, createEffect, on, untrack } from 'solid-js';
import { notificationToasts, dismissNotificationToast, activateNotification } from '../store';
import { holdNotificationToast, releaseNotificationToast, releaseNotificationToasts } from '../store/notificationToasts';
import type { Notification } from '../types';
import { isWorkspaceNotification } from '../workspaceContent';

/// In-app cards live inside the uppermost full view, or top-right. Mirrors the in-app
/// notification center entries — when a notification lands, both the
/// persistent center and a transient toast in this stack get one. The
/// toast auto-dismisses after ~9s (longer than snackbars since they
/// carry richer content). Clicking the body navigates via action_url and
/// marks non-failures read; failures keep explicit acknowledgment. The "×" only dismisses
/// the toast — the center entry stays put until the user marks/deletes
/// it explicitly.
export function NotificationToastStack() {
  createEffect(on(toastMount, () => untrack(releaseNotificationToasts), { defer: true }));
  return (
    <Portal mount={toastMount()}>
    <div class="notif-toast-stack">
      <For each={notificationToasts().filter(toast => isWorkspaceNotification(toast.notification))}>
        {(t) => <NotificationToast notification={t.notification} />}
      </For>
    </div>
    </Portal>
  );
}

function NotificationToast(props: { notification: Notification }) {
  const n = () => props.notification;
  const kindClass = () => `notif-toast notif-kind-${n().kind}`;
  return (
    <div class={kindClass()} role={n().kind === 'error' ? 'alert' : 'status'}
      onFocusIn={() => holdNotificationToast(n().id, 'focus')}
      onFocusOut={e => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) releaseNotificationToast(n().id, 'focus'); }}
      onMouseEnter={() => holdNotificationToast(n().id, 'hover')}
      onMouseLeave={() => releaseNotificationToast(n().id, 'hover')}>
      <button
        type="button"
        class="notif-toast-body"
        // Opening a failure is navigation; its explicit Mark read acknowledges it.
        onClick={() => activateNotification({ ...n(), read: n().read || n().kind === 'error' })}
        title={n().actionUrl ? 'Open' : undefined}
      >
        <span class={`notif-toast-dot kind-${n().kind}`} aria-hidden="true" />
        <div class="notif-toast-text">
          <div class="notif-toast-title">{n().title}</div>
          <Show when={n().body}>
            <div class="notif-toast-snippet">{trim(n().body, 180)}</div>
          </Show>
          <Show when={n().source || n().actionUrl}>
            <div class="notif-toast-meta">
              <Show when={n().source}>
                <span class="notif-toast-source">{n().source}</span>
              </Show>
              <Show when={n().actionUrl}>
                <span class="notif-toast-cta">{n().actionLabel || 'Open'} →</span>
              </Show>
            </div>
          </Show>
        </div>
      </button>
      <button
        type="button"
        class="notif-toast-close"
        onClick={() => dismissNotificationToast(n().id)}
        aria-label="Dismiss notification"
        title="Dismiss"
      >×</button>
    </div>
  );
}

function trim(s: string, n: number): string {
  if (s.length <= n) return s;
  return s.slice(0, n - 1).trimEnd() + '…';
}
