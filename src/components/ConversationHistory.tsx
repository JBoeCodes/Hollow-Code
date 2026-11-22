import { For, Show, Suspense, createEffect, createMemo, createResource, createSignal, createUniqueId, lazy, onCleanup, onMount } from 'solid-js';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { ResizeHandle } from './ResizeHandle';
import { ChatBody } from './ChatBody';
import { Icon } from '../icons';
import { MoreButton } from './MoreButton';
import {
  chatAsideWidth, setChatAsideWidth, persistChatAsideWidth, CHAT_ASIDE_MIN, CHAT_ASIDE_MAX,
  chatTurns, chatSessionId,
  chatCwd, setChatCwdPersistent, chatBusy, chatAgent, AGENT_LABELS,
  chatSessionLoading, setChatSessionLoading, chatConversationVersion,
  resetChatState, applyNewChatPermissionMode, requestChatFocus, chatAgentSessionKey, type ChatAgent, type ChatTurn,
  setChatPermissionMode, setChatSessionIdPersistent,
  flashFooter, openContextMenu, type ContextMenuItem, openSkills,
  chatQueuedTurn, ensureQueuedTurnVisible,
} from '../store';
import { fmtTime } from '../utils';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { conversationPermission } from '../permModes';
import { windowAtMost } from '../windowLayout';
import type { AgentSessionSummary } from '../types';
import { executionReceiptFromWire } from '../executionReceipt';
import { unsafeCwdText } from '../chatCwd';
import { appSessionRequest, appSessionsTick, setAppSessionRequest } from '../appBridge';
import { SESSION_REQUEST_BUSY, historyWorkspace, sessionRequestStep, setHistoryWorkspace } from '../store/sessionRequest';
import { sessionDragSource } from '../dragdrop';
import { moveBlockedReason, workspaces } from '../store/workspaces';
import { restoreConversation } from '../conversationActions';
import { organizeConversation } from '../conversationOrganizationActions';
import { conversationOrganization } from '../store/conversationOrganization';
import { conversationAbout } from '../conversationAbout';
import { isMainAgent, isQuickChats } from '../workspaceTypes';
import { oneMainConversation } from '../chatChainWords';
import { openHistoryConversation, readHistoryConversation, retireLegacyHistorySelection, showHistoryTurns } from '../store/historyOpen';
import { conversationBusy, historyMenuItems } from '../workspaceMenu';
import '../styles/chat-workspace.css';
import { PlainErrorText, Segmented } from './ui';
import { steerRowsOf } from '../chatSteer';
import { savedTurnAt, savedUserText } from '../chatHydrate';
import { peerMachines } from '../store/peerSessions';
import { libraryMachineRequest, takeLibraryMachineRequest } from '../store/peerUi';
import { libraryMacs } from './machineStrip';

// M2 leftovers: another Mac's chats ("On <Mac>", and in a project's list).
const RemoteConversations = lazy(() => import('./RemoteConversations'));

// Conversation history: Library → Conversations since S4 (2026-11-03), moved
// here from the hidden page pages/Chat.tsx (page id `chat`, which now opens
// this view). Hollow's own conversations only: the Claude history tab (Claude
// Code's .jsonl transcripts) was removed with From other apps in run 15 P4.
// Its Hollow · Archived switch replaced Show archived.
type HistoryEntry = { key: string; title: string; lastActivity: string; cwd: string | null; agent: string; hollow: AgentSessionSummary };
const entryOf = (h: AgentSessionSummary): HistoryEntry => ({ key: h.key, title: h.title || 'Untitled conversation', lastActivity: h.lastActivityAt, cwd: h.cwd, agent: h.agent, hollow: h });

