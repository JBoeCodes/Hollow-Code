import { For, Show, Suspense, createEffect, createMemo, createSignal, createUniqueId, lazy, on, onCleanup, onMount, type JSX } from 'solid-js';
import { observeChatMotion } from './ChatMotion';
import { DictateButton } from './DictateButton';
import { dictationSent } from '../store/dictation';
import { fieldElement, isRichField, type ComposerField } from '../composerField';
import { sendThenBackground } from '../composerActions';
import { composerPlaceholder, type ComposerDestination } from '../composerChip';
import { ChatMessage } from './ChatMessage';
import { ThinkingRow } from './ThinkingRow';
import { transcriptWarning } from '../transcriptWarning';
import { DoorwayLandscape } from './DoorwayLandscape';
import { visualExplanationPrompt } from '../visualExplain';
import { effectiveCanvases, settingCanvases } from '../canvasFrequency';
import { ToolLedger } from './ToolLedger';
import { describeTool } from './ToolCall';
import { ExecutionReceipt } from './ExecutionReceipt';
import { chatHomes } from '../store/moveFlags';
import { isOfflineCopy } from '../store/offlineFlags';
import { liveSwitchNotices, switchRows } from '../accountSwitchRows';
import { LiveTurnFooter, TurnFooter } from './TurnFooter';
import { ReplyMedia } from './ReplyMedia';
import { endsReply, replyQuestion, replyText, typicalLine } from '../turnFooter';
import { ComposerResources, createTurnConnectors } from './ConversationConnectors';
import { Icon } from '../icons';
import { sideChatOpen, toggleSideChatChecked } from '../store/sideChat';
import { SHARED_PAGE_LABEL, sharedPageSummary } from '../sharedPage';
import { SteerActions } from './SteerActions';
import { MainAgentIndicator } from './SessionIndicator';
import { enterSends } from '../store/prefs';
import { ChatModelMenu } from './ChatModelMenu';
import { ChatUsageMenu } from './ChatUsageMenu';
import { selfHostedChatEntry, wakeFromComposer } from '../store/selfHostedWake';
import { QuestionDock, QuestionsWaiting } from './QuestionDock';
import { createQuestionDock } from '../questionDock';
import { ThreadVisitWatermark } from './chat/ThreadVisitWatermark';
import { createVisitMark } from './chat/visitMark';
import '../styles/chat-workspace.css';
import '../styles/conversation-nav.css';
import { TURN_WINDOW, branchIntoNewChat, conversationMarkdown, conversationNoteTitle, findInTurns, forThisChat, handoffText, lastUserIndex } from '../conversationTools';
import { readAloud, readAloudSupported, readingKey, stopReading } from '../store/readAloud';
import { clearFindMarks, paintFindMarks, sameHits, touchesFindText } from '../findHighlight';
import { workspaceId as openWorkspaceId, workspaceDetail as openWorkspaceDetail, newWorkspaceSessionOpened, startQuickChat, railGitStates, pickConversation } from '../store/workspaces';
import { modalOpen } from '../modalGuard';
import { StatusPill } from './ui';
import { RUN_STAGE, runStage } from '../workState';
import { openAgentUpdates } from '../store/agentHealth';
import {
  chatTurns,
  chatSessionId,
  chatAgentSessionKey,
  chatCwd,
  chatBusy,
  chatInput, setChatInput,
  chatAgent, AGENT_LABELS, currentAgentCapabilities, agentSettings, steerSupportNote, steeringAgent,
  chatAttachments as attachments, setChatAttachments as setAttachments, nextChatAttachmentId, registerChatImageSink,
  chatSessionLoading, chatConversationVersion, fileClick,
  chatPermissionMode, chatCanvases,
  toggleToolExpanded,
  chatQueuedTurn, withdrawQueuedChatTurn, stopChatRun,
  canPauseChatRun, pauseChatRun, chatPausingHere, chatPausedHere, chatResumingHere, resumePausedChat,
  chatHistory, chatHistoryCursor, setChatHistoryCursor,
  chatFocusRequest, chatFocusRequestedWithin, requestChatFocus,
  chatPrefillRequest, setChatPrefillRequest,
  flashFooter, flashWithUndo,
  pageContext, chatContextEnabled, toggleChatContext,
  operatorProfile, operatorContextEnabled,
  teamworkEnabled, agentsAvailable, activePage, switchChatAgent, openSettingsFromOutside,
  chatModel, setChatModelPersistent,
  chatArtifacts, openChatPreview, openFileInSidePane,
  pendingApprovals, approvalsResponding, respondToApproval, approvalTool,
  chatInteractions,
  chatLiveRunId,
  chatRunStartedAt,
  type ChatTurn,
  type ChatAgent,
  heldChipsFor, removeHeldChip, returnHeldToBox, chatStopKey, sameChatAsNow,
  pushChatTurn,
} from '../store';
import { ActionMenu } from './ActionMenu';
import '../styles/chat-steer.css';
import { busyChoiceLabel, busyHint, busyMenuItems, busyPlaceholder, steerStateText, steerTime, stopTitle } from '../chatSteer';
import { HeldChips } from './HeldChips';
import { chainCardKeysFor, chatContinuedIn } from '../store/chatChainState';
import { approvalOriginLabel, approvalPlacement, approvalWaitingLabel } from '../chatApprovalOrigin';
import { cooChatShown, openMainAgent } from '../store/mainAgent';
import { conversationDrafts } from '../store/conversationDrafts';
import type { ChatAttachment } from '../store/chatRuntime';
import { api, type TurnCheckpoint, type TurnCheckpoints } from '../api';
import { showConfirm } from '../store/overlays';
import { workspaceDetail, chooseMoveTarget, moveBlockedReason } from '../store/workspaces';
import { sendsAsMessage, submitBlockReason } from '../questionCards';
import { hollowCardsNotice } from '../hollowCards';
import type { CannedRequest } from '../types';
import { shipOffer } from '../shipActions';
import { splitNextFence } from '../chatSegments';
import { addToDraft, restoredDraft } from '../chatQueue';
import { proposalRequest, proposalUsable } from '../planProposal';
import { queuedTurnLabel } from '../runQueue';
import { unsafeCwdText } from '../chatCwd';
import { FOLDER_REFUSED, carriesFiles, takeComposerDrop, type DropLabel } from '../composerDrop';
import { addToSourcesNotice, attachRefusal, attachmentsHoldSend, attachmentSavingLabel, LARGE_PASTE_CHARS, fileSizeLabel, isImageFile, messageCapRefusal, pendingAttachment } from '../composerFiles';
import { errMsg } from '../utils';
import { chooseAndAttachSource } from '../store/sourceAttach';
import { escOwnerOpen, escStopsRun } from '../escStop';
import { retainedOffscreen } from '../retainedPage';
import { activeChord, keyFor } from '../shortcuts';
import { foldKeepsText, foldRoles, foldSteps, foldSummary, foldTail, runFolds, type RunFold } from '../runFold';
import { canVote } from '../replyActions';
import { ReplyFileChips } from './ReplyFileChips';
import { CARET_IDLE_MS, showsCaret } from '../streamCaret';
import '../styles/chat-frontier.css';
import { FrameBudgetContext } from '../canvas/frameBudgetScope';
import { visibleUserText } from '../canvas/askAnswer';
import { mainTurnKind, wakeText } from '../mainAgent';
import { regenerateIndex, retryIndex } from '../chatContinue';
import { loadCatalog } from '../modelCatalog';
import { retryModelChoices, type MenuModel } from '../modelMenu';
import { caretEdges, editIndex, recallAnchor, recallList, repeatable, sentTime } from '../messageActions';
import { ConversationTitle } from './ConversationTitle';
import { Portal } from 'solid-js/web';
import { topBarSlot } from '../store/topBar';
import { previewCardShown } from '../store/preview';
import type { ContextMenuItem } from '../store/overlays';
import { QuoteSelection } from './QuoteSelection';
import { findKeepsOpen, findShowsWhole, foldsUserText } from '../userMessageFold';
import { registerCanvasAnswers, registerCanvasComposer, type CanvasAnswerRequest, type CanvasAnswerResult, type CanvasDraftRequest } from './ChatCanvas';
import type { DraftRefusal } from '../canvasBridge';
import { ComposerCommands } from './ComposerCommands';
import { ComposerAccessChip } from './ComposerAccessChip';
import { ComputerChip } from './ComputerChip';
import { createChatScroll } from './chat/chatScrollController';
import { createSlashCommands, type SlashCmd } from './chat/slashCommands';
import { fileMention, mentionAt, mentionedFiles, replaceMention, type MentionChoice } from '../composerMentions';
import { buildCatalog, skillAvailability, useSkillPrompt } from '../skillCatalog';
import { vaultRoot } from '../store/chatRuntime';
import { inVault } from '../projectFiles';
import type { Skill } from '../types';
import { createChatSend, preflightChatContext } from './chat/chatSendController';
import { cannedSend, type CannedDraft } from '../chatQueue';
import { RunsOnControl } from './RunsOnControl';

/** A self-hosted chat's start chip (Q6): loaded only for such a chat. */
const SelfHostedStatus = lazy(() => import('./SelfHostedStatus').then(m => ({ default: m.SelfHostedStatus })));
const ProposedPlan = lazy(() => import('./ProposedPlan'));
// One conversation (2026-11-07): the banner of a chat that continued elsewhere
// and the current one's earlier chats, lazy (the App chunk budget).
const ChatChainBar = lazy(() => import('./ChatChainBar'));
const ChatMoveNotice = lazy(() => import('./ChatMoveNotice'));
const ChatBranchNotice = lazy(() => import('./ChatBranchNotice'));
const QuickChatMoveOffer = lazy(() => import('./QuickChatMoveOffer'));
const ChatOutlineRail = lazy(() => import('./ChatOutlineRail'));
const SidePanelCard = lazy(() => import('./SidePanelCard').then(m => ({ default: m.SidePanelCard })));
// P1c: a paused chat's "Resume at..." control, loaded only once one is paused.
const ResumeAtControl = lazy(() => import('./chat/ResumeAtControl'));
const ComposerMarkdown = lazy(() => import('../editor/MarkdownEditor').then(m => ({ default: m.MarkdownEditor })));
const LearningLine = lazy(() => import('./Learnings').then(m => ({ default: m.LearningLine })));

/**
 * Composer + transcript for the Claude Code chat. Owns input behavior
 * (send, slash popup, prompt history, key handling) but reads/writes
 * chat state from the global store — so the full /chat page and the
 * right-side ChatPanel can both render this without forking state.
 *
 * `compact` tightens padding for the narrower side-panel layout.
 *
 * Three controllers own the stateful domains (plan L3 X17, 2026-10-14;
 * review V06/F11), each created once, synchronously, in the body below: the
 * transcript's scrolling (chat/chatScrollController.ts: render window,
 * jumps, scroll memory, follow, canvas focus, settle, reading anchor), the
 * slash commands (chat/slashCommands.ts) and sending
 * (chat/chatSendController.ts: send, the queued dispatch, Continue, question
 * cards, Stop, the activity line). This component keeps the JSX, the
 * composer's keys and suggestions, find and the outline, attachments,
 * folds and the canvas draft.
 */

