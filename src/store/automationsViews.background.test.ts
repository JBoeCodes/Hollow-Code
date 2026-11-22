import { beforeEach, expect, it, vi } from 'vitest';
// @ts-expect-error: browser build has no declarations; Node effects are inert.
vi.mock('solid-js', () => import('solid-js/dist/solid.js'));

// The background-work explainer (plan completion L2) and the windows fold
// (Settings PR 4) open only through an accepted navigation, as Main's action
// history does; the navigation test next to this one covers that history.
const nav = vi.hoisted(() => ({ page: 'today', pending: null as null | (() => unknown), visits: [] as string[] }));
vi.mock('./runtime', () => ({ activePage: () => nav.page }));
vi.mock('./workspaces', () => ({ goToPage: (page: string) => { nav.visits.push(page); nav.page = page; } }));
vi.mock('./navigationGuard', () => ({ guardNavigation: (action: () => unknown) => { nav.pending = action; } }));
import {
  automationWindowsOpen, automationWindowsRequest, backgroundWorkOpen, clearAutomationWindowsRequest, openAutomationsView,
  openAutomationWindows, openBackgroundWork, setAutomationWindowsOpen, setBackgroundWorkOpen,
} from './automationsViews';

beforeEach(() => {
  nav.page = 'today'; nav.pending = null; nav.visits = [];
  setBackgroundWorkOpen(false); setAutomationWindowsOpen(false); clearAutomationWindowsRequest();
});

it('opens the explainer at once, and the page only when the navigation guard accepts', () => {
  openBackgroundWork();
  expect(backgroundWorkOpen()).toBe(true);
  expect(nav.visits).toEqual([]);
  nav.pending!();
  expect(nav.visits).toEqual(['workflows']);
});

it('on Automations already, opens the explainer without a navigation or a guard', () => {
  nav.page = 'workflows';
  openBackgroundWork();
  expect(backgroundWorkOpen()).toBe(true);
  expect(nav.pending).toBeNull();
  expect(nav.visits).toEqual([]);
});

it('opening the list while it is already shown does not even ask the guard', () => {
  nav.page = 'workflows';
  expect(openAutomationsView()).toBeUndefined();
  expect(nav.pending).toBeNull();
});

it('opens the windows fold on the named tab only once the visit is accepted', () => {
  const onOpen = vi.fn();
  openAutomationWindows('queue', onOpen);
  expect(automationWindowsOpen()).toBe(false);
  expect(automationWindowsRequest()).toBeNull();
  expect(onOpen).not.toHaveBeenCalled();
  nav.pending!();
  expect(onOpen).toHaveBeenCalledTimes(1);
  expect(automationWindowsOpen()).toBe(true);
  expect(automationWindowsRequest()?.tab).toBe('queue');
  expect(nav.visits).toEqual(['workflows']);
});

it('defaults to the windows tab, and a request clears once the fold has scrolled to it', () => {
  openAutomationWindows();
  nav.pending!();
  expect(automationWindowsRequest()?.tab).toBe('windows');
  clearAutomationWindowsRequest();
  expect(automationWindowsRequest()).toBeNull();
  expect(automationWindowsOpen()).toBe(true);
});
