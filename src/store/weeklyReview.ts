// The weekly review (overnight 10-11 A7): whether the view is open and which
// week it shows (`''`: this week). Its own module so the shell can mount the
// lazy view without importing it.
// Since consolidation D (2026-10-13) it is a view of Your work, and since S3
// (2026-11-03) the Week range of its Today tab (a tab of its own until then).
import { createSignal } from 'solid-js';

const [week, setWeek] = createSignal('');
/** The Weekly review screen was cut (overnight 11-08 S3). The week signal stays for the date math. */
export const weeklyReviewOpen = () => false;
export const weeklyReviewWeek = week;
export const setWeeklyReviewWeek = setWeek;
export const [weeklyReviewSeen, setWeeklyReviewSeen] = createSignal((() => {
  try { return localStorage.getItem('hollow.weeklyReviewDismissed'); } catch { return null; }
})());
export function acknowledgeWeeklyReview(w: string): void {
  if (!w) return;
  setWeeklyReviewSeen(w);
  try { localStorage.setItem('hollow.weeklyReviewDismissed', w); } catch { /* keep this session's acknowledgement */ }
}
/** Remember a week without opening a screen. The Weekly review view is gone. */
export function openWeeklyReview(w = ''): void { setWeek(w); }
export function closeWeeklyReview(): void { /* no screen to close */ }

/** The Today tab prompts for the weekly review from Friday 15:00 to Monday noon. */
export function reviewCardDue(now: Date): boolean {
  const day = now.getDay();
  const hour = now.getHours();
  return (day === 5 && hour >= 15) || day === 6 || day === 0 || (day === 1 && hour < 12);
}
