// Quick chats in the sidebar (2026-11-14, Alex's quick chats design and
// card 2): under the Projects list, newest first, like ChatGPT. A row opens
// its chat; right-click is the conversation menu (Rename, Move to project…,
// Archive and the rest). The heading folds the section, remembered, and its
// + starts a new quick chat (New chat and ⌘N do too). Lazy, so none of it
// weighs on the App chunk.
import { For, Index, Show, createMemo, createRoot } from 'solid-js';
import { activePage, canLeaveConversation, openContextMenu } from '../store';
import { chatAgentSessionKey } from '../store/chatRuntime';
import { quickChatsWorkspace, startQuickChat, visitWorkspaceConversation, workspaceId, workspaceSessions } from '../store/workspaces';
import { railFoldedSections, toggleRailSection } from '../store/prefs';
import { sessionMenuItems } from '../workspaceMenu';
import { QUICK_CHATS_ID, QUICK_CHATS_LABEL } from '../workspaceTypes';
import type { AgentSessionSummary } from '../types';
import { Icon } from '../icons';
import { RailViewButtons } from './RailViewMenu';
import { RailBulkBar } from './RailBulkBar';
import { railView } from '../store/railView';
import { selecting, selectedKeys, selectRange, startSelecting, toggleSelected } from '../store/rowSelection';
import { applyRailView, groupRailRows, isDefaultView, VIEW_LIST_CAP } from '../railView';
import { chatRailFacts } from '../store/rowStatus';
import { SessionIndicator } from './SessionIndicator';
import { runningSince, sessionState } from '../store/sessionStates';
import { RunTimer } from './RunTimer';
import { RowMarks } from './RowMarks';
import { ChildRows, ChildToggle, groupRunningSince } from './RowChildren';
import { Renamable, renameChatFromRow } from './RowRename';
import { childCount, descendantState, groupState, splitChildren } from '../railChildren';
import { childrenOpen, toggleChildren } from '../store/rowChildren';
import { setRenamingKey } from '../store/rowRename';
import { refreshWorkspaceSessions } from '../store/workspaces';
import { peerMachines, peerRowsHeld } from '../store/peerSessions';
import { mergeChatRows } from '../store/peerMerge';
import { chatHomes } from '../store/moveFlags';
import { peerNow } from '../store/peerUi';
import { openRemoteChat } from '../store/peerChat';
import { MachineBadge } from './MachineBadge';
import { OFFLINE_BADGE, offlineBadgeLabel } from '../store/chatOfflineWords';
import { openRemoteRowMenu, remoteRowMenuItems } from './remoteRowMenu';
import { MoreButton } from './MoreButton';
import { activityGroups } from '../railWorkspaces';
import { conversationOrganization, organizationNow } from '../store/conversationOrganization';
import { organizationGroup } from '../conversationOrganization';
import { railChatDragSource } from '../railChatDrag';
import { railRowSummary, withRailDetails } from '../railRowDetails';
import { railRowFileTarget } from '../railRowFiles';

/** Quick chats the section lists; Library → Conversations has every one. */
export const VISIBLE_QUICK_CHATS = 8;

const label = (s: AgentSessionSummary) => s.title?.trim() || 'New chat';

/** The section and the Working shelf read the same rows, so they are built
 *  once, in a root of their own: the shelf sits elsewhere in the sidebar (at
 *  its end, with the other shelves), not inside the section. */
