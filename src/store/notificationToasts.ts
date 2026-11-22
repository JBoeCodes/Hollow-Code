import { createSignal, untrack } from 'solid-js';
import type { Notification } from '../types';
import { noticeTimer, trimNotices, type NoticeTimer } from './noticeTimers';
import { recordAppMessage } from './appMessages';

export interface NotificationToast { notification: Notification }
export const [notificationToasts, setNotificationToasts] = createSignal<NotificationToast[]>([]);
const clocks = new Map<string, NoticeTimer>();

export function addNotificationToast(notification: Notification) {
  dismissNotificationToast(notification.id);
  recordAppMessage(notification.title, notification.kind === 'error', notification.body || undefined);
  setNotificationToasts([...untrack(notificationToasts), { notification }]);
  clocks.set(notification.id, noticeTimer(9000, () => dismissNotificationToast(notification.id)));
  trimToasts(notification.id);
}
function trimToasts(incoming?: string) {
  trimNotices(untrack(notificationToasts).map(t => t.notification.id), clocks, dismissNotificationToast, incoming);
}
export function holdNotificationToast(id: string, reason: string) { clocks.get(id)?.hold(reason); }
export function releaseNotificationToast(id: string, reason: string) {
  const clock = clocks.get(id); clock?.release(reason);
  if (!clock?.held()) trimToasts();
}
export function releaseNotificationToasts() {
  for (const clock of clocks.values()) { clock.release('focus'); clock.release('hover'); }
  trimToasts();
}
export function dismissNotificationToast(id: string) {
  clocks.get(id)?.dispose(); clocks.delete(id);
  setNotificationToasts(previous => previous.filter(t => t.notification.id !== id));
}
