import { createSearchFocusReturn } from './railSearchFocus';
import { MoreButton } from './MoreButton';
import { awakeOrganizationProjects, hiddenOrganizationProjects, conversationOrganization, organizationNow } from '../store/conversationOrganization';
import { organizationGroup } from '../conversationOrganization';
import { projectChildGraph, visibleProjectChildren } from '../railProjectChildren';
import { railRowSummary, withRailDetails } from '../railRowDetails';
import { railRowFileTarget } from '../railRowFiles';
import { railFoldedSections, toggleRailSection, railWidth, setRailWidth, persistRailWidth, RAIL_WIDTH_DEFAULT, RAIL_WIDTH_MIN, RAIL_WIDTH_MAX } from '../store/prefs';
import { RAIL_BOTTOM, RAIL_PRIMARY } from '../navigation';
import { ErrorBoundary, For, Show, Suspense, lazy, createEffect, createMemo, createSignal, on, onCleanup, onMount } from 'solid-js';
import { RecoverableView } from './RecoverableView';
import { chatAgentSessionKey, activePage, toggleSidebar, sidebarCollapsed, openContextMenu, canLeaveConversation, searchChatsRequest } from '../store';
import { openSessionByKey, renameWorkspaceTo, workspaceEditLock, workspaceId, workspaceSessions, railWorkspaces, refreshWorkspaces, goToPage, railGitStates, refreshRailGitStates } from '../store/workspaces';
import type { WorkWorkspace, RailGitState } from '../workspaceTypes';
import { sessionDropTarget } from '../sessionDragHandlers';
import type { AgentSessionSummary } from '../types';
import { sessionMenuItems, workspaceMenuItems } from '../workspaceMenu';
import { Icon, type IconName } from '../icons';
import { ProjectIcon } from './ProjectIcon';
import { HollowMark } from './HollowMark';
import { SessionRollupBadge } from './SessionIndicator';
import { sessionState, workspaceRollup, workspaceRunningSince } from '../store/sessionStates';
import { RunTimer } from './RunTimer';
import { ChildRows, ChildToggle } from './RowChildren';
import { Renamable } from './RowRename';
import { descendantState } from '../railChildren';
import { childrenOpen, toggleChildren } from '../store/rowChildren';
import { setRenamingKey } from '../store/rowRename';
import { adoptLegacyPins, moveWorkspacePin, pinnedWorkspaces } from '../store/workspacePins';
import { dropPosition } from '../workspacePins';
import { projectWorkspaces, railHiddenIds, workspaceIcons, requestNewWorkspace, openProjectAgent } from '../store/workspaces';
import { distinguishing, workspaceTooltip } from '../workspaceOrigin';
import { activityGroups, railGroups } from '../railWorkspaces';
import type { PageId } from '../store';
import { pageShortcut } from '../shortcuts';
import { morphFrom } from '../viewTransitions';
import { rollupSessions, type SessionRollup } from '../sessionIndicator';
import { workspaceListError, workspaceListLoading, workspaceListLoaded } from '../store/workspaces';
import { closePreview } from '../store/preview';
import { windowAtMost } from '../windowLayout';
import '../styles/workbench.css';

const RailMinis = lazy(() => import('./RailMinis'));
// Search chats (overnight 11-04 S2): loaded on first use.
const RailSearch = lazy(() => import('./RailSearch'));
// Quick chats (2026-11-14): its section loads just after the sidebar.
const RailQuickChats = lazy(() => import('./RailQuickChats'));
const RailQuickWorking = lazy(() => import('./RailQuickChats').then(m => ({ default: m.RailQuickWorking })));
const RailOrganizedChats = lazy(() => import('./RailOrganizedChats'));
const RailDropZones = lazy(() => import('./RailDropZones'));
const RailRowDetails = lazy(() => import('./RailRowDetails'));
// The machine strip (M2 PR 4): shows only once another Mac is paired.
const RailMachines = lazy(() => import('./RailMachines'));
// The edge's drag handle loads just after the sidebar, outside the App
// chunk's budget (it is shared with the Preview pane and Conversations).
const ResizeHandle = lazy(() => import('./ResizeHandle').then(m => ({ default: m.ResizeHandle })));


