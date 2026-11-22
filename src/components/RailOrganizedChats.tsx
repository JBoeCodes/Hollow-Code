import { For, Show, createEffect, createMemo, onCleanup, onMount, type JSX } from 'solid-js';
import { activePage, canLeaveConversation, chatAgentSessionKey, conversationHistoryShown, sidebarCollapsed } from '../store';
import { conversationOrganization, organizationError, organizationNow, setAwakeOrganizationProjects, setHiddenOrganizationProjects, setOrganizationNow } from '../store/conversationOrganization';
import { refreshConversationOrganization } from '../conversationOrganizationLoad';
import { openSessionByKey, quickChatsWorkspace, workspaceId, workspaceSessions, workspaceSessionsRead } from '../store/workspaces';
import { pinnedWorkspaces } from '../store/workspacePins';
import { railFoldedSections, toggleRailSection } from '../store/prefs';
import { organizationGroup, type ConversationOrganization } from '../conversationOrganization';
import { runningSince, sessionState, workspaceRollup } from '../store/sessionStates';
import { RunTimer } from './RunTimer';
import { RowMarks } from './RowMarks';
import { Renamable, renameChatFromRow } from './RowRename';
import { setRenamingKey } from '../store/rowRename';
import { historyMenuItems } from '../workspaceMenu';
import { MoreButton } from './MoreButton';
import { Icon } from '../icons';
import { SessionIndicator } from './SessionIndicator';
import { QUICK_CHATS_ID } from '../workspaceTypes';
import { checkSettlement } from '../conversationSettlement';
import { railChatDragSource } from '../railChatDrag';
import { railRowSummary, withRailDetails } from '../railRowDetails';
import { railRowFileTarget } from '../railRowFiles';