export function ChatBody(props: { compact?: boolean; workspace?: boolean }) {
  const [popupIndex, setPopupIndex] = createSignal(0);
  const [dragOver, setDragOver] = createSignal(false);
  // Refused steers already sent again (P1-30): their actions go, so once;
  // they come back if the send didn't happen (review).
  const [resentSteers, setResentSteers] = createSignal(new Set<string>());
  const resendSteer = (id: string, text: string) => {
    setResentSteers(s => new Set(s).add(id));
    void sendAsNext(text).then(sent => { if (!sent) setResentSteers(s => { const left = new Set(s); left.delete(id); return left; }); });
  };
  const [dropLabel, setDropLabel] = createSignal<DropLabel>('Drop files to attach');
  // Replies whose full receipt is shown under the footer, by run id (or
  // turn index for a turn without one).
  const [openDetails, setOpenDetails] = createSignal<ReadonlySet<string>>(new Set());
  const detailsKey = (t: ChatTurn, index: number) => t.runId ?? `turn-${index}`;
  const toggleDetails = (key: string) => setOpenDetails(prev => {
    const next = new Set(prev);
    if (next.has(key)) next.delete(key); else next.add(key);
    return next;
  });
  // Finished runs fold their steps into one "Worked for …" row (frontier F5,
  // runFold.ts). Opened folds are remembered by the head turn's run id (or
  // index) until the conversation changes.
  const folds = createMemo(() => runFolds(chatTurns, { busy: chatBusy(), liveRunId: chatLiveRunId() }));
  // Usual length of a finished chat of this folder's work kind (K4). Null
  // until three of that kind have finished. The elapsed clock stays the
  // fold's or the footer's; this only adds the usual time beside it.
  // Reread when the folder changes and when a turn starts or finishes
  // (`chatBusy`), so the third finished chat of a kind shows up. Streaming
  // stays busy, so this does not reread on every token.
  const [jobTiming, setJobTiming] = createSignal<{ workKind: string | null; typicalMs: number | null } | null>(null);
  createEffect(on(() => [chatCwd(), chatBusy()] as const, ([cwd]) => {
    if (!cwd) { setJobTiming(null); return; }
    let current = true;
    void api.jobTiming(cwd).then(
      row => { if (current) setJobTiming(row); },
      () => { if (current) setJobTiming(null); },
    );
    onCleanup(() => { current = false; });
  }));
  const usualTime = () => typicalLine(jobTiming()?.typicalMs, jobTiming()?.workKind);
  // Each row's fold role, once per fold change (overnight C5), not a loop over
  // every fold per row.
  const foldRoleOf = createMemo(() => foldRoles(folds(), chatTurns));
  // The caret follows text that is actually arriving: each growth of the last
  // partial assistant turn keeps it on for CARET_IDLE_MS (streamCaret.ts).
  const [typing, setTyping] = createSignal(false);
  let typingTimer: ReturnType<typeof setTimeout> | undefined;
  createEffect(on(() => {
    const last = chatTurns[chatTurns.length - 1];
    return last && last.role === 'assistant' && last.partial ? last.text.length : -1;
  }, (len, prev) => {
    if (len < 0) { clearTimeout(typingTimer); setTyping(false); return; }
    if (prev !== undefined && len !== prev) {
      setTyping(true);
      clearTimeout(typingTimer);
      typingTimer = setTimeout(() => setTyping(false), CARET_IDLE_MS);
    }
  }));
  onCleanup(() => clearTimeout(typingTimer));
  const foldKey = (f: RunFold) => chatTurns[f.start]?.runId ?? `fold-${f.start}`;
  const [openFolds, setOpenFolds] = createSignal<ReadonlySet<string>>(new Set());
  createEffect(() => { chatConversationVersion(); chatAgentSessionKey(); setOpenFolds(new Set<string>()); });
  const toggleFold = (key: string) => setOpenFolds(prev => {
    const next = new Set(prev);
    if (next.has(key)) next.delete(key); else next.add(key);
    return next;
  });
  // Long messages of yours opened with Show more (overnight 11-04, a C3
  // follow-up; userMessageFold.ts), by the send's id (or index, for a saved
  // turn), until the conversation changes; never saved. Here rather than in
  // each row, so Find can open the one it was on as it closes (closeFind).
  const textKey = (t: ChatTurn, index: number) => t.clientTurnId ?? `turn-${index}`;
  const [openTexts, setOpenTexts] = createSignal<ReadonlySet<string>>(new Set());
  createEffect(() => { chatConversationVersion(); chatAgentSessionKey(); setOpenTexts(new Set<string>()); });
  const setTextOpen = (key: string, open: boolean) => setOpenTexts(prev => {
    if (prev.has(key) === open) return prev;
    const next = new Set(prev);
    if (open) next.add(key); else next.delete(key);
    return next;
  });
  /** The head turn of the fold holding `turnIndex` (where its chips and
   *  "Worked for" line render), or -1. A head before the render window
   *  isn't drawn, so the footer keeps the chips then (C2). */
  function foldHeadOf(turnIndex: number): number {
    for (const f of folds().values()) if (f.start <= turnIndex && turnIndex <= f.end) return f.start;
    return -1;
  }
  /** Open the finished run's fold that holds this turn (a canvas drawn
   *  before a tool call sits in it, and a closed fold renders nothing).
   *  Returns the first turn that must render to show it: the fold's head. */
  function openFoldAt(turnIndex: number): number {
    const role = foldRoleOf().get(turnIndex);
    const fold = role === 'hidden' || role === 'head' ? [...folds().values()].find(f => f.start <= turnIndex && turnIndex <= f.end) : undefined;
    if (!fold) return turnIndex;
    setOpenFolds(prev => new Set(prev).add(foldKey(fold)));
    return fold.start;
  }
  let textareaEl: ComposerField | undefined;
  const [richComposer, setRichComposer] = createSignal(false);
  let pendingComposerSelection: { from: number; to: number; version: number } | undefined;
  function toggleRichComposer() {
    if (textareaEl) dictationSent(textareaEl);
    pendingComposerSelection = { from: textareaEl?.selectionStart ?? chatInput().length, to: textareaEl?.selectionEnd ?? chatInput().length, version: chatConversationVersion() };
    setRichComposer(on => !on);
  }
  function composerReady(field: ComposerField) {
    textareaEl = field;
    watchShown(field);
    const selection = pendingComposerSelection;
    pendingComposerSelection = undefined;
    queueMicrotask(() => {
      if (textareaEl !== field || retainedOffscreen(fieldElement(field))) return;
      growTextarea();
      if (selection && selection.version === chatConversationVersion()) {
        field.focus();
        field.setSelectionRange(Math.min(selection.from, field.value.length), Math.min(selection.to, field.value.length));
      }
    });
  }
  // A footer is the latest when no user turn follows it (overnight C5: one
  // pass, not a slice of the rest per rendered turn).
  const lastUser = createMemo(() => lastUserIndex(chatTurns));
  // ⌘F's mount comes before the scroll controller's, as it did: mounts keep
  // their order (⌘F, scroll intent, reading anchor, follow, then Esc). It
  // only reads the scroller and opens find when a key is pressed.
  onMount(() => {
    // ⌘F / Ctrl+F finds in the conversation on screen. In-surface command:
    // it only fires while this conversation is visible and no modal is open.
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey || e.key.toLowerCase() !== 'f') return;
      const el = scrollEl();
      if (modalOpen() || !el || !el.offsetParent || chatTurns.length === 0) return;
      e.preventDefault(); openFind();
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });
  // The transcript's scrolling (chat/chatScrollController.ts). Created here,
  // where its first region (the frame budget, then the render window) used to
  // start: everything registered above (the caret and fold effects) still
  // runs before it, and its session-switch effect still precedes find.
  // `lastUser` (a pure memo, moved up)
  // must exist first: a mount mid-turn reads it at setup. Its later regions
  // (scroll memory, follow, canvas focus, settle) now register before find's
  // effect, the image sink and Esc's mount, which were interleaved with them;
  // none of those shares their state, so the order isn't observable.
  const { frameScope, scrollEl, setScrollEl, onScroll, atBottom, finishedBelow, renderFrom, renderTo, lastOnScreen, showEarlier, showLater, loadAtEdge, jumpToTurn, jumpToLatest, followSend, toggleInPlace, focusChip, focusTitle, backToCanvas, dismissFocusChip, setChipFocused } =
    createChatScroll(props, { lastUser, openFoldAt, textarea: () => textareaEl });
  // "New since you were here": the divider for turns that arrived while this conversation was closed.
  const visit = createVisitMark({ key: chatAgentSessionKey, total: () => chatTurns.length, loading: chatSessionLoading, atBottom, lastOnScreen, version: chatConversationVersion });
  // The divider goes before the first turn that draws at or after the first new one (a folded step draws
  // nothing of its own), and no earlier than the window's start.
  const visitRow = createMemo(() => {
    const m = visit.mark();
    if (!m) return null;
    for (let i = Math.max(m.index, renderFrom()); i < renderTo(); i++) if ((foldRoleOf().get(i) ?? 'normal') !== 'hidden') return i;
    return null;
  });
  const [findOpen, setFindOpen] = createSignal(false);
  const [findQuery, setFindQuery] = createSignal('');
  // Equal by value (overnight 11-04, C4): a streamed token rebuilt the list,
  // so the jump below went back to the first match on every token.
  const findHits = createMemo(() => findOpen() ? findInTurns(chatTurns, findQuery()) : [], [], { equals: sameHits });
  // While Find searches, long messages of yours show whole and their Show
  // more steps aside, so no match is hidden under a fold (userMessageFold.ts).
  // A plain function, read by each folded row.
  const findUnfolds = () => findShowsWhole(findOpen(), findQuery());
  const [findAt, setFindAt] = createSignal(0);
  const [outlineOpen, setOutlineOpen] = createSignal(false);
  function stepFind(delta: number) {
    const hits = findHits(); if (!hits.length) return;
    const at = (findAt() + delta + hits.length) % hits.length;
    setFindAt(at); jumpToTurn(hits[at]);
  }
  // A changed hit list keeps the match you are on when it is still a hit
  // (overnight 11-04, C4): a reply streaming in no longer pulls you back to
  // the first match. Otherwise it starts at the first.
  // A hit is a turn index, so another conversation swapped in (a
  // background chat restored) never keeps the old one's place. Keyed by the
  // session, since a mode or agent switch bumps the version, not the chat.
  let findConversation = chatAgentSessionKey();
  createEffect(on(findHits, (hits, prev) => {
    const sameConversation = findConversation === chatAgentSessionKey();
    findConversation = chatAgentSessionKey();
    const was = sameConversation ? prev?.[findAt()] : undefined;
    const keep = was == null ? -1 : hits.indexOf(was);
    if (keep >= 0) { setFindAt(keep); return; }
    setFindAt(0); if (hits.length) jumpToTurn(hits[0]);
  }, { defer: true }));
  let findInput: HTMLInputElement | undefined;
  function openFind() { setFindOpen(true); setOutlineOpen(false); queueMicrotask(() => { findInput?.focus(); findInput?.select(); }); }
  /** Find is closing: the long message of yours it was on stays open, so
   *  the match it took you to isn't folded away (userMessageFold.ts). */
  function keepFindPlace() {
    const at = findKeepsOpen(chatTurns, findHits(), findAt());
    if (at != null) setTextOpen(textKey(chatTurns[at], at), true);
  }
  function closeFind() { keepFindPlace(); setFindOpen(false); setFindQuery(''); textareaEl?.focus(); }
  // The matched words themselves (overnight 11-04, C4; findHighlight.ts),
  // repainted on the next frame after the searched text changes while the
  // bar is open (a jump that shows earlier turns, a streaming reply, an
  // opened fold); a footer's clock ticking doesn't count.
  createEffect(() => {
    const el = scrollEl();
    if (!findOpen() || !el) { clearFindMarks(); return; }
    const query = findQuery();
    const current = findHits()[findAt()] ?? null;
    let frame = requestAnimationFrame(() => { frame = 0; paintFindMarks(el, query, current); });
    const watch = new MutationObserver(records => {
      if (frame || !touchesFindText(records)) return;
      frame = requestAnimationFrame(() => { frame = 0; paintFindMarks(el, query, current); });
    });
    watch.observe(el, { subtree: true, childList: true, characterData: true });
    onCleanup(() => { cancelAnimationFrame(frame); watch.disconnect(); });
  });
  onCleanup(clearFindMarks);
  // CP1: this chat's turn checkpoints, read when the chat changes and when a
  // turn ends (none outside its own isolated worktree).
  const [checkpoints, setCheckpoints] = createSignal<TurnCheckpoints | null>(null);
  createEffect(on(() => [chatAgentSessionKey(), chatLiveRunId()] as const, ([key, live]) => {
    setCheckpoints(null);
    if (!key || live) return;
    void api.turnCheckpoints(key).then(c => { if (chatAgentSessionKey() === key) setCheckpoints(c); }).catch(() => {});
  }));
  function restoreAction(runId?: string): (() => void) | undefined {
    const c = checkpoints();
    const cp = c?.isolated && runId && !chatLiveRunId() ? c.turns.find(t => t.runId === runId) : undefined;
    return cp ? () => void restoreTo(cp) : undefined;
  }
  /** Restore to before a turn (CP1): asks, restores, and says so in the chat. */
  async function restoreTo(cp: TurnCheckpoint) {
    const key = chatAgentSessionKey();
    if (!key) return;
    const changed = cp.files.length === 1 ? '1 file' : `${cp.files.length} files`;
    const ok = await showConfirm({
      title: `Restore to before turn ${cp.ord}?`,
      body: `This chat's worktree goes back to how it was before turn ${cp.ord} (${changed} changed in that turn), and that turn and every later one are undone. The agent starts a fresh session without them; they stay in this transcript. The current state is saved in a git ref first.`,
      confirmLabel: 'Restore',
      danger: true,
    });
    if (!ok || chatAgentSessionKey() !== key) return;
    try {
      const done = await api.restoreTurnCheckpoint(key, cp.ord);
      pushChatTurn({ role: 'system', text: done.text, tools: [] });
    } catch (e) {
      pushChatTurn({ role: 'system', text: `Restore refused: ${String(e)}`, tools: [] });
    }
    const c = await api.turnCheckpoints(key).catch(() => null);
    if (chatAgentSessionKey() === key) setCheckpoints(c);
  }
  /** CH1: Branch from here, on a reply with a run (the turn it ended). */
  function branchHereAction(runId?: string): (() => void) | undefined {
    const key = chatAgentSessionKey();
    if (!props.workspace || !openWorkspaceId() || !key || !runId || chatLiveRunId() === runId) return undefined;
    return () => {
      if (chatAgentSessionKey() !== key) return;
      const title = openWorkspaceDetail()?.sessions.find(x => x.key === key)?.title ?? 'this conversation';
      void import('../store/chatBranch').then(m => m.branchChat(key, runId, title, checkpoints()?.isolated ?? null));
    };
  }
  /** CH1: the open chat is a branch (its notice shows). */
  const isBranch = () => openWorkspaceDetail()?.sessions.find(x => x.key === chatAgentSessionKey())?.forkKind === 'branch';
  /** Continue from a turn in a new conversation: a written handoff, labelled. */
  async function branchFrom(idx: number) {
    const wid = openWorkspaceId();
    if (!wid) return;
    const title = openWorkspaceDetail()?.sessions.find(x => x.key === chatAgentSessionKey())?.title ?? 'this conversation';
    const text = handoffText(chatTurns, idx, { title, agentLabel: AGENT_LABELS[chatAgent()] });
    // Refused (a send in its preflight): this chat's draft stays as it was (ENG-78).
    await branchIntoNewChat(() => newWorkspaceSessionOpened(wid), () => {
      // After the workspace's draft, never over it (morning M5).
      setChatInput(addToDraft(chatInput(), text));
      requestAnimationFrame(() => textareaEl?.focus());
    });
  }
  /** The placeholder names the project where the message goes. */
  const destination = (): ComposerDestination => {
    const d = openWorkspaceDetail();
    if (!props.workspace || !d || d.workspace.cwd !== chatCwd()) return { kind: 'vault' };
    if (d.workspace.kind === 'quick') return { kind: 'quick' };
    if (d.workspace.kind === 'main') return { kind: 'main' };
    return { kind: 'project', title: d.workspace.title };
  };
  /** How the last run ended, for the recovery row: a live stop notice or a
   *  restored turn's status. */
  const endedStage = () => {
    const last = chatTurns[chatTurns.length - 1];
    if (!last) return null;
    if (last.role === 'system' && last.stopNotice) return 'stopped' as const;
    if (last.resultKind === 'stalled') return 'interrupted' as const;
    return runStage({ status: last.status ?? null });
  };
  // The agent that answered last, when the composer now targets another one:
  // the next turn gets the written conversation, not the other provider's session.
  const providerSwitch = () => {
    const last = [...chatTurns].reverse().find(t => t.role === 'assistant' && t.agent);
    return last && last.agent !== chatAgent() ? last.agent as ChatAgent : null;
  };
  let fileInputEl: HTMLInputElement | undefined;
  // Passive connector status for the two-line resource menu.
  const turnCx = createTurnConnectors(() => ({ agent: chatAgent(), surface: 'chat' as const, perm: chatPermissionMode() }));

  // Read an image File → preview data-URL + persist to disk (async) so the
  // next prompt can hand Claude a path to Read. Its chip shows (Saving…,
  // blocking Send) before the read starts (C5 review follow-up).
  function addImageFile(file: File) {
    if (!file.type.startsWith('image/')) return;
    const version = chatConversationVersion();
    const chip = pendingAttachment(nextChatAttachmentId(), file, false);
    setAttachments(a => [...a, chip]);
    saveAttachmentFile(file, chip.id, chip.name, version, true, dataUrl => settleChip(chip.id, { dataUrl }));
  }
  // C5 (2026-11-04): a file that isn't a thumbnail image attaches as a chip.
  // It is saved like an image (same folder, cap and refusals) under a name
  // that keeps its own, and its line at send gives the agent both.
  function addOtherFile(file: File) {
    const version = chatConversationVersion();
    const chip = pendingAttachment(nextChatAttachmentId(), file, true);
    setAttachments(a => [...a, chip]);
    saveAttachmentFile(file, chip.id, chip.name, version, false);
  }
  /** Update chip `id` here and in a stored draft, if its chat was left (C-D1). */
  function settleChip(id: number, patch: Partial<ChatAttachment>) {
    setAttachments(a => a.map(x => (x.id === id ? { ...x, ...patch } : x)));
    conversationDrafts.settle(id, patch);
  }
  /** Read `file` and save it for chip `id`: an image without a name (its
   *  saved name is generated), any other file with its name. A switch of
   *  conversation, a failed read or a refused save leaves the chip Failed. */
  function saveAttachmentFile(file: File, id: number, name: string, version: number, image: boolean, onRead?: (dataUrl: string) => void) {
    const fail = (reason?: string) => {
      settleChip(id, { saving: false, error: true });
      if (reason) flashFooter(`× ${name} didn't attach: ${reason}`, 4000);
    };
    const reader = new FileReader();
    reader.onprogress = e => { if (e.lengthComputable) settleChip(id, { readPercent: Math.min(99, Math.floor(e.loaded / e.total * 100)) }); };
    reader.onerror = () => fail("it couldn't be read");
    reader.onload = async () => {
      if (version !== chatConversationVersion()) { fail(); return; }
      const dataUrl = String(reader.result || '');
      const comma = dataUrl.indexOf(',');
      if (comma < 0) { fail("it couldn't be read"); return; }
      onRead?.(dataUrl);
      settleChip(id, { readPercent: 100 });
      try {
        // Resolve before writing so attachment persistence cannot recreate a
        // deleted worktree and make the send-side cwd guard trust it again.
        // Do not reset global session state under an already-running turn;
        // the fail-closed saver will reject if its cwd vanished mid-run.
        if (!chatBusy()) await preflightChatContext();
        if (version !== chatConversationVersion()) { fail(); return; }
        const data = dataUrl.slice(comma + 1);
        const path = image
          ? await api.chatSaveAttachment(data, file.type, chatCwd() ?? undefined)
          : await api.chatSaveAttachment(data, file.type || 'application/octet-stream', chatCwd() ?? undefined, name);
        // The saver marks actual conversions, regardless of the input's MIME
        // or name. Replace the source bytes, MIME and accounting on every route.
        if (/-heic-converted\.png$/.test(path)) {
          const converted = await api.chatPreviewFile(path);
          if (converted.kind !== 'image' || !converted.dataUrl) throw new Error('Converted image could not be read');
          if (version !== chatConversationVersion()) { fail(); return; }
          const refused = messageCapRefusal(attachments().filter(a => a.id !== id), { name, size: converted.size });
          if (refused) { fail(refused); return; }
          settleChip(id, { path, dataUrl: converted.dataUrl, mime: 'image/png', file: false, size: converted.size, saving: false });
          return;
        }
        settleChip(id, { path, saving: false });
      } catch (err) {
        // An unsafe working folder (Paperclip PR 19) says why, not just "failed".
        const unsafe = unsafeCwdText(err);
        if (unsafe) {
          settleChip(id, { saving: false, error: true });
          flashFooter(`× ${unsafe}`, 6000);
        } else fail(errMsg(err));
      }
    };
    reader.readAsDataURL(file);
  }
  /** Any file the + menu, a drop, a paste or Annotate brings (C5): a folder,
   *  an empty file or one over 20 MB is refused in words before it is read,
   *  and so is one that would take the message past 10 attachments or 50 MB;
   *  a PNG, JPEG, GIF or WebP takes the image path above; any other file
   *  attaches as a chip. `knownFile`: a drop's entry said it is a file, or
   *  the picker gave it (a picker never returns a folder). False when it
   *  was refused (Annotate and Ask to fix read that). */
  function attachFile(file: File, knownFile = false): boolean {
    const refused = attachRefusal(file, knownFile) ?? messageCapRefusal(attachments(), file);
    if (refused) { flashFooter(refused, 5000); return false; }
    if (isImageFile(file)) addImageFile(file);
    else addOtherFile(file);
    return true;
  }
  function removeAttachment(id: number) { setAttachments(a => a.filter(x => x.id !== id)); }
  // The side panel browser's Annotate hands its marked-up screenshot here,
  // and a file dropped anywhere in the window comes here too (2026-11-04,
  // windowFileDrop.ts). On screen: laid out, and neither on a page behind
  // another (hidden) nor under a panel that covers the conversation (inert).
  let composerEl: HTMLDivElement | undefined;
  const composerOnScreen = () => !!composerEl?.isConnected && composerEl.getClientRects().length > 0 && !composerEl.closest('[inert],[hidden]');
  onCleanup(registerChatImageSink((file, known) => attachFile(file, known), composerOnScreen));

  // Explicit attach button → hidden file input. Same path as paste/drop.
  function onFilePick(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const files = input.files;
    if (files) Array.from(files).forEach(f => attachFile(f, true));
    input.value = '';
  }

  function onPaste(e: ClipboardEvent) {
    const items = e.clipboardData?.items ?? [];
    const files: File[] = [];
    for (let i = 0; i < items.length; i++) {
      const it = items[i];
      if (it.kind === 'file') {
        const f = it.getAsFile();
        if (f) files.push(f);
      }
    }
    if (files.length === 0) {
      const text = e.clipboardData?.getData('text/plain') ?? '';
      if (text.length < LARGE_PASTE_CHARS) return;
      // Refusal leaves the clipboard in the message box's normal paste path.
      if (attachFile(new File([text], 'pasted-text.txt', { type: 'text/plain' }), true)) {
        e.preventDefault();
        flashFooter('Long paste attached as pasted-text.txt. Remove its chip to discard it.', 5000);
      }
      return;
    }
    e.preventDefault();
    files.forEach(f => attachFile(f));
  }
  // A drop with files is always the composer's (P0-07): every file attaches
  // (C5), a folder is refused in words, and the webview never opens a file.
  function onDrop(e: DragEvent) {
    setDragOver(false);
    takeComposerDrop(e, { attach: (f, known) => attachFile(f, known), refuse: refuseDroppedFiles });
  }
  function onDragOver(e: DragEvent) {
    // `types`, not only `items`: WebKit may list no items until the drop.
    if (carriesFiles(e.dataTransfer)) {
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
      setDropLabel('Drop files to attach');
      setDragOver(true);
    }
  }
  /** The workspace this chat's Sources belong to; none in a quick chat. */
  const sourcesWorkspace = () => {
    const w = openWorkspaceId(); const d = openWorkspaceDetail();
    return props.workspace && w && d?.workspace.id === w && d.workspace.cwd === chatCwd() && d.workspace.kind !== 'quick' ? w : null;
  };
  /** + → Add to Sources… (C5): in a workspace chat, a file can still go to
   *  the workspace's Sources instead of one message. */
  async function addToSources(id = sourcesWorkspace()) {
    if (!id) return;
    // chooseAndAttachSource throws (a file over the Sources limit, an
    // unreadable one); the person reads why.
    const notice = await addToSourcesNotice(() => chooseAndAttachSource(id));
    if (notice) flashFooter(notice.text, notice.ms);
  }
  function refuseDroppedFiles() {
    const id = sourcesWorkspace();
    // An HTML5 drop carries no path, so Add to Sources… opens the picker.
    flashWithUndo({
      message: FOLDER_REFUSED, kind: 'status', ttlMs: 8000,
      ...(id ? { actionLabel: 'Add to Sources…', undo: () => addToSources(id) } : {}),
    });
  }

  // The slash commands (chat/slashCommands.ts): no effects, so where it is
  // created only matters for `followSend` (above).
  const { allCommands, ensureDiscovered, handleLocalCommand, runClaudeAdmin } = createSlashCommands({ followSend });

  const [composerCaret, setComposerCaret] = createSignal(0);
  const [dismissedCompletion, setDismissedCompletion] = createSignal('');
  const [mentionSkills, setMentionSkills] = createSignal<Skill[]>([]);
  const [mentionRoot, setMentionRoot] = createSignal('');
  const [mentionError, setMentionError] = createSignal('');
  const [mentionLoading, setMentionLoading] = createSignal(false);
  const mention = createMemo(() => mentionAt(chatInput(), composerCaret()));
  const mentionKind = createMemo(() => mention()?.kind);
  const completionKey = () => `${chatInput()}\n${composerCaret()}`;
  createEffect(on([mentionKind, chatConversationVersion], ([kind, version]) => {
    let cancelled = false;
    setMentionError('');
    setMentionLoading(!!kind);
    if (kind) {
      const read = kind === '$'
        ? api.listSkills().then(rows => { if (!cancelled && version === chatConversationVersion()) setMentionSkills(rows); })
        : vaultRoot().then(root => { if (!cancelled && version === chatConversationVersion()) setMentionRoot(root ?? ''); });
      read.catch(() => { if (!cancelled) setMentionError('Could not load suggestions. Close and reopen the picker to retry.'); })
        .finally(() => { if (!cancelled) setMentionLoading(false); });
    }
    onCleanup(() => { cancelled = true; });
  }));
  const slashPrefix = createMemo(() => {
    const v = chatInput();
    const m = /^\/(\w*)$/.exec(v);
    return m ? m[1] : null;
  });
  const matches = createMemo<(SlashCmd | MentionChoice)[]>(() => {
    if (dismissedCompletion() === completionKey()) return [];
    const m = mention();
    if (m) {
      if (mentionLoading() || mentionError()) return [];
      const query = m.query.toLowerCase();
      if (m.kind === '$') return buildCatalog(mentionSkills()).filter(s => s.name.toLowerCase().includes(query)).slice(0, 30).map(s => ({
        name: '$' + s.name, desc: s.description, source: 'skill' as const,
        insert: useSkillPrompt(skillAvailability(s, chatAgent(), agentName()).copy).trimEnd(),
      }));
      const detail = workspaceDetail();
      const files = detail?.workspace.cwd === chatCwd() && mentionRoot()
        ? detail.files.map(f => ({ name: f.name, path: inVault(mentionRoot(), f.path) })) : [];
      const unique = new Map([...files, ...chatArtifacts()].map(f => [f.path, f]));
      return [...unique.values()].filter(f => `${f.name} ${f.path}`.toLowerCase().includes(query)).slice(0, 30).map(f => ({
        name: '@' + f.name, desc: f.path, source: 'file' as const, insert: fileMention(f.name, f.path),
      }));
    }
    const p = slashPrefix();
    if (p === null) return [];
    const ql = p.toLowerCase();
    return allCommands().filter(c => c.name.slice(1).toLowerCase().startsWith(ql));
  });
  const popupOpen = () => matches().length > 0;
  function openMention(kind: '@' | '$') {
    const prefix = chatInput() + (chatInput() && !/\s$/.test(chatInput()) ? ' ' : '');
    setChatInput(prefix + kind);
    setComposerCaret(prefix.length + 1);
    setDismissedCompletion('');
    queueMicrotask(() => { textareaEl?.focus(); textareaEl?.setSelectionRange(prefix.length + 1, prefix.length + 1); });
  }

  // Overnight 11-04 (C2): short windows (under 560px tall) start the empty
  // box one line tall so the transcript keeps room; taller ones keep two.
  const shortQuery = typeof window.matchMedia === 'function' ? window.matchMedia('(max-height: 560px)') : null;
  const [shortWindow, setShortWindow] = createSignal(!!shortQuery?.matches);
  function growTextarea() {
    const ta = textareaEl;
    if (!ta) return;
    if (isRichField(ta)) {
      const pane = ta.closest('.chat-main')?.getBoundingClientRect().height ?? window.innerHeight;
      ta.style.setProperty('--composer-editor-cap', `${props.compact ? 140 : Math.max(200, Math.round(pane * 0.4))}px`);
      return;
    }
    // An empty box takes its natural height (rows and min-height), so a
    // resize never leaves it at a height measured for another window.
    if (!ta.value) { ta.style.height = ''; ta.style.maxHeight = ''; return; }
    // A hidden composer (Work keeps the chat mounted under the Workspace
    // view) measures 0; keep its size until it shows and is typed in again.
    if (!ta.getClientRects().length) return;
    ta.style.height = 'auto';
    // Long prompts get room, as in ChatGPT and Claude (overnight 11-04, C2):
    // up to 40% of the chat pane, never under the old 200px. Set inline,
    // since the stylesheet caps the box at 200px.
    const pane = ta.closest('.chat-main')?.getBoundingClientRect().height ?? window.innerHeight;
    const cap = props.compact ? 140 : Math.max(200, Math.round(pane * 0.4));
    ta.style.maxHeight = `${cap}px`;
    ta.style.height = `${Math.min(ta.scrollHeight, cap)}px`;
  }
  onMount(() => {
    let frame = 0;
    const regrow = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(growTextarea); };
    const changed = () => { setShortWindow(!!shortQuery?.matches); regrow(); };
    shortQuery?.addEventListener('change', changed);
    window.addEventListener('resize', regrow);
    onCleanup(() => { cancelAnimationFrame(frame); shortQuery?.removeEventListener('change', changed); window.removeEventListener('resize', regrow); });
  });
  // A box filled while hidden (Work keeps the chat mounted under the
  // Workspace view: a canvas's Refresh, a saved draft on a rail pick)
  // regrows once it shows again (overnight 11-04, C2 follow-up).
  let shownWatch: ResizeObserver | undefined;
  onCleanup(() => shownWatch?.disconnect());
  function watchShown(field: ComposerField) {
    const el = fieldElement(field);
    shownWatch?.disconnect();
    if (typeof ResizeObserver === 'undefined') return;
    let shown = el.getClientRects().length > 0;
    shownWatch = new ResizeObserver(() => {
      const now = el.getClientRects().length > 0;
      if (now && !shown) requestAnimationFrame(growTextarea);
      shown = now;
    });
    shownWatch.observe(el);
  }

  // Esc stops the running turn (frontier F6) when focus is in this
  // conversation and nothing else owns Escape. A window capture listener
  // runs before any menu, popup or modal handler, so it still sees them open.
  onMount(() => {
    const onEsc = (e: KeyboardEvent) => {
      // Esc is the default; Settings, Keyboard shortcuts can turn it off
      // (or give Stop a ⌘ chord, which the app's handler runs instead).
      if (e.key !== 'Escape' || activeChord('stop-run') !== 'Escape') return;
      const focus = document.activeElement as HTMLElement | null;
      const el = scrollEl();
      const conversation = el?.closest('.chat-main') ?? el?.parentElement?.parentElement ?? null;
      // A retained conversation out of sight (its Work page behind another
      // page, or covered by its workspace panel; overnight 11-04 U4) never
      // owns Escape, even if WebKit left focus inside it.
      if (retainedOffscreen(conversation)) return;
      // Esc in the question dock hides it; it never stops the run.
      if (focus?.closest('.question-dock')) return;
      const editable = !!focus && (focus.tagName === 'INPUT' || focus.tagName === 'TEXTAREA' || focus.tagName === 'SELECT' || focus.isContentEditable);
      if (!escStopsRun(e, {
        busy: chatBusy(),
        inConversation: !!focus && !!conversation?.contains(focus),
        onComposer: !!textareaEl && focus === fieldElement(textareaEl),
        onOtherField: editable && (!textareaEl || focus !== fieldElement(textareaEl)),
        ownerOpen: escOwnerOpen(),
        composerEmpty: !composerHasMessage(),
      })) return;
      e.preventDefault();
      stopChatRun();
    };
    window.addEventListener('keydown', onEsc, true);
    onCleanup(() => window.removeEventListener('keydown', onEsc, true));
  });

  // Grow the textarea with its content (capped) on every change — typing,
  // prompt-history recall, or the programmatic clear after send.
  createEffect(() => {
    chatInput();
    queueMicrotask(growTextarea);
  });

  // Focus-on-request: Cmd+Shift+C (and any other caller of requestChatFocus)
  // bumps chatFocusRequest; we focus the textarea on every change after the
  // first. queueMicrotask defers until the textarea ref is assigned — when
  // focusChatPanel() opens a closed panel, this effect's first run coincides
  // with the fresh mount, so the ref may not exist synchronously yet.
  let lastFocusReq = chatFocusRequest();
  // A request that came just before this composer mounted (⌘N from Your
  // work opens a quick chat whose conversation loads lazily) still counts.
  if (props.workspace && chatFocusRequestedWithin(1500)) queueMicrotask(() => textareaEl?.focus());
  createEffect(() => {
    const n = chatFocusRequest();
    if (n === lastFocusReq) return;
    lastFocusReq = n;
    // A retained conversation out of sight (overnight 11-04 U4) never takes
    // focus from the page on screen; one shown first (a sidebar row's visit)
    // is on screen by the time this runs.
    queueMicrotask(() => { if (textareaEl && !retainedOffscreen(fieldElement(textareaEl))) textareaEl.focus(); });
  });

  /** A quick chat (the Quick chats workspace): no workspace starters. */
  const quickChat = () => !!props.workspace && workspaceDetail()?.workspace.kind === 'quick' && workspaceDetail()?.workspace.cwd === chatCwd();
  /** The Main agent (main agent M1): its own heading and starters. */
  const mainChat = () => !!props.workspace && workspaceDetail()?.workspace.kind === 'main' && workspaceDetail()?.workspace.cwd === chatCwd();

  // Sending (chat/chatSendController.ts), created where its first effect
  // (the pending first message, below) used to be. The suggestion it reads
  // is defined further down: reached through closures, never at setup.
  const { send, continueOffered, continueRun, submittingCard, cardErrors, submitInteraction, dismissInteraction, stop, queuedBadge, activity, quiet, busyChoice, steerNative, steerWhy, sendWith, sendAsNext, sendAnswers } =
    createChatSend(props, {
      textarea: () => textareaEl,
      // A send in a self-hosted chat wakes its model too (Q6), so the chip
      // shows the start a message waits for.
      followSend: () => { followSend(); wakeFromComposer(); },
      handleLocalCommand,
      suggesting: () => suggesting(),
      suggestion: () => suggestion(),
      placeSuggestion: text => placeSuggestion(text),
      cannedFor: prompt => cannedSend(cannedDraft, prompt),
    });

  // ── Retry (P1-27) ──
  // Under a failed last turn: send its message again, as typed, leaving the
  // draft alone. One send per click: the send marks the chat busy, which
  // hides the row. Another agent switches first; the switch makes the next
  // turn a written handoff (`providerSwitch`).
  // The row shows under any failed last reply to a message of yours (a
  // wake's is Hollow's); its Retry buttons only when the message can go
  // again as it went (C3 review, `repeatable`): not the Continue prompt or a
  // card's or canvas's answers, which went with checks a plain send skips,
  // and not one with images, files or a shared page, which a resend would drop.
  const failedAt = () => retryIndex(chatTurns, { busy: chatBusy() || chatSessionLoading(), queued: !!chatQueuedTurn() }, t => mainTurnKind(t) === 'wake');
  const retryAt = () => retryIndex(chatTurns, { busy: chatBusy() || chatSessionLoading(), queued: !!chatQueuedTurn() }, t => !repeatable(t));
  const retryAgents = () => agentsAvailable().filter(a => a !== chatAgent());
  // Retry with another model (overnight 11-07, D1, audit gap 10): this agent's
  // other models, from the model menu's own catalog (one request per agent per
  // session, shared with it). A signal, not a resource: a resource read under
  // the transcript would suspend it.
  const [catalog, setCatalog] = createSignal<{ models: MenuModel[]; newest: Record<string, string> }>({ models: [], newest: {} });
  createEffect(() => {
    const agent = chatAgent();
    let live = true;
    onCleanup(() => { live = false; });
    setCatalog({ models: [], newest: {} });
    void loadCatalog(agent).then(c => { if (live) setCatalog({ models: c.models, newest: c.newestByFamily ?? {} }); });
  });
  const retryModels = () => retryModelChoices(catalog().models, chatModel(), { newestByFamily: catalog().newest });
  let retrying = false;
  // A model change is the chat's own, like the model menu's: it stays for the
  // messages after this one, as an agent switch does.
  async function retry(agent?: ChatAgent, at = retryAt(), model?: string) {
    if (at < 0 || retrying) return;
    const text = chatTurns[at].text;
    if (agent) switchChatAgent(agent);
    if (model != null) setChatModelPersistent(model);
    retrying = true;
    try { await send(undefined, text); } finally { retrying = false; }
  }
  const retryRecordIndex = () => {
    if (failedAt() < 0) return -1;
    for (let i = chatTurns.length - 1; i >= 0; i--) {
      if (chatTurns[i].role === 'assistant') return chatTurns[i].receipt ? i : -1;
    }
    return -1;
  };
  // Retry under a finished last reply (C3, ChatGPT's regenerate): its
  // footer sends the message again on the same path, and its More menu
  // keeps Retry with another agent. Only for a message that can go again
  // as it went (`repeatable`, as Retry above). It sends the message alone:
  // anything you said while the reply ran (a steer) isn't sent again.
  const regenerate = createMemo(() => regenerateIndex(chatTurns, { busy: chatBusy() || chatSessionLoading(), queued: !!chatQueuedTurn() }, t => !repeatable(t)));

  // ── Edit your last message (C3) ──
  // Puts it back in the box to change and send, before any draft already
  // there (as a queued message's Edit does), with the caret at the end.
  // Hollow can't rewrite history: sending is a new message and the earlier
  // answer stays.
  const editAt = createMemo(() => editIndex(chatTurns, { busy: chatBusy() || chatSessionLoading(), queued: !!chatQueuedTurn() }, t => !repeatable(t)));
  function editMessage(at: number) {
    const text = chatTurns[at]?.text;
    if (!text?.trim()) return;
    setChatInput(restoredDraft(text, chatInput()));
    queueMicrotask(() => {
      if (!textareaEl) return;
      textareaEl.focus();
      const end = textareaEl.value.length;
      textareaEl.setSelectionRange(end, end);
      growTextarea();
    });
  }

  // Quote (overnight 11-07, D1, audit gap 9): a passage selected in the
  // transcript goes into the box as a Markdown quote after any draft, with the
  // caret under it. Nothing is sent.
  function quoteIntoBox(quote: string) {
    setChatInput(addToDraft(chatInput(), quote));
    queueMicrotask(() => {
      if (!textareaEl) return;
      textareaEl.focus();
      const end = textareaEl.value.length;
      textareaEl.setSelectionRange(end, end);
      growTextarea();
    });
  }

  const retryControls = () => <div class="chat-continue chat-retry" role="group" aria-label="The last message failed">
    <StatusPill quiet label={RUN_STAGE.failed.label} tone={RUN_STAGE.failed.tone} hint={RUN_STAGE.failed.hint} />
    <Show when={retryAt() >= 0}>
      <button type="button" class="btn chat-continue-btn" onClick={() => void retry()} title="Send your last message again">Retry</button>
    </Show>
    <button type="button" class="btn ghost chat-continue-btn" onClick={openAgentUpdates}>Agent updates</button>
    <Show when={retryAt() >= 0 && retryAgents().length}>
      <ActionMenu label="Retry with another agent" align="start" icon={<span class="btn ghost chat-continue-btn">Retry with another agent</span>}
        items={retryAgents().map(a => ({ label: AGENT_LABELS[a] ?? a, action: () => void retry(a) }))} />
    </Show>
    <Show when={retryAt() >= 0 && retryModels().length}>
      <ActionMenu label="Retry with another model" align="start" icon={<span class="btn ghost chat-continue-btn">Retry with another model</span>}
        items={retryModels().map(m => ({ label: m.label, action: () => void retry(undefined, undefined, m.id) }))} />
    </Show>
  </div>;

  // Agent actions (Vault note header, etc.) call askClaudeInChat(prompt). We
  // prefill the composer with the composed prompt + focus — the user reviews
  // and hits Enter. Deliberately not auto-sent: an agent run shouldn't fire
  // without an explicit keystroke. Both ChatBody instances (panel + full page)
  // listen, but only the visible one's textarea takes focus.
  createEffect(() => {
    const text = chatPrefillRequest();
    if (text == null) return;
    setChatPrefillRequest(null);
    setChatInput(addToDraft(chatInput(), text));
    queueMicrotask(() => { textareaEl?.focus(); growTextarea(); });
  });

  // ── Suggested next message ──
  // After a finished last reply, the empty composer shows the most likely
  // next message as its placeholder: the agent's own suggestion (its reply's
  // hidden `hollow-next` block), or a ship offer when the reply left a pull
  // request open (Merge and install / Merge and deploy) or merged one without
  // installing (Build and install it). Enter sends those words as if typed,
  // Tab puts them in the box to edit, Esc dismisses, and the other
  // suggestions are links above the box (src/shipActions.ts). It never acts
  // by itself. After a reply that read outside content (web, browser, a
  // connector), and for any line that merges, installs or deploys (P0-01),
  // Enter places the text in the box first and a second Enter sends it
  // (durability D2); everywhere else Enter sends (#248).
  // Install offers only in a Hollow build workspace (P0-01): the build
  // status says whether this workspace's worktree is Hollow's repository.
  // A signal, not a resource: a resource under the reply suspends the transcript.
  // Memoized: a workspace-detail refresh (every turn end) keeps the same id,
  // so the offer doesn't flicker back to Merge while the status is re-read.
  // This chat is the COO's (the Main agent workspace's own conversation): a card raised to the COO is answered here.
  const inCooChat = cooChatShown;
  const buildWorkspace = createMemo(() => { const w = openWorkspaceId(); const d = openWorkspaceDetail(); return w && d?.workspace.cwd === chatCwd() && d.workspace.build?.exists ? w : null; });
  const [hollowBuild, setHollowBuild] = createSignal(false);
  createEffect(on(buildWorkspace, w => {
    setHollowBuild(false);
    if (w) api.appBuildStatus(w).then(b => { if (buildWorkspace() === w) setHollowBuild(!!b.isHollow); }, () => {});
  }));
  const shipActions = createMemo(() => {
    // The PR state Hollow checked for this build workspace (rail git state)
    // outranks the reply's words (next-level P20).
    const w = openWorkspaceId();
    const g = w && openWorkspaceDetail()?.workspace.cwd === chatCwd() ? railGitStates()[w] : undefined;
    return shipOffer(chatTurns, { busy: chatBusy() || chatSessionLoading(), queued: !!chatQueuedTurn(), liveRunId: chatLiveRunId() }, g ? { state: g.prState ?? null, number: g.prNumber } : null, hollowBuild());
  });
  /** Which reply the suggestion belongs to, so Esc dismisses only that one. */
  const suggestKey = () => `${chatSessionId() ?? ''}:${chatTurns.length}`;
  const [dismissedSuggest, setDismissedSuggest] = createSignal<string | null>(null);
  const [suggestPick, setSuggestPick] = createSignal(0);
  const suggestion = createMemo(() => {
    const list = shipActions();
    if (!list.length || dismissedSuggest() === suggestKey()) return null;
    return list[Math.min(suggestPick(), list.length - 1)];
  });
  createEffect(on(suggestKey, () => setSuggestPick(0), { defer: true }));
  /** The suggestion is live only while the composer is empty. */
  const suggesting = () => !!suggestion() && !chatInput() && attachments().length === 0 && !popupOpen();
  /** Send (and Enter) would place a review suggestion in the box, not send it (P0-01). */
  const placesSuggestion = () => suggesting() && !!suggestion()!.review;
  /** Put a suggestion in the box to read and edit (Tab, or Enter on a review suggestion). */
  function placeSuggestion(text: string) {
    setChatInput(text);
    queueMicrotask(() => { textareaEl?.focus(); textareaEl?.setSelectionRange(text.length, text.length); });
  }


  function applySuggestion(cmd: SlashCmd | MentionChoice) {
    if ('insert' in cmd) {
      const range = mention();
      if (!range) return;
      const next = replaceMention(chatInput(), range, cmd.insert);
      setChatInput(next.text);
      setComposerCaret(next.caret);
      queueMicrotask(() => { textareaEl?.focus(); textareaEl?.setSelectionRange(next.caret, next.caret); });
    } else setChatInput(cmd.name + (cmd.hasArgs ? ' ' : ''));
    setPopupIndex(0);
    queueMicrotask(() => textareaEl?.focus());
  }

  // ↑ starts again from the newest message when another conversation opens.
  createEffect(on(chatAgentSessionKey, () => setChatHistoryCursor(null), { defer: true }));

  function onKeyDown(e: KeyboardEvent) {
    if (e.isComposing) return;
    const offer = suggesting() ? suggestion() : null;
    if (offer) {
      const plainEnter = e.key === 'Enter' && enterSends() && !e.shiftKey && !e.altKey;
      if (plainEnter || (e.key === 'Enter' && (e.metaKey || e.ctrlKey))) {
        e.preventDefault();
        if (offer.review) placeSuggestion(offer.prompt);
        else void send(undefined, offer.prompt);
        return;
      }
      if (e.key === 'Tab' && !e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        placeSuggestion(offer.prompt);
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        setDismissedSuggest(suggestKey());
        return;
      }
    }
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      send();
      return;
    }
    if (e.key === 'Escape' && mention() && dismissedCompletion() !== completionKey()) {
      e.preventDefault();
      setDismissedCompletion(completionKey());
      return;
    }
    if (popupOpen()) {
      const list = matches();
      if (e.key === 'ArrowDown') { e.preventDefault(); setPopupIndex(i => (i + 1) % list.length); return; }
      if (e.key === 'ArrowUp')   { e.preventDefault(); setPopupIndex(i => (i - 1 + list.length) % list.length); return; }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
        e.preventDefault();
        const pick = list[popupIndex()] ?? list[0];
        if (pick) applySuggestion(pick);
        return;
      }
      if (e.key === 'Escape') { e.preventDefault(); setDismissedCompletion(completionKey()); return; }
      return;
    }
    // Enter sends; Shift/Option+Enter is a new line (Settings, General can
    // turn this off, leaving ⌘Enter as the only send key).
    if (e.key === 'Enter' && enterSends() && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      send();
      return;
    }
    // Option+Enter has no default action in a textarea: insert the newline
    // ourselves (execCommand keeps it on the undo stack).
    if (e.key === 'Enter' && e.altKey && !e.metaKey && !e.ctrlKey) {
      e.preventDefault();
      if (textareaEl && isRichField(textareaEl)) textareaEl.replaceText(textareaEl.selectionStart, textareaEl.selectionEnd, '\n');
      else document.execCommand('insertText', false, '\n');
      return;
    }

    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      const ta = textareaEl;
      if (!ta) return;
      // Shift extends a selection, ⌥ and ⌘ jump by paragraph or to either end,
      // and an arrow over a selected range collapses it: the box's own, never
      // a recall that replaces or wipes the text (a Codex read).
      if (e.shiftKey || e.altKey || e.metaKey || e.ctrlKey || ta.selectionStart !== ta.selectionEnd) return;
      // Shown lines, not `\n` lines: a wrapped message's second line moves
      // the caret (ENG-93).
      const { atTop, atBottom } = isRichField(ta) ? ta.caretEdges() : caretEdges(ta.value, ta.selectionStart, textHeightIn(ta));
      // This conversation's own messages first, then every chat's earlier
      // prompts (overnight 11-07, D1: Claude's ↑ edits this chat's last
      // message). Read per key press, not as a memo, so a streamed token
      // doesn't read every turn.
      const h = recallList(chatTurns, chatHistory());
      // The place is only as good as what the box shows: a list shifted by a
      // turn that arrived, a Quote put in the box, or another conversation's
      // longer list leave it pointing at something else (a Codex read).
      const place = recallAnchor(h, chatHistoryCursor(), ta.value);
      if (place !== chatHistoryCursor()) setChatHistoryCursor(place);

      if (e.key === 'ArrowUp' && atTop && h.length > 0 && (!ta.value || place != null)) {
        e.preventDefault();
        const nextIdx = place == null ? 0 : Math.min(place + 1, h.length - 1);
        setChatHistoryCursor(nextIdx);
        setChatInput(h[nextIdx]);
        queueMicrotask(() => { if (ta) { ta.selectionStart = ta.selectionEnd = ta.value.length; } });
        return;
      }
      if (e.key === 'ArrowDown' && atBottom) {
        const cur = place;
        if (cur == null) return;
        e.preventDefault();
        if (cur === 0) {
          setChatHistoryCursor(null);
          setChatInput('');
          return;
        }
        const nextIdx = cur - 1;
        setChatHistoryCursor(nextIdx);
        setChatInput(h[nextIdx]);
        queueMicrotask(() => { if (ta) { ta.selectionStart = ta.selectionEnd = ta.value.length; } });
        return;
      }
    }
  }


  // The agent's name heads a reply only when it differs from the previous
  // reply's (Conductor shows none); a conversation that switches agents
  // still says who answered.
  function agentLabelShown(index: number): boolean {
    const turn = chatTurns[index];
    if (turn?.role !== 'assistant' || !turn.agent) return false;
    for (let i = index - 1; i >= 0; i--) {
      const prev = chatTurns[i];
      if (prev.role !== 'assistant') continue;
      // Only the first bubble of a reply can carry the label.
      let first = false;
      for (let j = i + 1; j < index; j++) if (chatTurns[j].role === 'user') first = true;
      if (!first) return false;
      return !!prev.agent && prev.agent !== turn.agent;
    }
    return false;
  }

  async function copyTurn(t: Pick<ChatTurn, 'text'>) {
    try {
      await navigator.clipboard.writeText(splitNextFence(t.text).text);
      flashFooter('Copied', 1000);
    } catch {
      flashFooter('× copy failed', 1500);
    }
  }
  /** The workspace this conversation belongs to (its Results take a saved note). */
  const noteWorkspace = () => props.workspace && openWorkspaceDetail()?.workspace.id === openWorkspaceId() ? openWorkspaceDetail()?.workspace : undefined;
  /** A reply's Save as note (One Ask, 2026-11-03): into this workspace's
   *  Results, or the Library outside one (`store/replyNote.ts`). */
  async function saveReplyNote(index: number) {
    // Everything is read before the lazy import: opening another conversation
    // or workspace while the module loads must not mix this reply with that one.
    const question = replyQuestion(chatTurns, index);
    const text = splitNextFence(replyText(chatTurns, index)).text;
    const workspace = noteWorkspace();
    const { saveReplyAsNote } = await import('../store/replyNote');
    await saveReplyAsNote(text, question == null ? null : visibleUserText(question), workspace);
  }
  const conversationTitle = () => openWorkspaceDetail()?.sessions.find(x => x.key === chatAgentSessionKey())?.title ?? undefined;
  // A live reply carries no agent of its own: it is this conversation's.
  const conversationMd = (title?: string) => conversationMarkdown(chatTurns, { title, agentLabel: a => AGENT_LABELS[(a || chatAgent()) as ChatAgent] ?? 'Agent' });
  /** Conversation tools' Copy conversation (P1-27): every turn, as Markdown. */
  async function copyConversation() {
    const md = conversationMd(conversationTitle());
    try {
      await navigator.clipboard.writeText(md);
      flashFooter('Copied the conversation as Markdown', 1600);
    } catch {
      flashFooter('× copy failed', 1500);
    }
  }
  /** Conversation tools' Save conversation as note (overnight 11-07, D1): the
   *  same Markdown into a note, the way a reply's Save as note goes. */
  async function saveConversation() {
    // The title, the text and the workspace are all read before the lazy
    // import (see `saveReplyNote`): one conversation's text never lands under
    // another's title or in another workspace.
    const title = conversationNoteTitle(conversationTitle(), chatTurns);
    const md = conversationMd();
    const workspace = noteWorkspace();
    const { saveConversationAsNote } = await import('../store/replyNote');
    await saveConversationAsNote(md, title, workspace);
  }
  /** A reply's Read aloud / Stop reading (the reply row's button, `store/readAloud.ts`):
   *  one reply at a time, keyed by its conversation and turn. */
  const readKey = (index: number) => `${chatAgentSessionKey() ?? ''}:${index}`;
  // Another ChatBody can be mounted at once (the quick-chat popover, the Work
  // page behind another page): this view's own going-away and hiding stop only
  // a read it started, through this token (`store/readAloud.ts`).
  const reader = {};
  function toggleReading(index: number) {
    if (readingKey() === readKey(index)) stopReading();
    else readAloud(readKey(index), splitNextFence(replyText(chatTurns, index)).text, reader);
  }
  // The voice stops with the conversation it was reading and with the page (the
  // Work page stays mounted behind the others), whichever view started it, and
  // with the view that started it.
  createEffect(on(chatAgentSessionKey, () => stopReading(), { defer: true }));
  createEffect(on(activePage, () => stopReading(), { defer: true }));
  // A narrow desk can hide this conversation behind the workspace panel: no
  // Stop shows then, so the voice stops when the transcript has no size.
  createEffect(() => {
    const el = scrollEl();
    if (!el || typeof ResizeObserver === 'undefined') return;
    const watch = new ResizeObserver(() => { if (readingKey() != null && !el.clientWidth && !el.clientHeight) stopReading(reader); });
    watch.observe(el);
    onCleanup(() => watch.disconnect());
  });
  onCleanup(() => stopReading(reader));

  // While a turn runs, the empty box says what Enter does and how to stop
  // (P1-10); the hint line under the box shows once something is typed.
  const placeholder = () => chatSessionLoading() ? 'Opening conversation…' : suggesting() ? suggestion()!.prompt
    : chatBusy() ? busyPlaceholder(busyChoice(), agentName(), steerNative(), enterSends() ? 'Enter' : '⌘Enter', keyFor('stop-run'))
    : composerPlaceholder(AGENT_LABELS[chatAgent()], destination());
  // Mid-turn steering (chatSteer.ts): this agent's name, whether the box holds
  // a message, the Send options, and the sends held for after this turn.
  // The running turn's agent (repo ideas 10-27 #8: a continued turn may run on another).
  const agentName = () => AGENT_LABELS[steeringAgent(chatAgent()) as keyof typeof AGENT_LABELS] ?? 'The agent';
  const composerHasMessage = () => !!chatInput().trim() || attachments().length > 0;
  // The stop key as the hint and Stop's tooltip show it: Esc stops only with
  // an empty box (FIX-ESC), so over a message only a ⌘ chord is named.
  const shownStopKey = () => (activeChord('stop-run') === 'Escape' && composerHasMessage() ? '' : keyFor('stop-run'));
  const sendOptions = () => busyMenuItems(agentName(), steerNative(), { empty: !composerHasMessage(), images: attachments().length > 0, note: steerSupportNote(steeringAgent(chatAgent())) || steerWhy().replace(/^./, c => c.toUpperCase()), blocked: steerNative() ? steerWhy() : '' }, choice => void sendWith(choice));
  const [stashRevision, setStashRevision] = createSignal(0);
  const draftOwner = () => props.workspace && workspaceDetail()?.workspace.cwd === chatCwd() ? openWorkspaceId() : null;
  const stashes = () => {
    stashRevision(); chatInput(); chatConversationVersion();
    const owner = draftOwner();
    return owner ? conversationDrafts.stashes(owner, chatAgentSessionKey()) : [];
  };
  function stashDraft() {
    const owner = draftOwner();
    if (!owner) return;
    if (textareaEl) dictationSent(textareaEl);
    const result = conversationDrafts.stash(owner, chatAgentSessionKey(), crypto.randomUUID(), chatInput(), attachments());
    if (!result) { flashFooter('Draft stays here. Keep at most five stashes per chat, each within the draft text limit.', 5000); return; }
    setChatInput(''); setAttachments([]); setStashRevision(n => n + 1);
    flashFooter(result.saved ? 'Draft stashed in this chat. Restore it from +.' : 'Draft stashed in memory only. Keep Hollow open until storage is available.', 6000);
  }
  function restoreStash(id: string) {
    const owner = draftOwner();
    if (!owner) return;
    if (textareaEl) dictationSent(textareaEl);
    const result = conversationDrafts.restoreStash(owner, chatAgentSessionKey(), id, chatInput(), attachments());
    if (!result) { flashFooter('Stash stays saved. The combined draft must fit the text and attachment limits.', 5000); return; }
    setChatInput(result.text); setAttachments(result.attachments); setStashRevision(n => n + 1);
    if (!result.saved) flashFooter('Restored in memory only. Keep Hollow open until storage is available.', 6000);
  }
  let backgroundSending = false;
  async function sendInBackground() {
    const owner = draftOwner(), version = chatConversationVersion(), cwd = chatCwd(), sourceKey = chatAgentSessionKey(), page = activePage();
    if (!owner || !composerOnScreen() || backgroundSending || chatBusy() || sendBlocked() || !composerHasMessage()) return;
    backgroundSending = true;
    try {
      let acceptedKey: string | null = null;
      await sendThenBackground(async () => {
        const sent = await send(undefined, undefined, true);
        acceptedKey = chatAgentSessionKey();
        return sent;
      }, () => activePage() === page && chatConversationVersion() === version && draftOwner() === owner && chatCwd() === cwd
        && !!acceptedKey && chatAgentSessionKey() === acceptedKey && (!sourceKey || sourceKey === acceptedKey),
      guard => newWorkspaceSessionOpened(owner, guard));
    } finally { backgroundSending = false; }
  }
  const composerExtraItems = () => [
    ...(draftOwner() ? [
      { label: 'Stash this draft', action: stashDraft, disabled: !composerHasMessage() },
      ...stashes().map((row, i) => ({ label: `Restore draft ${i + 1}: ${row.text.trim().slice(0, 60) || 'Attachments'}`, action: () => restoreStash(row.id) })),
      { label: 'Stash text is saved. Attachments stay in this window.', informational: true, action: () => {} },
      { label: 'Send in background (opens a new chat)', action: () => void sendInBackground(), disabled: chatBusy() || sendBlocked() || !composerHasMessage() },
    ] : []),
  ];
  const limitRecovery = () => {
    if (chatBusy() || chatSessionLoading()) return null;
    const last = [...chatTurns].reverse().find(t => t.role === 'assistant');
    return last?.resultKind === 'providerQuota' ? 'Usage limit reached. Check Accounts or choose another model, then retry your message.'
      : last?.resultKind === 'maxTurns' ? 'Turn limit reached. Continue in a new conversation to keep working with a written handoff.'
      : last?.resultKind === 'budgetStop' ? 'Budget reached. Check the limit in Settings before continuing.' : null;
  };
  const heldHere = () => heldChipsFor(chatStopKey());
  async function cancelHeld(id: string) {
    const key = chatStopKey();
    if (!key) return;
    try {
      if (await api.chatCancelHeld(key, id)) {
        returnHeldToBox(key, removeHeldChip(key, id), null);
      }
    } catch (err) { flashFooter(`× Couldn't cancel it: ${err instanceof Error ? err.message : String(err)}`, 4500); }
  }
  const sendBlocked = () => chatSessionLoading() || attachmentsHoldSend(attachments())
    || (!chatInput().trim() && !attachments().some(a => a.path || a.page) && !suggesting());


  function useSuggestion(text: string) {
    setChatInput(text);
    queueMicrotask(() => { textareaEl?.focus(); growTextarea(); });
  }

  // A request Hollow drafted into the composer (a curated Refresh, Explain
  // visually): sent unchanged it carries its tag, so its words are never
  // taken as the person's rules (overnight 9 A1). Added to their own text,
  // or edited, it is their message.
  let cannedDraft: CannedDraft | null = null;

  // Preparing an explanation is a draft edit, never an implicit send.
  function draftVisualRequest(text: string, canned?: CannedRequest) {
    const held = chatInput().trim();
    cannedDraft = canned && !held ? { text, canned } : null;
    useSuggestion(held ? `${chatInput()}\n\n${text}` : text);
  }

  // ── Ask-canvas answers (question cards, 2026-10-14) ──
  // The person pressed Hollow's own Send under the canvas (never a canvas
  // script: `HtmlArtifact` takes the frame's answers only as the reply to
  // that click), so the answers go straight out as their next message, as a
  // question card's do. The composer and what they typed there stay as is.
  // While the agent is still answering (P1-33), they are held for after the
  // turn (Queue for after, never Steer) and the canvas says so. A send that
  // didn't go never puts them in the message box: the canvas keeps them and
  // offers Send now and Take them back (`askHold`).
  const dock = createQuestionDock(chatInteractions);
  async function sendFromCanvas(req: CanvasAnswerRequest): Promise<CanvasAnswerResult> {
    if (req.locked()) return 'locked';
    if (req.sessionKey !== chatAgentSessionKey()) return 'unavailable';
    if (chatSessionLoading()) return 'busy';
    let held: string | null = null;
    if (await sendAnswers(req.text, req.locked, (id) => { held = id; })) return held ? { held } : null;
    if (req.locked()) return 'locked';
    // A reset folder (preflight) moved the conversation: Send now would only
    // be refused there.
    if (req.sessionKey !== chatAgentSessionKey()) return 'unavailable';
    return 'not-sent';
  }
  // A card that arrives while the message box holds text waits behind the
  // chip, so the person's typing never answers it; and anything that fills
  // the message box (a prefill, a curated Refresh's Replace/Cancel) hides
  // the dock so they see it.
  createEffect(on(dock.cardId, (id) => { if (id && chatInput().trim()) dock.hideFor(id); }));
  createEffect(on(() => [chatInput(), canvasDraft()] as const, ([text, pending]) => {
    if (dock.open() && (text.trim() || pending)) dock.hide();
  }, { defer: true }));
  registerCanvasAnswers(sendFromCanvas);

  // ── Drafts for the composer (Canvas C4 A2; C6 · K2) ──
  // A curated panel's Refresh only FILLS the composer; the person sends it
  // with Enter. Text the person wrote is never replaced silently: the
  // composer asks first (Replace / Cancel). Only the same view's own
  // untouched text is replaced without asking.
  type PendingCanvasDraft = { req: CanvasDraftRequest; settle: (r: DraftRefusal | null) => void };
  const [canvasDraft, setCanvasDraft] = createSignal<PendingCanvasDraft | null>(null);
  let canvasFilled: { canvasId: string; text: string } | null = null;
  // A canvas's own fill never moves focus: the frame must have focus to
  // draft, so the person is typing in it, and a canvas script could draft
  // from a keydown and steal their next Enter into the composer. Only the
  // host's own Replace button (a click in Hollow) focuses the composer.
  function placeCanvasDraft(req: CanvasDraftRequest, focus: boolean) {
    const text = req.text;
    canvasFilled = { canvasId: req.canvasId, text };
    cannedDraft = req.canned ? { text, canned: req.canned } : null;
    setChatInput(text);
    queueMicrotask(() => { if (focus) textareaEl?.focus(); growTextarea(); });
  }
  function fillFromCanvas(req: CanvasDraftRequest): Promise<DraftRefusal | null> {
    if (req.locked()) return Promise.resolve('locked');
    const held = chatInput();
    if (!held.trim() || (held === canvasFilled?.text && req.canvasId === canvasFilled.canvasId)) {
      placeCanvasDraft(req, false);
      return Promise.resolve(null);
    }
    return new Promise((resolve) => {
      canvasDraft()?.settle('cancelled');
      const pending: PendingCanvasDraft = {
        req,
        settle: (r) => { if (canvasDraft() === pending) setCanvasDraft(null); resolve(r); },
      };
      setCanvasDraft(pending);
      req.interim('confirm');
    });
  }
  function replaceWithCanvasDraft() {
    const p = canvasDraft();
    if (!p) return;
    if (p.req.locked()) return p.settle('locked');
    placeCanvasDraft(p.req, true);
    p.settle(null);
  }
  // Answered meanwhile, or another conversation: the question goes away.
  createEffect(() => { const p = canvasDraft(); if (p?.req.locked()) p.settle('locked'); });
  createEffect(on(() => [chatAgentSessionKey(), chatConversationVersion()], () => canvasDraft()?.settle('cancelled'), { defer: true }));
  onCleanup(() => canvasDraft()?.settle('cancelled'));
  registerCanvasComposer(fillFromCanvas);

  /** Teach Hollow prefills go after a half-written draft, never over it,
   *  and only once (P1-27). */
  function prefillDraft(prefill: string) { useSuggestion(addToDraft(chatInput(), prefill)); }

  function teachFromTurn(turn: ChatTurn) {
    const excerpt = turn.text.trim().replace(/\s+/g, ' ').slice(0, 480);
    prefillDraft(`Teach Hollow from this exchange: [describe the durable correction, preference, or decision here]. The response began: "${excerpt}". Propose it for my review; do not write trusted context directly.`);
  }

  const conversationItems = (): ContextMenuItem[] => [
    { label: 'Find in this conversation', disabled: !chatTurns.length || chatSessionLoading(), action: openFind },
    { label: 'Outline', disabled: !chatTurns.length || chatSessionLoading(), action: () => { setOutlineOpen(true); keepFindPlace(); setFindOpen(false); } },
    { label: 'Copy conversation (Markdown)', disabled: !chatTurns.length || chatSessionLoading(), action: copyConversation },
    { label: 'Save conversation as a note', disabled: !chatTurns.length || chatSessionLoading(), action: saveConversation },
    ...(props.workspace && !props.compact && !mainChat() ? [{ label: sideChatOpen() ? 'Close side chat' : 'Open side chat', action: toggleSideChatChecked }] : []),
    ...(quickChat() ? [{ label: 'Add to project', disabled: !chatAgentSessionKey() || !!moveBlockedReason(chatAgentSessionKey()!), action: () => { const key = chatAgentSessionKey(); if (key) chooseMoveTarget(key); } }] : []),
  ];
  const conversationControls = () => <>
    <ConversationTitle onDone={() => textareaEl?.focus()} items={conversationItems} showTitle={!!props.workspace && !props.compact && !mainChat()} />
    <Show when={props.workspace && !props.compact && !mainChat()}>
      <button type="button" class="chat-nav-btn chat-side-chat-toggle" aria-pressed={sideChatOpen()} aria-label={sideChatOpen() ? 'Close side chat' : 'Open side chat'}
        title={sideChatOpen() ? 'Close the side chat' : 'Side chat: ask a quick question about this chat without disturbing it'} onClick={toggleSideChatChecked}><Icon name="columns" size={14} /></button>
    </Show>
    <Show when={readingKey() != null}>
      <button type="button" class="chat-nav-btn chat-nav-reading" title="Stop reading" aria-label="Stop reading" onClick={() => stopReading()}><Icon name="stop" size={13} /><span>Stop reading</span></button>
    </Show>
  </>;

  return (
    <>
      <div class="chat-scroll-wrap">
      <Show when={props.workspace && !props.compact && topBarSlot()} fallback={<Show when={!props.workspace || props.compact}><div class="chat-navigation">{conversationControls()}</div></Show>}>
        {slot => <Show when={activePage() === 'today'}><Portal mount={slot() as HTMLElement}><div class="chat-topbar-controls">{conversationControls()}</div></Portal></Show>}
      </Show>
      <Show when={!props.compact && (!props.workspace || activePage() === 'today') && previewCardShown()}><Suspense fallback={null}><SidePanelCard /></Suspense></Show>
      <Show when={findOpen()}>
        <div class="chat-find" role="search" aria-label="Find in this conversation">
          <Icon name="search" size={14} />
          <input ref={findInput} type="search" aria-label="Find in this conversation" placeholder="Find in this conversation" value={findQuery()} onInput={e => setFindQuery(e.currentTarget.value)}
            onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); stepFind(e.shiftKey ? -1 : 1); } else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closeFind(); } }} />
          <span class="chat-find-count" aria-live="polite">{findQuery().trim().length < 2 ? '' : findHits().length ? `${findAt() + 1} of ${findHits().length}` : 'No matches'}</span>
          <button type="button" class="chat-nav-btn" aria-label="Previous match" title="Previous (⇧Enter)" disabled={!findHits().length} onClick={() => stepFind(-1)}><Icon name="arrowUp" size={13} /></button>
          <button type="button" class="chat-nav-btn" aria-label="Next match" title="Next (Enter)" disabled={!findHits().length} onClick={() => stepFind(1)}><Icon name="chevronDown" size={13} /></button>
          <button type="button" class="chat-nav-btn" aria-label="Close find" title="Close (Esc)" onClick={closeFind}><Icon name="x" size={13} /></button>
        </div>
      </Show>
      {/* The outline rail in the left gutter and its card (E9, 11-12). */}
      <Show when={!chatSessionLoading()}><Suspense><ChatOutlineRail turns={chatTurns} scrollEl={scrollEl} jump={jumpToTurn} open={outlineOpen()} onClose={() => setOutlineOpen(false)} /></Suspense></Show>
      {/* Canvas C3: one frame budget per transcript (at most six live
          canvas and html frames), measured against this scroller. */}
      <FrameBudgetContext.Provider value={frameScope}>
      <div
        class="chat-scroll"
        classList={{ 'chat-scroll-compact': props.compact }}
        ref={el => setScrollEl(el)}
        tabindex="-1"
        onScroll={() => { onScroll(); visit.noteRead(); }}
      >
        <Show when={chatSessionLoading()}><div class="chat-opening" role="status"><span class="live-dot on" />Opening conversation…</div></Show>
        <Suspense><ChatChainBar part="earlier" /></Suspense>
        <Show when={chatTurns.length === 0 && !chatSessionLoading()}>
          <div class="chat-empty">
            <Show when={!props.compact}><DoorwayLandscape placement="chat"/></Show>
            <h2 class="chat-empty-title">{mainChat() ? 'What can I take off your plate?' : props.workspace && !quickChat() ? `What should we build in ${destination().kind === 'project' ? workspaceDetail()?.workspace.title ?? 'this project' : 'this project'}?` : quickChat() ? 'Ask anything' : 'What should we work on?'}</h2>
            <Show when={mainChat()}>
              <p class="chat-empty-sub">I can see all your workspaces and start chats in them. My notes and your standing orders go with every message.</p>
            </Show>
            <Show when={props.workspace && !quickChat() && !mainChat()}>
              <p class="chat-empty-sub">The brief, plan and sources go with every message. <button type="button" class="btn ghost" onClick={() => void startQuickChat()}>or start without a project</button></p>
            </Show>
            {/* The Mac mini runner: where a workspace's new conversation runs.
                A build workspace's is the same setting as its Changes tab
                (2026-11-01); a vault workspace's has its own (Phase 3,
                2026-11-02). The COO and quick chats stay here. */}
            <Show when={!quickChat() && !mainChat() && workspaceDetail()?.workspace.cwd === chatCwd() ? workspaceDetail()!.workspace : null}>
              {ws => <RunsOnControl class="chat-empty-runs-on" workspaceId={ws().id} saved={ws().build?.runsOn}
                kind={ws().build?.exists ? 'build' : 'workspace'} />}
            </Show>
            <div class="chat-empty-suggestions">
              <For each={mainChat() ? MAIN_AGENT_SUGGESTIONS : props.workspace && !quickChat() ? WORKSPACE_SUGGESTIONS : SUGGESTIONS}>
                {(s) => (
                  <button type="button" class="chat-suggestion" onClick={() => useSuggestion(props.workspace && chatInput().trim() ? `${chatInput()}\n\n${s.prompt}` : s.prompt)}>
                    <Icon name={s.icon} size={15} />
                    <span><strong>{s.title}</strong><span>{s.description}</span></span>
                  </button>
                )}
              </For>
            </div>
            <Show when={teamworkEnabled() && currentAgentCapabilities()?.ownedMcp && agentsAvailable().some(a => a !== chatAgent())}>
              <p class="chat-empty-teamwork">
                Teamwork on: {AGENT_LABELS[chatAgent()]} can hand parts to{' '}
                {agentsAvailable().filter(a => a !== chatAgent()).map(a => AGENT_LABELS[a]).join(' and ')}.
              </p>
            </Show>
          </div>
        </Show>
        <Show when={renderFrom() > 0}>
          <button type="button" class="chat-show-earlier" ref={loadAtEdge('earlier')} onClick={() => showEarlier()}>
            <Icon name="history" size={13} />Show {Math.min(TURN_WINDOW, renderFrom())} earlier message{Math.min(TURN_WINDOW, renderFrom()) === 1 ? '' : 's'} <small>{chatTurns.length} in this conversation</small>
          </button>
        </Show>
        <For each={chatTurns}>
          {(t, turnIdx) => {
            // One footer per reply, on its last assistant bubble, once the
            // run is over (Conductor-style): time, copy, feedback, files.
            const tail = createMemo(() => endsReply(chatTurns, turnIdx()) && !t.partial && !(t.runId && t.runId === chatLiveRunId()));
            const reply = createMemo(() => (tail() ? { ...t, text: replyText(chatTurns, turnIdx()) } : t));
            const key = () => detailsKey(t, turnIdx());
            const recordRetryActions = createMemo(() => retryRecordIndex() === turnIdx() ? retryControls() : undefined);
            // The full receipt stays on screen by itself only when the run
            // failed; otherwise the footer's chips and menu open it.
            const showReceipt = () => !!t.receipt && (!tail() || t.receipt.status === 'failed' || retryRecordIndex() === turnIdx() || openDetails().has(key()));
            // Frontier F5: inside a finished run's fold, a step turn renders
            // nothing of its own (the fold row holds it); the answer keeps its
            // text and footer; the head turn carries the fold row.
            const role = () => foldRoleOf().get(turnIdx()) ?? 'normal';
            const fold = () => { const r = role(); return r === 'head' || r === 'head-answer' ? folds().get(turnIdx()) : undefined; };
            const folded = () => role() !== 'normal';
            const textShown = () => { const r = role(); return r === 'normal' || r === 'answer' || r === 'head-answer'; };
            // Overnight D4: the reply's account moves, from its receipt, as
            // timeline rows (less the ones its live notices still show), unless
            // the full receipt, which lists them too, is open.
            const switched = createMemo(() => (tail() && !showReceipt() ? switchRows(t.receipt?.switches, liveSwitchNotices(chatTurns, turnIdx(), renderFrom())) : []));
            // A long message of yours folds behind Show more (overnight 11-04,
            // C3 follow-up; open state in `openTexts`, above). The id ties the
            // button to the words it shows.
            const textOpen = () => openTexts().has(textKey(t, turnIdx()));
            const textId = createUniqueId();
            const proposalOwner = chatConversationVersion();
            const proposalCurrent = () => proposalOwner === chatConversationVersion() && proposalUsable(chatTurns, turnIdx(), chatBusy(), chatSessionLoading());
            const warning = createMemo(() => transcriptWarning(chatTurns, turnIdx()));
            return (
            <Show when={turnIdx() >= renderFrom() && turnIdx() < renderTo() && (role() !== 'hidden' || tail())}>
            <Show when={visitRow() === turnIdx()}>
              <ThreadVisitWatermark count={visit.mark()!.count} onDismiss={visit.dismiss} />
            </Show>
            <div class={`turn ${t.role}`} classList={{ partial: !!t.partial, 'run-folded': folded(), 'turn-notice': !!t.notice, 'turn-hit': findOpen() && findHits()[findAt()] === turnIdx() && role() !== 'head' }} data-notice={t.notice} data-turn-index={turnIdx()}>
              {/* A wake note keeps its own Copy in its corner; your messages
                  have theirs in the row under them (C3). */}
              <Show when={t.text && t.role === 'user' && mainTurnKind(t) === 'wake'}>
                <button type="button" class="turn-copy" title="Copy message" aria-label="Copy message" onClick={() => void copyTurn(t)}><Icon name="copy" size={13} /></button>
              </Show>
              <div class="turn-content">
                <Show when={t.proposedPlan}>
                  <Suspense><ProposedPlan text={t.proposedPlan!} disabled={!proposalCurrent()} onChoice={choice => {
                    if (!proposalCurrent()) return;
                    const request = proposalRequest(t, choice);
                    if (!chatInput().includes(request)) setChatInput(addToDraft(chatInput(), request));
                    textareaEl?.focus();
                  }} /></Suspense>
                </Show>
                <Show when={mainChat() && t.role === 'assistant' && textShown()}>
                  <span class="main-agent-chat-avatar"><MainAgentIndicator state="resting" size={20} decorative /></span>
                </Show>
                <Show when={agentLabelShown(turnIdx())}>
                  <div class="chat-turn-label">{AGENT_LABELS[t.agent as ChatAgent] ?? 'Assistant'}</div>
                </Show>
                {/* Mid-turn steering: the person's own message to the running
                    turn, with when and whether the agent took it. */}
                <Show when={t.notice === 'steer' && t.steer}>
                  {(st) => (
                    <div class="chat-steer-row" data-state={st().state} role="group" aria-label={`You said, while ${agentName()} worked`}>
                      <div class="chat-steer-head"><span class="chat-steer-who">You</span><span class="chat-steer-at">{steerTime(st().at)}</span><span class="chat-steer-kind">· steered</span></div>
                      <div class="chat-steer-text">{t.text}</div>
                      <div class="chat-steer-state">{steerStateText(st().state, st().reason)}</div>
                      <Show when={st().state === 'notDelivered' && !resentSteers().has(st().id)}>
                        <SteerActions
                          onSend={() => resendSteer(st().id, t.text)}
                          onPutBack={() => { setChatInput(addToDraft(chatInput(), t.text)); textareaEl?.focus(); }} />
                      </Show>
                    </div>
                  )}
                </Show>
                {/* A quiet notice (R1's account switch) carries no kicker. */}
                <Show when={t.handoff}>
                  <details class="chat-context-divider"><summary>{t.handoff?.state === 'pending' ? 'Context handoff pending' : t.handoff?.state === 'complete' ? 'Context handoff ready' : 'Context handoff unavailable'}</summary><p>{t.text}</p></details>
                </Show>
                <Show when={t.role === 'system' && !t.notice && !t.handoff && !warning()}>
                  <div class="chat-system-kicker">system</div>
                </Show>
                <Show when={fold()}>
                  {(f) => {
                    // C2 (overnight 11-12): the reply's changed files sit
                    // right-aligned above its "Worked for" line, from the
                    // receipt on its last assistant turn (its footer's).
                    const tailAt = () => foldTail(f(), chatTurns);
                    const tailTurn = () => chatTurns[tailAt()];
                    return (
                    <>
                    <Show when={tailTurn()?.receipt}>
                      <ReplyFileChips
                        receipt={tailTurn().receipt}
                        detailsOpen={openDetails().has(detailsKey(tailTurn(), tailAt()))}
                        onToggleDetails={() => toggleDetails(detailsKey(tailTurn(), tailAt()))}
                        workspace={props.workspace && openWorkspaceDetail()?.workspace.id === openWorkspaceId() ? openWorkspaceDetail()?.workspace : undefined}
                        mainAgent={mainChat()}
                      />
                    </Show>
                    <RunFoldRow
                      fold={f()}
                      typicalMs={jobTiming()?.typicalMs}
                      workKind={jobTiming()?.workKind}
                      open={openFolds().has(foldKey(f()))}
                      onToggle={() => toggleFold(foldKey(f()))}
                    >
                      <For each={f().members}>
                        {(m) => {
                          const member = () => chatTurns[m];
                          // A find or outline jump to this turn lands here
                          // (P1-27); the answer's own row is the turn's.
                          const step = m !== f().answer;
                          return (
                            <div class="chat-run-step" data-turn-index={step ? m : undefined}
                              classList={{ 'turn-hit': step && findOpen() && findHits()[findAt()] === m }}>
                              <ThinkingRow text={member()?.reasoning} />
                              <Show when={m !== f().answer && !foldKeepsText(f(), m) && member()?.text}>
                                <ChatMessage text={member().text} role="assistant" class="turn-md assistant chat-run-narration" onAsk={draftVisualRequest} onCanvasAnswer={sendFromCanvas} />
                              </Show>
                              <Show when={member()?.tools.length}>
                                <ToolLedger tools={member().tools} onToggle={(toolIdx) => toggleToolExpanded(m, toolIdx)} />
                              </Show>
                            </div>
                          );
                        }}
                      </For>
                    </RunFoldRow>
                    </>
                    );
                  }}
                </Show>
                <Show when={!fold() && textShown()}><ThinkingRow text={t.reasoning} /></Show>
                <Show
                  when={t.role === 'user'}
                  fallback={
                    <Show when={t.text && textShown() && t.notice !== 'steer' && !t.handoff && warning()?.count !== 0}>
                      <Show when={warning()} fallback={
                      <Show when={mainTurnKind(t) !== 'check-in'} fallback={<div class="chat-pulse-ok" role="note">Checked in · nothing needs you</div>}>
                      <ChatMessage
                        text={t.text}
                        role={t.role as 'assistant' | 'system'}
                        streaming={t.partial}
                        liveRun={!!t.runId && t.runId === chatLiveRunId()}
                        runId={t.runId ?? null}
                        caret={showsCaret(t, turnIdx() === chatTurns.length - 1, typing())}
                        class={`turn-md ${t.role}`}
                        onAsk={draftVisualRequest}
                        onCanvasAnswer={sendFromCanvas}
                      />
                      </Show>
                      }>{w => <details class="chat-inline-warning"><summary>{w().title}<Show when={w().count > 1}><span> ({w().count} occurrences)</span></Show></summary><pre>{t.text}</pre></details>}</Show>
                    </Show>
                  }
                >
                  {/* A wake turn (main agent M2): Hollow wrote it, not you. */}
                  <Show when={mainTurnKind(t) !== 'wake'} fallback={<details class="chat-host-note"><summary>Hollow · {wakeText(t.text).split('\n')[0]}</summary><div class="text">{wakeText(t.text)}</div></details>}>
                  {/* A quiet bubble fitted to its text (C3, Claude-desktop
                      style). "Prompt" stays its name for screen readers:
                      neutral, since user turns also hold handoffs and
                      workflow prompts. */}
                  <div class="chat-user-chip">
                    <span class="sr-only">Prompt</span>
                    <Show when={t.images && t.images.length > 0}>
                      <div class="turn-images">
                        <For each={t.images!}>
                          {(src) => (
                            <button type="button" class="turn-image-open" title="Expand image" aria-label="Expand the attached image" onClick={() => {
                              const version = chatConversationVersion();
                              void import('./ImageZoom').then(({ showImageZoom }) => { if (version === chatConversationVersion()) showImageZoom(src, 'Attached image', src); });
                            }}>
                              <img class="turn-image" src={src} alt="attached image" loading="lazy" />
                            </button>
                          )}
                        </For>
                      </div>
                    </Show>
                    {/* C5: attached files, each opening in the Preview pane. */}
                    <Show when={t.files && t.files.length > 0}>
                      <div class="turn-files">
                        <For each={t.files!}>
                          {(f) => (
                            <button type="button" class="turn-file-open" title={f.path} aria-label={`Open ${f.name}`} onClick={() => openFileInSidePane(f.path)}>
                              <Icon name="fileText" size={14} />
                              <span class="turn-file-name">{f.name}</span>
                              <Show when={f.size != null}><span class="turn-file-size">{fileSizeLabel(f.size!)}</span></Show>
                            </button>
                          )}
                        </For>
                      </div>
                    </Show>
                    {/* An ask-canvas's answer shows its lines; the hidden JSON
                        block is for the agent (Canvas C4 A2). */}
                    <Show when={t.text && visibleUserText(t.text)}>
                      {(text) => {
                        // Decided from the words, so the fold is there from
                        // the first paint (userMessageFold.ts): no measuring.
                        const long = () => foldsUserText(text());
                        const shut = () => long() && !textOpen() && !findUnfolds();
                        return (
                          <>
                            <div class="text" id={textId} classList={{ 'chat-user-clipped': shut() }}>{text()}</div>
                            <Show when={long() && !findUnfolds()}>
                              <button type="button" class="chat-user-more" aria-expanded={textOpen()} aria-controls={textId} onClick={e => { const opening = !textOpen(); toggleInPlace(e.currentTarget, opening, () => setTextOpen(textKey(t, turnIdx()), opening)); }}>
                                {textOpen() ? 'Show less' : 'Show more'}
                              </button>
                            </Show>
                          </>
                        );
                      }}
                    </Show>
                    <Show when={queuedBadge(t)}>
                      {(q) => (
                        <div class="chat-queued-badge" role="group" aria-label={q().waitingFor || q().heldReason ? 'Queued message' : 'Message being prepared'}>
                          {/* The label shows only for a turn that truly waits (a limit,
                              memory, another run); while it prepares, the working line
                              says Preparing and only Edit and Discard stay. */}
                          <Show when={q().waitingFor || q().heldReason}>
                            <span class="live-dot" aria-hidden="true" />
                            <span class="chat-queued-label" role="status">{queuedTurnLabel(q())}</span>
                          </Show>
                          {/* P1-46: the conversation holding this build worktree. */}
                          <Show when={q().waitingSession}>{key => <button type="button" class="btn ghost sm" onClick={() => { const ws = openWorkspaceId(); if (ws) void pickConversation(ws, key()); }} title="Open the conversation using this worktree">Open it</button>}</Show>
                          <button type="button" class="btn ghost sm" disabled={!!q().intent} onClick={() => void withdrawQueuedChatTurn('edit')} title="Take it out of the queue and back into the composer">Edit</button>
                          <button type="button" class="btn ghost sm" disabled={!!q().intent} onClick={() => void withdrawQueuedChatTurn('discard')} title="Take it out of the queue without sending it">Discard</button>
                        </div>
                      )}
                    </Show>
                  </div>
                  {/* Under every message of yours, on hover or keyboard focus
                      (C3, ChatGPT style): when it was sent, Copy (its visible
                      words), and Edit on the last one. */}
                  <div class="chat-user-actions" role="group" aria-label="Message actions">
                    <Show when={sentTime(t.at)}>
                      {(sent) => <time class="chat-user-time" dateTime={t.at} title={sent().title}>{sent().label}</time>}
                    </Show>
                    <Show when={t.text}>
                      <button type="button" class="turn-copy" title="Copy message" aria-label="Copy message" onClick={() => void copyTurn({ text: visibleUserText(t.text) })}><Icon name="copy" size={13} /></button>
                    </Show>
                    <Show when={editAt() === turnIdx()}>
                      <button type="button" class="chat-user-edit" aria-label="Edit message"
                        title="Edit: put this message back in the box to change and send (the earlier answer stays)"
                        onClick={() => editMessage(turnIdx())}><Icon name="edit" size={13} /></button>
                    </Show>
                  </div>
                  </Show>
                </Show>
                <Show when={t.tools.length > 0 && !folded()}>
                  <ToolLedger tools={t.tools} onToggle={(toolIdx) => toggleToolExpanded(turnIdx(), toolIdx)} />
                </Show>
                <Show when={switched().length > 0}>
                  <ul class="chat-switch-rows" role="list" aria-label="Which account or agent ran this reply">
                    <For each={switched()}>
                      {(row) => (
                        <li class="chat-switch-row" data-kind={row.kind}>
                          <Icon name={row.kind === 'login' ? 'lock' : 'refresh'} size={12} />
                          <span>{row.text}</span>
                          <Show when={row.why}><span class="chat-switch-why">{row.why}</span></Show>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
                <Show when={tail()}>
                  <ReplyMedia receipt={t.receipt} text={reply().text} />
                  <Show when={t.receipt?.historyRunId}>{id => <LearningLine runId={id()}/>}</Show>
                  <TurnFooter
                    latest={turnIdx() >= lastUser()}
                    chipsAbove={folded() && foldHeadOf(turnIdx()) >= renderFrom()}
                    runId={t.runId}
                    canVote={canVote(reply(), chatLiveRunId())}
                    hideDuration={folded()}
                    typical={folded() ? null : usualTime()}
                    receipt={t.receipt}
                    at={t.at}
                    onCopy={() => void copyTurn(reply())}
                    onExplain={reply().text.trim() && effectiveCanvases(chatCanvases(), settingCanvases(agentSettings())) !== 'off' ? () => draftVisualRequest(visualExplanationPrompt(reply().text), 'explain') : undefined}
                    onTeach={() => teachFromTurn(reply())}
                    detailsOpen={openDetails().has(key())}
                    onToggleDetails={() => toggleDetails(key())}
                    onBranch={props.workspace && openWorkspaceId() && chatAgentSessionKey() ? forThisChat(sameChatAsNow(), turnIdx(), i => void branchFrom(i)) : undefined}
                    onBranchHere={branchHereAction(reply().runId)}
                    onRestore={restoreAction(reply().runId)}
                    onSaveNote={reply().text.trim() ? forThisChat(sameChatAsNow(), turnIdx(), i => void saveReplyNote(i)) : undefined}
                    onReadAloud={reply().text.trim() && readAloudSupported() ? () => toggleReading(turnIdx()) : undefined}
                    reading={readingKey() === readKey(turnIdx())}
                    onRetry={regenerate()?.reply === turnIdx() ? () => void retry(undefined, regenerate()?.ask ?? -1) : undefined}
                    retryAgents={regenerate()?.reply === turnIdx() ? retryAgents().map(a => ({ label: AGENT_LABELS[a] ?? a, action: forThisChat(sameChatAsNow(), turnIdx(), () => void retry(a, regenerate()?.ask ?? -1)) })) : undefined}
                    retryModels={regenerate()?.reply === turnIdx() ? retryModels().map(m => ({ label: m.label, action: forThisChat(sameChatAsNow(), turnIdx(), () => void retry(undefined, regenerate()?.ask ?? -1, m.id)) })) : undefined}
                    workspace={props.workspace && openWorkspaceDetail()?.workspace.id === openWorkspaceId() ? openWorkspaceDetail()?.workspace : undefined}
                    mainAgent={mainChat()}
                  />
                  {/* Quick chats (2026-11-14): Move to <project> when the
                      latest exchange is clearly one project's work. */}
                  <Show when={turnIdx() >= lastUser() && lastUser() >= 0 && quickChat() && !chatBusy() ? chatAgentSessionKey() : null}>{k =>
                    <Suspense fallback={null}><QuickChatMoveOffer sessionKey={k()} replyId={key()} prompt={chatTurns[lastUser()]?.text ?? ''} reply={reply().text}/></Suspense>
                  }</Show>
                </Show>
                <Show when={showReceipt() && t.receipt}>
                  {(receipt) => <ExecutionReceipt hideRecall receipt={receipt()} defaultOpen={tail() && openDetails().has(key())}
                    onOpenOutput={path => { const owner = fileClick(); void import('../store/fileOpen').then(m => m.openNamedFile(path, null, { literal: true, owner })); }}
                    head={{ retryActions: recordRetryActions() }}
                    onAskRerun={text => { setChatInput(addToDraft(chatInput(), text)); requestChatFocus(); }}
                    workspace={props.workspace && openWorkspaceDetail()?.workspace.id === openWorkspaceId() ? openWorkspaceDetail()?.workspace : undefined} />}
                </Show>
              </div>
            </div>
            </Show>
            );
          }}
        </For>
        {/* Reading back through a long thread draws at most TURN_CAP turns (transcriptWindow.ts): the end
            gives way, and this loads it again, pressed or scrolled to. Jump to latest is the way to the end. */}
        <Show when={renderTo() < chatTurns.length}>
          <button type="button" class="chat-show-earlier chat-show-later" ref={loadAtEdge('later')} onClick={() => showLater()}>
            <Icon name="chevronDown" size={13} />Show {Math.min(TURN_WINDOW, chatTurns.length - renderTo())} later message{Math.min(TURN_WINDOW, chatTurns.length - renderTo()) === 1 ? '' : 's'} <small>{chatTurns.length - renderTo()} below</small>
          </button>
        </Show>
        <Show when={activity()}>
          <LiveTurnFooter verb={activity()!} startedAt={chatRunStartedAt()} quiet={quiet()} typical={usualTime()} compact={props.compact} />
        </Show>
        {/* Mid-turn steering: sends held for after the running turn, oldest
            first, each with Cancel (the words go back in the box). */}
        <HeldChips chips={heldHere()} onCancel={id => void cancelHeld(id)} />
        {/* Inline approval cards (Claude-desktop style): a confirm-gated MCP
            tool — such as a tier-2 computer verb — is blocked on this Allow/Deny. Rendered at the
            tail of the transcript on both chat surfaces. */}
        {/* 2026-11-20: a card from a project lead's chain or a subagent of
            one is raised to the COO chat: a card there, a one-line note
            here. One id answers both. */}
        <For each={pendingApprovals().filter(approval => approval.kind !== 'm365'
          && approvalPlacement(approval, chainCardKeysFor(chatAgentSessionKey()), inCooChat()) === 'waiting')}>
          {(a) => (
            <div class="chat-approval chat-approval-waiting" role="status" aria-label={approvalWaitingLabel(a)}>
              <div class="chat-approval-head">
                <Icon name="alert" size={14} />
                <span class="chat-approval-kicker">{approvalWaitingLabel(a)}</span>
                <button type="button" class="btn ghost sm" onClick={() => void openMainAgent()} title="Open the COO chat, where this card is answered">Open the COO chat</button>
              </div>
              <div class="chat-approval-action">{a.action}</div>
            </div>
          )}
        </For>
        <For each={pendingApprovals().filter(approval => approval.kind !== 'm365'
          // Another conversation's request waits for it to come back on screen
          // (build programme PR B4); the rail marks it.
          // A request from a chat before this one in its chain shows here
          // (one Main conversation, 2026-11-07): it is answered by its id.
          // SA1: a subagent's request also shows in the chat that started it.
          && approvalPlacement(approval, chainCardKeysFor(chatAgentSessionKey()), inCooChat()) === 'card')}>
          {(a) => {
            const [why, setWhy] = createSignal('');
            const origin = approvalOriginLabel(a);
            const routedFrom = a.routedTo ? (a.routedTo.askedTitle?.trim() || (a.routedTo.askedKind === 'lead' ? 'a project lead' : 'a subagent')) : null;
            return (
            <div class="chat-approval" role="group" aria-label={`Approval required — ${a.title}`}>
              <div class="chat-approval-head">
                <Icon name="alert" size={14} />
                <span class="chat-approval-kicker">Approval required</span>
                <span class="chat-approval-source">{a.title}</span>
              </div>
              <Show when={approvalTool(a)}>
                {(tool) => {
                  const meta = describeTool(tool().name, tool().input);
                  return (
                    <div class="chat-approval-tool">
                      For: <strong>{meta.verb}</strong>
                      <Show when={meta.target}> {meta.target}</Show>
                    </div>
                  );
                }}
              </Show>
              <Show when={origin}><div class="chat-approval-origin">{origin}</div></Show>
              <Show when={routedFrom}><div class="chat-approval-origin">Asked by {routedFrom}</div></Show>
              <div class="chat-approval-action">{a.action}</div>
              <input
                type="text"
                class="chat-approval-reason"
                placeholder="Reason for declining (optional, the agent sees it)"
                aria-label="Reason for declining"
                maxLength={500}
                value={why()}
                disabled={approvalsResponding().includes(a.id)}
                onInput={e => setWhy(e.currentTarget.value)}
              />
              <div class="chat-approval-btns">
                <button
                  type="button"
                  class="chat-approval-btn deny"
                  disabled={approvalsResponding().includes(a.id)}
                  onClick={() => respondToApproval(a.id, false, why().trim() || undefined)}
                >Deny</button>
                <button
                  type="button"
                  class="chat-approval-btn allow"
                  disabled={approvalsResponding().includes(a.id)}
                  onClick={() => respondToApproval(a.id, true)}
                >Allow</button>
              </div>
            </div>
            );
          }}
        </For>
        {/* P1a PR 2: a paused chat offers Resume in the composer instead. */}
        <Show when={continueOffered() && !chatPausedHere()}>
          <div class="chat-continue">
            {/* Next-level P04: say which stage the run ended in (one vocabulary
                with the rail and Your work), then offer the recoveries. */}
            <Show when={endedStage()}>{st => <StatusPill quiet label={RUN_STAGE[st()].label} tone={RUN_STAGE[st()].tone} hint={RUN_STAGE[st()].hint} />}</Show>
            <button type="button" class="btn chat-continue-btn" onClick={() => void continueRun()} title="Send: Your previous run was interrupted. Continue from where you left off.">
              Continue
            </button>
            <Show when={props.workspace && openWorkspaceId() && chatAgentSessionKey()}>
              <button type="button" class="btn ghost chat-continue-btn" onClick={() => void branchFrom(chatTurns.length - 1)} title="Start a new conversation with a written handoff of this one">Continue in a new conversation</button>
            </Show>
          </div>
        </Show>
        {/* P1-27: a failed last turn offers its message again, here or with
            another agent; the same row as Continue, with existing controls. */}
        <Show when={failedAt() >= 0 && retryRecordIndex() < 0}>
          {retryControls()}
        </Show>
      </div>
      </FrameBudgetContext.Provider>
        <QuoteSelection scroller={scrollEl} onQuote={quoteIntoBox} />
        <div class="chat-recovery-controls">
        <Show when={!atBottom() && chatTurns.length > 0}>
          {/* P1-27: "Finished ↓" once a run ended below the view; its name
              starts with the words it shows. */}
          <button
            class="chat-jump-latest"
            type="button"
            onClick={() => jumpToLatest()}
            aria-label={finishedBelow() ? 'Finished: jump to latest' : 'Jump to latest'}
            title={finishedBelow() ? 'The run finished: jump to latest' : 'Jump to latest'}
          ><Icon name="chevronDown" size={13} /><span>{finishedBelow() ? 'Finished' : 'Jump to latest'}</span></button>
        </Show>
        <Show when={focusChip()}>
          <button
            class="chat-back-canvas"
            classList={{ stacked: !atBottom() }}
            type="button"
            aria-label={focusTitle() ? `Back to canvas: ${focusTitle()}` : 'Back to canvas'}
            title="Back to canvas (Escape hides this)"
            onClick={e => backToCanvas(e.detail === 0)}
            onFocus={() => setChipFocused(true)}
            onBlur={() => setChipFocused(false)}
            on:keydown={(e: KeyboardEvent) => {
              if (e.key !== 'Escape') return;
              e.preventDefault(); e.stopPropagation();
              dismissFocusChip();
              textareaEl?.focus();
            }}
          ><Icon name="arrowUp" size={13} /><span>Back to canvas</span></button>
        </Show>
        </div>
      </div>
      {/* A compact conversation (a narrow host) offers its newest file; the
          Preview pane opens beside whatever page is showing. */}
      <Show when={props.compact && chatArtifacts().length > 0 && !chatBusy()}>
        <div class="chat-preview-chip-row">
          <button
            type="button"
            class="chat-preview-chip"
            onClick={() => openChatPreview()}
            title="Preview this conversation’s newest file"
          >
            <Icon name="externalLink" size={12} />
            <span class="chat-preview-chip-label">Preview {chatArtifacts()[0].name}</span>
            <Show when={chatArtifacts().length > 1}>
              <span class="chat-preview-chip-count">+{chatArtifacts().length - 1}</span>
            </Show>
          </button>
        </div>
      </Show>
      <Show when={pageContext() || (operatorContextEnabled() && operatorProfile()?.body)}>
        <div class="chat-context-pill-row chat-context-chips" classList={{ 'chat-context-pill-row-compact': props.compact }}>
          <Show when={pageContext()}>
            {(ctx) => (
              <button
                type="button"
                class="chat-context-pill"
                classList={{ off: !chatContextEnabled() }}
                onClick={toggleChatContext}
                title={chatContextEnabled()
                  ? `Page context ON — the agent sees a snapshot of ${ctx().label} with each message. Click to disable.`
                  : `Page context OFF — click to re-enable. ${ctx().label}: ${ctx().summary}`
                }
                aria-pressed={chatContextEnabled()}
              >
                <span class="pill-label">{ctx().label === 'Chat' ? 'Conversation context' : ctx().label}</span>
                <Show when={!chatContextEnabled()}><span class="pill-state">off</span></Show>
                <Show when={ctx().label !== 'Chat'}><span class="pill-sep">·</span><span class="pill-summary">{ctx().summary}</span></Show>
              </button>
            )}
          </Show>
          <Show when={operatorContextEnabled() && operatorProfile()?.body}>
            <button
              type="button"
              class="chat-context-pill"
              onClick={() => { openSettingsFromOutside('operator'); }}
              title={chatSessionId()
                ? 'About you went in at the start of this session (your me/ digest: house style, guardrails, decisions). Manage it in Settings → Personalization.'
                : 'About you is on: your me/ digest goes in with the first message of the session. Manage it in Settings → Personalization.'}
            >
              <span class="pill-label">Preferences</span>
            </button>
          </Show>
        </div>
      </Show>
      {/* Structured question cards (Paperclip PR 13), docked where the
          message box is (2026-10-14): one question at a time; the answers
          go out as the next turn. Esc hides them behind a chip. */}
      {/* Keyed by the card's id, not its object: a re-read of the cards
          must never restart the person's answers. */}
      <Show when={dock.cardId()} keyed>{(_id) => { const card = () => dock.first()!; return (
        <div class="chat-input chat-input-dock" classList={{ 'chat-input-compact': props.compact }} hidden={!dock.open()}>
        <QuestionDock
          card={card()}
          hidden={!dock.open()}
          blockReason={submitBlockReason({ card: card(), busy: chatBusy(), loading: chatSessionLoading() })}
          asMessage={sendsAsMessage(card(), chatSessionId())}
          submitting={submittingCard() === card().id}
          error={cardErrors()[card().id] ?? null}
          onSubmit={answer => void submitInteraction(card(), answer)}
          onDismiss={() => void dismissInteraction(card())}
          onHide={() => { dock.hide(); queueMicrotask(() => textareaEl?.focus()); }}
        />
        </div>
      ); }}</Show>
      <Suspense><ChatChainBar part="banner" /></Suspense>
      {/* M5: a chat moving to, or moved to, another Mac says so here, and an offline copy of one. */}
      <Show when={chatHomes()[chatAgentSessionKey() ?? ''] || isOfflineCopy(chatAgentSessionKey())}><Suspense><ChatMoveNotice /></Suspense></Show>
      <Show when={isBranch() && chatAgentSessionKey()}>{key => <Suspense><ChatBranchNotice chatKey={key()} turnsNow={chatTurns.length} /></Suspense>}</Show>
      <form
        class="chat-input"
        classList={{ 'chat-input-compact': props.compact }}
        hidden={dock.open() || !!chatContinuedIn()}
        onSubmit={send}
      >
        <Show when={limitRecovery()}>{notice => <div class="chat-busy-hint" role="status">{notice()}</div>}</Show>
        <Show when={!dock.open() && chatInteractions().length > 0}>
          <QuestionsWaiting cards={chatInteractions()} onOpen={() => dock.show()} />
        </Show>
        <input
          ref={el => (fileInputEl = el)}
          type="file"
          multiple
          style={{ display: 'none' }}
          onChange={onFilePick}
        />
        <Show when={providerSwitch()}>{prev =>
          <p class="chat-provider-switch" role="note"><Icon name="alert" size={12} />{AGENT_LABELS[chatAgent()]} continues from the written conversation. It doesn’t resume {AGENT_LABELS[prev()] ?? prev()}’s session or its tool state.</p>}
        </Show>
        {/* Merged plan slice 2 item 4: Grok and Antigravity can't take Hollow's
            per-run MCP config, so their chats have no Hollow cards or app tools. */}
        <Show when={hollowCardsNotice(chatAgent(), AGENT_LABELS[chatAgent()] ?? chatAgent(), currentAgentCapabilities())}>{notice =>
          <p class="chat-provider-switch" role="note" data-notice="no-hollow-cards"><Icon name="alert" size={12} />{notice()}</p>}
        </Show>
        {/* Self-hosted models Q6: whether the chat's own model is asleep,
            starting or ready, with Start now (a lazy chunk). */}
        <Show when={selfHostedChatEntry()}>{id => <Suspense><SelfHostedStatus entryId={id()} /></Suspense>}</Show>
        <Show when={canvasDraft()}>{p =>
          <div class="chat-canvas-confirm" role="group" aria-label={p().req.confirm ? "A draft for the message box" : "Answers from a canvas"}>
            <Icon name="alert" size={12} />
            <Show when={p().req.confirm} fallback={<span>Answers from <strong>{p().req.title || p().req.canvasId}</strong> are ready. Replace what you wrote with them?</span>}>{text => <span>{text()}</span>}</Show>
            <button type="button" class="btn sm" onClick={replaceWithCanvasDraft} title={p().req.confirm ? 'Put it in the message box instead (not sent)' : 'Put the canvas’s answers in the message box instead (not sent)'}>Replace</button>
            <button type="button" class="btn ghost sm" onClick={() => p().settle('cancelled')} title="Keep your message">Cancel</button>
          </div>}
        </Show>
        <div
          ref={composerEl}
          class="chat-composer"
          classList={{ 'drag-over': dragOver(), working: chatBusy() }}
          data-drop-label={dropLabel()}
          onDrop={onDrop}
          onDragOver={onDragOver}
          onDragLeave={() => setDragOver(false)}
        >
          <Show when={attachments().length > 0}>
            <div class="chat-attachments">
              <For each={attachments()}>
                {(a) => (
                  <Show when={a.page} fallback={
                    <Show when={a.file} fallback={
                      <div class={`chat-attach ${a.error ? 'error' : ''} ${a.saving ? 'saving' : ''}`} title={a.name}>
                        <Show when={a.dataUrl}><img src={a.dataUrl} alt={a.name} /></Show>
                        <span class="chat-attach-state" role="status">{a.saving ? attachmentSavingLabel(a.readPercent) : a.error ? 'Failed: remove to send' : a.name}</span>
                        <button type="button" class="chat-attach-remove" title="Remove image" onClick={() => removeAttachment(a.id)}>×</button>
                      </div>
                    }>
                      {/* C5: a file that isn't an image: its name and size. */}
                      <div class={`chat-attach chat-attach-file ${a.error ? 'error' : ''} ${a.saving ? 'saving' : ''}`} title={a.path ?? a.name}>
                        <Icon name="fileText" size={14} />
                        <span class="chat-attach-page-words">
                          <span class="chat-attach-page-label chat-attach-file-name">{a.name}</span>
                          <span class="chat-attach-state" role="status">{a.saving ? attachmentSavingLabel(a.readPercent) : a.error ? 'Failed: remove to send' : fileSizeLabel(a.size ?? 0)}</span>
                        </span>
                        <button type="button" class="chat-attach-remove" title={`Remove ${a.name}`} aria-label={`Remove ${a.name}`} onClick={() => removeAttachment(a.id)}>×</button>
                      </div>
                    </Show>
                  }>{page => (
                    // Repo ideas 2026-10-27 #3: page text the person shared
                    // from Annotate, sent only with their message.
                    <div class="chat-attach chat-attach-page" title={page().url}>
                      <Icon name="fileText" size={14} />
                      <span class="chat-attach-page-words">
                        <span class="chat-attach-page-label">{SHARED_PAGE_LABEL}</span>
                        <span class="chat-attach-state">{sharedPageSummary(page())}</span>
                      </span>
                      <button type="button" class="chat-attach-remove" title="Remove page text" aria-label="Remove page text" onClick={() => removeAttachment(a.id)}>×</button>
                    </div>
                  )}</Show>
                )}
              </For>
            </div>
          </Show>
          <div class="chat-input-wrap">
            <Show when={mention() && !popupOpen() && dismissedCompletion() !== completionKey()}>
              <div class="composer-mention-status" role="status">{mentionLoading() ? 'Loading suggestions…' : mentionError() || (mention()?.kind === '@' ? 'No matching project or conversation files. Use + to attach another file.' : 'No matching skills.')}</div>
            </Show>
            <Show when={mentionedFiles(chatInput()).length > 0}>
              <div class="composer-file-mentions" aria-label="Mentioned files">
                <For each={mentionedFiles(chatInput())}>{file => <span class="composer-file-mention">
                  <button type="button" title={file.path} onClick={() => openFileInSidePane(file.path)}>{file.name}</button>
                  <button type="button" aria-label={`Remove mention of ${file.name}`} onClick={() => setChatInput(text => text.slice(0, file.from) + text.slice(file.to))}>×</button>
                </span>}</For>
              </div>
            </Show>
            <Show when={popupOpen()}>
              <div class="slash-popup" role="listbox" aria-label="Composer suggestions">
                <For each={matches()}>
                  {(cmd, i) => (
                    <button
                      type="button"
                      role="option"
                      aria-selected={i() === popupIndex()}
                      class={`slash-item ${i() === popupIndex() ? 'active' : ''}`}
                      onMouseEnter={() => setPopupIndex(i())}
                      onMouseDown={e => e.preventDefault()}
                      onClick={() => applySuggestion(cmd)}
                    >
                      <span class="slash-name">{cmd.name}</span>
                      <span class="slash-desc">{cmd.desc || (cmd.source === 'claude' ? 'Claude Code skill' : '')}</span>
                      <span class={`slash-tag ${cmd.source}`}>{cmd.source}</span>
                    </button>
                  )}
                </For>
              </div>
            </Show>
            <Show when={suggesting()}>
              <div class="chat-suggest" id={props.compact ? 'chat-suggest-panel' : 'chat-suggest-page'}
                data-review={suggestion()!.review ? '' : undefined}
                title={suggestion()!.reviewWhy === 'outside' ? 'This reply read outside content, so it’s placed in the box for you to check.' : suggestion()!.review ? 'A step that ships (merge, install, deploy, release) is placed in the box for you to check; a second Enter sends it.' : suggestion()!.basis === 'agent' ? 'The agent’s guess at your next message, from this conversation and workspace.' : suggestion()!.basis === 'checked' ? 'The pull request is open on GitHub (checked by Hollow).' : 'Suggested from the reply; Hollow hasn’t checked the pull request.'}>
                <Icon name={suggestion()!.basis === 'agent' ? 'bot' : 'gitMerge'} size={12} />
                <Show when={suggestion()!.review} fallback={<span>Suggested · <kbd>↵</kbd> send · <kbd>Tab</kbd> edit · <kbd>Esc</kbd> dismiss</span>}>
                  <span>{suggestion()!.reviewWhy === 'outside' ? 'Read outside content' : 'Suggested'} · <kbd>↵</kbd> place to check · <kbd>Tab</kbd> edit · <kbd>Esc</kbd> dismiss</span>
                </Show>
                <For each={shipActions().filter(a => a.id !== suggestion()!.id)}>
                  {(alt) => (
                    <button type="button" class="chat-suggest-alt" title={alt.prompt} onMouseDown={e => e.preventDefault()}
                      onClick={() => { setSuggestPick(shipActions().findIndex(a => a.id === alt.id)); textareaEl?.focus(); }}>
                      {alt.label}
                    </button>
                  )}
                </For>
              </div>
            </Show>
            <Show when={richComposer()} fallback={<textarea
              aria-label="Chat message"
              aria-describedby={`${props.compact ? 'chat-compose-help-panel' : 'chat-compose-help-page'}${suggesting() ? (props.compact ? ' chat-suggest-panel' : ' chat-suggest-page') : ''}`}
              ref={composerReady}
              placeholder={placeholder()}
              data-suggesting={suggesting() ? '' : undefined}
              value={chatInput()}
              onInput={e => {
                const v = e.currentTarget.value;
                setChatInput(v);
                setComposerCaret(e.currentTarget.selectionStart);
                setPopupIndex(0);
                if (chatHistoryCursor() != null) setChatHistoryCursor(null);
                if (v.startsWith('/')) ensureDiscovered();
                // Self-hosted models Q6: typing wakes the chat's model (at
                // most once per 2 minutes; not a `/` command); opening or
                // reading never does.
                wakeFromComposer(v);
              }}
              onKeyDown={onKeyDown}
              onClick={e => setComposerCaret(e.currentTarget.selectionStart)}
              onKeyUp={e => setComposerCaret(e.currentTarget.selectionStart)}
              onSelect={e => setComposerCaret(e.currentTarget.selectionStart)}
              onPaste={onPaste}
              rows={shortWindow() ? 1 : 2}
            />}>
              <Suspense fallback={<div role="status">Opening Markdown editor…</div>}>
                <ComposerMarkdown initialDoc={chatInput()} onChange={v => {
                  setChatInput(v); setPopupIndex(0); setChatHistoryCursor(null);
                  if (v.startsWith('/')) ensureDiscovered();
                  wakeFromComposer(v);
                }} composer={{
                  get key() { return chatConversationVersion(); },
                  label: 'Chat message',
                  description: props.compact ? 'chat-compose-help-panel' : 'chat-compose-help-page',
                  onKeyDown, onPaste, onSelection: setComposerCaret,
                  onReady: composerReady,
                  onDispose: () => { if (textareaEl && isRichField(textareaEl)) { dictationSent(textareaEl); textareaEl = undefined; } },
                  onIdentityChange: () => { if (textareaEl) dictationSent(textareaEl); },
                }} />
              </Suspense>
            </Show>
          </div>
          <div class="chat-composer-bar">
            {/* 2026-11-01: one control per job. + holds Attach and the
                commands; the model menu holds the model, effort, permissions
                and canvases; the folder button holds folder and connector access. Each control clips its own text, so a
                narrow composer drops words, never overlaps them.
                Overnight 11-04 (C2): the mic sits beside Send, as in ChatGPT
                and Claude, and the last turn's cost lives in the usage ring
                (its hover text and its card), not as words on the bar.
                Overnight 11-12 (C3): + and the access chip (the permission
                mode) at left; the model picker moved right, first there, so
                the usage ring keeps its place by the mic and its popover's
                right-side anchor. The right group shrinks too: the model
                name clips before any button goes. */}
            <div class="chat-composer-left">
              {/* L4 (plan completion, N2): the slash commands by a click. */}
              <ComposerCommands agent={chatAgent()} commands={allCommands()} busy={chatBusy()} draft={chatInput()} onInsert={applySuggestion}
                onMentionFile={() => openMention('@')} onMentionSkill={() => openMention('$')}
                onToggleMarkdown={toggleRichComposer} markdown={richComposer()}
                extraItems={composerExtraItems()}
                onAttach={() => fileInputEl?.click()} onAddSource={sourcesWorkspace() ? () => void addToSources() : undefined}
                onList={name => runClaudeAdmin(name, '')} onShowAll={() => { setChatInput('/'); setPopupIndex(0); ensureDiscovered(); queueMicrotask(() => textareaEl?.focus()); }}
                onRefuse={why => flashFooter(why, 4500)} />
              <ComposerAccessChip />
              <ComputerChip />
              <ComposerResources state={turnCx} />
            </div>
            <div class="chat-composer-right">
              <ChatModelMenu />
              <ChatUsageMenu onContinue={props.workspace && openWorkspaceId() && chatAgentSessionKey() ? () => void branchFrom(chatTurns.length - 1) : undefined} />
              <DictateButton target={() => textareaEl} />
              {/* P1a PR 2: a paused chat's Resume, where Pause and Stop were. */}
              <Show when={!chatBusy() && chatPausedHere()}>{p => (
                <>
                  <Suspense><ResumeAtControl sessionKey={chatAgentSessionKey()!} resumeAt={p().resumeAt} onResumeNow={() => void resumePausedChat()} /></Suspense>
                  <button class="btn chat-resume" type="button" disabled={chatResumingHere()} aria-busy={chatResumingHere() ? 'true' : undefined} title={p().note ? `Paused: ${p().note}` : 'Resume the paused turn'} onClick={() => void resumePausedChat()}><Icon name="play" size={12} /><span>Resume</span></button>
                </>
              )}</Show>
              <Show
                when={chatBusy()}
                fallback={
                  <button class="btn primary chat-send" type="submit" aria-label={placesSuggestion() ? 'Place suggestion' : 'Send'} title={placesSuggestion() ? 'Place suggestion' : enterSends() ? 'Send (Enter)' : 'Send (⌘Enter)'} disabled={sendBlocked()}><Icon name="arrowUp" size={16} /></button>
                }
              >
                {/* Mid-turn steering: while a turn runs the box stays open.
                    This button does what Enter does (its name says which);
                    Send options picks another for this one send. */}
                <button class="btn primary chat-send chat-send-busy" type="submit" aria-label={busyChoiceLabel(busyChoice())} title={`${busyChoiceLabel(busyChoice())} (${enterSends() ? 'Enter' : '⌘Enter'})`} disabled={sendBlocked() || !composerHasMessage()}>
                  <Icon name={busyChoice() === 'steer' ? 'arrowUp' : busyChoice() === 'stop' ? 'stop' : 'clock'} size={14} />
                  <span class="chat-send-busy-label">{busyChoiceLabel(busyChoice())}</span>
                </button>
                <span class="chat-send-options">
                  <ActionMenu label="Send options" align="end" icon={<Icon name="chevronDown" size={14} />} items={sendOptions()} />
                </span>
                {/* P1a PR 2: Pause ends the turn at its next safe point and
                    keeps its context; while asked it reads Pausing…, and
                    Stop beside it still works and wins. */}
                <Show when={canPauseChatRun() || chatPausingHere()}>
                  <button class="btn ghost chat-pause" type="button" disabled={chatPausingHere()} aria-busy={chatPausingHere() ? 'true' : undefined} aria-label={chatPausingHere() ? 'Pausing…' : 'Pause'}
                    title={chatPausingHere() ? 'Pausing at the next safe point. Stop still ends it now.' : 'Pause at the next safe point, keeping this chat’s context for Resume'}
                    onClick={() => void pauseChatRun()}>{chatPausingHere() ? 'Pausing…' : 'Pause'}</button>
                </Show>
                <button class="btn ghost danger chat-stop" type="button" aria-label="Stop" title={stopTitle(shownStopKey())} onClick={stop}><Icon name="stop" size={14} /></button>
              </Show>
            </div>
          </div>
        </div>
        <Show when={chatBusy() && composerHasMessage()}>
          <div class="chat-busy-hint" role="status" aria-live="polite">{busyHint(busyChoice(), agentName(), steerWhy(), enterSends() ? 'Enter' : '⌘Enter', shownStopKey())}</div>
        </Show>
        <div class="chat-compose-help sr-only" id={props.compact ? 'chat-compose-help-panel' : 'chat-compose-help-page'}>{enterSends() ? 'Enter to send, Shift+Enter for a new line.' : '⌘Enter to send, Enter for a new line.'} Type / for commands.</div>
      </form>
    </>
  );
}