export function Rail() {
  onMount(()=>{
    // Then move any pins this window kept before pins lived in the briefs.
    void refreshWorkspaces({ join: true }).then(adoptLegacyPins);
    // PR states change on GitHub, not here: re-ask now and then (the backend
    // caches each for two minutes).
    // The workspace page's minute refresh asks too; skip when it just did.
    const timer = window.setInterval(() => void refreshRailGitStates(50_000), 60_000);
    onCleanup(() => window.clearInterval(timer));
  });
  // Pinned workspaces in their own group on top (the user's own choice, in
  // pin order; 2026-10-21), then the workspaces with a conversation, used
  // here first (railWorkspaces.ts), each listed once. The rest are on the
  // Projects page, the full list with search (L1, 11-12).
  const groups = createMemo(() => railGroups(railWorkspaces(), projectWorkspaces(), pinnedWorkspaces().filter(id => !railHiddenIds().includes(id))));
  // Folded, the Recent heading keeps every project chat's live state
  // (working, waiting, failed; overnight 11-04).
  const projectsLive = createMemo(() => rollupSessions(projectWorkspaces()
    .flatMap(w => workspaceSessions()[w.id]?.sessions ?? []).map(s => ({ title: sessionLabel(s), state: sessionState(s.key) }))));
  const unpinned = createMemo(() => {
    const pins = new Set(groups().pinned.map(w => w.id));
    const sessions = new Map(projectWorkspaces().filter(w => !pins.has(w.id))
      .flatMap(w => workspaceSessions()[w.id]?.sessions ?? []).map(s => [s.key, s]));
    return rollupSessions([...sessions.values()].map(s => ({ title: sessionLabel(s), state: sessionState(s.key) })));
  });
  // Projects (overnight 11-04 S1, Alex's ChatGPT- and Claude-like
  // order). The heading folds the section, remembered. Chats with no
  // project list under it in Quick chats again (2026-11-14; L8 had folded
  // them into the COO).
  const projectsFolded = () => railFoldedSections().includes('projects');
  // The newest few projects, no Show more (L1, 11-12: the Projects page
  // lists them all). The open project and one with a live chat stay listed
  // past the eight.
  const projectActivity = createMemo(() => activityGroups(groups().rest.filter(w => !hiddenOrganizationProjects().has(w.id)),
    w => workspaceRollup(w.id)?.state ?? 'idle',
    w => (activePage() === 'today' && workspaceId() === w.id) || awakeOrganizationProjects().has(w.id), VISIBLE_PROJECTS));
  const workingFolded = () => !railFoldedSections().includes('working-projects-open');
  // Search chats (overnight 11-04 S2): its field and results take the
  // sidebar's place; the lists stay mounted underneath and come back on
  // Close or Escape. Opened by the top bar's search icon or ⌘⇧K (L1, 11-12:
  // the sidebar's Search chats row went).
  const [searching, setSearching] = createSignal(false);
  const searchShown = () => searching() && !sidebarCollapsed();
  // From the hidden sidebar (a narrow window's too), Search shows it for the
  // search and hides it again on close. Focus goes back at once to where it
  // was asked from (the top bar's icon), so a palette opened next (Search
  // everything) returns focus there too.
  let shownForSearch = false;
  const focusReturn = createSearchFocusReturn<HTMLElement>();
  const openSearch = (from: HTMLElement | null) => {
    // A repeat while searching keeps where focus goes back to (not the search box).
    focusReturn.open(from, searchShown());
    if (searching() && !sidebarCollapsed()) { document.querySelector<HTMLInputElement>('.work-rail-search input')?.focus(); return; }
    shownForSearch = sidebarCollapsed(); if (shownForSearch) toggleSidebar(); setSearching(true);
  };
  createEffect(on(searchChatsRequest, r => openSearch(r.returnTo), { defer: true }));
  const closeSearch = () => {
    setSearching(false);
    // Where focus goes back, read before the sidebar hides: hiding it runs
    // the effect below at once, which forgets the place (2026-11-15: Escape
    // in a narrow window sent focus to the search icon, not the toggle).
    const back = focusReturn.close(() => document.querySelector<HTMLElement>('.top-bar-search-chats'));
    // Hide it again only if it is still shown: opening a result in a narrow
    // window may have closed the drawer already.
    if (shownForSearch) { shownForSearch = false; if (!sidebarCollapsed()) toggleSidebar(); }
    back?.focus();
  };
  // Hiding the sidebar ends the search: forget where it was opened from too (#1332 review), so nothing stale is kept.
  createEffect(on(sidebarCollapsed, hidden => { if (hidden) { setSearching(false); focusReturn.forget(); shownForSearch = false; } }, { defer: true }));
  // One key list (five asks A4): j / k walk its rows, . opens a row's menu.
  return <aside class="rail work-rail" classList={{collapsed:sidebarCollapsed()}} style={!sidebarCollapsed() && railWidth() != null ? { '--rail-w': `${railWidth()}px` } : undefined} aria-label="Main navigation" data-key-list>
    {/* One top row (2026-09-29, Conductor-density): the mark and name. */}
    {/* Also the window's drag handle: with the overlay title bar the traffic
        lights sit at this row's left edge (workbench.css leaves them room under data-chrome="overlay"). */}
    <div class="work-brand" data-tauri-drag-region><strong data-tauri-drag-region><span class="work-brand-mark" aria-hidden="true"><HollowMark/></span><span class="studio-brand-name" data-tauri-drag-region>Hollow</span></strong>
      {/* consolidation lane B, 2026-10-13: below 860px the shown sidebar lies
          over the page and over the top bar's own toggle, so it closed only by
          Esc, ⌘B or choosing a destination; this is its visible close. */}
      <button type="button" class="icon-btn work-rail-close" aria-label="Close sidebar" title="Close sidebar" onClick={toggleSidebar}><Icon name="x" size={15}/></button></div>
    {/* Search chats in place (overnight 11-04 S2), opened from the top
        bar's search icon or ⌘⇧K. The sidebar's New chat and Search chats
        rows went in L1 (11-12): New chat is the COO's page (⌘⇧M), the
        palette's New chat and a project's menu. */}
    <Show when={searchShown()}>
      {/* Its chunk failing to load must not leave the sidebar hidden behind
          an empty field: say so, with a way back. */}
      <ErrorBoundary fallback={() => <div class="work-rail-search"><p class="work-rail-search-hint" role="alert">Search didn’t load.</p><button ref={el => queueMicrotask(() => el.focus())} type="button" class="btn small" onClick={closeSearch} onKeyDown={e => { if (e.key === 'Escape') { e.preventDefault(); closeSearch(); } }}>Close search</button></div>}>
        <Suspense fallback={<div class="work-rail-search"/>}><RailSearch onClose={closeSearch}/></Suspense>
      </ErrorBoundary>
    </Show>
    {/* Labelled destinations (next-level P01): Library and Automations are daily
        tools, so they get names and their ⌘ digits, not a bottom icon bar.
        Tasks is back among them (overnight 11-04 S1). Work is called
        Projects since L1 (11-12): Projects, Tasks, Library, Automations. */}
    <nav class="work-rail-primary" aria-label="Main" hidden={searchShown()}>
      {/* P1-05: the rows are navigation.ts's NAV_PAGES, so the rail, the
          palette and the ⌘ digits share one id and one icon per page. */}
      <For each={RAIL_PRIMARY}>{p =>
        <PrimaryRow page={p.id} icon={p.icon} label={p.label} selected={activePage()===p.id&&(p.id!=='today'||!workspaceId())} rollup={p.id==='today'&&sidebarCollapsed()?unpinned():null}/>
      }</For>
    </nav>
    <nav class="work-rail-scroll" aria-label="Project navigation" hidden={searchShown()}>
      <Show when={!sidebarCollapsed()}><Suspense fallback={null}><RailDropZones/></Suspense></Show>
      <Show when={sidebarCollapsed()}><button type="button" class="work-nav-row" aria-label="New project" title="New project" disabled={!canLeaveConversation()} onClick={()=>void requestNewWorkspace()}><Icon name="plus" size={15}/></button></Show>
      <Show when={workspaceListError()}>
        <div class="work-rail-list-error">
          <span role="alert" title="Couldn't load workspaces"><Icon name="alert" size={14}/><span class="work-rail-list-error-text">Couldn't load workspaces</span></span>
          <button class="btn-link" aria-label="Retry loading workspaces" title="Retry loading workspaces" disabled={workspaceListLoading()} onClick={()=>void refreshWorkspaces({join:true})}>Retry</button>
        </div>
      </Show>
      <Show when={workspaceListLoading()&&!workspaceListLoaded()&&!workspaceListError()}>
        <div class="work-rail-loading" role="status" aria-label="Loading workspaces"><i/><i/><i/></div>
      </Show>
      {/* The collapsed sidebar's Pinned group: one icon each, on top. */}
      <Show when={sidebarCollapsed()}>
        <RecoverableView name="Workspace shortcuts" compact fallback={<div class="work-rail-loading" role="status" aria-label="Loading workspace shortcuts"><i/><i/><i/></div>}>
          <RailMinis pinned={groups().pinned}/>
        </RecoverableView>
      </Show>
      {/* Projects (overnight 11-04 S1): pinned ones first, each with its pin
          (the separate Pinned heading went), then the newest with a chat.
          Headed Recent since L1 (11-12): the row above is Projects now, the
          page with every project, and two "Projects" read as one thing twice. */}
      <div class="work-rail-project-heading"><button data-key-item type="button" class="work-rail-section-toggle" aria-expanded={!projectsFolded()} title={projectsFolded()?'Show recent projects':'Hide recent projects'} onClick={()=>toggleRailSection('projects')}><span class="work-rail-label">Recent</span><Icon name={projectsFolded()?'chevronRight':'chevronDown'} size={12}/></button><Show when={projectsFolded()}><SessionRollupBadge rollup={projectsLive()}/></Show><button type="button" class="icon-btn" aria-label="New project" title="New project" disabled={!canLeaveConversation()} onClick={()=>void requestNewWorkspace()}><Icon name="plus" size={15}/></button></div>
      <Show when={!projectsFolded()}>
        <For each={groups().pinned}>{w=><ProjectRow w={w} pinned/>}</For>
        <For each={projectActivity().active}>{w=><ProjectRow w={w}/>}</For>
      </Show>
      {/* Quick chats under Projects (2026-11-14): chats outside any project. */}
      <Show when={!sidebarCollapsed()}><Suspense fallback={null}><RailQuickChats/></Suspense></Show>
      {/* Shelves (3.1): Working projects and Working quick chats lead the
          shelves Snoozed and Settled close, after Ready and Pinned chats, so
          keyboard order is the drawn order (workbench.css `.rail-shelf`). */}
      <Suspense fallback={null}><RailOrganizedChats shelves={<>
        <Show when={projectActivity().working.length && !sidebarCollapsed()}>
          <div class="rail-shelf" role="group" aria-label="Working projects">
            <button data-key-item type="button" class="work-rail-section-toggle" aria-expanded={!workingFolded()} aria-label={`Working projects (${projectActivity().working.length})`} onClick={() => toggleRailSection('working-projects-open')}><span class="work-rail-label">Working ({projectActivity().working.length})</span><Icon name={workingFolded() ? 'chevronRight' : 'chevronDown'} size={12}/></button>
            <Show when={!workingFolded()}><For each={projectActivity().working}>{w => <ProjectRow w={w}/>}</For></Show>
          </div>
        </Show>
        <Show when={!sidebarCollapsed()}><Suspense fallback={null}><RailQuickWorking/></Suspense></Show>
      </>}/></Suspense>
    </nav>
    <Suspense fallback={null}><RailMachines/></Suspense>
    <Suspense fallback={null}><RailRowDetails/></Suspense>
    <nav class="work-rail-bottom" aria-label="Shared tools"><For each={RAIL_BOTTOM}>{p => <PrimaryRow page={p.id} icon={p.icon} label={p.label} selected={activePage()===p.id}/>}</For></nav>
    {/* Drag the edge to size the sidebar (overnight 11-04 S1), remembered;
        double-click resets it, arrow keys move it when focused. */}
    <Show when={!sidebarCollapsed()}>
      <Suspense fallback={null}><ResizeHandle label="Resize sidebar" current={()=>railWidth() ?? RAIL_WIDTH_DEFAULT} min={RAIL_WIDTH_MIN} max={RAIL_WIDTH_MAX} onResize={setRailWidth} onCommit={persistRailWidth} onReset={()=>{setRailWidth(null);persistRailWidth();}}/></Suspense>
    </Show>
  </aside>;
}