/** `shelves`: more shelves drawn with Snoozed and Settled, ahead of them. */
export default function RailOrganizedChats(props: { shelves?: JSX.Element }) {
  // Read after each finished sessions read, not before the first: at launch
  // that read is already on its way (every workspace list read starts one),
  // so reading at mount too doubled the launch call (launch-ipc.spec.ts).
  createEffect(() => { if (workspaceSessionsRead() > 0) void refreshConversationOrganization(); });
  onMount(() => { const timer = setInterval(() => setOrganizationNow(Date.now()), 1000); onCleanup(() => clearInterval(timer)); });
  onMount(() => { const timer = setInterval(() => void checkSettlement(), 60_000); onCleanup(() => clearInterval(timer)); });
  const current = (key: string) => chatAgentSessionKey() === key && (activePage() === 'today' || conversationHistoryShown());
  const latest = createMemo(() => new Map(Object.values(workspaceSessions()).flatMap(group => group.sessions).map(s => [s.key, s])));
  createEffect(() => {
    const hidden = new Set<string>(), awake = new Set<string>();
    for (const [id, sessions] of Object.entries(workspaceSessions())) {
      const key = sessions.projectAgentKey, row = key ? conversationOrganization()[key] : undefined;
      if (!workspaceRollup(id) && organizationGroup(row, latest().get(key ?? '') ?? row?.session, 'idle', current(key ?? '') || (activePage() === 'today' && workspaceId() === id), organizationNow()) !== 'active') hidden.add(id);
      else if (row?.snoozedUntil) awake.add(id);
    }
    const same = (a: ReadonlySet<string>, b: ReadonlySet<string>) => a.size === b.size && [...a].every(id => b.has(id));
    setHiddenOrganizationProjects(old => same(old, hidden) ? old : hidden);
    setAwakeOrganizationProjects(old => same(old, awake) ? old : awake);
  });
  onCleanup(() => { setHiddenOrganizationProjects(new Set<string>()); setAwakeOrganizationProjects(new Set<string>()); });
  const rows = createMemo(() => {
    const pinnedLeads = new Set(pinnedWorkspaces().map(id => workspaceSessions()[id]?.projectAgentKey));
    for (const [id, group] of Object.entries(workspaceSessions())) if (workspaceRollup(id)) pinnedLeads.add(group.projectAgentKey);
    return Object.values(conversationOrganization()).filter(row => !pinnedLeads.has(row.key));
  });
  const groupOf = (row: ConversationOrganization) => organizationGroup(row, latest().get(row.key) ?? row.session, sessionState(row.key), current(row.key), organizationNow());
  const ready = (row: ConversationOrganization) => !!row.snoozedUntil && !current(row.key) && !Object.values(workspaceSessions()).some(s => s.projectAgentKey === row.key) && !workspaceSessions()[QUICK_CHATS_ID]?.sessions.some(s => s.key === row.key) && row.session.cwd !== quickChatsWorkspace()?.cwd;
  const names = { active: 'Ready chats', pinned: 'Pinned chats', snoozed: 'Snoozed', settled: 'Settled' } as const;
  const group = (group: 'active' | 'pinned' | 'snoozed' | 'settled') => {
      // Keep the saved row objects: a clock tick must not replace focused DOM.
      const listed = () => rows().filter(row => groupOf(row) === group && (group !== 'active' || ready(row)));
      const folded = () => group === 'pinned' || group === 'active' ? railFoldedSections().includes(`organized-${group}`) : !railFoldedSections().includes(`organized-${group}`);
      // Snoozed and Settled are shelves: the sidebar anchors them at its bottom (workbench.css).
      return <Show when={listed().length}><div role="group" aria-label={names[group]} classList={{ 'rail-shelf': group === 'snoozed' || group === 'settled' }}>
        <button data-key-item type="button" class="work-rail-section-toggle" aria-expanded={!folded()} onClick={() => toggleRailSection(`organized-${group}`)}><span class="work-rail-label">{names[group]} ({listed().length})</span><Icon name={folded() ? 'chevronRight' : 'chevronDown'} size={12}/></button>
        <Show when={!folded()}><For each={listed()}>{row => {
          const label = () => row.session.title?.trim() || 'New chat';
          const detail = () => row.snoozedUntil ? `Snoozed until ${new Date(row.snoozedUntil).toLocaleString()}` : row.reason ?? 'Pinned conversation';
          const rename = () => setRenamingKey(row.key);
          const summary = () => latest().get(row.key) ?? row.session;
          const items = () => withRailDetails(historyMenuItems(summary(), { archived: row.settled, rename, open: () => openSessionByKey(row.key), openBlocked: canLeaveConversation() ? null : 'wait: the current message is still being sent', after: refreshConversationOrganization }), summary());
          return <div class="work-tree-head" data-session-key={row.key} data-rail-motion-key={row.key} {...railChatDragSource(() => row.key, label, items)} {...railRowSummary(summary)} {...railRowFileTarget(() => row.key)}>
            <Renamable id={row.key} value={label()} label={`Rename ${label()}`} commit={n => renameChatFromRow(row.key, n, refreshConversationOrganization)}>
              <button data-key-item type="button" class="work-nav-row work-workspace-row" title={`${label()} · ${detail()}`} disabled={!canLeaveConversation()} onClick={() => void openSessionByKey(row.key)} onDblClick={rename}><SessionIndicator state={sessionState(row.key)} size={8} decorative/><span class="work-workspace-title">{label()}</span><RunTimer since={sessionState(row.key) === 'running' ? runningSince(row.key) : null}/><RowMarks session={latest().get(row.key) ?? row.session} here={current(row.key)} merged={row.autoSettle?.pr ?? null}/></button>
            </Renamable>
            <MoreButton class="work-row-actions" label={`Actions for chat ${label()}`} items={items}/>
          </div>;
        }}</For></Show>
      </div></Show>;
  };
  return <Show when={!sidebarCollapsed()}>
    <Show when={organizationError()}><button class="btn-link" onClick={() => void refreshConversationOrganization()}>Retry loading organized chats</button></Show>
    {/* Ready and Pinned stay in the list; the shelves (Working, Snoozed, Settled)
        come last in the DOM, so keyboard order is the order drawn. */}
    <For each={['active', 'pinned'] as const}>{group}</For>
    {props.shelves}
    <For each={['snoozed', 'settled'] as const}>{group}</For>
  </Show>;
}