function build() {
  const here = (key: string) => activePage() === 'today' && workspaceId() === QUICK_CHATS_ID && chatAgentSessionKey() === key;
  // This Mac's quick chats and another Mac's (M2 PR 4) as one merged list
  // (`peerMerge.ts`): newest first, and (M5) a chat that moved to another
  // Mac shows once, at its new home (badged moved); while that Mac lists
  // nothing (offline past a day, forgotten) this Mac's row stays, badged moved.
  const movedTo = (key: string) => { const h = chatHomes()[key]; return h?.state === 'away' ? h.peer : null; };
  const localSessions = createMemo(() => {
    const stored = workspaceSessions()[QUICK_CHATS_ID]?.sessions ?? [];
    const cwd = quickChatsWorkspace()?.cwd;
    const byKey = new Map(stored.map(s => [s.key, s]));
    for (const row of Object.values(conversationOrganization())) if (cwd && row.session.cwd === cwd && !byKey.has(row.key)) byKey.set(row.key, row.session);
    return [...byKey.values()].sort((a, b) => b.lastActivityAt.localeCompare(a.lastActivityAt));
  });
  const merged = createMemo(() => mergeChatRows(
    localSessions().map(s => ({
      key: s.key, title: s.title, lastActivityAt: s.lastActivityAt || s.createdAt, workspaceId: QUICK_CHATS_ID, movedTo: movedTo(s.key), session: s,
    })),
    peerRowsHeld(),
    { thisMac: peerMachines()?.thisMac ?? null, knownWorkspaces: new Set([QUICK_CHATS_ID]), now: peerNow() },
  ));
  // Subagent chats nest under the chat that started them (3.6); only the rest are rows.
  const split = createMemo(() => splitChildren(merged().flatMap(r => (r.local ? [r.local.session] : []))
    .filter(s => organizationGroup(conversationOrganization()[s.key], s, sessionState(s.key), here(s.key), organizationNow()) === 'active')));
  const groupOfRow = (key: string) => groupState(key, split().kids, sessionState);
  // The open chat stays listed past the cap. A chosen view (sort, filter, group; 3.5) lists every
  // match in the order asked, with no Working split; the default keeps attention first and the cap.
  const chats = createMemo(() => {
    const view = railView();
    if (isDefaultView(view)) return activityGroups(split().top, s => groupOfRow(s.key), s => here(s.key) || !!conversationOrganization()[s.key]?.snoozedUntil, VISIBLE_QUICK_CHATS);
    const shown = applyRailView(split().top, view, s => chatRailFacts(s, here(s.key))).slice(0, VIEW_LIST_CAP);
    // The open chat stays listed, as in the default list.
    const open = split().top.find(s => here(s.key) && !shown.some(x => x.key === s.key));
    return { active: open ? [...shown, open] : shown, working: [] as AgentSessionSummary[] };
  });
  /** The rows in the order shown, for a Shift-click range and Select all. */
  const shownKeys = () => [...chats().active, ...(railFoldedSections().includes('working-quick-open') ? chats().working : [])].map(s => s.key);
  const groups = createMemo(() => groupRailRows(chats().active, railView().group, Date.now()));
  // Another Mac's quick chats, under this Mac's own; they open read live from their home.
  const remote = createMemo(() => merged().flatMap(r => (r.remote && r.workspaceId === QUICK_CHATS_ID ? [{ row: r.remote, stale: r.stale }] : []))
    .slice(0, VISIBLE_QUICK_CHATS));
  const menuItems = (s: AgentSessionSummary, rename: () => void) => { const w = quickChatsWorkspace(); return withRailDetails(w ? sessionMenuItems(w, s, label(s), rename) : [], s); };
  const open = (s: AgentSessionSummary) => void visitWorkspaceConversation(QUICK_CHATS_ID, s.key);
  const localRow = (s: AgentSessionSummary) => {
    const rename = () => setRenamingKey(s.key);
    const kids = () => split().kids;
    const count = () => childCount(s.key, kids());
    const open_ = () => childrenOpen(s.key, descendantState(s.key, kids(), sessionState) !== 'idle');
    return <>
      <div class="work-tree-head" classList={{ current: here(s.key), selected: selectedKeys().has(s.key) }} data-session-key={s.key} data-rail-motion-key={s.key} {...railChatDragSource(() => s.key, () => label(s), () => menuItems(s, rename))} {...railRowSummary(() => s)} {...railRowFileTarget(() => s.key)} onContextMenu={e => openContextMenu(e, menuItems(s, rename))}>
        <Show when={selecting()}><input type="checkbox" class="work-row-check" aria-label={`Select ${label(s)}`} checked={selectedKeys().has(s.key)} onChange={() => toggleSelected(s.key)} onClick={e => e.stopPropagation()}/></Show>
        <Renamable id={s.key} value={label(s)} label={`Rename ${label(s)}`} commit={n => renameChatFromRow(s.key, n, refreshWorkspaceSessions)}>
          <button data-key-item type="button" class="work-nav-row work-workspace-row" aria-current={here(s.key) ? 'true' : undefined} title={label(s)}
            disabled={!canLeaveConversation() && !here(s.key)} onClick={e => {
              // Selecting (or Cmd-click, Shift-click for a stretch) ticks the row instead of opening it.
              if (selecting() || e.metaKey || e.shiftKey) { if (!selecting()) startSelecting(); if (e.shiftKey) selectRange(shownKeys(), s.key); else toggleSelected(s.key); return; }
              if (!e.ctrlKey) open(s);
            }} onDblClick={() => { if (!selecting()) rename(); }}>
            <SessionIndicator state={groupOfRow(s.key)} size={8} decorative/><span class="work-workspace-title">{label(s)}</span>
            <RunTimer since={groupRunningSince(s.key, kids())}/>
            <RowMarks session={s} here={here(s.key)}/>
            <Show when={s.forkKind === 'offline'}><span class="chat-offline-badge" role="img" aria-label={offlineBadgeLabel(false)} title={offlineBadgeLabel(false)}>{OFFLINE_BADGE}</span></Show>
            <Show when={movedTo(s.key)}>{to => { const m = () => peerMachines()?.machines.find(x => x.machineId === to()); return <MachineBadge machine={m() ? { machineId: to(), slug: m()!.slug, name: m()!.name } : null} moved="to"/>; }}</Show>
          </button>
        </Renamable>
        <Show when={count() > 0}><ChildToggle count={count()} open={open_()} onToggle={() => toggleChildren(s.key, open_())}/></Show>
        <MoreButton class="work-row-actions" label={`Actions for chat ${label(s)}`} disabled={!quickChatsWorkspace()} items={() => menuItems(s, rename)}/>
      </div>
      <Show when={count() > 0 && open_()}>
        <ChildRows parent={s.key} kids={kids()} here={here} locked={() => !canLeaveConversation()} open={open} menu={menuItems}/>
      </Show>
    </>;
  };
  return { here, movedTo, chats, groups, shownKeys, remote, localRow };
}
let shared: ReturnType<typeof build> | undefined;
const rows = () => (shared ??= createRoot(build));