export function ConversationHistory() {
  const [filter, setFilter] = createSignal('');
  const [archived, setArchived] = createSignal(false);
  const [historyOpen, setHistoryOpen] = createSignal(!windowAtMost('compact'));
  // "On <Mac>" (M2 leftovers): that paired Mac's chats instead of this Mac's.
  const [onMac, setOnMac] = createSignal<string | null>(null);
  const macs = createMemo(() => libraryMacs(peerMachines()));
  // A Mac forgotten meanwhile goes back to this Mac's list.
  createEffect(() => { const id = onMac(); if (id && peerMachines() && !macs().some(m => m.machineId === id)) setOnMac(null); });
  // The machine strip's Open all in Library.
  createEffect(() => {
    if (!libraryMachineRequest()) return;
    const id = takeLibraryMachineRequest();
    if (!id) return;
    setArchived(false); setOnMac(id); setHistoryOpen(true);
  });
  const [loadError, setLoadError] = createSignal('');
  const [selectedSession, setSelectedSession] = createSignal<HistoryEntry | null>(null);
  // One workspace's, from the rail's "N older" (P1-42): its own list reaches
  // past the 300 newest of all; archived ones come from the shared list.
  const [ownedSessions, { refetch: refreshOwned }] = createResource(() => ({ archived: archived(), agents: appSessionsTick(), ws: historyWorkspace() }),
    value => value.ws && !value.archived ? api.readWorkspace(value.ws).then(d => d.sessions) : api.listAgentSessions(300, value.archived));
  const scope = createMemo(() => workspaces().find(w => w.id === historyWorkspace()));
  // A filter for a workspace that no longer exists lists everything again.
  createEffect(() => { if (historyWorkspace() && workspaces().length && !scope()) setHistoryWorkspace(null); });
  onCleanup(() => setHistoryWorkspace(null));
  const loads = latestRequest();
  let disposed = false;
  // A Claude history row left open before that tab went (run 15 P4).
  retireLegacyHistorySelection();
  let restorePending = !!(chatAgentSessionKey() || chatSessionId()) && chatTurns.length === 0;
  if (restorePending && !chatBusy()) {
    setChatSessionLoading(true);
    setChatPermissionMode(conversationPermission(chatAgentSessionKey()!).mode); setChatSessionIdPersistent(null);
  }
  let wasBusy = chatBusy();

  // One Main agent conversation (2026-11-07): its newest stands for it here;
  // the earlier ones open from the Main page's "Earlier in this conversation".
  const mainCwd = createMemo(() => workspaces().find(w => isMainAgent(w))?.cwd ?? null);
  const entries = createMemo<HistoryEntry[]>(() => {
    const listed = (ownedSessions() ?? []).filter(h => !scope() || h.cwd === scope()!.cwd);
    const main = mainCwd();
    // A search still finds the earlier ones by title (review of #1220).
    return (archived() || !main || filter().trim() ? listed : oneMainConversation(listed, h => h.cwd === main)).map(entryOf);
  });
  const isSelected = (s: HistoryEntry) => chatAgentSessionKey() === s.key;
  // A conversation outside any workspace handed over by Home, Activity or
  // an agent's navigate(session) (PR A4, B·F15): opened by its exact key,
  // from the summary the sender found when this list (the 300 newest
  // unarchived) doesn't carry it; a key no conversation has says so.
  createEffect(() => {
    const want = appSessionRequest();
    if (!want) return;
    const listed = ownedSessions.state === 'errored' ? [] : ownedSessions.latest;
    // openSession does nothing while a turn runs, and the sender already
    // reported success: say so instead of leaving the other one on screen.
    const running = chatBusy() ? chatAgentSessionKey() ?? '' : null;
    const step = sessionRequestStep(want, listed, running);
    if (step === 'wait') return;
    setAppSessionRequest(null);
    if (step === 'shown') return;
    if (step === 'running') { flashFooter(SESSION_REQUEST_BUSY, 5000); return; }
    if ('open' in step) { void openSession(entryOf(step.open)); return; }
    void api.sessionTarget(step.lookUp).then(target => {
      if (disposed) return;
      if (!target) setLoadError(`No conversation has the key ${step.lookUp}.`);
      else if (chatBusy() && chatAgentSessionKey() !== target.session.key) flashFooter(SESSION_REQUEST_BUSY, 5000);
      else void openSession(entryOf(target.session));
    }).catch(err => { if (!disposed) setLoadError(String(err)); });
  });
  const filteredSessions = createMemo(() => {
    const q = filter().trim().toLowerCase();
    return entries().filter(s => !q || [s.title, s.cwd, s.agent].some(value => value?.toLowerCase().includes(q)));
  });
  const sessionGroups = createMemo(() => {
    const out: { label: string; items: HistoryEntry[] }[] = [];
    const day = new Date(); day.setHours(0, 0, 0, 0);
    const d0 = day.getTime();
    for (const s of filteredSessions()) {
      const t = new Date(s.lastActivity).getTime();
      const label = t >= d0 ? 'Today' : t >= d0 - 86400000 ? 'Yesterday' : t >= d0 - 6 * 86400000 ? 'This week' : 'Earlier';
      const group = out[out.length - 1];
      if (group?.label === label) group.items.push(s); else out.push({ label, items: [s] });
    }
    return out;
  });
  // Plan L3 (2026-10-14; review V05, matrix X10): the header used to say
  // "Workspace conversation" for every row, quick chats, the Main agent and
  // vault chats included. It names the kind, and "About this conversation"
  // says who owns it, where it is kept and how it continues.
  const about = createMemo(() => {
    const cwd = chatCwd();
    const ws = cwd ? workspaces().find(w => w.cwd === cwd) : undefined;
    const sel = selectedSession();
    const session = sel && isSelected(sel) ? sel.hollow ?? null : null;
    return conversationAbout({
      kind: !ws ? 'vault' : isQuickChats(ws) ? 'quick' : isMainAgent(ws) ? 'main' : 'workspace',
      workspaceTitle: ws?.title, session, agent: chatAgent(), nativeSession: !!chatSessionId(), turns: chatTurns.length,
    });
  });
  // The listed entry first, so a rename from the title's ⋯ shows at once (P1-39).
  const conversationTitle = createMemo(() => entries().find(isSelected)?.title || (selectedSession() && isSelected(selectedSession()!) ? selectedSession()!.title : null) || chatTurns.find(t => t.role === 'user')?.text || 'New conversation');
  createEffect(() => {
    const busy = chatBusy();
    if (wasBusy && !busy) void refreshOwned();
    wasBusy = busy;
  });
  createEffect(() => {
    if (restorePending && ownedSessions.state === 'errored') {
      setLoadError(String(ownedSessions.error)); setChatSessionLoading(false);
      return;
    }
    const ready = ownedSessions();
    if (!restorePending || !ready || chatBusy()) return;
    restorePending = false;
    const meta = entries().find(isSelected);
    if (meta && chatTurns.length === 0) void loadSessionTurns(meta);
    else if (chatAgentSessionKey() && chatTurns.length === 0) {
      // The current conversation may be archived or outside the recent page.
      // Its durable key remains readable; a listing omission is not deletion.
      const key = chatAgentSessionKey()!;
      const request = loads.start();
      const version = chatConversationVersion();
      const current = () => !disposed && loads.isLatest(request) && version === chatConversationVersion() && chatAgentSessionKey() === key && !chatBusy();
      setChatSessionLoading(true);
      setChatPermissionMode(conversationPermission(key).mode); setChatSessionIdPersistent(null);
      void api.readAgentSession(key).then(async detail => {
        if (!current()) return;
        await loadSessionTurns(entryOf(detail.summary));
      }).catch(err => { if (current()) setLoadError(String(err)); })
        .finally(() => { if (!disposed && loads.isLatest(request)) setChatSessionLoading(false); });
    }
  });
  // A queued turn that never started has no stored turn row, so a history
  // restore (or a reload that adopted it from the run queue) would not show
  // it. Put its bubble back at the end once the transcript has settled.
  createEffect(() => {
    if (chatQueuedTurn() && !chatSessionLoading()) ensureQueuedTurnVisible();
  });
  onCleanup(() => { disposed = true; loads.cancel(); setChatSessionLoading(false); });

  async function loadSessionTurns(s: HistoryEntry) {
    const request = loads.start();
    const version = chatConversationVersion();
    setSelectedSession(s); setLoadError(''); setChatSessionLoading(true);
    const current = () => !disposed && loads.isLatest(request) && version === chatConversationVersion() && isSelected(s) && !chatBusy();
    try {
      const detail = await readHistoryConversation(s.key, current);
      if (!detail || !current()) return;
      const turns: ChatTurn[] = detail.turns.flatMap(t => [...steerRowsOf(t), { role: t.role, ...savedUserText(t),
        tools: t.tools.map(tool => ({ ...tool, result: tool.result ?? undefined })),
        receipt: t.receipt ? executionReceiptFromWire(t.receipt) : undefined,
        agent: t.agent,
        // Exact identity (turns.run_id), for saved feedback (PR 17).
        runId: t.runId ?? undefined,
        // Continue (Paperclip PR 14b) reads the last turn's status and kind.
        status: t.status,
        resultKind: t.resultKind ?? t.receipt?.resultKind ?? null,
        stoppedBeforeSpawn: t.stoppedBeforeSpawn ?? false,
        // When you sent it (C3), shown under your message.
        at: savedTurnAt(t),
        reasoning: t.reasoning || undefined,
      } as ChatTurn]);
      // These notices describe earlier turns, so Continue looks past them.
      if (detail.turns.some(t => t.transcriptIncomplete)) turns.push({ role: 'system', text: 'Some older transcript content is unavailable. The retained conversation and run records are shown here.', tools: [], hydratedNotice: true });
      for (const t of detail.turns.filter(t => t.error)) turns.push({ role: 'system', text: t.error!, tools: [], hydratedNotice: true });
      if (!current()) return;
      showHistoryTurns(turns);
    } catch (err) { if (current()) setLoadError(err instanceof Error ? err.message : String(err)); }
    finally { if (!disposed && loads.isLatest(request)) setChatSessionLoading(false); }
  }
  function newChat() {
    if (chatBusy()) return;
    restorePending = false; loads.cancel(); resetChatState(); applyNewChatPermissionMode(); setArchived(false);
    setLoadError(''); setSelectedSession(null);
    if (windowAtMost('compact')) setHistoryOpen(false);
    requestChatFocus();
  }
  async function openSession(s: HistoryEntry) {
    if (chatBusy()) return;
    if (isSelected(s) && !chatSessionLoading() && !loadError()) { if (windowAtMost('compact')) setHistoryOpen(false); return; }
    restorePending = false;
    // A conversation left running in a workspace comes back attached to its
    // live stream, still busy, rather than as a stored copy (review A·D4).
    const how = openHistoryConversation({
      key: s.hollow.key, agent: s.agent, cwd: s.cwd ?? null,
      sessionId: s.hollow.resumable === 'native' ? s.hollow.harnessSessionId : null,
    });
    if (!how) return;
    if (windowAtMost('compact')) setHistoryOpen(false);
    if (how === 'live') { loads.cancel(); setSelectedSession(s); setLoadError(''); return; }
    await loadSessionTurns(s);
  }
  // Opening another is off while the conversation on screen runs.
  const openBlocked = () => chatBusy() ? 'the conversation on screen is working' : null;
  /** A history row's menu: right-click, or its visible ⋯ (L4, N3): the one
   *  conversation menu (P1-39). */
  function sessionItems(s: HistoryEntry): ContextMenuItem[] {
    return historyMenuItems(s.hollow, { archived: archived(), open: () => openSession(s), openBlocked: openBlocked(), after: refreshOwned });
  }
  function sessionMenu(e: MouseEvent, s: HistoryEntry) {
    openContextMenu(e, sessionItems(s));
  }
  /** A row's own button: Archive or Restore. */
  function rowAction(s: HistoryEntry) {
    if (archived()) return { label: 'Restore conversation', icon: 'history' as const, off: false, run: () => restoreConversation(s.key, refreshOwned) };
    const why = conversationBusy(s.key) || (conversationOrganization()[s.key]?.pinned ? 'unpin this conversation first' : null);
    return { label: why ? `Settle conversation (${why})` : 'Settle conversation', icon: 'archive' as const, off: !!why, run: () => organizeConversation(s.key, 'settle', refreshOwned) };
  }
  // The ⋯ beside the open conversation's title (P1-39): its history row's menu.
  const openEntry = () => entries().find(isSelected);
  async function pickCwd() {
    if (chatBusy() || chatSessionLoading()) return;
    try {
      const picked = await openDialog({ directory: true, multiple: false,
        defaultPath: chatCwd() ?? undefined, title: 'Choose working directory' });
      if (typeof picked !== 'string' || !picked || chatBusy() || chatSessionLoading()) return;
      // Paperclip PR 19: a system or shared folder is refused, not set.
      try {
        await api.chatPreflight(picked);
      } catch (err) {
        const unsafe = unsafeCwdText(err);
        if (unsafe) { flashFooter(`× ${unsafe}`, 6000); return; }
        throw err;
      }
      if (!chatBusy() && !chatSessionLoading()) setChatCwdPersistent(picked);
    } catch (err) { console.debug('cwd picker:', err); }
  }

  const headingId = createUniqueId();
  return (
    <section class="library-conversations" aria-labelledby={headingId}>
      <header class="library-conversations-head">
        <h2 id={headingId}>Conversation history</h2>
        <div class="chat-header-actions">
          <button class="btn small ghost chat-history-toggle" aria-label={historyOpen() ? 'Hide conversations' : 'Show conversations'}
            aria-expanded={historyOpen()} aria-controls="chat-conversations" onClick={() => setHistoryOpen(v => !v)}>
            <Icon name="message" size={14} /><span>Conversations</span>
          </button>
          <button class="panel-link chat-skills-link" onClick={() => openSkills()}>Skills</button>
          <button class="btn small primary" onClick={newChat} disabled={chatBusy()}><Icon name="plus" size={14} />New chat</button>
        </div>
      </header>
      <div class="split chat-split chat-workspace" classList={{ 'history-open': historyOpen() }}>
        <Show when={historyOpen()}>
          <aside id="chat-conversations" class="split-aside chat-aside" aria-label="Conversations" style={{ '--chat-aside-w': `${chatAsideWidth()}px` }}>
            <ResizeHandle current={chatAsideWidth} onResize={setChatAsideWidth} onCommit={persistChatAsideWidth} min={CHAT_ASIDE_MIN} max={CHAT_ASIDE_MAX} />
            <div class="chat-history-heading"><strong>Conversations</strong><Show when={!onMac()}><span>{entries().length}</span></Show></div>
            <div class="chat-history-sources">
              <Segmented level="secondary" label="Conversation list" value={onMac() ? `mac:${onMac()}` : archived() ? 'archived' : 'hollow'}
                onChange={v => { setOnMac(v.startsWith('mac:') ? v.slice(4) : null); setArchived(v === 'archived'); }}
                items={[{ value: 'hollow', label: 'Hollow' }, { value: 'archived', label: 'Archived' }, ...macs().map(m => ({ value: `mac:${m.machineId}`, label: m.label }))]} />
            </div>
            <Show when={scope()}>{w => <div class="chat-history-sources" role="status">Only {w().title}<button class="btn small ghost" onClick={() => setHistoryWorkspace(null)}>Show all</button></div>}</Show>
            <Show when={ownedSessions.error}><div class="chat-load-error" role="alert">Couldn’t load conversations.<button class="btn small" onClick={() => refreshOwned()}>Retry</button></div></Show>
            <div class="split-search chat-history-search">
              <Icon name="search" size={14} />
              <input aria-label="Search conversations" placeholder="Search conversations…" value={filter()} onInput={e => setFilter(e.currentTarget.value)} />
              <Show when={filter()}><button type="button" aria-label="Clear search" onClick={() => setFilter('')}>×</button></Show>
            </div>
            <div class="split-list">
              <Show when={onMac()} fallback={<>
              <Show when={filteredSessions().length > 0} fallback={
                <div class="chat-history-empty"><Icon name="message" size={22} /><strong>{filter() ? 'No conversations found' : 'No conversations yet'}</strong>
                  <p>{filter() ? 'Try another title or folder.' : 'Conversations with every agent live here.'}</p>
                </div>
              }>
                <For each={sessionGroups()}>{group => <>
                  <div class="rail-group-label chat-session-group-label">{group.label}</div>
                  <For each={group.items}>{s =>
                    <div class="session-row" classList={{ active: isSelected(s) }} onContextMenu={e => sessionMenu(e, s)}>
                      <button type="button" class="chat-session-open" aria-label={`Open ${s.title}`} aria-current={isSelected(s) ? 'true' : undefined}
                        disabled={chatBusy()} onClick={() => openSession(s)}
                        {...sessionDragSource(() => ({ kind: 'session', key: s.key }), () => moveBlockedReason(s.key))}>
                        <span class="title">{s.title}</span>
                        <span class="meta"><span class="proj">{AGENT_LABELS[s.agent as ChatAgent] ?? s.agent}</span><span>{fmtTime(s.lastActivity)}</span></span>
                      </button>
                      <button type="button" class="session-delete" title={rowAction(s).label} aria-label={rowAction(s).label} disabled={rowAction(s).off} onClick={e => { e.stopPropagation(); void rowAction(s).run(); }}><Icon name={rowAction(s).icon} size={13} /></button>
                      <MoreButton class="session-more" label={`Actions for ${s.title}`} items={() => sessionItems(s)} />
                    </div>
                  }</For>
                </>}</For>
              </Show>
              {/* A project's list: its chats on each paired Mac too. */}
              <Show when={!archived() && scope() && macs().length > 0}>
                <Suspense fallback={null}><RemoteConversations workspaceId={scope()!.id} query={filter()} /></Suspense>
              </Show>
              </>}>
                {id => <Suspense fallback={null}><RemoteConversations machineId={id()} workspaceId={scope()?.id ?? null} query={filter()} /></Suspense>}
              </Show>
            </div>
            <div class="chat-cwd chat-aside-foot">
              <div class="chat-cwd-label">Working folder</div>
              <button type="button" class="chat-cwd-display" onClick={pickCwd} disabled={chatBusy() || chatSessionLoading()} title={chatCwd() ?? 'The Hollow folder'}>
                <Icon name="folder" size={14} /><span class="chat-cwd-path">{chatCwd()?.split('/').filter(Boolean).pop() ?? 'Hollow folder'}</span><Icon name="chevronRight" size={12} />
              </button>
              <Show when={chatCwd()}><button type="button" class="chat-cwd-reset" disabled={chatBusy() || chatSessionLoading()} onClick={() => setChatCwdPersistent(null)}>Reset to Hollow folder</button></Show>
            </div>
          </aside>
        </Show>
        <div class="chat-main">
          <div class="chat-conversation-head">
            <div class="chat-conversation-heading"><div class="chat-conversation-title"><h3 title={conversationTitle()}>{conversationTitle()}</h3><Show when={openEntry()}>{s => <MoreButton label="Actions for this conversation" items={() => sessionItems(s())} />}</Show></div><span>{AGENT_LABELS[chatAgent()]}<span aria-hidden="true"> · </span>{chatBusy() ? 'Working' : chatSessionLoading() ? 'Opening conversation' : about().kind}</span>
              <details class="chat-conversation-about"><summary>About this conversation</summary><dl>
                <For each={about().rows}>{row => <div><dt>{row.label}</dt><dd>{row.detail}</dd></div>}</For>
              </dl></details>
            </div>
          </div>
          <Show when={loadError()}><div class="chat-load-error" role="alert"><span>Couldn’t open this conversation. <PlainErrorText text={loadError()} /></span><button class="btn small" onClick={() => { if (selectedSession()) void loadSessionTurns(selectedSession()!); else { restorePending = true; void refreshOwned(); } }}>Retry</button></div></Show>
          <ChatBody />
        </div>
      </div>
    </section>
  );
}
