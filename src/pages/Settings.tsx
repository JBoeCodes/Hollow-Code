import { experimentalVisuals, setExperimentalVisuals, studioMotion, setStudioMotion, studioDensity, setStudioDensity, studioTextScale, setStudioTextScale, studioTexture, setStudioTexture, TEXT_SCALE_OPTIONS } from '../store/studio';
import { panelAnimationMs, setPanelAnimationMs, PANEL_ANIMATION_MAX_MS } from '../store/panelAnimation';
import { pageEntryWhere } from '../settings/pageTargets';
import { ExtractionModelSetting } from '../settings/ExtractionModelSetting';
import { Updates } from '../settings/Updates';
import { AgentUpdates } from '../settings/AgentUpdates';
import { setBackupManagedElsewhere, enterSends, setEnterSends } from '../store/prefs';
import { ArchivedPanel, BrowserPanel, DiffsPanel, WorktreesPanel } from '../settings/ViewSections';
import { shortcutsEnabled } from '../shortcuts';
import { DirectorySetup } from '../components/DirectorySetup';
const WhereThingsWent = lazy(() => import('../settings/WentSheet'));
const ConnectorsLibrary = lazy(() => import('../settings/ConnectorsLibrary'));
const ContextPrioritizer = lazy(() => import('../settings/ContextPrioritizer'));
const ComputerControlSheet = lazy(() => import('../settings/ComputerControlSheet'));
import { lazy, Suspense, Show, createResource, createSignal, createMemo, createEffect, on, onMount, onCleanup, For } from 'solid-js';
import { Button, PageHeader, Segmented } from '../components/ui';
import { chatAnnouncements, setChatAnnouncements } from '../store/threadAnnounce';
import { plaudStatus } from '../store/connectors';
import { theme, customTheme } from '../store/theme';
import { exportSettings, importSettings } from '../store/settingsTransfer';
import { AgentToolPathPanel } from '../settings/AgentToolPath';
import { DiskSpacePanel } from '../settings/DiskSpacePanel';
import { chatPermissionMode, chatAgent, flashFooter, notifyInApp, setNotifyInApp, notifyNative, setNotifyNative, nudgesEnabled, setNudgesEnabled, runOnStartup, setRunOnStartup, minimizeToTray, setMinimizeToTray, heartbeatEnabled, setHeartbeatEnabled, heartbeatDraftsEnabled, setHeartbeatDraftsEnabled, clearAllNotifications, enableNotificationsResource, notifications, notificationTotal, trackTask, teamworkEnabled, setTeamworkEnabled, agentsAvailable, AGENT_LABELS, KNOWN_AGENTS, refreshAgentSettings, agentDescriptors, agentRoles, setAgentRole, DEFAULT_AGENT_ROLES, computerControlEnabled, setComputerControlEnabled, vaultMcpEnabled, setVaultMcpEnabled, codeContextMcpEnabled, setCodeContextMcpEnabled, tasksMcpEnabled, setTasksMcpEnabled, appMcpEnabled, setAppMcpEnabled, semanticSearchEnabled, setSemanticSearchEnabled, operatorProfile, refetchOperatorProfile, operatorContextEnabled, toggleOperatorContext, setActivePage } from '../store';
import { agentModeDanger, agentModeLabels } from '../agentPermModes';
import { api } from '../api';
import { preserveDuringRefetch } from '../stableResource';
import { Spinner } from '../components/Spinner';
import { Icon } from '../icons';
import { SETTINGS_SECTION_BY_ID, pickerControl } from '../settingsSections';
import { flashSaved } from '../settings/settingsDirty';
import { Switch } from '../components/Switch';
import '../styles/connectors-library.css';
import { refreshReviewInbox } from '../reviewInbox';
import type { Diagnostics, Workflow, SemanticStatus, ConnectedIndexStatus } from '../types';
import { contentIndexFilling, contentIndexProgress } from '../sourceCoverage';
import { type DeclineableCapabilityId } from '../capabilityDoctor';
import { LaunchIpcPanel, capabilityTone, capabilityWord } from '../settings/SystemHealth';
import { CompanionDevicesPanel } from '../settings/CompanionDevicesPanel';
import { CooDispatcherRow } from '../settings/CooDispatcherRow';
import { MainAgentSettingsRows } from '../settings/MainAgentSettings';
import { Select } from '../components/Select';
import type { ConnectorStatus, ConnectorView } from '../connectorsLibrary';
// Its own chunk (and stylesheet), loaded when the Usage section opens.
const UsagePanel = lazy(() => import('../settings/UsagePanel').then(m => ({ default: m.UsagePanel })));
const ProviderAccountsPanel = lazy(() => import('../settings/ProviderAccountsPanel').then(m => ({ default: m.ProviderAccountsPanel })));
const OnePasswordPanel = lazy(() => import('../settings/OnePasswordPanel').then(m => ({ default: m.OnePasswordPanel })));
const ClaudeAccountsPanel = lazy(() => import('../settings/ClaudeAccountsPanel').then(m => ({ default: m.ClaudeAccountsPanel })));
const SelfHostedModelsPanel = lazy(() => import('../settings/SelfHostedModelsPanel').then(m => ({ default: m.SelfHostedModelsPanel })));
const ShortcutsPanel = lazy(() => import('../settings/ShortcutsPanel').then(m => ({ default: m.ShortcutsPanel })));
const AboutYouPanel = lazy(() => import('../settings/AboutYouPanel').then(m => ({ default: m.AboutYouPanel })));
const DatabasesPanel = lazy(() => import('../settings/DatabasesPanel').then(m => ({ default: m.DatabasesPanel })));
const MoveHomePanel = lazy(() => import('../settings/MoveHomePanel').then(m => ({ default: m.MoveHomePanel })));
const ContextLayersPanel = lazy(() => import('../settings/ContextLayersPanel').then(m => ({ default: m.ContextLayersPanel })));
const SourcesPanel = lazy(() => import('../settings/SourcesPanel').then(m => ({ default: m.SourcesPanel })));
const MachinesPanel = lazy(() => import('../settings/MachinesPanel').then(m => ({ default: m.MachinesPanel })));
const PricesPanel = lazy(() => import('../settings/PricesPanel').then(m => ({ default: m.PricesPanel })));
import { SettingsSheet, openerRef } from '../settings/SettingsSheet';
import '../styles/vault-settings.css';
import { Skeleton } from '../components/ui';
import { createSettingsNavigation } from '../settings/controllers/settingsNavigation';
import { createCapabilityDoctor, needAttention } from '../settings/controllers/capabilityDoctor';
import { PageLinkRow } from '../settings/PageLinkRow';
import { errMsg } from '../utils';
import { whereThingsWentShown } from '../settings/whereThingsWent';
import { openAutomationWindows } from '../store/automationsViews';
import { ActiveSectionCtx, MoreFold, SettingsGroup, SettingsPart, SettingsSection, SettingsSubhead, KV } from '../settings/settingsLayout';
import { LearningSuggestionsSetting, ChatCanvasesSetting, AgentDefaults, AppBuildsRow, PermissionModeRow, RunLimits, RunnerSettings } from '../settings/AgentDefaults';
import { HeartbeatConfigRows, BuildIdentityRow } from '../settings/SystemRows';
import { ChatLimitsRows } from '../settings/ChatLimitsRows';
import { PlaudConnectorRow } from '../settings/ConnectorRows';
import { TaskTrackerPanel } from '../settings/TaskTrackerPanel';
import { AppearanceRows, FontRows } from '../settings/AppearanceRows';