/** A labelled destination row with its shortcut, shown on hover and focus. */
/** `count`: null when unknown (the pill says "–", never a quiet zero);
 *  `note`: quiet words after the label (Review's drafts, D3). */
/** A window too narrow for the Preview pane beside a page shows the pane over
 *  it: a destination picked here steps it aside, or the click shows nothing. */
const leaveTheCover = () => { if (windowAtMost('medium')) closePreview(); };

function PrimaryRow(props: { page: PageId; icon: IconName; label: string; selected: boolean; count?: number | null; name?: string; note?: string; rollup?: SessionRollup | null }) {
  const key = () => pageShortcut(props.page);
  const name = () => props.name ?? [props.label,props.rollup?.label].filter(Boolean).join(', ');
  return <button data-key-item aria-label={name()} title={key() ? `${name()} (${key()})` : name()} class="work-nav-row work-primary-row" classList={{selected:props.selected}} aria-current={props.selected?'page':undefined} onClick={()=>goToPage(props.page,{onNavigate:leaveTheCover})}>
    <Icon name={props.icon} size={16}/><span>{props.label}<Show when={props.note}><small class="work-primary-note" aria-hidden="true">{props.note}</small></Show></span><SessionRollupBadge rollup={props.rollup??null}/><Show when={props.count !== undefined && props.count !== 0}><small class="work-primary-count" aria-hidden="true">{props.count == null ? '–' : props.count > 99 ? '99+' : props.count}</small></Show><Show when={key()}><kbd class="work-primary-key" aria-hidden="true">{key()}</kbd></Show>
  </button>;
}