/** A finished run's steps as one quiet disclosure row (frontier F5;
 *  overnight 11-12, C1): "Worked for 1m 10s ›", expanding to the steps
 *  themselves. The step count is read out and in the tooltip. */
/** The content height a text would take in `ta` at its width (ENG-93,
 *  `caretEdges`): measured in one hidden copy of the box, 0 with no layout. */
let measureBox: HTMLTextAreaElement | null = null;
function textHeightIn(ta: HTMLTextAreaElement): (text: string) => number {
  return (text: string) => {
    if (!ta.clientWidth) return 0;
    if (!measureBox) {
      measureBox = document.createElement('textarea');
      measureBox.setAttribute('aria-hidden', 'true');
      measureBox.tabIndex = -1;
    }
    const cs = getComputedStyle(ta);
    const box = measureBox;
    box.style.cssText = 'position:absolute;visibility:hidden;left:-9999px;top:0;height:0;min-height:0;max-height:none;overflow:hidden;resize:none;border:0;box-sizing:content-box;';
    for (const prop of ['font', 'letterSpacing', 'wordSpacing', 'textIndent', 'textTransform', 'whiteSpace', 'overflowWrap', 'wordBreak', 'tabSize', 'paddingLeft', 'paddingRight'] as const) {
      (box.style as unknown as Record<string, string>)[prop] = cs[prop] as string;
    }
    // clientWidth leaves out the border and any scroll bar, as the text does.
    box.style.width = `${Math.max(0, ta.clientWidth - parseFloat(cs.paddingLeft || '0') - parseFloat(cs.paddingRight || '0'))}px`;
    if (!box.isConnected) document.body.appendChild(box);
    box.value = text;
    return box.scrollHeight;
  };
}

