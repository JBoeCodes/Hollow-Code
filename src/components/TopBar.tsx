import { Show, Suspense, lazy, onMount } from 'solid-js';
import { refreshRunQueue } from '../store/runQueue';
import { Icon } from '../icons';
import { MainAgentOrb } from './MainAgentOrb';
import { NotificationBell } from './NotificationBell';
import {
  activePage, navBack, navForward, canNavBack, canNavForward,
  toggleSidebar, sidebarCollapsed, requestSearchChats,
} from '../store';
import { PAGE_NAV_BY_ID } from '../navigation';
import { keyFor } from '../shortcuts';

/** A tooltip with the command's current key, if it has one. */
const withKey = (label: string, command: string) => keyFor(command) ? `${label} (${keyFor(command)})` : label;
import { workspaceDetail, projectWorkspaces, workspaceIcons } from '../store/workspaces';
import { distinguishing, workspaceTooltip } from '../workspaceOrigin';
import { setTopBarPaneSlot, setTopBarSlot } from '../store/topBar';
import { ProjectIcon } from './ProjectIcon';

// Lazy: the shell chunk is at its size budget; the button loads at idle and renders
// nothing until a verified update is ready.
const UpdateButton = lazy(() => import('./UpdateButton').then(m => ({ default: m.UpdateButton })));

export function TopBar() {
  const pageMeta = () => PAGE_NAV_BY_ID[activePage()];
  // One SQLite read at startup; `run-queue-event` keeps it fresh after that.
  onMount(() => refreshRunQueue(0));

  return (
    // `data-tauri-drag-region` makes the bar move the window now that the
    // native title strip is gone (overlay title bar). Tauri only honours it on
    // the element actually pressed, so each non-interactive layer carries it.
    <header class="top-bar" data-tauri-drag-region>
      <div class="top-bar-left" data-tauri-drag-region>
        <button
          type="button"
          class="top-bar-iconbtn"
          onClick={toggleSidebar}
          title={withKey(`${sidebarCollapsed() ? 'Show' : 'Hide'} sidebar`, 'toggle-sidebar')}
          aria-label={sidebarCollapsed() ? 'Show sidebar' : 'Hide sidebar'}
          aria-expanded={!sidebarCollapsed()}
        >
          <Icon name="panelLeft" size={16} />
        </button>
        {/* Search chats (L1, 11-12): its sidebar row moved here, beside the
            sidebar's toggle. It opens the sidebar's search in place, showing
            the sidebar first if it is hidden; Escape comes back here. */}
        <button
          type="button"
          class="top-bar-iconbtn top-bar-search-chats"
          onClick={e => requestSearchChats(e.currentTarget)}
          title={withKey('Search chats', 'search-chats')}
          aria-label="Search chats"
        >
          <Icon name="search" size={15} />
        </button>
        <div class="top-bar-nav" role="group" aria-label="History" data-tauri-drag-region>
          <button
            type="button"
            class="top-bar-iconbtn"
            disabled={!canNavBack()}
            onClick={navBack}
            title={withKey('Back', 'go-back')}
            aria-label="Back"
          ><Icon name="arrowLeft" size={15} /></button>
          <button
            type="button"
            class="top-bar-iconbtn"
            disabled={!canNavForward()}
            onClick={navForward}
            title={withKey('Forward', 'go-forward')}
            aria-label="Forward"
          ><Icon name="arrowRight" size={15} /></button>
        </div>
        <div class="top-bar-crumbs" aria-label="Breadcrumb" data-tauri-drag-region>
          <span class="crumb crumb-file" data-tauri-drag-region>{pageMeta().label}</span>
          {/* consolidation lane B, 2026-10-13: of three "Q3 review"s the
              header couldn't say which was open, so a twin showed its origin
              beside the title. In the bar's room that read "Hollow works…"
              (11-06), so the origin is on hover and for screen readers only;
              the rail and Home still show it. A long title ends in an
              ellipsis with the full name on hover. */}
          <Show when={activePage() === 'today' && workspaceDetail()}>{d => {
            const w = () => projectWorkspaces().find(x => x.id === d().workspace.id) ?? d().workspace;
            const owner = () => workspaceIcons()[w().id]?.owner;
            const twin = () => distinguishing(w(), projectWorkspaces(), owner());
            return <><span class="crumb-sep">/</span><span class="crumb crumb-project" data-tauri-drag-region title={workspaceTooltip(w(), owner(), twin())}><ProjectIcon workspaceId={w().id}/><span class="crumb-project-title">{w().title}</span><Show when={twin()}>{t => <span class="sr-only"> ({t()})</span>}</Show></span></>;
          }}</Show>
        </div>
        {/* The open workspace's own state (live conversations, branch) sits
            with its name (asks 10-11 A3). */}
        <div class="top-bar-page-actions" ref={setTopBarSlot} data-tauri-drag-region />
      </div>
      <div class="top-bar-right" data-tauri-drag-region>
        {/* Creation lives in the sidebar and owning pages; Search chats sits
            left, by the sidebar's toggle. Running holds queue controls and
            background operation details. */}
        <div class="top-bar-global" role="group" aria-label="Everywhere" data-tauri-drag-region>
          <Suspense fallback={null}><UpdateButton /></Suspense>
          <NotificationBell />
          <MainAgentOrb />
        </div>
        {/* The open workspace's side-panel toggle: the far right edge, as the
            sidebar's is the far left. */}
        <div class="top-bar-pane-slot" ref={setTopBarPaneSlot} />
      </div>
    </header>
  );
}
