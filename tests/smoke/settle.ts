import { expect, type Locator } from '@playwright/test';

// Shared by smoke specs (moved from toast-placement.spec.ts, overnight
// 2026-10-16 A01). An axe scan or a box read taken while an entrance is still
// running samples a half-faded colour or a half-moved box: the Disk space
// confirm's title read #c05655 on #fefefd (4.42:1) mid-fade, while the settled
// --error on the card is 5.6:1.

/** Wait until nothing that moves `target` (or, with `within`, its matching
 * descendants) is still running. A snackbar springs in (`snackbar-in`:
 * translateY(14px) scale(.96) over --t-slow), the views' headers rise in
 * (today-rise, cl-rise, wr-rise), and a portal move to another view's region
 * replays the entrance; a box read mid-flight sits up to 14px off, which at
 * 390×844 is the difference between inside and outside the viewport. Counts
 * finite animations and transitions on the element or any ancestor (never
 * pseudo-elements or ambient infinite loops, which would never settle and do
 * not move it). A `within` that matches nothing fails rather than passing on
 * an empty set (a renamed class would otherwise make this a no-op). */
export async function settled(target: Locator, within?: string) {
  await expect.poll(() => target.evaluate((el, within) => {
    const watched = within ? [...el.querySelectorAll(within)] : [el];
    if (!watched.length) return `nothing inside matches ${within}`;
    return document.getAnimations().filter(animation => {
      const effect = animation.effect as KeyframeEffect | null;
      const moving = effect?.target;
      return animation.playState === 'running' && !effect?.pseudoElement && moving instanceof Element
        && watched.some(node => moving.contains(node)) && effect!.getComputedTiming().endTime !== Infinity;
    }).length;
  }, within), { message: `${target}${within ? ` (${within})` : ''} still animating` }).toBe(0);
}