function RunFoldRow(props: { fold: RunFold; typicalMs?: number | null; workKind?: string | null; open: boolean; onToggle: () => void; children: JSX.Element }) {
  const summary = () => foldSummary({ ...props.fold, typicalMs: props.typicalMs, workKind: props.workKind });
  return (
    <div class="chat-run-fold" classList={{ open: props.open }}>
      <button type="button" class="chat-run-fold-head" aria-expanded={props.open} title={`${props.fold.steps} ${props.fold.steps === 1 ? 'step' : 'steps'}: show them`} onClick={() => props.onToggle()}>
        <span class="chat-run-fold-text">{summary()}</span>
        <span class="sr-only">{foldSteps(props.fold)}</span>
        <Show when={props.fold.failed > 0}><span class="chat-run-fold-failed">· {props.fold.failed} failed</span></Show>
        <span class="chat-run-fold-caret" aria-hidden="true"><Icon name="chevronRight" size={12} /></span>
      </button>
      <Show when={props.open}><div class="chat-run-fold-body chat-run-timeline" data-motion-visible="false" ref={observeChatMotion}>{props.children}</div></Show>
    </div>
  );
}

/** Starter prompts shown on the empty chat — click to drop into the composer. */
type ChatSuggestion = { title: string; description: string; icon: import('../icons').IconName; prompt: string };
const WORKSPACE_SUGGESTIONS: ChatSuggestion[] = [
  { title: 'Find the next step', description: 'Summarize where things stand', icon: 'checkSquare', prompt: 'Read WORKSPACE.md and PLAN.md. Summarize where this work stands and suggest the most useful next step. Flag any missing context before making changes.' },
  { title: 'Make a first draft', description: 'Save it as a Result (in outputs/)', icon: 'fileText', prompt: 'Read the workspace brief, plan, and attached sources. Create a first draft of the requested result in outputs/. Make assumptions and unanswered questions clear.' },
  { title: 'Review the work', description: 'Check the outputs against the goal', icon: 'search', prompt: 'Review the current outputs (outputs/) against the workspace outcome and attached evidence. List concrete gaps, unsupported claims, and the checks needed before this is ready.' },
];
const MAIN_AGENT_SUGGESTIONS: ChatSuggestion[] = [
  { title: 'What needs me today', description: 'Across every workspace', icon: 'target', prompt: 'Look across my workspaces, tasks and what is waiting on me. What needs my attention today, in order, and what can you take off my plate?' },
  { title: 'Check on running work', description: 'Subagents and automations in progress', icon: 'eye', prompt: 'Check on my running and recent subagents. What finished, what is still working, what is stuck, and what is waiting on me?' },
  { title: 'Hand off a task', description: 'Send it to the right project lead', icon: 'send', prompt: 'Send this to the right project lead: ' },
  { title: 'Plan my week', description: 'Priorities from my workspaces', icon: 'calendar', prompt: 'From my workspaces, plans and calendar, what should my priorities be this week? Keep it to five.' },
];
const SUGGESTIONS: ChatSuggestion[] = [
  { title: 'Plan my day', description: 'What needs doing today', icon: 'checkSquare', prompt: "What's on my plate today?" },
  { title: 'Prepare for a meeting', description: 'Notes and context for what’s next', icon: 'calendar', prompt: 'Prepare me for my next meeting' },
  { title: 'Review my workspaces', description: 'Which ones need attention', icon: 'folder', prompt: 'Which of my workspaces need attention, and what is the next step in each?' },
  { title: 'Find something', description: 'Search projects and Library', icon: 'search', prompt: 'Search my projects and Library for ' },
];
