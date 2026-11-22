import { beforeEach, expect, it, vi } from 'vitest';
// @ts-expect-error: browser build has no declarations; Node effects are inert.
vi.mock('solid-js', () => import('solid-js/dist/solid.js'));

const nav = vi.hoisted(() => ({ page: 'today', pending: null as null | (() => unknown), visits: [] as string[] }));
vi.mock('./runtime', async () => {
  const { createSignal } = await import('solid-js');
  const [page, setPage] = createSignal(nav.page);
  Object.defineProperty(nav, 'page', { get: page, set: setPage });
  return { activePage: page };
});
vi.mock('./workspaces', () => ({ goToPage: (page: string) => { nav.visits.push(page); nav.page = page; } }));
vi.mock('./navigationGuard', () => ({ guardNavigation: (action: () => unknown) => { nav.pending = action; } }));
import panelSource from '../pages/workflows/AutomationWindowsPanel.tsx?raw';
import workflowsSource from '../pages/Workflows.tsx?raw';
import settingsSource from '../pages/Settings.tsx?raw';
import { automationWindowsOpen, automationWindowsRequest, clearAutomationWindowsRequest, openAutomationWindows, setAutomationWindowsOpen } from './automationsViews';
import { automationsView, mainActionsOpen, mainActionsRevealRequest, consumeMainActionsReveal, openMainAgentActions, resetAutomationsView, setMainActionsOpen } from './automationsViews';

beforeEach(() => { nav.page = 'today'; nav.pending = null; nav.visits = []; setMainActionsOpen(false); resetAutomationsView(); });

it('does not open action history before accepted navigation', () => {
  openMainAgentActions();
  expect(mainActionsOpen()).toBe(false);
  expect(mainActionsRevealRequest()).toBe(0);
  expect(automationsView()).toBe('automations');
  expect(nav.visits).toEqual([]);
  nav.pending!();
  expect(mainActionsOpen()).toBe(true);
  expect(automationsView()).toBe('automations');
  expect(nav.visits).toEqual(['workflows']);
  expect(mainActionsRevealRequest()).toBeGreaterThan(0);
});

it('consumes a reveal once and does not replay it on a later mount', () => {
  openMainAgentActions(); nav.pending!();
  const request = mainActionsRevealRequest();
  expect(consumeMainActionsReveal(request)).toBe(true);
  expect(consumeMainActionsReveal(request)).toBe(false);
  expect(mainActionsRevealRequest()).toBe(0);
});

it('cancels on page leave even before the lazy Workflows view registers cleanup', () => {
  openMainAgentActions(); nav.pending!();
  const request = mainActionsRevealRequest();
  nav.page = 'today'; // no Workflows mount or resetAutomationsView call
  nav.page = 'workflows';
  expect(mainActionsRevealRequest()).toBe(0);
  expect(consumeMainActionsReveal(request)).toBe(false);
});

it('does not consume a superseded request or reveal after leaving the destination', () => {
  openMainAgentActions(); nav.pending!();
  const first = mainActionsRevealRequest();
  openMainAgentActions(); nav.pending!();
  const second = mainActionsRevealRequest();
  expect(second).not.toBe(first);
  expect(consumeMainActionsReveal(first)).toBe(false);
  nav.page = 'today';
  expect(consumeMainActionsReveal(second)).toBe(false);
  resetAutomationsView();
  nav.page = 'workflows';
  expect(consumeMainActionsReveal(second)).toBe(false);
});

it('leaves a closed disclosure untouched', () => {
  openMainAgentActions(); nav.pending!();
  const latest = mainActionsRevealRequest();
  setMainActionsOpen(false);
  expect(consumeMainActionsReveal(latest)).toBe(false);
});

it('opens the disclosure in place without a second page navigation', () => {
  nav.page = 'workflows';
  openMainAgentActions(); nav.pending!();
  expect(mainActionsOpen()).toBe(true);
  expect(nav.visits).toEqual([]);
});

// Settings PR 4 (overnight 11-12): automation windows and the run queue live on Automations.
it('opens the windows fold only after accepted navigation, then goes to Automations', () => {
  setAutomationWindowsOpen(false); clearAutomationWindowsRequest();
  openAutomationWindows('queue');
  expect(automationWindowsOpen()).toBe(false);
  expect(automationWindowsRequest()).toBeNull();
  nav.pending!();
  expect(automationWindowsOpen()).toBe(true);
  expect(automationWindowsRequest()).toMatchObject({ tab: 'queue' });
  expect(nav.visits).toEqual(['workflows']);
});

it('opens the windows fold in place when Automations is already shown', () => {
  nav.page = 'workflows';
  openAutomationWindows('windows'); nav.pending!();
  expect(automationWindowsRequest()).toMatchObject({ tab: 'windows' });
  expect(nav.visits).toEqual([]);
});

it('keeps the editor on Automations: Add automation from a window opens it without leaving, and Settings keeps one link row', () => {
  expect(workflowsSource).toContain('<WindowsAndQueue />');
  expect(panelSource).toContain('setNewAutomationWindow(name)');
  expect(panelSource).not.toContain("goToPage('workflows')");
  expect(settingsSource).toContain('Automations, windows and the run queue are on the Automations page');
  expect(settingsSource).not.toContain('AutomationWindowsPanel');
  expect(settingsSource).not.toContain('RunQueuePanel');
});
