/**
 * The webview half of Hollow app tools (`hollow_app`, `src-tauri/src/app_broker.rs`).
 *
 * PR A2: an agent's changes arrive as events, so what the app shows keeps up
 * with them. `app-session-started` (a session an agent started or messaged)
 * puts its workspace in the rail, the same way the user's first send does,
 * and refreshes the session lists. `app-changed` (any change an agent made)
 * refreshes the lists the vault watcher can't see: workspaces and the
 * session store. Vault notes, tasks and goals already refresh through
 * `vault-changed`. A changed backend setting (Learning suggestions, the
 * extraction model) bumps `store/settingsRevision.ts`, so an open Settings
 * panel reads it again (overnight H6).
 *
 * PR A4: `app-ui-request {id, op, args}` asks the webview for what only it
 * owns, and every answer goes back through `app_ui_respond`. `get` / `set`
 * read and change UI-owned settings through their store setters (the only
 * writers of their localStorage keys, so every derived signal follows);
 * `navigate` opens a page or a record. The keys here are the Ui rows of
 * `src-tauri/src/settings_keys.rs` (a vitest guard holds the two together).
 */
import { createSignal } from 'solid-js';
import {
  applyPendingWith, bool, ids, planForChanged, planForStarted, serializer, toggleTo, answerWith, navigateTarget, visiblePath,
  type AppChanged, type AppSessionStarted, type AppUiAnswer, type AppUiRequest, type PendingUiSettings, type RefreshPlan, type UiSetting, type UiSettingKey,
} from './appBridgeCore';
import { listen } from './nativeEvents';
import { invoke } from '@tauri-apps/api/core';
import { api } from './api';
import type { HeartbeatConfig } from './types';
import { addToRail, refreshWorkspaces, openSessionForAgent, refreshWorkspaceSessions, selectWorkspace } from './store/workspaces';
import { openBrowserUrl } from './store/browser';
import { openPreview } from './store/preview';
import { normalizeAddress } from './browserUrl';
import {
  agentRoles, chatContextEnabled, chatPanelOpen, computerControlEnabled, heartbeatConfig, heartbeatDraftsEnabled,
  heartbeatEnabled, minimizeToTray, notifyInApp, notifyNative, nudgesEnabled, openVaultNote, openWorkflowBySlug,
  operatorContextEnabled, persistSidePaneWidth, openSettingsFromOutside, requestOpenTask, runOnStartup,
  semanticSearchEnabled, setAgentRole, setComputerControlEnabled, setHeartbeatConfig,
  setHeartbeatDraftsEnabled, setHeartbeatEnabled, setMinimizeToTray, setNotifyInApp, setNotifyNative,
  setNudgesEnabled, setOperatorContextEnabled, setRunOnStartup, setSemanticSearchEnabled, setSidePaneWidth,
  setTeamworkEnabled, setVaultMcpEnabled, sidePaneWidth, sidebarCollapsed, teamworkEnabled, toggleChatContext,
  toggleChatPanel, toggleSidebar, vaultMcpEnabled, codeContextMcpEnabled, setCodeContextMcpEnabled,
} from './store/runtime';
import { beginThemeSelection, customTheme, glass, matchSystem, refetchCustomThemes, setActiveTheme, theme, themeSelection } from './store/theme';
import { bumpSettingsRevision } from './store/settingsRevision';
import {
  backupManagedElsewhere, notifFilter, NOTIF_FILTERS, paletteRecents, quotaOptIn, railCollapsed,
  setBackupManagedElsewhere, setNotifFilter, setPaletteRecents, setQuotaOptIn, setRailCollapsed, type NotifFilter,
} from './store/prefs';

/** Bumped whenever an agent changed Hollow's session list; Conversation
 *  history's list refetches on it. */
export const [appSessionsTick, setAppSessionsTick] = createSignal(0);

function apply(plan: RefreshPlan) {
  if (plan.rail) addToRail(plan.rail);
  if (plan.workspaces) void refreshWorkspaces();
  else if (plan.sessions) void refreshWorkspaceSessions();
  if (plan.sessions) setAppSessionsTick(n => n + 1);
  if (plan.themes) void refetchCustomThemes();
  if (plan.settings) bumpSettingsRevision();
}

// ── Settings the webview owns (PR A4) ─────────────────────────────────────

/** The saved preference, including system rather than its effective polarity. */
function themeValue(): string {
  const custom = customTheme();
  if (custom) return `custom:${custom.name}`;
  return matchSystem() ? 'system' : glass() ? `glass-${theme()}` : theme();
}
async function setThemeValue(v: unknown) {
  if (typeof v !== 'string') throw new Error('The theme must be a string.');
  if (v === 'system' || v === 'light' || v === 'dark') return setActiveTheme(v);
  if (v === 'glass-light' || v === 'glass-dark') return setActiveTheme({ base: v === 'glass-dark' ? 'dark' : 'light', glass: true });
  if (v.startsWith('custom:')) {
    const owner = beginThemeSelection();
    const custom = await api.readCustomTheme(v.slice('custom:'.length));
    if (owner === themeSelection()) await setActiveTheme({ custom });
    return;
  }
  throw new Error('The theme must be system, light, dark, glass-light, glass-dark or custom:<name>.');
}