/** Projects (besides pinned ones) the sidebar lists; the rest are on the
 *  Projects page. */
const VISIBLE_PROJECTS = 8;

/** The glyph a build workspace's branch shows: its PR's state, or a plain branch. */
function gitGlyph(state: RailGitState | undefined): { icon: IconName; tone: string; title: string } | null {
  if (!state) return null;
  const pr = state.prNumber ? `PR #${state.prNumber}` : 'PR';
  switch (state.prState) {
    case 'merged': return { icon: 'gitMerge', tone: 'merged', title: `${pr} merged · ${state.branch}` };
    case 'open': return { icon: 'gitPullRequest', tone: 'open', title: `${pr} open · ${state.branch}` };
    case 'closed': return { icon: 'gitPullRequestClosed', tone: 'closed', title: `${pr} closed · ${state.branch}` };
    default: return { icon: 'gitBranch', tone: 'branch', title: state.ahead ? `${state.branch} · ${state.ahead} commit${state.ahead === 1 ? '' : 's'} ahead, no PR` : `${state.branch} · no PR` };
  }
}

const sessionLabel = (s: AgentSessionSummary) => s.title?.trim() || 'Conversation';

/** The pinned workspace being dragged to a new place in Pinned. */
const [pinDragging, setPinDragging] = createSignal<string | null>(null);
const PIN_DRAG_MIME = 'application/x-hollow-pin';
const isPinDrag = (e: DragEvent) => !!pinDragging() && !!e.dataTransfer?.types.includes(PIN_DRAG_MIME);
const PIN_DROP_CLASSES = ['pin-drop-before', 'pin-drop-after'];
/** Below the middle of the header: the dragged pin goes after it. */
const dropsAfter = (e: DragEvent) => { const r = (e.currentTarget as HTMLElement).getBoundingClientRect(); return e.clientY > r.top + r.height / 2; };