export default function RailQuickChats() {
  const { groups, shownKeys, remote, localRow } = rows();
  const folded = () => railFoldedSections().includes('quick-chats');
  return <div class="work-rail-quick-chats" role="group" aria-label={QUICK_CHATS_LABEL}>
    <div class="work-rail-project-heading">
      <button data-key-item type="button" class="work-rail-section-toggle" aria-expanded={!folded()} title={folded() ? 'Show quick chats' : 'Hide quick chats'} onClick={() => toggleRailSection('quick-chats')}>
        <span class="work-rail-label">{QUICK_CHATS_LABEL}</span><Icon name={folded() ? 'chevronRight' : 'chevronDown'} size={12}/>
      </button>
      <RailViewButtons/>
      <button type="button" class="icon-btn" aria-label="New quick chat" title="New quick chat (⌘N)" disabled={!canLeaveConversation()} onClick={() => void startQuickChat()}><Icon name="plus" size={15}/></button>
    </div>
    <Show when={!folded()}>
      <Show when={selecting()}><RailBulkBar visible={shownKeys}/></Show>
      {/* Index, not For: the groups are rebuilt on every change, and keying them by reference would
          remount every row (losing focus and a rename in progress). */}
      <Index each={groups()}>{g => <>
        <Show when={g().label}><div class="work-rail-subhead">{g().label}</div></Show>
        <For each={g().rows}>{localRow}</For>
      </>}</Index>
      <For each={remote()}>{({ row, stale }) =>
        <div class="work-tree-head" data-session-key={row.key} {...railRowFileTarget(() => null, 'Files cannot attach to a conversation on another Mac from this row.')} onContextMenu={e => openRemoteRowMenu(e, row.home, row, stale, () => void openRemoteChat(row.home.machineId, row.key))}>
          <button data-key-item type="button" class="work-nav-row work-workspace-row" classList={{ 'is-stale': stale }} title={`${row.title?.trim() || 'New chat'} · on ${row.home.name}`}
            onClick={e => { if (!e.ctrlKey) void openRemoteChat(row.home.machineId, row.key); }}>
            <SessionIndicator state={row.needsAnswer ? 'waiting' : row.state === 'running' ? 'running' : 'idle'} size={8} decorative/><span class="work-workspace-title">{row.title?.trim() || 'New chat'}</span><MachineBadge machine={row.home} needsAnswer={row.needsAnswer} moved={row.movedFrom ? 'from' : undefined}/>
          </button>
          <MoreButton class="work-row-actions" label={`Actions for chat ${row.title?.trim() || 'New chat'} on ${row.home.name}`} items={() => remoteRowMenuItems(row.home, row, stale, () => void openRemoteChat(row.home.machineId, row.key))}/>
        </div>
      }</For>
    </Show>
  </div>;
}

/** The Working quick chats shelf: rendered with the other shelves (at the end
 *  of the sidebar's list, so keyboard order matches what is drawn). */
export function RailQuickWorking() {
  const { chats, localRow } = rows();
  const folded = () => railFoldedSections().includes('quick-chats');
  const workingFolded = () => !railFoldedSections().includes('working-quick-open');
  return <Show when={!folded() && chats().working.length}>
    <div class="rail-shelf" role="group" aria-label="Working quick chats">
      <button data-key-item type="button" class="work-rail-section-toggle" aria-expanded={!workingFolded()} aria-label={`Working quick chats (${chats().working.length})`} onClick={() => toggleRailSection('working-quick-open')}><span class="work-rail-label">Working ({chats().working.length})</span><Icon name={workingFolded() ? 'chevronRight' : 'chevronDown'} size={12}/></button>
      <Show when={!workingFolded()}><For each={chats().working}>{localRow}</For></Show>
    </div>
  </Show>;
}