type Agent = 'claude' | 'codex' | 'antigravity' | 'grok';
function role(agent: Agent) {
  return {
    get: () => agentRoles()[agent],
    set: (v: unknown) => {
      const r = v as { role?: unknown; brief?: unknown } | null;
      if (!r || typeof r.role !== 'string' || typeof r.brief !== 'string') throw new Error('The value must be {role, brief}.');
      setAgentRole(agent, { role: r.role.trim(), brief: r.brief.trim() });
    },
  };
}

/** Every UI-owned setting: its store getter and setter. */
export const UI_SETTINGS: Record<UiSettingKey, UiSetting> = {
  'agentTools.vaultSearch': { get: vaultMcpEnabled, set: v => setVaultMcpEnabled(bool(v)) },
  'agentTools.codeContext': { get: codeContextMcpEnabled, set: v => setCodeContextMcpEnabled(bool(v)) },
  'agentTools.computerControl': { get: computerControlEnabled, set: v => setComputerControlEnabled(bool(v)) },
  'agentTools.teamwork': { get: teamworkEnabled, set: v => setTeamworkEnabled(bool(v)) },
  'agentRole.claude': role('claude'),
  'agentRole.codex': role('codex'),
  'agentRole.antigravity': role('antigravity'),
  'agentRole.grok': role('grok'),
  'operator.contextEnabled': { get: operatorContextEnabled, set: v => setOperatorContextEnabled(bool(v)) },
  semanticSearchEnabled: { get: semanticSearchEnabled, set: v => setSemanticSearchEnabled(bool(v)) },
  claudeQuotaOptIn: { get: quotaOptIn, set: v => setQuotaOptIn(bool(v)) },
  'app.runOnStartup': { get: runOnStartup, set: v => setRunOnStartup(bool(v)) },
  'app.minimizeToTray': { get: minimizeToTray, set: v => setMinimizeToTray(bool(v)) },
  'heartbeat.enabled': { get: heartbeatEnabled, set: v => setHeartbeatEnabled(bool(v)) },
  'heartbeat.drafts': { get: heartbeatDraftsEnabled, set: v => setHeartbeatDraftsEnabled(bool(v)) },
  'heartbeat.config': {
    get: heartbeatConfig,
    set: v => {
      const next = { ...heartbeatConfig(), ...(v as Partial<HeartbeatConfig>) };
      const fine = (n: unknown, min: number, max: number) => Number.isInteger(n) && (n as number) >= min && (n as number) <= max;
      if (!fine(next.workHoursStart, 0, 23) || !fine(next.workHoursEnd, 1, 24) || !fine(next.minIntervalMin, 5, 1440) || !fine(next.dailyBudget, 0, 500)) {
        throw new Error('heartbeat.config: workHoursStart 0-23, workHoursEnd 1-24, minIntervalMin 5-1440, dailyBudget 0-500.');
      }
      setHeartbeatConfig(next);
    },
  },
  theme: { get: themeValue, set: setThemeValue },
  notifyInApp: { get: notifyInApp, set: v => setNotifyInApp(bool(v)) },
  notifyNative: { get: notifyNative, set: v => setNotifyNative(bool(v)) },
  nudgesEnabled: { get: nudgesEnabled, set: v => setNudgesEnabled(bool(v)) },
  sidebarCollapsed: { get: sidebarCollapsed, set: toggleTo(sidebarCollapsed, toggleSidebar) },
  chatPanelOpen: { get: chatPanelOpen, set: toggleTo(chatPanelOpen, toggleChatPanel) },
  sidePaneWidth: {
    get: sidePaneWidth,
    set: v => {
      if (v !== null && (typeof v !== 'number' || !Number.isFinite(v))) throw new Error('The width must be a number or null.');
      setSidePaneWidth(v as number | null);
      persistSidePaneWidth();
    },
  },
  chatContextEnabled: { get: chatContextEnabled, set: toggleTo(chatContextEnabled, toggleChatContext) },
  railCollapsed: { get: () => ({ ids: railCollapsed() }), set: v => setRailCollapsed(ids(v)) },
  notifFilter: {
    get: notifFilter,
    set: v => {
      if (!(NOTIF_FILTERS as readonly unknown[]).includes(v)) throw new Error(`The filter must be one of ${NOTIF_FILTERS.join(', ')}.`);
      setNotifFilter(v as NotifFilter);
    },
  },
  paletteRecents: { get: () => ({ ids: paletteRecents() }), set: v => setPaletteRecents(ids(v)) },
  backupManagedElsewhere: { get: backupManagedElsewhere, set: v => setBackupManagedElsewhere(bool(v)) },
};

// ── Navigation (PR A4) ──────────────────────────────────────────────────

/** A Hollow session to open in Conversation history; defined in its own module so
 *  the workspace store can hand one over too. */
export { appSessionRequest, setAppSessionRequest } from './store/sessionRequest';