/** A Pinned group header's drop handlers: a dragged pin is placed before or
 *  after it; a dragged conversation still moves into the workspace. */
function pinAwareDrop(w: () => WorkWorkspace, inPinned: () => boolean, lead: () => string | null) {
  const session = sessionDropTarget(() => w().id);
  const files = railRowFileTarget(lead);
  const clear = (el: HTMLElement) => el.classList.remove(...PIN_DROP_CLASSES);
  const over = (e: DragEvent) => {
    const el = e.currentTarget as HTMLElement;
    e.preventDefault();
    e.dataTransfer!.dropEffect = 'move';
    clear(el);
    if (pinDragging() !== w().id) el.classList.add(dropsAfter(e) ? 'pin-drop-after' : 'pin-drop-before');
  };
  return {
    onDragEnter(e: DragEvent) { files.onDragEnter(e); if (e.defaultPrevented) return; if (inPinned() && isPinDrag(e)) over(e); else session.onDragEnter(e); },
    onDragOver(e: DragEvent) { files.onDragOver(e); if (e.defaultPrevented) return; if (inPinned() && isPinDrag(e)) over(e); else session.onDragOver(e); },
    onDragLeave(e: DragEvent) {
      files.onDragLeave(e);
      const el = e.currentTarget as HTMLElement;
      if (!el.contains(e.relatedTarget as Node)) clear(el);
      session.onDragLeave(e);
    },
    onDrop(e: DragEvent) {
      files.onDrop(e);
      const el = e.currentTarget as HTMLElement;
      clear(el);
      if (e.defaultPrevented) return;
      const id = pinDragging();
      if (!inPinned() || !id || !isPinDrag(e)) { session.onDrop(e); return; }
      e.preventDefault();
      // Counted over every pin, hidden-from-sidebar ones included, as the
      // backend places it.
      const position = dropPosition(pinnedWorkspaces(), id, w().id, dropsAfter(e));
      const dragged = projectWorkspaces().find(x => x.id === id);
      if (position && dragged) void moveWorkspacePin(dragged, position);
    },
  };
}

