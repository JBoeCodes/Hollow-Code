import { afterEach, expect, it, vi } from 'vitest';
import { noticeTimer, setWindowAway, type NoticeTimer } from './noticeTimers';
const clocks: NoticeTimer[] = [];
afterEach(() => { clocks.splice(0).forEach(c => c.dispose()); setWindowAway(false); vi.useRealTimers(); });

it('a failure created away survives twenty seconds and keeps seven seconds on return', () => {
  vi.useFakeTimers(); setWindowAway(true);
  const expire = vi.fn(); clocks.push(noticeTimer(7000, expire));
  vi.advanceTimersByTime(20000); expect(expire).not.toHaveBeenCalled();
  setWindowAway(false); vi.advanceTimersByTime(6999); expect(expire).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1); expect(expire).toHaveBeenCalledOnce();
});

it('focus holds a toast independently of away and repeated focus does not reset its remainder', () => {
  vi.useFakeTimers(); const expire = vi.fn(); const clock = noticeTimer(9000, expire); clocks.push(clock);
  vi.advanceTimersByTime(4000); clock.hold(); clock.hold();
  setWindowAway(true); vi.advanceTimersByTime(20000); setWindowAway(false);
  vi.advanceTimersByTime(20000); expect(expire).not.toHaveBeenCalled();
  clock.release(); vi.advanceTimersByTime(4999); expect(expire).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1); expect(expire).toHaveBeenCalledOnce();
});

it('leaving focus while away does not start the countdown; return grants at least three seconds', () => {
  vi.useFakeTimers(); const expire = vi.fn(); const clock = noticeTimer(4000, expire); clocks.push(clock);
  vi.advanceTimersByTime(3500); clock.hold(); setWindowAway(true); clock.release();
  vi.advanceTimersByTime(20000); expect(expire).not.toHaveBeenCalled();
  setWindowAway(false); vi.advanceTimersByTime(2999); expect(expire).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1); expect(expire).toHaveBeenCalledOnce();
});

it('sticky notices and disposed notices never expire', () => {
  vi.useFakeTimers(); const expire = vi.fn(); clocks.push(noticeTimer(0, expire));
  const disposed = noticeTimer(7000, expire); disposed.dispose();
  setWindowAway(true); setWindowAway(false); vi.advanceTimersByTime(60000);
  expect(expire).not.toHaveBeenCalled();
});

it('pointer leave cannot release a keyboard-focused notice', () => {
  vi.useFakeTimers(); const expire = vi.fn(); const clock = noticeTimer(4000, expire); clocks.push(clock);
  clock.hold('focus'); clock.hold('hover'); clock.release('hover');
  vi.advanceTimersByTime(20000); expect(expire).not.toHaveBeenCalled();
  clock.release('focus'); vi.advanceTimersByTime(4000); expect(expire).toHaveBeenCalledOnce();
});

it('repeated window state writes keep the running deadline and held remainder', () => {
  vi.useFakeTimers(); const expire = vi.fn(); clocks.push(noticeTimer(7000, expire));
  vi.advanceTimersByTime(5000); setWindowAway(false);
  vi.advanceTimersByTime(1000); setWindowAway(true);
  vi.advanceTimersByTime(20000); setWindowAway(true);
  setWindowAway(false); vi.advanceTimersByTime(2000); setWindowAway(false);
  vi.advanceTimersByTime(999); expect(expire).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1); expect(expire).toHaveBeenCalledOnce();
});