/** Open a page, or a record on it: a task (tasks + url), a note (library +
 *  path), an automation (automations + slug), a Settings section (settings +
 *  id), a workspace (workspace + id), a conversation (session + key) or a
 *  web page in the Preview pane's web mode (browser + url). */
export async function navigate(page: string, record?: string | null): Promise<string> {
  const target = record?.trim() || '';
  switch (page) {
    case 'workspace':
      if (!target) throw new Error('navigate: workspace needs the workspace id.');
      await selectWorkspace(target);
      return `workspace ${target}`;
    case 'session': {
      if (!target) throw new Error('navigate: session needs the session key.');
      // By exact key, never through the rail's 25 newest (B·F15): only a
      // conversation in no workspace goes to Conversation history.
      await openSessionForAgent(target);
      return `session ${target}`;
    }
    case 'browser': {
      const url = normalizeAddress(target);
      if (!url || !/^https?:/i.test(url)) throw new Error('navigate: browser needs an http(s) address.');
      // The Preview pane's web mode, beside whatever page is showing (run 15
      // P1); the pane reuses this tab rather than opening the address again.
      openPreview({ kind: 'url', url });
      await openBrowserUrl(url, { newTab: true });
      return `browser ${url}`;
    }
  }
  // A go-to target, opened by the same handler as its `g` key (P0-06); it
  // answers with where it landed as the person reads it (P1-05), and the
  // record when one was opened. Running is its own page. The workspace map
  // was cut (overnight 11-08 S4).
  const shown = navigateTarget(page);
  if (target && shown === 'tasks') requestOpenTask(target);
  else if (target && shown === 'library') openVaultNote(target);
  else if (target && shown === 'automations') openWorkflowBySlug(target);
  else if (target && shown === 'settings') {
    // A target that moved to Library or Automations opens there, and the
    // answer names the tab the person sees, not Settings.
    const { target: t, opened, direct } = openSettingsFromOutside(target);
    if (direct && t.kind === 'page') {
      // Name the destination only once it opened: an unsaved-draft guard may refuse.
      if (!(await opened)) throw new Error(`navigate: ${t.page === 'vault' ? 'library' : 'automations'} can't open right now.`);
      return visiblePath(t.page === 'vault' ? 'library' : 'automations', t.tab ? t.tab[0].toUpperCase() + t.tab.slice(1) : null);
    }
    // A section too: a refused jump into Settings is not reported as landed (11-13 S6).
    if (!(await opened)) throw new Error("navigate: settings didn't open (refused or replaced by another navigation).");
  }
  else {
    if (target) throw new Error(shown === 'home' ? 'navigate: Projects takes no record; open Running as its own page.' : `navigate: ${shown} takes no record.`);
    if (!(await import('./actionRun')).runGo(shown)) throw new Error(`navigate: ${shown} can't open right now.`);
    return visiblePath(shown);
  }
  return visiblePath(shown, target);
}

// ── Requests ─────────────────────────────────────────────────────────────

/** Answer one request through the store setters. */
export function answer(req: AppUiRequest): Promise<AppUiAnswer> {
  return answerWith(UI_SETTINGS, navigate, req);
}

/** Live requests and drains of the queue, one at a time. */
const inTurn = serializer();

listen<AppUiRequest>('app-ui-request', async e => {
  const result = await inTurn(() => answer(e.payload));
  invoke('app_ui_respond', { id: e.payload.id, result }).catch(() => { /* the request gave up */ });
}).catch(() => { /* not in tauri */ });

/** Apply the settings the COO queued while the window was closed, then
 *  acknowledge them (the backend keeps each until then), in turn with live
 *  requests. */
function drainPending(): Promise<string[]> {
  return inTurn(() => applyPendingWith(
    UI_SETTINGS,
    () => invoke<PendingUiSettings>('take_pending_ui_settings'),
    versions => invoke('ack_pending_ui_settings', { versions }),
  )).catch(() => []); // not in tauri
}

/** Settings the COO queued while the window was closed, applied at boot, as
 *  the module loads (before the first settings sync back to the backend), so
 *  the store's old value doesn't overwrite them. */
export const pendingUiSettingsApplied: Promise<string[]> = drainPending();

// Closing Hollow's window only hides it, so this page usually stays loaded:
// drain when the backend says it queued a change, and whenever the window is
// shown or focused again, never only at the next launch (where a stale value
// could land over the person's newer edit).
listen('app-ui-pending', () => { void drainPending(); }).catch(() => { /* not in tauri */ });
if (typeof window !== 'undefined') window.addEventListener('focus', () => { void drainPending(); });
if (typeof document !== 'undefined') {
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') void drainPending();
  });
}

// Linear's Custom link (hollow://issue?...): the backend queued a draft; the
// handler loads on first use so none of it sits in the App chunk.
listen('linear-link-pending', () => { void import('./linearLink').then(m => m.openPendingLinearLinks()).catch(() => { /* never blocks the window */ }); }).catch(() => { /* not in tauri */ });

listen<AppSessionStarted>('app-session-started', e => apply(planForStarted(e.payload))).catch(() => { /* not in tauri */ });
listen<AppChanged>('app-changed', e => apply(planForChanged(e.payload))).catch(() => { /* not in tauri */ });