/** One project, one row (L8, 11-10): a click opens its lead, the
 *  project's one conversation (its current one, after any handover). No
 *  conversation list or fold arrow: the project's
 *  other chats are in the lead's Subagents tab (Earlier chats). Its
 *  right-click (a Control-click, VoiceOver's Show Menu) is the project's
 *  menu. A pinned row drags to a new place in Pinned; a conversation
 *  dropped on any row moves into that project. */
function ProjectRow(props: { w: WorkWorkspace; pinned?: boolean }) {
  const w = () => props.w;
  // Same title as another workspace (a local folder and its Notion mirror):
  // say where this one comes from, so the right one is picked (consolidation E).
  const twin = () => distinguishing(w(), projectWorkspaces(), workspaceIcons()[w().id]?.owner);
  const named = () => twin() ? `${w().title} (${twin()})` : w().title;
  const git = () => gitGlyph(railGitStates()[w().id]);
  const here = () => activePage() === 'today' && workspaceId() === w().id;
  // The row says whether anything in the project works, waits or failed.
  const rollup = () => workspaceRollup(w().id);
  // A running conversation keeps running when another is opened (build
  // programme PR B4), so only a send still being accepted pins it.
  const locked = () => !canLeaveConversation() && workspaceId() !== w().id;
  // The lead's subagent chats nest under the row (3.6); inline rename on a double-click or Rename….
  const group = () => workspaceSessions()[w().id];
  const lead = () => group()?.projectAgentKey ?? null;
  const leadSummary = () => group()?.sessions.find(s => s.key === lead()) ?? conversationOrganization()[lead() ?? '']?.session;
  const kids = createMemo(() => projectChildGraph(group()?.sessions ?? [], lead()));
  const childHere = (key: string) => here() && chatAgentSessionKey() === key;
  const childVisible = (s: AgentSessionSummary) => organizationGroup(conversationOrganization()[s.key], s, sessionState(s.key), childHere(s.key), organizationNow()) === 'active';
  const count = () => visibleProjectChildren(lead(), kids(), childVisible);
  const needsLook = () => { const l = lead(); return !!l && descendantState(l, kids(), sessionState) !== 'idle'; };
  const kidsOpen = () => childrenOpen(w().id, needsLook());
  const rename = () => { if (!workspaceEditLock(w().id)) setRenamingKey(w().id); };
  const items = () => withRailDetails(workspaceMenuItems(w(), rename), leadSummary());
  const menu = (e: MouseEvent) => openContextMenu(e, items());
  // The menu is the row's right-click; a Control-click is the menu's too,
  // never a click.
  const open = (e: MouseEvent) => {
    if (e.ctrlKey) return;
    morphFrom(e.currentTarget);
    void openProjectAgent(w().id);
  };
  return <div class="work-tree" data-project-id={w().id}>
    <div class="work-tree-head" data-session-key={lead() ?? undefined} data-rail-motion-key={`project:${w().id}`} classList={{current:here()}} onContextMenu={menu} {...railRowSummary(leadSummary)} {...pinAwareDrop(w, () => !!props.pinned, lead)}>
      <Renamable id={w().id} value={w().title} label={`Rename ${w().title}`} maxLength={100} commit={n => renameWorkspaceTo(w(), n)}>
        <button data-key-item class="work-nav-row work-workspace-row" aria-current={here()?'true':undefined} draggable={props.pinned ? true : undefined} onDragStart={e=>{ if(!props.pinned||!e.dataTransfer) return; e.dataTransfer.setData(PIN_DRAG_MIME, w().id); e.dataTransfer.effectAllowed='move'; setPinDragging(w().id); e.currentTarget.classList.add('session-dragging'); }} onDragEnd={e=>{ setPinDragging(null); e.currentTarget.classList.remove('session-dragging'); }} title={`${workspaceTooltip(w(), workspaceIcons()[w().id]?.owner)} · opens its lead`} aria-label={[w().title, twin(), rollup()?.label].filter(Boolean).join(', ')} disabled={locked()} onClick={open} onDblClick={rename}><ProjectIcon workspaceId={w().id}/><span class="work-workspace-title">{w().title}</span><RunTimer since={rollup()?.state === 'running' ? workspaceRunningSince(w().id) : null}/><Show when={twin()}>{t=><small class="work-workspace-origin">{t()}</small>}</Show><SessionRollupBadge rollup={rollup()}/></button>
      </Renamable>
      <Show when={git()}>{g=><span class={`work-workspace-git ${g().tone}`} role="img" aria-label={g().title} title={g().title}><Icon name={g().icon} size={13}/></span>}</Show>
      {/* A pinned row's pin says so, quietly; Unpin is in its right-click
          menu, beside Pin, and in the project's ⋯ on Work and in the top bar. */}
      <Show when={props.pinned}><span class="work-tree-pin" role="img" aria-label="Pinned" title="Pinned"><Icon name="pin" size={12}/></span></Show>
      <Show when={count() > 0}><ChildToggle count={count()} open={kidsOpen()} onToggle={() => toggleChildren(w().id, kidsOpen())}/></Show>
      <MoreButton class="work-row-actions" label={`Actions for project ${w().title}`} items={items}/>
    </div>
    <Show when={lead() && count() > 0 && kidsOpen()}>
      <ChildRows parent={lead()!} kids={kids()} visible={childVisible} here={childHere} locked={() => !canLeaveConversation()} open={c => void openSessionByKey(c.key)} menu={(c, r) => sessionMenuItems(w(), c, c.title?.trim() || 'Subagent chat', r)}/>
    </Show>
  </div>;
}
