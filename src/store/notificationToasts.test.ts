import { afterEach, expect, it, vi } from 'vitest';
import { addNotificationToast, dismissNotificationToast, holdNotificationToast, notificationToasts, releaseNotificationToast, releaseNotificationToasts } from './notificationToasts';
import { appMessages, setAppMessages } from './appMessages';
import { setWindowAway } from './noticeTimers';
import type { Notification } from '../types';
const notification = (id: string): Notification => ({ id, title: `Notice ${id}`, body: 'Full detail', kind: 'error', source: null, actionLabel: null, actionUrl: null, created: '', read: false });
afterEach(() => {
  notificationToasts().forEach(t => dismissNotificationToast(t.notification.id));
  setAppMessages([]); setWindowAway(false); vi.useRealTimers();
});

it('eviction retains text and disposes the old timer', () => {
  vi.useFakeTimers();
  for (let i = 0; i < 6; i++) addNotificationToast(notification(String(i)));
  expect(notificationToasts()).toHaveLength(5);
  expect(vi.getTimerCount()).toBe(5);
  expect(appMessages()[5]).toMatchObject({ message: 'Notice 0', failed: true, detail: 'Full detail' });
  vi.advanceTimersByTime(9000);
  expect(notificationToasts()).toHaveLength(0);
  expect(vi.getTimerCount()).toBe(0);
});

it('replacing the same id cancels its older deadline', () => {
  vi.useFakeTimers(); addNotificationToast(notification('same'));
  vi.advanceTimersByTime(8000); addNotificationToast(notification('same'));
  expect(vi.getTimerCount()).toBe(1); expect(notificationToasts()).toHaveLength(1);
  vi.advanceTimersByTime(1000); expect(notificationToasts()).toHaveLength(1);
  vi.advanceTimersByTime(8000); expect(notificationToasts()).toHaveLength(0);
});

it('moving a focused toast to another portal does not release the away hold', () => {
  vi.useFakeTimers(); addNotificationToast(notification('held'));
  vi.advanceTimersByTime(8000); holdNotificationToast('held', 'focus'); holdNotificationToast('held', 'hover');
  releaseNotificationToast('held', 'hover'); setWindowAway(true); releaseNotificationToasts();
  vi.advanceTimersByTime(20000); expect(notificationToasts()).toHaveLength(1);
  setWindowAway(false); vi.advanceTimersByTime(2999); expect(notificationToasts()).toHaveLength(1);
  vi.advanceTimersByTime(1); expect(notificationToasts()).toHaveLength(0);
});

it('a sixth notification preserves the focused oldest toast', () => {
  vi.useFakeTimers();
  for (let i = 0; i < 5; i++) addNotificationToast(notification(String(i)));
  holdNotificationToast('0', 'focus'); addNotificationToast(notification('5'));
  expect(notificationToasts().map(t => t.notification.id)).toEqual(['0', '2', '3', '4', '5']);
  expect(vi.getTimerCount()).toBe(4);
});

it('all-held overflow survives until release and portal release trims it', () => {
  vi.useFakeTimers();
  for (let i = 0; i < 5; i++) {
    addNotificationToast(notification(String(i)));
    holdNotificationToast(String(i), 'focus'); holdNotificationToast(String(i), 'hover');
  }
  addNotificationToast(notification('5'));
  expect(notificationToasts()).toHaveLength(6);
  releaseNotificationToast('0', 'focus');
  expect(notificationToasts().some(t => t.notification.id === '0')).toBe(true);
  releaseNotificationToasts();
  expect(notificationToasts().map(t => t.notification.id)).toEqual(['1', '2', '3', '4', '5']);
  expect(vi.getTimerCount()).toBe(5);
});
