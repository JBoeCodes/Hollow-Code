import { afterEach, expect, it } from 'vitest';
import { activityDockOpen, activityTasks, openRunningActivity, setActivityDockOpen, setActivityTasks } from './activityDock';
import { bindDestinationNav, resetWorkView, workView, workViewShown } from './destinations';

afterEach(() => {
  setActivityDockOpen(false);
  setActivityTasks([]);
  resetWorkView();
  bindDestinationNav({ onHome: () => true, goHome: () => {} });
});

it('Manage opens Running, closes the popover and retains the in-flight operation', () => {
  let atHome = false;
  let visits = 0;
  bindDestinationNav({ onHome: () => atHome, goHome: arrived => { atHome = true; visits++; arrived(); } });
  const task = { id: 'pending-read', name: 'Read workspace', status: 'pending' as const, startedAt: 1 };
  setActivityTasks([task]);
  setActivityDockOpen(true);
  openRunningActivity();
  expect(workViewShown('activity')).toBe(true);
  expect(activityDockOpen()).toBe(false);
  expect(activityTasks()).toEqual([task]);
  expect(visits).toBe(1);
  openRunningActivity();
  expect(visits).toBe(1);
});

it('keeps the navigation guard authoritative when leaving a chat is refused', () => {
  bindDestinationNav({ onHome: () => false, goHome: () => {} });
  setActivityDockOpen(true);
  openRunningActivity();
  expect(workView()).toBe('home');
  expect(workViewShown('activity')).toBe(false);
  expect(activityDockOpen()).toBe(true);
});