/** Settings, Sources, Where your work lives, Run setup again (H6): every step open, then the sheet. */
async function runSetupAgain() {
  try {
    const [{ firstRunReopen }, { openFirstRun }] = await Promise.all([import('../firstRunApi'), import('../firstRunMount')]);
    openFirstRun(await firstRunReopen());
  } catch (e) { flashFooter(`× ${errMsg(e)}`, 6000); }
}

export function Settings() {
  // Every page-local resource here is wrapped. Read under the app-level
  // Suspense boundary, a resource whose promise is in flight increments that
  // boundary whether or not it already holds a value — and `runDoctorChecks`
  // refetches four of these at once, so running the checks blanked the very
  // console you were reading them in.
  const [_agentDescriptors] = createResource(() => api.agentDescriptors());
  const agentDescriptors = preserveDuringRefetch(_agentDescriptors);
  const toolAgents = createMemo(() => (agentDescriptors() ?? []).filter(agent => agent.available && agent.capabilities.ownedMcp));
  // Whether the installed Codex runs Hollow's tool-free helpers (F9): its
  // version, asked once per binary, against the versions Hollow verified.
  const [codexHelper] = createResource(() => api.codexHelperStatus().catch(() => null));
  // section itself always exists (computer control lives there too).
  const [rolesOpen, setRolesOpen] = createSignal(false);
  const rolesOpener = openerRef();
  const [computerOpen, setComputerOpen] = createSignal(false);
  const computerOpener = openerRef();
  // The redesign's old-to-new list, linked from the header until 2026-11-26.
  const [wentOpen, setWentOpen] = createSignal(false);
  const wentOpener = openerRef();
  const teamworkVisible = createMemo(() => toolAgents().length > 0 && agentsAvailable().length > 1);
  const connectorSummary = createMemo<'ok' | 'warn' | 'error'>(() => {
    if (plaudStatus() !== 'ok') return 'warn';
    return 'ok';
  });
  const linkedCount = createMemo(() => (plaudStatus() === 'ok' ? 1 : 0));

  // Which section is open, the search over sections and controls, jumps to a
  // control, and deep links (settings/controllers/settingsNavigation.ts).
  // A switch with unsaved changes asks first (P1-21: pendingSwitch).
  const { activeSection, sectionQuery, setSectionQuery, indexGroups, filteredIndexGroups, hitControls, pageHits, openPageHit, openPageTarget, jumpTo, openSection, openTopHit, pendingSwitch, keepEditing, leaveWithoutSaving } = createSettingsNavigation();
  const sectionLabel = (id: string) => SETTINGS_SECTION_BY_ID[id]?.label ?? id;
  // The diagnostics, backup and runtime reads and the capability doctor
  // (settings/controllers/capabilityDoctor.ts).
  const { setTrackerStatus, diag, refetchDiag, diagError, backupManaged, backupHealth, refetchBackupHealth, runtimeAgents, refetchRuntimeAgents, doctorPrefs, doctorProbeErrors, doctorRunning, doctorChecks, doctorSummary, saveDecline, saveVerified, probeCapability, runDoctorChecks, capabilityAction, clearing, clearLogs, savingDiag, saveDiag } = createCapabilityDoctor({ openSection, openControl: jumpTo });
  // What the connector library loaded, for the service summary (H04).
  const [libraryConnectors, setLibraryConnectors] = createSignal<ConnectorView[]>([]);
  const [libraryStatuses, setLibraryStatuses] = createSignal<Record<string, ConnectorStatus>>({});

  // Live backfill progress for the Semantic search section. Polled only while
  // the feature is enabled (cheap; the worker embeds in the background).
  const [semStatus, setSemStatus] = createSignal<SemanticStatus | null>(null);
  const refreshSemStatus = () => { api.semanticSearchStatus().then(setSemStatus).catch(() => { /* not in tauri */ }); };
  onMount(() => {
    refreshSemStatus();
    const t = setInterval(() => { if (semanticSearchEnabled()) refreshSemStatus(); }, 4000);
    onCleanup(() => clearInterval(t));
  });

  // Search inside project files (repo features R14b): the content index's
  // switch and progress, read when Advanced opens (Search & indexes, D5) and polled only while it is open and still filling in.
  const [indexStatus, setIndexStatus] = createSignal<ConnectedIndexStatus | null>(null);
  const [indexBusy, setIndexBusy] = createSignal(false);
  const refreshIndexStatus = () => { api.connectedIndexStatus().then(setIndexStatus).catch(() => { /* not in tauri */ }); };
  createEffect(on(activeSection, section => { if (section === 'advanced') refreshIndexStatus(); }));
  // General's Clear all is off for an empty bell, so the count is read here too (P0-13).
  createEffect(on(activeSection, section => { if (section === 'general') enableNotificationsResource(); }));
  onMount(() => {
    const t = setInterval(() => { if (activeSection() === 'advanced' && contentIndexFilling(indexStatus())) refreshIndexStatus(); }, 5000);
    onCleanup(() => clearInterval(t));
  });
  async function toggleContentIndex(on: boolean) {
    setIndexBusy(true);
    try { setIndexStatus(await api.setConnectedIndexEnabled(on)); flashSaved('Search inside project files'); }
    catch (e) { flashFooter(`× could not change Search inside project files: ${e instanceof Error ? e.message : String(e)}`); }
    finally { setIndexBusy(false); }
  }

  const profileFreshness = createMemo(() => operatorProfile()?.freshness ?? []);
  const profileCurrent = createMemo(() => profileFreshness().filter(item => item.status === 'fresh').length);

  const setBackupManaged = setBackupManagedElsewhere;

  const [creatingProfile, setCreatingProfile] = createSignal(false);
  async function createProfileNotes() {
    const vaultPath = diag()?.vaultPath;
    if (!vaultPath || creatingProfile()) return;
    setCreatingProfile(true);
    try {
      await trackTask('Create profile notes', () => api.createProfileNotes());
      await refetchOperatorProfile();
      flashFooter('Profile notes created. Open one to fill it in.');
    } catch (e) {
      flashFooter(`× could not create profile notes: ${e instanceof Error ? e.message : String(e)}`);
    } finally { setCreatingProfile(false); }
  }

  return (
    <>
      <PageHeader title="Settings" actions={
        <Show when={whereThingsWentShown()}>
          <button type="button" class="btn ghost small" title="Where every old setting is now" onClick={e => { wentOpener.mark(e); setWentOpen(true); }}>Where things went</button>
        </Show>
      } />
      <Show when={wentOpen()}><Suspense><WhereThingsWent returnTo={wentOpener.get} onClose={() => setWentOpen(false)} /></Suspense></Show>
      <ActiveSectionCtx.Provider value={activeSection}>
      <div class="settings-page">
        {/* Below 860 px the index shows only its search (and its hits while
            searching), above the section picker (P1-21). */}
        <nav class="settings-index" aria-label="Settings sections" data-searching={sectionQuery().trim() ? '' : undefined}>
          <label class="settings-search"><Icon name="search" size={14}/><input type="search" aria-label="Find a setting" placeholder="Find a setting…" value={sectionQuery()} onInput={e=>setSectionQuery(e.currentTarget.value)} onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); openTopHit(); } }}/></label>
          <Show when={filteredIndexGroups().length || pageHits().length} fallback={<div class="settings-search-empty"><p>No setting matches “{sectionQuery().trim()}”.</p><button class="btn small" onClick={()=>setSectionQuery('')}>Clear search</button></div>}>
          <For each={filteredIndexGroups()}>
            {(g) => (
              <div class="settings-index-group">
                {/* One list of twelve since PR 3: a cap only names a group when there are several. */}
                <Show when={indexGroups().length > 1}><div class="settings-index-cap" title={g.hint}>{g.label}</div></Show>
                <For each={g.items}>
                  {(s) => (<>
                    <button
                      class="settings-index-item"
                      data-active={activeSection() === s.id ? '' : undefined}
                      aria-current={activeSection() === s.id ? 'page' : undefined}
                      onClick={() => jumpTo(s.id)}
                    >
                      <span class="settings-index-tick" aria-hidden="true" />
                      <Icon name={s.icon} size={13} />
                      <span class="settings-index-label">{s.label}</span>
                      <Show when={s.id === 'connectors'}>
                        <span class="live-dot settings-index-dot" data-state={connectorSummary()} />
                      </Show>
                    </button>
                    <For each={hitControls(s.id)}>{c =>
                      <button class="settings-index-item settings-index-control" onClick={() => jumpTo(s.id, c.label)}>
                        {/* The arrow is decoration: as CSS content it became part
                            of the accessible name ("↳ Motion"), so a search hit
                            could not be found by its own label (H06). */}
                        <span class="settings-index-arrow" aria-hidden="true">↳</span>
                        <span class="settings-index-label">{c.label}</span>
                      </button>
                    }</For>
                  </>
                  )}
                </For>
              </div>
            )}
          </For>
          <Show when={pageHits().length}>
            <div class="settings-index-group" data-page-hits>
              <For each={pageHits()}>{e =>
                <button class="settings-index-item settings-index-control settings-index-page" data-page-hit={e.label} onClick={() => openPageHit(e)}>
                  <span class="settings-index-arrow" aria-hidden="true">↳</span>
                  <span class="settings-index-label">{e.label}</span>
                  <span class="settings-index-where">{pageEntryWhere(e)}</span>
                </button>
              }</For>
            </div>
          </Show>
        </Show>
        </nav>
        <label class="settings-mobile-index">
          <span>Section</span>
          <Select label="Settings section" value={activeSection()} onChange={(id, query) => jumpTo(id, pickerControl(id, query))}
            options={indexGroups().flatMap(group => group.items.map(section => ({ value: section.id, label: section.label, group: indexGroups().length > 1 ? group.label : undefined, keywords: `${section.keywords} ${(section.controls ?? []).map(c => `${c.label} ${c.keywords ?? ''}`).join(' ')}` })))} />
        </label>
        <div class="settings-body">
        {/* A switch would unmount forms holding unsaved changes: ask first
            (P1-21). Keep editing is focused, so Enter or Space keeps them. */}
        <Show when={pendingSwitch()}>{p => (
          <div class="settings-banner settings-unsaved" role="alertdialog" aria-label="Unsaved changes" aria-describedby="settings-unsaved-text"
            onKeyDown={e => { if (e.key === 'Escape') { e.stopPropagation(); keepEditing(); } }}>
            <strong>Unsaved changes</strong>
            <p id="settings-unsaved-text">{p().unsaved.join(', ')} in {sectionLabel(activeSection())} {p().unsaved.length === 1 ? 'is' : 'are'} not saved. Opening {sectionLabel(p().to)} drops {p().unsaved.length === 1 ? 'it' : 'them'}.</p>
            <div class="settings-actions">
              <button class="btn primary" ref={el => queueMicrotask(() => el.focus())} onClick={keepEditing}>Keep editing</button>
              <button class="btn" onClick={leaveWithoutSaving}>Discard changes and open {sectionLabel(p().to)}</button>
            </div>
          </div>
        )}</Show>
        <Show when={diagError()}>
          <div class="settings-banner danger">
            <strong>Diagnostics unavailable.</strong>
            <p>
              Hollow couldn’t read its own diagnostics:{' '}
              <code>{diagError()}</code>. The folder, program and log
              sections below need them. Try again; if it keeps happening,
              restart Hollow.
            </p>
            <button class="btn" onClick={() => refetchDiag()}>Retry</button>
          </div>
        </Show>

        {/* ── General: appearance, notifications and startup (Settings PR 3) ── */}
        <SettingsSection
          id="general"
          icon="moon"
          title="General"
          status={<span class="settings-state-chip">{customTheme()?.name ?? (theme() === 'dark' ? 'Obsidian' : 'Alabaster')}</span>}
        >
          <SettingsSubhead
            title="Appearance"
            hint="Match your Mac, choose a theme or make your own."
          />
          <AppearanceRows />
          <div class="settings-row studio-motion-setting"><div class="settings-row-label"><div class="settings-row-k">Motion</div><div class="settings-row-v">Transitions and animations. Your system’s Reduce Motion setting always wins.</div></div><Segmented label="Motion preference" value={studioMotion()} onChange={v => { setStudioMotion(v); flashSaved('Motion'); }} items={[{ value: 'full', label: 'Full' }, { value: 'reduced', label: 'Reduced' }] as const}/></div>
          <div class="settings-row"><div class="settings-row-label"><div class="settings-row-k">Texture</div><div class="settings-row-v">Full artwork, quieter identity details, or plain drawings. Text and controls stay crisp.</div></div><Segmented label="Texture" value={studioTexture()} onChange={v=>{setStudioTexture(v);flashSaved('Texture');}} items={[{value:'full',label:'Full'},{value:'quiet',label:'Quiet'},{value:'off',label:'Off'}] as const}/></div>
          <div class="settings-row panel-animation-setting"><div class="settings-row-label"><div class="settings-row-k">Panel animation</div><div class="settings-row-v">How long panels take to slide. 0 is off. Reduced motion and page changes skip it.</div></div><div class="settings-range"><input type="range" aria-label="Panel animation" min="0" max={PANEL_ANIMATION_MAX_MS} step="10" value={panelAnimationMs()} onInput={e => setPanelAnimationMs(e.currentTarget.value)} onChange={() => flashSaved('Panel animation')}/><output class="settings-range-value" aria-live="off">{panelAnimationMs() === 0 ? 'Off' : `${panelAnimationMs()} ms`}</output><button type="button" class="btn small ghost" disabled={panelAnimationMs() === 0} onClick={() => { setPanelAnimationMs(0); flashSaved('Panel animation'); }}>Turn off</button></div></div>
          <div class="settings-row"><div class="settings-row-label"><div class="settings-row-k">Ambient motion</div><div class="settings-row-v">Extra movement in live indicators, independent of texture.</div></div><Switch checked={experimentalVisuals()} label="Ambient motion" onChange={on=>{setExperimentalVisuals(on);flashSaved('Ambient motion');}}/></div>
          <div class="settings-row studio-density-setting"><div class="settings-row-label"><div class="settings-row-k">Density</div><div class="settings-row-v">How tightly lists and controls are spaced. Reading text keeps its line length.</div></div><Segmented label="Density" value={studioDensity()} onChange={v => { setStudioDensity(v); flashSaved('Density'); }} items={[{ value: 'comfortable', label: 'Comfortable' }, { value: 'compact', label: 'Compact' }] as const}/></div>
          <div class="settings-row studio-text-scale-setting"><div class="settings-row-label"><div class="settings-row-k">Text size</div><div class="settings-row-v">Scales every label, list and document together.</div></div><Segmented label="Text size" value={studioTextScale()} onChange={v => { setStudioTextScale(v); flashSaved('Text size'); }} items={TEXT_SCALE_OPTIONS}/></div>
          <FontRows />

          <SettingsSubhead
            title="Notifications"
            hint="Failures and time-critical notices pop up; the rest waits in the bell."
          />
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">In-app pop-ups</div>
              <div class="settings-row-v">
                Short cards for failures and time-critical notices; they fade after 9 seconds.
              </div>
            </div>
            <Switch checked={notifyInApp()} label="Show in-app pop-ups" onChange={() => { setNotifyInApp(!notifyInApp()); flashSaved('In-app pop-ups'); }} />
          </div>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">macOS system notifications</div>
              <div class="settings-row-v">
                Also show them in Notification Center. macOS asks the first time.
              </div>
            </div>
            <Switch checked={notifyNative()} label="Show macOS system notifications" onChange={() => { setNotifyNative(!notifyNative()); flashSaved('macOS system notifications'); }} />
          </div>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Chat announcements</div>
              <div class="settings-row-v">
                A banner and a dock badge when a chat finishes or needs you. Needs macOS notifications.
              </div>
            </div>
            <Switch checked={chatAnnouncements()} label="Announce finished chats" onChange={() => { setChatAnnouncements(!chatAnnouncements()); flashSaved('Chat announcements'); }} />
          </div>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Proactive nudges</div>
              <div class="settings-row-v">
                A scan every 5 minutes, with no model use: meetings, overdue tasks, stalled projects.
              </div>
            </div>
            <div class="settings-actions">
              <button
                class="btn"
                onClick={() =>
                  trackTask('Nudge scan', async () => {
                    const n = await api.runNudgeScan();
                    flashFooter(n > 0 ? `${n} nudge${n === 1 ? '' : 's'} fired` : 'Nothing to nudge');
                  })
                }
                title="Run one nudge scan right now"
              >Scan now</button>
              <Switch checked={nudgesEnabled()} label="Proactive nudges" onChange={() => { setNudgesEnabled(!nudgesEnabled()); flashSaved('Proactive nudges'); }} />
            </div>
          </div>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Clear history</div>
              <div class="settings-row-v">
                Delete every notification in the bell. The next event starts a new history.
              </div>
            </div>
            {/* It asks first; an empty bell has nothing to clear (P0-13). */}
            <button class="btn" disabled={notifications.state === 'ready' && notificationTotal() === 0} onClick={() => void clearAllNotifications()}>Clear all</button>
          </div>

          <SettingsSubhead title="Startup" />
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Run at login</div>
              <div class="settings-row-v">
                Start Hollow when you log in, hidden in the menu bar.
              </div>
            </div>
            <Switch checked={runOnStartup()} label="Run Hollow at login" onChange={() => setRunOnStartup(!runOnStartup())} />
          </div>
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Keep running when I close the window</div>
              <div class="settings-row-v">
                Closing the window hides Hollow so routines keep running. Off quits on close.
              </div>
            </div>
            <Switch checked={minimizeToTray()} label="Keep Hollow running when the window closes" onChange={() => setMinimizeToTray(!minimizeToTray())} />
          </div>
        </SettingsSection>

        {/* ── Chat (Canvas C3 wave 2 C4; a section again in PR 3) ───────── */}
        <SettingsSection
          id="chat"
          icon="message"
          title="Chat"
          status={<span class="settings-state-chip" data-state={agentModeDanger(chatAgent())[chatPermissionMode()] ? 'error' : 'ok'}>{agentModeLabels(chatAgent())[chatPermissionMode()]}</span>}
        >
          <ChatCanvasesSetting />
          <div class="settings-row"><div class="settings-row-label"><div class="settings-row-k">Enter sends a message</div><div class="settings-row-v">Shift+Enter starts a new line. When off, Enter starts a new line and ⌘Enter sends.</div></div><Switch checked={enterSends()} label="Enter sends a message" onChange={() => { setEnterSends(!enterSends()); flashSaved('Enter sends a message'); }} /></div>
          <ExtractionModelSetting part="retitle" />
          <PermissionModeRow />
          <SettingsSubhead
            title="Chats at once"
            hint="How many agent turns run side by side; the rest wait in line."
          />
          <ChatLimitsRows />
        </SettingsSection>

        {/* ── Models: agents, models and effort, then Self-hosted models (PR 3; S5b moved the runner, run limits and App builds out) ── */}
        <SettingsSection
          id="models"
          icon="settings"
          title="Models"
        >
          <AgentDefaults installed={runtimeAgents() ?? []} />
          {/* ── Self-hosted models (self-hosted models Q4): your own servers ── */}
          <SettingsPart
            id="selfhosted"
            lazy
            title="Self-hosted models"
            hint="Your own GPU or server. Each one switched on is in a chat's model menu."
          >
            <Suspense fallback={<Skeleton label="Loading self-hosted models" rows={2} />}><SelfHostedModelsPanel part="list" /></Suspense>
          </SettingsPart>
        </SettingsSection>

        {/* ── Accounts: every provider's sign-in, then agent updates (PR 3) ── */}
        <SettingsSection
          id="accounts"
          icon="users"
          title="Accounts"
        >
          <SettingsSubhead title="Claude" hint="Add Claude plans; usage can spread across them." />
          <Suspense fallback={<Skeleton label="Loading accounts" />}><ClaudeAccountsPanel /></Suspense>
          <SettingsSubhead title="Other providers" hint="Each agent’s own sign-in. Hollow stores none of these credentials." />
          <Suspense fallback={<p>Checking sign-ins…</p>}><ProviderAccountsPanel /></Suspense>
          <SettingsSubhead title="1Password" hint="Optional: read secrets from 1Password instead of the Keychain. Off by default." />
          <Suspense fallback={<p>Checking 1Password…</p>}><OnePasswordPanel /></Suspense>
          <SettingsSubhead title="Agent updates" hint="Each agent’s CLI version, install method and update policy." />
          <AgentUpdates />
        </SettingsSection>

        {/* ── Personalization: About you and memory (was Knowledge) ── */}
        <SettingsSection
          id="personalization"
          icon="book"
          title="Personalization"
          status={<span class="settings-state-chip" data-state={!operatorContextEnabled() ? undefined : profileCurrent() === profileFreshness().length && profileFreshness().length ? 'ok' : 'warn'}>{!operatorContextEnabled() ? 'off' : `${profileCurrent()}/${profileFreshness().length} current`}</span>}
        >
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Share with agents</div>
              <div class="settings-row-v">
                <Show
                  when={(operatorProfile()?.files?.length ?? 0) > 0}
                  fallback={<>No profile notes yet. Create them, then fill in what matters.</>}
                >
                  Added to every chat and run. From {operatorProfile()!.files.join(', ')}{' '}
                  ({operatorProfile()!.charCount.toLocaleString()} chars{operatorProfile()!.truncated ? ', capped' : ''}).
                </Show>
              </div>
            </div>
            <Switch checked={operatorContextEnabled()} label="Share About you with agents" onChange={() => toggleOperatorContext()} />
          </div>
          <Show when={profileFreshness().some(item => !item.exists)}>
            <div class="settings-actions">
              <button class="btn" disabled={creatingProfile() || !diag()?.vaultPath} onClick={createProfileNotes}>
                <Icon name="plus" /> {creatingProfile() ? 'Creating…' : (operatorProfile()?.files?.length ?? 0) > 0 ? 'Create the missing notes' : 'Create profile notes'}
              </button>
            </div>
          </Show>
          <Suspense fallback={<Skeleton label="Loading your profile" rows={4} />}><AboutYouPanel /></Suspense>
          {/* Memory: approved learnings, recalled into every run (P1-22: was its own section). */}
          <SettingsSubhead title="Memory" hint="What Hollow learned from your work and you approved." />
          <p class="settings-note">Nothing becomes a memory until you approve it.</p>
          <LearningSuggestionsSetting />
        </SettingsSection>

        {/* ── Connectors: connectors, Hollow's tools, recordings, then Skills ── */}
        <SettingsSection
          id="connectors"
          icon="globe"
          title="Connectors"
        >
          <Suspense fallback={<Skeleton label="Loading connectors" rows={4} />}><ConnectorsLibrary onState={(c, st) => { setLibraryConnectors(c); setLibraryStatuses(st); }} /></Suspense>
          <SettingsSubhead title="Built into Hollow" hint={toolAgents().length ? 'Hollow’s own tools for your agents. Nothing to sign in to.' : 'No installed agent supports Hollow’s tools yet.'} />
          <div class="cx-tools">
            <div class="cx-tool">
              <span class="cx-icon cx-icon-md" aria-hidden="true"><Icon name="search" size={16} /></span>
              <div class="cx-row-text"><strong>Search your vault and projects</strong><small>Agents find your notes and project files by keyword. Read-only. Chats only.</small></div>
              <Switch checked={vaultMcpEnabled()} label="Search your vault and projects" onChange={setVaultMcpEnabled} />
            </div>
            <div class="cx-tool">
              <span class="cx-icon cx-icon-md" aria-hidden="true"><Icon name="terminal" size={16} /></span>
              <div class="cx-row-text"><strong>Look up code: callers and impact</strong><small>Who calls a function and what a change reaches. Read-only; Claude and Codex.</small></div>
              <Switch checked={codeContextMcpEnabled()} label="Look up code: callers and impact" onChange={setCodeContextMcpEnabled} />
            </div>
            <div class="cx-tool">
              <span class="cx-icon cx-icon-md" aria-hidden="true"><Icon name="checkSquare" size={16} /></span>
              <div class="cx-row-text"><strong>Tasks</strong><small>List, create, update and complete tasks, or suggest new ones.</small></div>
              <Switch checked={tasksMcpEnabled()} label="Task tools" onChange={setTasksMcpEnabled} />
            </div>
            <div class="cx-tool">
              <span class="cx-icon cx-icon-md" aria-hidden="true"><Icon name="appWindow" size={16} /></span>
              <div class="cx-row-text"><strong>Hollow app</strong><small>Agents can see and change Hollow, builds and installs included.</small>
                {/* Risky: it reaches deletes and accounts, so the detail stays one click away. */}
                <MoreFold><small>Deleting, accounts and anything that leaves this Mac ask you first, except in Bypass, where only a short always-ask list still asks.</small></MoreFold></div>
              <Switch checked={appMcpEnabled()} label="Hollow app tools" onChange={setAppMcpEnabled} />
            </div>
            {/* `data-settings-control`: the search's "Computer control" (CU7) lands on this row; its name is otherwise only in a <strong> and an aria-label. */}
            <div class="cx-tool" data-settings-control="Computer control">
              <span class="cx-icon cx-icon-md" aria-hidden="true"><Icon name="globe" size={16} /></span>
              <div class="cx-row-text"><strong>Computer control</strong><small>Open links and apps. Tab reads and quits ask first. Chats only.</small></div>
              <button class="btn small" aria-label="Set up computer control" onClick={e => { computerOpener.mark(e); setComputerOpen(true); }}>Set up…</button>
              <Switch checked={computerControlEnabled()} label="Computer control" onChange={setComputerControlEnabled} />
            </div>
            <Show when={computerOpen()}>
              <Suspense fallback={null}><ComputerControlSheet returnTo={computerOpener.get} onClose={() => setComputerOpen(false)} /></Suspense>
            </Show>
            <Show when={teamworkVisible()}>
              <div class="cx-tool">
                <span class="cx-icon cx-icon-md" aria-hidden="true"><Icon name="users" size={16} /></span>
                <div class="cx-row-text"><strong>Teammates</strong><small>The active agent can hand subtasks to your other agents.</small></div>
                {/* Settings PR 5: the roles are a sheet; they save as you leave each field.
                    Browser fix b17: its button shares the Teammates row (it had a row of
                    its own) so CC2's code lookup row fits Connectors' 1.3 screens. */}
                <button class="btn small" aria-label="Edit teammate roles" onClick={e => { rolesOpener.mark(e); setRolesOpen(true); }}>Edit roles…</button>
                <Switch checked={teamworkEnabled()} label="Teammate delegation" onChange={setTeamworkEnabled} />
              </div>
              <Show when={rolesOpen()}>
                <SettingsSheet id="settings-sheet-roles" title="Teammate roles" returnTo={rolesOpener.get} onClose={() => setRolesOpen(false)}>
                {/* Role presets, one per installed teammate: the role names the
                    teammate in the delegation tool; the brief rides ahead of
                    every delegated task. */}
                <For each={KNOWN_AGENTS.filter(a => agentsAvailable().includes(a))}>
                  {agent => (
                    <div class="settings-role-fields">
                      <input
                        type="text"
                        class="settings-input"
                        aria-label={`${AGENT_LABELS[agent]} role`}
                        placeholder={`${AGENT_LABELS[agent]}: ${DEFAULT_AGENT_ROLES[agent].role}`}
                        value={agentRoles()[agent].role}
                        onChange={e => setAgentRole(agent, { ...agentRoles()[agent], role: e.currentTarget.value.trim() })}
                        title="Role label, e.g. Engineer"
                      />
                      <textarea
                        class="settings-input settings-role-brief"
                        rows={2}
                        aria-label={`${AGENT_LABELS[agent]} standing instructions`}
                        placeholder="Standing instructions (optional), e.g. Prefer small diffs; run tests before reporting done."
                        value={agentRoles()[agent].brief}
                        onChange={e => setAgentRole(agent, { ...agentRoles()[agent], brief: e.currentTarget.value.trim() })}
                      />
                    </div>
                  )}
                </For>
                </SettingsSheet>
              </Show>
            </Show>
          </div>

          <SettingsSubhead title="Recordings" hint="Plaud recordings and your calendar, separate from agent access." />
          <PlaudConnectorRow />

          {/* Skills moved to Library, Skills (PR 4); this row is the way back. */}
          <PageLinkRow title="Skills are in Library" text="The skills catalog, editor and import are in Library, Skills." button="Open Library, Skills" onOpen={() => openPageTarget({ page: 'vault', tab: 'skills' })} />
        </SettingsSection>

        {/* ── Sources: tasks and projects, then the Hollow folder (H4, A14) ── */}
        <SettingsSection
          id="sources"
          icon="folder"
          title="Sources"
        >
          {/* H4b: the three role rows (tasks, knowledge, personal context), workspace overrides and switches; its own lazy chunk. */}
          {/* Browser fix 2 (11-19): the role rows and Run setup again (H6, whose setup
              walks these rows) sit in one closed group, so Sources keeps its 1.3
              screens and the word budget; a search jump opens it. */}
          <SettingsGroup id="where-work-lives" title="Where your work lives" hint="Tasks and projects, knowledge and your personal context.">
            <Suspense fallback={<Skeleton label="Loading sources" rows={3} />}><SourcesPanel /></Suspense>
            {/* H6: the first-run setup sheet again, from its first step; its own lazy chunk. */}
            <PageLinkRow title="Run setup again" text="The setup steps from the first one. Nothing you set is undone." button="Start setup" onOpen={() => void runSetupAgain()} />
          </SettingsGroup>
          <TaskTrackerPanel onStatus={setTrackerStatus} />

          <SettingsPart
            id="vault"
            title="Hollow folder"
            hint="Projects in its Projects folder, workspaces and notes in its Vault folder."
            status={<span class="settings-state-chip" data-state={diag()?.vaultExists ? 'ok' : diag() ? 'error' : undefined}>{diag()?.vaultExists ? 'vault available' : diag() ? 'folder unavailable' : 'checking…'}</span>}
          >
            <Show when={diag()} fallback={<p class="settings-loading">{diagError() ? 'Vault details unavailable: retry diagnostics above.' : 'Reading your vault…'}</p>}>
              <KV k="Current vault" v={diag()!.vaultPath} warn={!diag()!.vaultExists ? 'This folder is unavailable. Choose an accessible vault below.' : undefined} mono
                action={<Button size="sm" disabled={!diag()?.vaultExists} onClick={() => setActivePage('vault')}>Open in Library</Button>} />
              <DirectorySetup />
            </Show>
            {/* Move Hollow home (2026-11-12): its own lazy chunk. */}
            {/* A closed group: its heading and one line show, the rest opens on a click or a jump (FIX-4, 11-13). */}
            <SettingsGroup id="move-home" title="Move Hollow home" hint="One Hollow folder for the vault and Projects.">
              <Suspense fallback={<Skeleton label="Loading Move Hollow home" />}><MoveHomePanel /></Suspense>
            </SettingsGroup>
          </SettingsPart>
          {/* Context (CX17): one row per context layer, a closed group so Sources keeps its height. Part id `layers` (`context` is an old Advanced alias). */}
          <SettingsPart id="layers" group lazy title="Context" hint="Home, personal, org, project and vault context, each in its own place.">
            <Suspense fallback={<Skeleton label="Loading context layers" rows={3} />}><ContextLayersPanel /></Suspense>
          </SettingsPart>
        </SettingsSection>

        {/* ── Background work: automations, the heartbeat and the COO ── */}
        <SettingsSection
          id="background"
          icon="refresh"
          title="Background work"
          status={<span class="settings-state-chip" data-state={heartbeatEnabled() ? 'ok' : 'off'}>{heartbeatEnabled() ? 'heartbeat on' : 'heartbeat off'}</span>}
        >
          <SettingsSubhead
            title="Scheduled automations"
            hint="Daily briefs and reviews are automations; schedule and pause them there."
          />
          <div class="settings-actions"><button class="btn" onClick={() => setActivePage('workflows')}>Open Automations</button></div>

          <PageLinkRow title="Automations, windows and the run queue are on the Automations page" text="Edit the automation windows and watch the run queue beside the schedule dial." button="Open windows and run queue" onOpen={() => void openAutomationWindows('windows')} />

          <SettingsSubhead
            title="Silent heartbeat"
            hint="A quiet pass over what changed. It runs on your default agent and sends nothing."
          />
          <div class="settings-row">
            <div class="settings-row-label">
              <div class="settings-row-k">Silent heartbeat</div>
              <div class="settings-row-v">
                Wakes only when something changed, within work hours and a daily cap.
              </div>
            </div>
            <div class="settings-actions">
              <button
                class="btn"
                onClick={async () => {
                  try {
                    const n = await api.runHeartbeatNow();
                    flashFooter(n > 0 ? `Heartbeat surfaced ${n}` : 'Heartbeat: nothing to surface', 2400);
                    refreshReviewInbox();
                  } catch (e: any) { flashFooter(`× ${e?.message || e}`, 3500); }
                }}
              >Run now</button>
              <Switch checked={heartbeatEnabled()} label="Silent heartbeat" onChange={() => setHeartbeatEnabled(!heartbeatEnabled())} />
            </div>
          </div>
          <Show when={heartbeatEnabled()}>
            <div class="settings-row">
              <div class="settings-row-label">
                <div class="settings-row-k">Prepare meeting notes</div>
                <div class="settings-row-v">
                  On: may write meeting prep and daily-note updates. Off: observations only.
                </div>
              </div>
              <Switch checked={heartbeatDraftsEnabled()} label="Let the heartbeat prepare meeting notes" onChange={() => setHeartbeatDraftsEnabled(!heartbeatDraftsEnabled())} />
            </div>
            <HeartbeatConfigRows />
          </Show>
          <p class="settings-note">Tasks the heartbeat suggests wait for you in Needs you. Approved preferences live in Library → Learnings.</p>
          <SettingsSubhead title="COO" hint="The always-on agent behind the orb. Orders: workspaces/main/STANDING_ORDERS.md." />
          <MainAgentSettingsRows />
        </SettingsSection>

        {/* ── Devices: the runner and the devices paired with this Mac ── */}
        <SettingsSection
          id="devices"
          icon="appWindow"
          title="Devices"
        >
          <CompanionDevicesPanel />
          {/* M2 PR 4: the other Macs this one opens chats on. */}
          <Suspense fallback={<Skeleton label="Loading Macs" rows={2} />}><MachinesPanel /></Suspense>
          {/* M3a PR2: COO on this Mac, beside the Macs it routes to: the dispatcher when a paired Mac runs a runner COO (default) or always the full COO. */}
          <CooDispatcherRow />
          {/* The Mac mini runner (S5b, plan 3.2 row 9): Remote runner and what runs there, saved with Save runner settings. */}
          <RunnerSettings />
        </SettingsSection>

        {/* ── Usage (Paperclip PR 6) ─────────────────────────────────── */}
        <SettingsSection
          id="usage"
          icon="graph"
          title="Usage"
        >
          <Suspense fallback={<Skeleton label="Loading usage" rows={4} />}><UsagePanel /></Suspense>
        </SettingsSection>

        {/* ── Keyboard shortcuts (a section again in PR 3) ─────────────── */}
        <SettingsSection
          id="shortcuts"
          icon="keyboard"
          title="Keyboard shortcuts"
          status={<span class="settings-state-chip">{shortcutsEnabled() ? 'On' : 'Off'}</span>}
        >
          <Suspense fallback={<Skeleton label="Loading shortcuts" rows={6} />}><ShortcutsPanel /></Suspense>
        </SettingsSection>

        {/* ── t3code parity 4.7 (A17): Diffs, Browser, Worktrees, Archived ── */}
        <SettingsSection id="diffs" icon="gitBranch" title="Diffs"><DiffsPanel /></SettingsSection>
        <SettingsSection id="browser" icon="globe" title="Browser"><BrowserPanel /></SettingsSection>
        <SettingsSection id="worktrees" icon="folder" title="Worktrees"><WorktreesPanel /></SettingsSection>
        <SettingsSection id="archived" icon="archive" title="Archived"><ArchivedPanel /></SettingsSection>

        {/* ── Advanced: closed groups, one line each (D5, plan 3.3) ──── */}
        <SettingsSection id="updates" icon="refresh" title="Updates">
          <Updates />
        </SettingsSection>

        <SettingsSection id="about" icon="book" title="About">
          <PageLinkRow title="Welcome to Hollow" text="Agents, imports and optional setup, together on one page." button="Open welcome page" onOpen={() => void runSetupAgain()} />
        </SettingsSection>

        <SettingsSection
          id="advanced"
          icon="terminal"
          title="Advanced"
          // Only while debug logs are on: an empty chip drew a stray dot beside the title.
          status={diag()?.debugLogEnabled ? <span class="settings-state-chip" data-state="warn">debug on</span> : undefined}
        >
          <SettingsGroup id="this-mac" title="This Mac" hint="Health check, backup, runtime and disk space.">
            <SettingsSubhead
              title="Capability doctor"
              hint="What Hollow can use right now, with a repair for what is broken."
              status={<span class="settings-state-chip" data-state={capabilityTone(doctorSummary().state)}>{capabilityWord(doctorSummary().state)}</span>}
            />
            <div class="capability-doctor-summary">
              <div>
                <div class="settings-row-k">{doctorSummary().live} working · {needAttention(doctorSummary().needsAttention)}</div>
                <div class="settings-row-v">
                  A check stays current for seven days. Declined ones stay quiet.
                </div>
              </div>
              <button class="btn primary" disabled={doctorRunning()} onClick={runDoctorChecks}>
                {doctorRunning() ? 'checking…' : 'Run all checks'}
              </button>
            </div>
            <div class="capability-doctor-list">
              <For each={doctorChecks()}>
                {(item) => (
                  <div class="capability-doctor-row" data-state={item.state}>
                    <span class="capability-doctor-dot" aria-hidden="true" />
                    <div class="capability-doctor-copy">
                      <div class="settings-row-k">{item.label}</div>
                      <div class="settings-row-v">{item.detail}</div>
                    </div>
                    <span class="settings-state-chip" data-state={capabilityTone(item.state)}>{capabilityWord(item.state)}</span>
                    <div class="capability-doctor-actions">
                      <button class="btn" onClick={() => capabilityAction(item)}>
                        {item.state === 'declined' && item.declinable ? 'Use this' : item.fixLabel}
                      </button>
                      <Show when={item.declinable && item.state !== 'declined'}>
                        <button class="btn ghost" onClick={() => saveDecline(item.id as DeclineableCapabilityId, true)}>Not using</button>
                      </Show>
                    </div>
                  </div>
                )}
              </For>
            </div>
            <Show when={backupHealth()}>
              {(health) => (
                <div class="backup-health-box" data-status={health().status}>
                  <div class="backup-health-head">
                    <div>
                      <div class="settings-row-k">Backup evidence</div>
                      <div class="settings-row-v">{health().summary}</div>
                    </div>
                    <span class="settings-state-chip" data-state={health().status === 'protected' ? 'ok' : health().status === 'unavailable' ? 'error' : 'warn'}>{health().status}</span>
                  </div>
                  <div class="backup-signal-list">
                    <For each={health().signals}>{signal => (
                      <div class="backup-signal" data-state={signal.state}>
                        <span class="capability-doctor-dot" />
                        <div><b>{signal.label}</b><span>{signal.detail}</span></div>
                      </div>
                    )}</For>
                  </div>
                  <label class="backup-managed-toggle" data-settings-control="Backup management">
                    <input type="checkbox" checked={backupManaged()} onChange={event => setBackupManaged(event.currentTarget.checked)} />
                    <span>I manage this vault with another backup system on this Mac.</span>
                  </label>
                </div>
              )}
            </Show>
            <SettingsSubhead title="Runtime" hint="The configuration and agent runtimes in use on this Mac." />
            <Show
              when={diag()}
              fallback={
                <p class="settings-loading">
                  <Show when={!diagError()}><Spinner  /> </Show>
                  {diagError() ? 'Diagnostics unavailable: see the banner above.' : 'loading diagnostics…'}
                </p>
              }
            >
              <KV k="Config file" v={diag()!.configPath} mono />
              <For each={KNOWN_AGENTS}>{agent => (
                <KV
                  k={`${AGENT_LABELS[agent]} runtime`}
                  v={(runtimeAgents() ?? []).includes(agent) ? 'Available · sign-in is checked when running' : 'Not installed'}
                />
              )}</For>
              <Show when={runtimeAgents.error}><p class="settings-row-warn">Runtime detection failed: {String(runtimeAgents.error)}</p></Show>
              <Show when={!runtimeAgents.loading && !runtimeAgents.error && (runtimeAgents() ?? []).length === 0}>
                <p class="settings-row-warn">Install and sign in to a supported agent to enable assistant features.</p>
              </Show>
              {/* Also Models' old Refresh installed agents (S5b): it reads the agent settings again too. */}
              <div class="settings-actions"><button class="btn" onClick={() => { void refetchRuntimeAgents(); void refreshAgentSettings(); }}>Refresh agent detection</button></div>
            </Show>
            <Show when={codexHelper()}>{status => (
              <KV k="Codex helpers" v={status().message} />
            )}</Show>
            <SettingsSubhead title="Disk space" hint="Warn me when free space drops under a threshold." />
            <DiskSpacePanel />
            {/* History & recovery moved to Library, History (PR 4). */}
            <PageLinkRow title="History & recovery is in Library" text="Restore one file or a whole run from the runs Hollow snapshotted, in Library, History." button="Open Library, History" onOpen={() => openPageTarget({ page: 'vault', tab: 'history' })} />
          </SettingsGroup>

          <SettingsPart
            id="indexes"
            group
            title="Search & indexes"
            hint="What Hollow indexes on this Mac so you and your agents find things."
            status={<span class="settings-state-chip" data-state={semanticSearchEnabled() ? 'ok' : undefined}>{semanticSearchEnabled() ? 'meaning on' : 'meaning off'}</span>}
          >
            <div class="settings-row">
              <div class="settings-row-label">
                <div class="settings-row-k">Search inside project files (local index)</div>
                <div class="settings-row-v">
                  A word index of your project files, on this Mac. Off by default; off deletes it.
                  {' '}<span class="settings-content-index" role="status">{contentIndexProgress(indexStatus())}</span>
                </div>
              </div>
              <Switch checked={indexStatus()?.enabled ?? false} label="Search inside project files (local index)" disabled={indexBusy()} onChange={on => void toggleContentIndex(on)} />
            </div>
            <SettingsSubhead title="Search by meaning" hint="Finds notes by what they mean. Powers Library’s Search by meaning." />
            <div class="settings-row">
              <div class="settings-row-label">
                <div class="settings-row-k">Index my vault for semantic search</div>
                <div class="settings-row-v">
                  Embeds your notes on this Mac; <strong>nothing leaves it</strong>. Off by default.
                </div>
              </div>
              <Switch checked={semanticSearchEnabled()} label="Index my vault for semantic search" onChange={() => { setSemanticSearchEnabled(!semanticSearchEnabled()); refreshSemStatus(); }} />
            </div>
            <Show when={semanticSearchEnabled()}>
              <div class="settings-row">
                <div class="settings-row-label">
                  <div class="settings-row-k">Indexing progress</div>
                  <div class="settings-row-v">
                    <Show when={semStatus()} fallback="Starting…">
                      {(s) => (
                        <>
                          {s().indexed.toLocaleString()} / {s().total.toLocaleString()} chunks embedded
                          <Show when={s().total > 0 && s().indexed < s().total}>. Indexing…</Show>
                          <Show when={s().total > 0 && s().indexed >= s().total}>. Up to date</Show>
                        </>
                      )}
                    </Show>
                  </div>
                </div>
              </div>
            </Show>
            <SettingsSubhead title="Context prioritizer" hint="Keeps each request lean so answers come faster and cost less." />
            <Suspense fallback={<Skeleton label="Loading the context prioritizer" rows={4} />}><ContextPrioritizer /></Suspense>
          </SettingsPart>

          <SettingsGroup id="model-calls" title="Hollow’s own model calls" hint="The extraction model, and the model each of Hollow’s background jobs uses.">
            <ExtractionModelSetting part="model" />
            {/* Background jobs (S5b, plan 3.3): one home with the extraction model. */}
            <Suspense fallback={<Skeleton label="Loading background jobs" rows={2} />}><SelfHostedModelsPanel part="jobs" /></Suspense>
          </SettingsGroup>

          {/* S5b (plan 3.3): each agent's watchdog and deadline, saved with Save run limits. */}
          <SettingsGroup id="run-limits" title="Run limits" hint="When a silent or long run is stopped, per agent.">
            <RunLimits />
          </SettingsGroup>

          {/* S5b (plan 3.3): App builds. The web app backend is still in Devices; this group links to it. */}
          <SettingsGroup id="app-builds" title="App builds and web backend" hint="Where Build app compiles Hollow, and the web app’s backend.">
            <AppBuildsRow />
            <div class="settings-row">
              <div class="settings-row-label">
                <div class="settings-row-k">Backend for the web app is in Devices</div>
                <div class="settings-row-v">Its address, applier token and what it copies.</div>
              </div>
              <Button size="sm" variant="ghost" onClick={() => jumpTo('devices', 'Backend for the web app')}>Open Devices</Button>
            </div>
          </SettingsGroup>

          <Suspense fallback={<Skeleton label="Loading prices" rows={3} />}><PricesPanel /></Suspense>

          <SettingsGroup id="settings-file" title="Settings file" hint="Export your settings to a JSON file, or import one.">
            <p class="settings-note">Hollow shows each change, old to new, before anything is applied. Secrets, API keys and sign-ins are never included, nor the limits only you set here (Chats at once and the COO’s).</p>
            <div class="settings-actions">
              <button class="btn" onClick={() => void exportSettings()}>Export settings…</button>
              <button class="btn" onClick={() => void importSettings()}>Import settings…</button>
            </div>
          </SettingsGroup>

          <SettingsGroup id="diagnostics" title="Diagnostics" hint="Save diagnostics, the build, Hollow’s databases and readouts.">
            <SettingsSubhead title="Diagnostics bundle" hint="A redacted zip for a bug report. Note text is never included; Hollow sends nothing." />
            <div class="settings-actions">
              {/* Busy, not disabled: a disabled button drops keyboard focus to
                  the page while the folder picker is open (overnight B8). */}
              <button class="btn" aria-disabled={savingDiag()} aria-busy={savingDiag()} onClick={saveDiag}>
                {savingDiag() ? 'saving…' : 'Save diagnostics…'}
              </button>
            </div>
            <BuildIdentityRow />
            {/* A12: the PATH agents get, folder by folder (read when opened). */}
            <AgentToolPathPanel />

            <SettingsSubhead title="Launch calls" hint="Requests this window made while it opened, and any it repeated." />
            <LaunchIpcPanel />
            <SettingsSubhead
              title="Debug logs"
              hint="Raw model output from agent runs. Off unless HOLLOW_DEBUG_LOG=1 is set at launch."
            />
            <Show
              when={diag()}
              fallback={
                <p class="settings-loading">
                  <Show when={!diagError()}><Spinner  /> </Show>
                  {diagError() ? 'Diagnostics unavailable: see the banner above.' : 'loading diagnostics…'}
                </p>
              }
            >
              <KV k="Log directory" v={diag()!.debugLogDir} mono />
              <KV
                k="Logging enabled"
                v={diag()!.debugLogEnabled
                  ? 'yes (HOLLOW_DEBUG_LOG is set)'
                  : 'no (set HOLLOW_DEBUG_LOG=1 to enable)'}
              />
              <KV k="Log files on disk" v={String(diag()!.debugLogFiles)} mono />
              <div class="settings-actions">
                <button
                  class="btn"
                  disabled={clearing() || (diag()?.debugLogFiles ?? 0) === 0}
                  onClick={clearLogs}
                >{clearing() ? 'clearing…' : 'Clear debug logs'}</button>
                <button class="btn ghost" onClick={() => refetchDiag()}>Refresh</button>
              </div>
            </Show>
            <SettingsSubhead title="Hollow's databases" hint="Hollow’s SQLite files, with size and version. Measure and Check integrity only read." />
            <Suspense fallback={<Skeleton label="Loading Hollow's databases" />}><DatabasesPanel /></Suspense>
          </SettingsGroup>
        </SettingsSection>

        </div>
      </div>
      </ActiveSectionCtx.Provider>
    </>
  );
}
