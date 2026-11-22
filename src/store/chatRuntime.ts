// The on-screen conversation's engine: agent settings and permission modes,
// chat state and its persistence, queued turns, question cards, the transcript
// reducers, the conversation's files for the Preview pane, the chat-event handler,
// conversations kept running in the background, and the `chat-event` listener.
//
// Import cycle with ./runtime (deliberate, documented): this module imports
// requestChatFocus from ./runtime, and runtime.ts imports
// and re-exports this module. Every one of those bindings is read only inside
// functions and listener callbacks that run after both modules have
// evaluated, never at this module's top level, so neither evaluation order can
// hit a TDZ: runtime.ts is always entered first (its only other importer,
// chatApprovals.ts, is itself imported only by runtime.ts), its import of this
// module runs this body before runtime's own, and this body touches no
// runtime binding. Keep it that way: a new top-level use of a
// ./runtime binding here would throw at launch.
// Moved verbatim out of store/runtime.ts (plan L3 X19, 2026-10-14); runtime.ts
// re-exports every symbol, so `store/runtime` imports keep working.
import { batch, createSignal, createEffect, createMemo, createRoot } from 'solid-js';
import { createStore, produce, unwrap } from 'solid-js/store';
import { createTranscript } from '../chatTranscript';
import { acceptFirstDraftSession, rememberBeforeDraftReset } from './conversationDrafts';
import { closePreview, dropConversationPreviews, dropSubagentPreviews, subagentTabParents, newPreviewIntent, openPreview, previewNonce, previewTarget, setPreviewNonce } from './preview';
import { listen } from '../nativeEvents';
import type { ChatEventEnvelope } from '../generated/chatEvent';
import { api } from '../api';
import { executionReceiptFromWire, type EvidenceReceiptWire, type EvidenceRunContextWire, type ExecutionReceipt } from '../executionReceipt';
import { REPLY_VIDEO_CAP, receiptVideos } from '../replyMedia';
import { cancelledNotice, pausedNotice, RESUME_PROMPT, type PauseReason } from '../chatStop';
import { beginResume, chatPausedFor, chatPausingFor, chatResumingFor, clearChatPaused, endResume, markChatPaused, refreshChatPaused, setChatPausing } from './chatPause';
import { resultEventTurn } from '../resultKind';
import { sessionResetNotice, stopOffersContinue, type SessionResetReason } from '../chatContinue';
import { accountSwitchNotice, agentContinuedNotice } from '../accountSwitchNotice';
import type { RemoteRunEvent } from '../remoteRunNotice';
import { unsafeCwdText } from '../chatCwd';
import { steerTurn, type HeldChip, type SteerSupportRow, type SteerWire } from '../chatSteer';
import { continuedFromAfter } from '../chatContinuedFrom';
import { collectVisuals } from '../chatVisuals';
import { bufferEvent, routeEvent, type Route } from '../chatRouting';
import { nextChatActivity, type ChatActivity } from '../runActivity';
import { attachmentImages, type SharedPage } from '../sharedPage';
import { ATTACH_MAX_COUNT, turnFilesField } from '../turnFiles';
import {
  acceptedShowsBadge, adoptedClientTurnId, CHAT_CONTEXT_RECOVERED_PREFIX, eventMatchesTurn, INTERRUPTED_TURN_NOTICE,
  isStaleTurnEvent, mergeRestoredKeys, planQueuedChatRows, queuedCancelNotice, queuedFailureNotice, queuedTurnEnd, rowWait,
  restoredDraft, RESTORED_QUEUED_TURNS_KEY, addToDraft, stopPreparingSend, type QueuedTurnDecision, type QueuedTurnIntent,
  newClientTurnId, turnStanding,
} from '../chatQueue';
import { errMsg } from '../utils';
import { agentSettingsPayload } from '../agentSettingsPayload';
import { bumpSettingsRevision } from './settingsRevision';
import { latestRequest, unchangedSince } from '../latestRequest';
import { chainCardKeysFor } from './chatChainState';
import type { AgentSettings, AgentDescriptor, TurnUsage, BudgetNotice } from '../types';
import { budgetNoticeText } from '../budgets';
import { agentModeChips, agentModeDanger, agentModeDescriptions, agentModeLabels, agentSwitchMode, hollowCardsLine, newChatModeFor as newChatModeForAgent, newChatModeKey } from '../agentPermModes';
import { permModeConfirmName, permModeDescriptions, rememberConversationMode, preserveCurrentModeOnUpgrade, conversationPermission, conversationModeRecord, conversationModeReport, modeClass } from '../permModes';
import { chatCanvasesOnKeyChange, chatCanvasesStorageKey, parseCanvasFrequency, type CanvasFrequency } from '../canvasFrequency';
import { cachedChatEffort, chatEffortWriteInFlight, loadChatEfforts, putChatEffort, putChatEffortTracked, refreshChatEfforts, type ChatEffortSave } from './chatEffortStore';
import { chatUltracodeStorageKey, effortOnKeyChange, parseStoredEffort, parseStoredUltracode, ultracodeOnKeyChange } from '../ultracode';
import type { RunQueueItem, SendAccepted, ChatInteraction, ProgressPhase } from '../types';
import type { ApprovalBadge } from '../chatApproval';
import { ANSWERS_IN_COMPOSER_NOTICE, answerSendFailureAction, applyInteractionUpdate, keepOpenCards } from '../questionCardList';
import { showConfirm } from './overlays';
import { flashFooter } from './snackbars';
import { answeredRevision, answersBack, settleHeldAnswers } from '../canvas/askHold';
import { requestChatFocus } from './runtime';

// ─────────────────────────────────────────────────────────────
// CHAT — global state survives page navigation, listener is
// installed at module init so streaming events never get lost.
// ─────────────────────────────────────────────────────────────

export type { ChatToolStatus, ChatToolUse, ChatTurn } from '../chatTurnTypes';
import type { ChatToolUse, ChatTurn } from '../chatTurnTypes';
import { markedReplyAt, turnsFromConversation } from '../chatHydrate';
import { chatArtifactKind, type ChatArtifactKind } from '../chatArtifactKind';
/// Approval badges by tool-use id (Paperclip PR 21). Written by both approval
/// listeners, read when the `toolUse` event creates its row, so the badge
/// lands whichever arrives first. Cleared when a new turn starts.
/// Exported only for `chatApprovals.ts`; nothing else should write it.
export const pendingApprovalBadges = new Map<string, ApprovalBadge>();

// Which agent CLI backs the chat. Each maps to a headless binary the
// backend spawns: claude / codex / agy (Antigravity) / grok. Per-session, persisted.
export type ChatAgent = 'claude' | 'codex' | 'antigravity' | 'grok';
export const KNOWN_AGENTS: ChatAgent[] = ['claude', 'codex', 'antigravity', 'grok'];
export const AGENT_LABELS: Record<ChatAgent, string> = {
  claude: 'Claude',
  codex: 'Codex',
  antigravity: 'Antigravity',
  grok: 'Grok',
};
// Models are provider-owned identifiers, not a hard-coded catalog.
export const AGENT_DEFAULT_MODEL: Record<ChatAgent, string> = { claude: '', codex: '', antigravity: '', grok: '' };
export const [agentSettings, setAgentSettings] = createSignal<AgentSettings>({ defaultAgent: null, models: {} });
export const [agentDescriptors, setAgentDescriptors] = createSignal<AgentDescriptor[]>([]);
export const [agentSettingsError, setAgentSettingsError] = createSignal('');
export const currentAgentCapabilities = () => agentDescriptors().find(a => a.id === chatAgent())?.capabilities;
function preferredAgent(): ChatAgent {
  const selected = agentSettings().defaultAgent;
  return selected && KNOWN_AGENTS.includes(selected as ChatAgent) ? selected as ChatAgent : agentsAvailable()[0] ?? initAgent;
}
/** The agent a one-off run that names none uses (`agent_settings::resolve`): the default, else the first available. */
export function defaultAgentLabel(): string { return AGENT_LABELS[preferredAgent()]; }
let chatAgentOverridden = false;

let effortSaves: Promise<unknown> = Promise.resolve();
/** Save one agent's reasoning effort at once (P0-19), the app default:
 *  Settings' effort rows come here, on the live settings, so no other save
 *  writes an older value back. `''` is the runtime default. Since Settings PR 7
 *  the composer's Effort list no longer does: it sets that chat only
 *  ([`setChatEffortPersistent`]). */
export function saveAgentEffort(agent: string, level: string): Promise<void> {
  // One at a time: two quick picks each read the file before writing it, so
  // run concurrently the later write could carry the earlier read (review).
  const next = effortSaves.then(() => writeAgentEffort(agent, level));
  effortSaves = next.catch(() => undefined);
  return next;
}
async function writeAgentEffort(agent: string, level: string): Promise<void> {
  const live = await api.agentSettings();
  const efforts = { ...(live.efforts ?? {}) };
  if (level) efforts[agent] = level; else delete efforts[agent];
  await api.setAgentSettings(agentSettingsPayload({ ...live, efforts }));
  await refreshAgentSettings();
  bumpSettingsRevision();
}

export async function refreshAgentSettings(preparingSend = false) {
  try {
    const [settings, descriptors] = await Promise.all([api.agentSettings(), api.agentDescriptors()]);
    setAgentSettings(settings);
    setAgentDescriptors(descriptors);
    setAgentsAvailable(descriptors.filter(a => a.available && KNOWN_AGENTS.includes(a.id as ChatAgent)).map(a => a.id as ChatAgent));
    setAgentSettingsError('');
    if (!chatAgentOverridden && (!chatBusy() || preparingSend) && !chatSessionLoading() && !chatSessionId() && !chatAgentSessionKey() && chatTurns.length === 0) {
      setChatAgent(preferredAgent());
      lsSet('chatAgent', chatAgent());
      setChatModelPersistent(settings.models[chatAgent()] ?? '');
    }
  } catch (err) { setAgentSettingsError(String(err)); }
}

/// Hollow's canonical modes (`app_mcp::PERMISSION_MODES`). A chat names them
/// in its agent's own words (`agentPermModes.ts`); `default` is Claude's and
/// Grok's Default and Codex's Ask for approval (per-agent menus, 2026-11-12).
export type ChatPermissionMode = 'bypassPermissions' | 'auto' | 'acceptEdits' | 'default' | 'plan';
const KNOWN_PERM_MODES: ChatPermissionMode[] = ['bypassPermissions', 'auto', 'acceptEdits', 'default', 'plan'];
/// Display order for permission-mode pickers. Safer modes first, with
/// `bypassPermissions` last so it's not the first thing a user lands on
/// when they open the dropdown. UI surfaces also flag it as danger.
export const PERM_MODE_ORDER: ChatPermissionMode[] = ['acceptEdits', 'auto', 'plan', 'bypassPermissions'];
/// Whether a mode skips all permission prompts. UI uses this to gate the
/// option behind an explicit confirmation step.
export const PERM_MODE_DANGER: Record<ChatPermissionMode, boolean> = {
  bypassPermissions: true,
  auto:              false,
  acceptEdits:       false,
  default:           false,
  plan:              false,
};
/** Friendly labels shown in the picker. */
export const PERM_MODE_LABELS: Record<ChatPermissionMode, string> = {
  bypassPermissions: 'Bypass all (danger)',
  auto:              'Auto',
  acceptEdits:       'Edits only',
  default:           'Ask first',
  plan:              'Plan',
};
/** The mode as the composer's access chip shows it, beside the + (overnight
 *  11-12, C3; it was a tag on the model button from 2026-11-01). The
 *  warning tint and the chip's name carry what "(danger)" said. */
export const PERM_MODE_SHORT_LABELS: Record<ChatPermissionMode, string> = {
  bypassPermissions: 'Full access',
  auto:              'Auto',
  acceptEdits:       'Edits only',
  default:           'Ask first',
  plan:              'Plan',
};
/** Short tooltip/description for each mode. */
export const PERM_MODE_DESC: Record<ChatPermissionMode, string> = {
  bypassPermissions: 'Everything runs with no permission prompts, teammates and Hollow’s own cards (build, delete) included, except six Hollow actions that always need you: installing Hollow, archiving a tracker issue, applying the gardener’s changes, superseding a decision, drafting a friction fix and confirming a runbook step. In a chat they ask first; an automation or check-in can’t run them.',
  auto:              'Everything runs on the agent’s automatic approval. Hollow still asks before anything leaves this Mac, deletes, touches secrets or gives agents more power.',
  acceptEdits:       'File edits are allowed within the agent’s permission scope. Other actions follow its approval policy.',
  default:           'The agent’s standard mode: what it would ask about is declined, since a Hollow run has no terminal to ask in.',
  plan:              'Use the agent’s read-only or planning mode.',
};
/** {@link PERM_MODE_DESC} as it reads for `agent`, with that agent's
 *  corrections (Antigravity's Edits only can't edit: `permModes.ts`). */
export function permModeDescFor(agent: string | null | undefined): Record<ChatPermissionMode, string> {
  return permModeDescriptions(PERM_MODE_DESC, agent);
}

/// Shared "are you sure you want to enable danger mode?" gate used by
/// every surface that lets the user pick a permission mode (Settings,
/// Chat dropdown, `/permissions` slash command). Resolves to true if
/// the change should proceed.
///
/// Non-danger transitions always resolve true without prompting. Uses
/// the in-app confirm modal (`showConfirm`) so there's no native
/// `window.confirm` blocking the renderer.
/// `agent` names the mode in that agent's words (`agentPermModes.ts`), and a
/// mode it flags (Grok's Auto runs with its sandbox off) asks too; with no
/// agent (an automation's picker) the general names and danger hold.
export async function confirmPermissionModeChange(next: ChatPermissionMode, agent?: string | null): Promise<boolean> {
  const danger = agent ? agentModeDanger(agent)[next] : PERM_MODE_DANGER[next];
  if (!danger) return true;
  const name = agent ? permModeConfirmName(agentModeLabels(agent)[next], agentModeChips(agent)[next]) : permModeConfirmName(PERM_MODE_LABELS[next], PERM_MODE_SHORT_LABELS[next]);
  const what = agent ? `${agentModeDescriptions(agent)[next]} ${hollowCardsLine(agent, agentDescriptors().find(a => a.id === agent)?.capabilities?.ownedMcp)}` : PERM_MODE_DESC[next];
  return showConfirm({
    title: `Enable ${name}?`,
    body:
      `${what}\n\n` +
      `You can switch back any time.`,
    confirmLabel: 'Enable danger mode',
    cancelLabel: 'Keep current mode',
    danger: true,
  });
}

/** The chat on screen now, checked after a mode confirm: false once another
 *  chat or a new conversation took its place while the dialog was open (a
 *  shortcut, an agent's navigate), so the answer never lands on that chat
 *  (lane S re-review of #1304). Every per-chat mode picker uses it: the
 *  access chip, the model menu's Permissions row and `/permissions`. */
export function sameChatAsNow(): () => boolean {
  return unchangedSince(() => [chatAgentSessionKey(), chatConversationVersion()]);
}

function lsGet(key: string): string | null {
  try { return localStorage.getItem(key); } catch { return null; }
}
function lsSet(key: string, val: string | null) {
  try {
    if (val == null) localStorage.removeItem(key);
    else localStorage.setItem(key, val);
  } catch { /* ignore */ }
}

// Up-arrow history of prompts the user has sent. Most-recent first.
// Capped at 200 entries. Persists across app restarts.
const CHAT_HISTORY_CAP = 200;
function readHistory(): string[] {
  try {
    const raw = localStorage.getItem('chatHistory');
    if (!raw) return [];
    const arr = JSON.parse(raw);
    return Array.isArray(arr) ? arr.filter(s => typeof s === 'string').slice(0, CHAT_HISTORY_CAP) : [];
  } catch { return []; }
}
export const [chatHistory, setChatHistory] = createSignal<string[]>(readHistory());
export const [chatHistoryCursor, setChatHistoryCursor] = createSignal<number | null>(null);

export function pushChatHistory(s: string) {
  const text = s.trim();
  if (!text) return;
  const h = chatHistory();
  if (h[0] === text) return; // dedup consecutive
  const next = [text, ...h.filter(x => x !== text)].slice(0, CHAT_HISTORY_CAP);
  setChatHistory(next);
  try { localStorage.setItem('chatHistory', JSON.stringify(next)); } catch { /* ignore */ }
}

export const [chatTurns, setChatTurns] = createStore<ChatTurn[]>([]);
export const [chatSessionId, setChatSessionId] = createSignal<string | null>(lsGet('chatSessionId'));
/** Hollow's own session key for the live conversation, minted by the backend
 *  and echoed back on every send. Distinct from `chatSessionId`, which is the
 *  HARNESS's id: that one is null for Antigravity, resets on an agent switch,
 *  and does not exist at all until the agent's stream announces it. */
export const [chatAgentSessionKey, setChatAgentSessionKey] = createSignal<string | null>(lsGet('chatAgentSessionKey'));
/** The conversation on screen whose stored transcript this window has not
 *  read yet: the key a reload restores from localStorage (its turns are not
 *  saved there) or one the run queue adopts. Opening it reads the store even
 *  when a live event or the adoption already made it look loaded (turns on
 *  screen, busy), so earlier turns come back after a reload mid-turn
 *  (overnight 11-13 c-11). Cleared once the stored read lands, and whenever
 *  the key changes to another conversation. */
export const [chatStoredTranscriptPending, setChatStoredTranscriptPending] = createSignal<string | null>(lsGet('chatAgentSessionKey'));
/** The earlier conversations this one continues from (`chatContinuedFrom.ts`):
 *  a Continue from summary or a folder-vanished reset. Every send names them,
 *  so the backend can bundle their turns on a fresh native session. */
export const [chatContinuedFrom, setChatContinuedFrom] = createSignal<string[]>([]);

/** Canvas C3 wave 2 C4: this chat's own Canvases choice (the model menu), or
 *  null to follow Settings, Chat, Canvases. It travels with the
 *  conversation: kept under its session key, read back whenever that
 *  conversation is on screen again, and sent with each of its turns. */
export const storedChatCanvases = (key: string | null) => (key ? parseCanvasFrequency(lsGet(chatCanvasesStorageKey(key))) : null);
export const [chatCanvases, setChatCanvases] = createSignal<CanvasFrequency | null>(storedChatCanvases(chatAgentSessionKey()));
export function setChatCanvasesPersistent(choice: CanvasFrequency | null) {
  setChatCanvases(choice);
  const key = chatAgentSessionKey();
  if (key) lsSet(chatCanvasesStorageKey(key), choice);
}

/// Merged plan Track U: this chat's Ultracode switch, the last option in
/// the model menu's Effort list (it never saves as a level). Claude runs standing dynamic-workflow orchestration
/// (`--effort ultracode`, or xhigh with a note on a CLI without it); every
/// other agent runs at its model's top effort. Never a Settings default.
/// Effort is per turn and not part of the native session, so switching it
/// keeps the conversation going. Since P1-10 it is the conversation's own,
/// kept under its session key like Canvases: New chat starts with it off
/// (the saved effort shows), and reopening a conversation reads its value.
export const storedChatUltracode = (key: string | null): boolean => !!key && parseStoredUltracode(lsGet(chatUltracodeStorageKey(key))) === true;
export const [chatUltracode, setChatUltracode] = createSignal<boolean>(storedChatUltracode(chatAgentSessionKey()));
export function setChatUltracodePersistent(on: boolean) {
  setChatUltracode(on);
  const key = chatAgentSessionKey();
  if (key) lsSet(chatUltracodeStorageKey(key), on ? 'true' : null);
}

/// Settings PR 7: this chat's own reasoning effort, set in the model menu's
/// Effort list. Since 2026-11-12 it lives in Rust (the session store, through
/// `chat_effort_set`; `chatEffortStore.ts` is this window's cache), so a run
/// Hollow starts itself (an automation in the chat, a wake-up, a handover
/// successor, a resume, a phone send) uses it too, not only the composer's
/// sends. It never changes the app default (Settings, Models, Effort).
/// `null`: the chat runs at the default.
export const storedChatEffort = (key: string | null): string | null => cachedChatEffort(key);
export const [chatEffort, setChatEffort] = createSignal<string | null>(null);
export function setChatEffortPersistent(level: string | null): Promise<boolean> {
  setChatEffort(level);
  const key = chatAgentSessionKey();
  return key ? putChatEffort(key, level) : Promise.resolve(true);
}

/** The model menu's pick: `setChatEffortPersistent`, saying whether a failed
 *  save is still the newest pick for its chat (R11b review, 2026-11-15). */
export function setChatEffortTracked(level: string | null): Promise<ChatEffortSave> {
  setChatEffort(level);
  const key = chatAgentSessionKey();
  return key ? putChatEffortTracked(key, level) : Promise.resolve('saved');
}

/** Put one chat's effort and Ultracode back after its newest save failed, by
 *  its session key: its stored Ultracode, its level (written back to Rust),
 *  and the on-screen signals only when that chat is the open one, so a
 *  failure that lands after the person moved on never touches another chat. */
export function restoreChatEffortChoice(key: string, effort: string | null, ultracode: boolean): void {
  lsSet(chatUltracodeStorageKey(key), ultracode ? 'true' : null);
  void putChatEffort(key, effort);
  if (chatAgentSessionKey() === key) {
    setChatUltracode(ultracode);
    setChatEffort(effort);
  }
}

/** After a read from Rust, show the open conversation's level (a handover
 *  successor inherits its predecessor's in Rust). A chat with no key yet
 *  keeps a level picked before its first send; a write on its way wins. */
function syncChatEffortFromStore() {
  const key = chatAgentSessionKey();
  if (!key || chatEffortWriteInFlight(key)) return;
  const stored = cachedChatEffort(key);
  if (stored !== chatEffort()) setChatEffort(stored);
}

/** Window start (App): move the old localStorage levels into Rust once, read
 *  every chat's level, and show the open chat's. */
export async function startChatEfforts(): Promise<void> {
  if (await loadChatEfforts()) syncChatEffortFromStore();
}

/** `ownTurn`: the key comes from this conversation's own turn (the send's
 *  reply or its events), so a new chat's first send keeps its Ultracode. */
export function setChatAgentSessionKeyPersistent(key: string | null, opts: { ownTurn?: boolean } = {}) {
  const prev = chatAgentSessionKey();
  const next = chatCanvasesOnKeyChange(prev, key, chatCanvases(), storedChatCanvases(key));
  const ultracode = ultracodeOnKeyChange(prev, key, chatUltracode(), storedChatUltracode(key), !!opts.ownTurn);
  const effort = effortOnKeyChange(prev, key, chatEffort(), storedChatEffort(key), !!opts.ownTurn);
  setChatAgentSessionKey(key);
  lsSet('chatAgentSessionKey', key);
  if (chatStoredTranscriptPending() !== key) setChatStoredTranscriptPending(null);
  setChatEffort(effort.level);
  if (effort.persist && key) putChatEffort(key, effort.level);
  setChatCanvases(next.choice);
  if (next.persist && key) lsSet(chatCanvasesStorageKey(key), next.choice);
  setChatUltracode(ultracode.on);
  if (ultracode.persist && key) lsSet(chatUltracodeStorageKey(key), 'true');
  // Rust may hold a level this window hasn't seen (a handover successor
  // inherits its predecessor's): read it for the conversation now on screen.
  if (key && key !== prev) void refreshChatEfforts().then(ok => { if (ok && chatAgentSessionKey() === key) syncChatEffortFromStore(); });
  // P1a PR 2: a conversation opened from the list shows Resume when its
  // record says paused (this window may never have seen its `paused` event).
  if (key && key !== prev) void refreshChatPaused(key);
  // Only this screen's first send establishes a new choice. Replayed or
  // adopted turn events must never replace an existing conversation choice.
  if (opts.ownTurn && key && !prev) {
    rememberConversationMode(key, chatPermissionMode());
    reportConversationMode(key, chatPermissionMode());
  }
}

/** What the phone chat API keeps of this window's modes goes there one
 *  report at a time, in the order they were made, so an older one never
 *  lands last (second review, 2026-11-04). Fire and forget: the phone's copy
 *  never stands in the way of the window's own choice. */
let phoneModeReports: Promise<unknown> = Promise.resolve();
function reportToPhone(send: () => Promise<unknown>) {
  phoneModeReports = phoneModeReports.then(send).catch(() => {});
}

/** A chat's mode choice reaches the phone chat API too: a phone's send to
 *  that chat runs in the mode this window shows for it
 *  (`phone_access_chat_modes`): the one choice, or the whole record when
 *  the one on disk changed wholesale (`conversationModeReport`). */
function reportConversationMode(key: string | null, mode: ChatPermissionMode) {
  const report = conversationModeReport(key, mode);
  if (report) reportToPhone(() => api.phoneAccessChatModes(report.choices, report.replace, report.plan));
}

/** Everything the phone chat API keeps of this window's modes, whole: the
 *  mode for new chats, then every chat's choice, replacing its copy. Sent
 *  when Companion devices opens and when Hollow asks
 *  (`phone-access-modes-wanted`, after a launch once a phone request relies
 *  on its copy: at once and every 250 ms for 30 seconds, until this window
 *  answers), so a record this window lost (cleared web storage, its Plan
 *  fallback) doesn't outlive it there. No call at launch. */
export function reportPhoneModes() {
  const agent = preferredAgent();
  const mode = newChatModeFor(agent);
  const record = conversationModeRecord();
  reportToPhone(() => api.phoneAccessNewChatMode(mode, agent));
  reportToPhone(() => api.phoneAccessChatModes(record.choices, true, record.plan));
}
// null = the vault (the backend default). Persisted ONLY so an app restart
// can resume the persisted session in its own directory (Claude sessions are
// keyed by project dir — resuming from elsewhere fails). Every NEW chat
// resets it to the vault in resetChatState(); /cwd is conversation-scoped.
export const [chatCwd, setChatCwd] = createSignal<string | null>(lsGet('chatCwd'));
/** The last finished turn's usage (Paperclip PR 6), from the `usage` chat
 *  event every post-lease exit emits. Reset when a turn starts, so the chip
 *  never shows the previous turn's figure; `costChip` hides it rather than
 *  show "$0". */
export const [chatTurnUsage, setChatTurnUsage] = createSignal<TurnUsage | null>(null);
/** How full the conversation's context window was after its last message
 *  (the `usage` event's `contextTokens`), tagged with the conversation
 *  version so another conversation never shows it. Unlike chatTurnUsage it
 *  survives the next turn's start, so the composer's ring stays put while a
 *  turn runs. Read it through `chatContextUsage`. `window` is the one the
 *  agent reported (Codex, overnight 9 A10), else absent. */
export interface ChatContextUse { tokens: number; model: string | null; window?: number | null }
const [chatContextRaw, setChatContextRaw] = createSignal<{ version: number } & ChatContextUse | null>(null);
export const [chatBusy, setChatBusy] = createSignal(false);
export const [chatSessionLoading, setChatSessionLoading] = createSignal(false);
/** Invalidates asynchronous conversation work when a new chat or agent is
 *  selected. Unlike a harness session id, this changes before any send. */
export const [chatConversationVersion, setChatConversationVersion] = createSignal(0);
/** The current conversation's context use, or null (none reported yet, or a
 *  fresh native session since). */
export function chatContextUsage(): ChatContextUse | null {
  const raw = chatContextRaw();
  return raw && raw.version === chatConversationVersion() ? raw : null;
}
export const [chatInput, setChatInput] = createSignal('');
/** Queued images and files share the draft's lifetime across the page and
 *  side panel. They stay in memory only; data URLs must not be written to
 *  localStorage. */
export interface ChatAttachment {
  id: number;
  name: string;
  mime: string;
  dataUrl: string;
  path?: string;
  saving: boolean;
  /** FileReader progress only; 100 still waits for the local save to finish. */
  readPercent?: number;
  error?: boolean;
  /** C5 (2026-11-04): a file that isn't an image. No thumbnail (`dataUrl`
   *  is empty): a chip with its name and `size` in bytes, and an
   *  `<attached-files>` line at send (composerFiles.ts; the chips, turnFiles.ts). */
  file?: boolean;
  size?: number;
  /** Repo ideas 2026-10-27 #3: page text the person shared from Annotate
   *  (Include page text). No image and no file: it rides in the message
   *  as a `<shared-page>` block (sharedPage.ts). */
  page?: SharedPage;
}
export const [chatAttachments, setChatAttachments] = createSignal<ChatAttachment[]>([]);
/** Add a shared page's text to the composer as a removable chip (Annotate's
 *  Include page text). Nothing is sent until the person sends. */
export function attachSharedPage(page: SharedPage): boolean {
  // A message holds at most ATTACH_MAX_COUNT attachments (C5 review).
  if (chatAttachments().length >= ATTACH_MAX_COUNT) return false;
  setChatAttachments(a => [...a, { id: nextChatAttachmentId(), name: page.title || page.url, mime: 'text/plain', dataUrl: '', saving: false, page }]);
  return true;
}
/** Add a file to the composer from outside it: the side panel browser's
 *  Annotate, Ask to fix, and since 2026-11-04 a file dropped anywhere in
 *  the window (windowFileDrop.ts). The composer registers its own attach
 *  path as the sink; files sent while it isn't mounted wait here until it
 *  is. `known`: a drop's entry said it is a file (never a folder), as the
 *  composer's own drop passes it. */
type ChatFileSink = { take: (file: File, known: boolean) => boolean | void; onScreen: () => boolean };
let chatFileSink: ChatFileSink | null = null;
const pendingChatFiles: { file: File; known: boolean }[] = [];
/** True when the composer took the file, or it waits for the composer to
 *  mount; false when the composer refused it (a full message, an empty or
 *  oversized file, said in words: C5 review) or, with `queueIfUnmounted`
 *  off, no composer is mounted. */
export function attachChatImage(file: File, queueIfUnmounted = true, known = false): boolean {
  if (chatFileSink) return chatFileSink.take(file, known) !== false;
  if (!queueIfUnmounted) return false;
  pendingChatFiles.push({ file, known });
  return true;
}
/** Whether a composer is mounted and on screen: not on a page behind
 *  another one, nor under a panel that covers the conversation. A file
 *  dropped on the window attaches straight to it (windowFileDrop.ts). */
export function chatComposerOnScreen(): boolean {
  return !!chatFileSink?.onScreen();
}
export function registerChatImageSink(sink: (file: File, known?: boolean) => boolean | void, onScreen: () => boolean = () => true): () => void {
  const entry: ChatFileSink = { take: (file, known) => sink(file, known), onScreen };
  chatFileSink = entry;
  pendingChatFiles.splice(0).forEach(p => { sink(p.file, p.known); });
  return () => { if (chatFileSink === entry) chatFileSink = null; };
}
let chatAttachmentSequence = 0;
export function nextChatAttachmentId(): number { return ++chatAttachmentSequence; }

/** The turn the composer sent (or adopted from the run queue after a reload)
 *  until it ends (Paperclip PR 7b, D3). `chat_send` only enqueues, so a turn
 *  can wait behind a workflow; while it has a `queueId` and has not started,
 *  its bubble shows a Queued badge with Edit and Discard. `chatBusy` stays
 *  true the whole time, so no second send or conversation switch can start.
 *  In memory only: attachments hold data URLs. */
export interface ChatQueuedTurn {
  clientTurnId: string;
  /** Null until `SendAccepted` resolves. */
  queueId: number | null;
  sessionKey: string | null;
  text: string;
  /** The images sent with the turn, restored with its text. */
  attachments: ChatAttachment[];
  /** The cwd the turn was sent from, named if it is recovered at claim. */
  requestedCwd: string | null;
  waitingFor: string | null;
  /** The conversation whose turn holds this build worktree, when
   *  `waitingFor` names it: the badge's Open it (P1-46). */
  waitingSession?: string | null;
  queuePosition: number | null;
  /** The row's permission hold note, when the permission mode holds it: the
   *  composer reads paused, as the dock does (2026-11-17). */
  heldReason?: string | null;
  started: boolean;
  /** Set while an Edit or Discard is in flight. */
  intent: QueuedTurnIntent;
  /** Adopted from `list_run_queue` rather than sent from this window. */
  adopted: boolean;
  /** Its `CHAT_CONTEXT_RECOVERED` error already reset the conversation. */
  recovered: boolean;
  /** Paperclip PR 13: the question card this turn answers (its text is the
   *  card's formatted answers). A turn that ends before it starts puts the
   *  card back, or the answers into the composer, instead of just restoring
   *  the text. */
  interactionId?: string | null;
  /** The mode it runs in, from its `runStarted` (2026-11-04). */
  permissionMode?: string | null;
}
export const [chatQueuedTurn, setChatQueuedTurn] = createSignal<ChatQueuedTurn | null>(null);
/** A queued turn's bubble shows the Queued badge only while the turn truly
 *  can't start: the queue has it, the pump hasn't started it, and the
 *  backend names a real wait (a chat-count limit, memory, another run's
 *  folder). A turn nothing holds back is preparing, not queued
 *  (`turnStanding`, 2026-11-16). */
export function isVisiblyQueued(q: ChatQueuedTurn | null): q is ChatQueuedTurn {
  return !!q && turnStanding(q) === 'queued';
}
/** The turn is accepted, not started and nothing truly holds it: the chat
 *  shows its working indicator as "Preparing". */
export function isPreparingTurn(q: ChatQueuedTurn | null): q is ChatQueuedTurn {
  return !!q && turnStanding(q) === 'preparing';
}
export function turnIsQueuedTurn(t: ChatTurn, q: ChatQueuedTurn | null): boolean {
  if (!q || t.role !== 'user') return false;
  return (!!t.clientTurnId && t.clientTurnId === q.clientTurnId) || (q.queueId != null && t.queueId === q.queueId);
}
const initAgent: ChatAgent = (() => {
  const v = lsGet('chatAgent');
  return (KNOWN_AGENTS as string[]).includes(v ?? '') ? (v as ChatAgent) : 'claude';
})();
export const [chatAgent, setChatAgent] = createSignal<ChatAgent>(initAgent);
// ─────────────────────────────────────────────────────────────
// Agent actions — surfaces (the Vault note header, etc.) compose a prompt and
// hand it to the chat composer. `equals: false` so the same action twice
// re-fires. ChatBody listens and prefills its input (the user reviews + hits
// Enter — we deliberately don't auto-send, so a misfire never spends tokens or
// trips a permission prompt). The caller brings a composer on screen first
// (the Preview pane's Ask the agent goes to the open conversation's page).
// ─────────────────────────────────────────────────────────────
export const [chatPrefillRequest, setChatPrefillRequest] = createSignal<string | null>(null, { equals: false });
export function askAgentInChat(prompt: string): void {
  if (!prompt.trim()) return;
  setChatPrefillRequest(prompt);
  requestChatFocus();
}

// Compatibility alias for existing action call sites.
export const askClaudeInChat = askAgentInChat;

// Model is now a free string (each agent has its own model namespace).
const initModel: string = lsGet('chatModel') ?? AGENT_DEFAULT_MODEL[initAgent];
export const [chatModel, setChatModel] = createSignal<string>(initModel);

// Never assume an agent is installed before discovery completes.
export const [agentsAvailable, setAgentsAvailable] = createSignal<ChatAgent[]>([]);
export async function loadAgentsAvailable() { await refreshAgentSettings(); }

const initPermMode: ChatPermissionMode = (() => {
  const v = lsGet('chatPermissionMode');
  // Default to `acceptEdits`: edits land without prompting, but Bash and
  // any unrecognized tool still ask. `bypassPermissions` is opt-in via the
  // chat header dropdown when the user explicitly wants the danger mode.
  return (KNOWN_PERM_MODES as string[]).includes(v ?? '') ? (v as ChatPermissionMode) : 'acceptEdits';
})();
preserveCurrentModeOnUpgrade(chatAgentSessionKey(), lsGet('chatPermissionMode'));
export const [chatPermissionMode, setChatPermissionModeSignal] = createSignal<ChatPermissionMode>(
  chatAgentSessionKey() ? conversationPermission(chatAgentSessionKey()!, initPermMode).mode : initPermMode);
/** Apply a mode without recording an explicit choice; all restoration paths
 * keep the startup copy paired with the visible conversation. */
export function setChatPermissionMode(mode: ChatPermissionMode) {
  setChatPermissionModeSignal(mode);
  lsSet('chatPermissionMode', mode);
}

/// P0-19: Settings owns the default, the chat owns its own mode. Since
/// 2026-11-12 each agent has its own mode for new chats (Settings, Chat,
/// keys `newClaudePermissionMode` and the like, `agentPermModes.ts`
/// `newChatModeFor`): the agent's own saved mode, else Auto for Claude and
/// Codex and Accept edits for Grok and Antigravity. The single
/// `newChatPermissionMode` from before is no longer read (owner decision,
/// 2026-11-12): it was seeded from the last chat's mode, never chosen.
/// Changing any of them never touches the chat on screen.
function readAgentNewModes(): Partial<Record<ChatAgent, ChatPermissionMode>> {
  const own: Partial<Record<ChatAgent, ChatPermissionMode>> = {};
  for (const a of KNOWN_AGENTS) {
    const v = lsGet(newChatModeKey(a));
    if ((KNOWN_PERM_MODES as string[]).includes(v ?? '')) own[a] = v as ChatPermissionMode;
  }
  return own;
}
const [agentNewModes, setAgentNewModes] = createSignal(readAgentNewModes());
/** The mode a new chat with `agent` starts in (never one it can't run). */
export function newChatModeFor(agent: ChatAgent): ChatPermissionMode {
  const caps = agentDescriptors().find(a => a.id === agent)?.capabilities;
  return newChatModeForAgent(agent, agentNewModes()[agent], caps?.unavailableModes);
}
/** Where `agent`'s mode for new chats comes from: its own saved one, or the
 *  agent's default. */
export function newChatModeSource(agent: ChatAgent): 'own' | 'default' {
  return agentNewModes()[agent] ? 'own' : 'default';
}
/** The mode for new chats with this chat's agent, as saved (before any
 *  fallback for a mode the agent can't run). */
export function newChatPermissionMode(): ChatPermissionMode {
  return newChatModeForAgent(chatAgent(), agentNewModes()[chatAgent()]);
}
/** Save `agent`'s own mode for new chats (Settings, Chat). */
export function setNewAgentPermissionModePersistent(agent: ChatAgent, m: ChatPermissionMode) {
  setAgentNewModes(prev => ({ ...prev, [agent]: m }));
  lsSet(newChatModeKey(agent), m);
  reportPhoneNewChatMode();
}
// A chat a phone starts begins in the mode for new chats with the default
// agent: the phone chat API keeps the last one the window said
// (`phone_access_new_chat_mode`).
function reportPhoneNewChatMode() {
  // With the agent it is for, so a phone chat on another agent never does
  // more than this mode does there (Q-R3, `agent::inherited_mode`).
  const agent = preferredAgent();
  reportToPhone(() => api.phoneAccessNewChatMode(newChatModeFor(agent), agent));
}

/** A brand-new conversation (New chat, `/new`, a workspace's new
 *  conversation) starts in the mode for new chats with its agent. Only on an
 *  empty chat: an opened one keeps the mode on screen, since its native
 *  session may carry the mode it began with. A mode this chat's agent can't
 *  run (no Auto on Antigravity) starts it in the nearest narrower one. */
export function applyNewChatPermissionMode() {
  if (chatBusy() || chatSessionId() || chatAgentSessionKey() || chatTurns.length > 0) return;
  const m = newChatModeFor(chatAgent());
  setChatPermissionMode(m);
  lsSet('chatPermissionMode', m);
}

/** A person switching this chat's agent (the model menu, `/agent`, Retry
 *  with another agent): the agent changes, and the chat's mode moves to its
 *  nearest equivalent there (Accept edits on Claude is Ask for approval on
 *  Codex; Auto on Antigravity, which can't run it, is Accept edits). Returns
 *  the one line that says so, also flashed, or '' when nothing changed name. */
export function switchChatAgent(a: ChatAgent): string {
  if (chatBusy() || chatSessionLoading()) return '';
  const from = chatAgent();
  const was = chatPermissionMode();
  setChatAgentPersistent(a);
  if (chatAgent() !== a || from === a) return '';
  const caps = agentDescriptors().find(d => d.id === a)?.capabilities;
  const { mode, note } = agentSwitchMode(from, a, was, caps?.unavailableModes);
  if (mode !== was) setChatPermissionModePersistent(mode);
  // The new agent may lack the level this chat shows (#1321): clear it, through
  // the one setter, so the cache and Rust stay in step.
  const effort = chatEffort();
  if (effort && caps?.effortLevels?.length && !caps.effortLevels.includes(effort)) setChatEffortPersistent(null);
  if (note) flashFooter(note, 6000);
  return note;
}

/** Why this chat's mode can't change right now, or ''. */
export function permissionChangeBlocked(): string {
  if (chatBusy()) return 'Stop or finish the running turn first';
  if (chatSessionLoading()) return 'Wait for the conversation to finish opening';
  return '';
}

/** Whether switching this chat to `m` starts a fresh native session. A
 *  session is bound to its mode's class (`modeClass`, the backend's HB01
 *  rule): Plan and the acting modes, or Grok's sandbox profiles, never
 *  resume each other's session, so a safer mode can never resume broader
 *  permissions. Across those it restarts with retained Hollow context;
 *  within one class (Edits only and Bypass, say) every process takes the
 *  new mode itself and the session goes on (2026-11-04). */
export function permissionChangeStartsFresh(m: ChatPermissionMode): boolean {
  return modeClass(chatAgent(), m) !== modeClass(chatAgent(), chatPermissionMode());
}

/** Change this chat's mode. Returns why it didn't, or '' once it did (or it
 *  already was `m`). */
export function setChatPermissionModePersistent(m: ChatPermissionMode): string {
  const blocked = permissionChangeBlocked();
  if (blocked) return blocked;
  if (!rememberConversationMode(chatAgentSessionKey(), m)) flashFooter('Permissions changed for this chat, but could not be fully saved. Reopened chats may use Plan. Check the mode after reopening Hollow.', 8000);
  reportConversationMode(chatAgentSessionKey(), m);
  if (m === chatPermissionMode()) return '';
  if (permissionChangeStartsFresh(m)) {
    setChatConversationVersion(version => version + 1);
    setChatSessionIdPersistent(null);
    setChatTurnUsage(null);
  }
  setChatPermissionMode(m);
  lsSet('chatPermissionMode', m);
  return '';
}


export function setChatSessionIdPersistent(id: string | null) {
  setChatSessionId(id);
  lsSet('chatSessionId', id);
}
export function setChatCwdPersistent(cwd: string | null) {
  setChatCwd(cwd);
  lsSet('chatCwd', cwd);
}
export function setChatModelPersistent(m: string) {
  setChatModel(m);
  lsSet('chatModel', m);
}
/// Switch the active agent and reset the model to that agent's default — a
/// model name is meaningless across agents (claude's `sonnet` ≠ codex's
/// `gpt-5-codex`). Persisted.
export function setChatAgentPersistent(a: ChatAgent) {
  if (chatBusy() || chatSessionLoading()) return;
  chatAgentOverridden = true;
  if (a === chatAgent()) return;
  setChatConversationVersion(version => version + 1);
  setChatAgent(a);
  lsSet('chatAgent', a);
  setChatModelPersistent(agentSettings().models[a] ?? '');
  // Sessions aren't portable across agents — you can't resume a Claude
  // session in Codex (different id namespace + CLI). Reset so the next turn
  // starts the new agent fresh rather than trying to --resume a foreign id.
  setChatSessionIdPersistent(null);
  setChatTurnUsage(null);
}

// When true, the next assistant text/tool starts a FRESH turn rather than
// appending to the current one. Set by the `turnBoundary` event (one per
// assistant message) so a multi-step turn renders as the real sequence —
// text, then its tools, then the next message's text, etc.
let activeChatEvidence: (EvidenceRunContextWire & { turnStartIndex: number }) | null = null;
/** The on-screen conversation's transcript reducer (chatTranscript.ts). */
const transcript = createTranscript(setChatTurns, () => activeChatEvidence?.runId, () => chatTurns);
export const [activeChatRecall, setActiveChatRecall] = createSignal<EvidenceRunContextWire['recall']>([]);
/** One candidate in a turn's recall trace (RC1, `recall_trace.rs`): kept in the
 *  prompt, or cut with the reason (`silence: a weak cue`). */
export interface ChatRecallTraceRow { path: string; subject: string; score: number; kept: boolean; why: string }
/** Why a turn's recall was what it was: the request it read, the silence rule
 *  that cut it (if one did) and every candidate with its reason. */
export interface ChatRecallTrace { sessionKey: string; query: string; silence?: string; alreadyGiven?: boolean; candidates: ChatRecallTraceRow[] }
/** The newest turn's trace, whichever chat ran it; the card shows it only in
 *  the chat it names (`sessionKey`), so a switch needs no save or restore. */
const [chatRecallTrace, setTrace] = createSignal<ChatRecallTrace>();
export { chatRecallTrace };
/** The run id of the chat turn in flight (set at `runStarted`, cleared at its
 *  `done`). Feedback buttons stay hidden on that turn's bubbles (PR 17). */
export const [chatLiveRunId, setChatLiveRunId] = createSignal<string | null>(null);
/** The chat turn in flight's activity (Paperclip PR 15b): when its agent last
 *  printed anything and its progress phase, from the backend's `activity`
 *  ticks. Reset at `runStarted` and `done`; the composer shows "quiet Nm". */
export const [chatActivity, setChatActivity] = createSignal<ChatActivity | null>(null);
/** AG6: the latest `panel` snapshot per panel name for the conversation on
 *  screen (a replace, never a merge). Panels read it with `chatPanel(name)`;
 *  it empties with the chat state. */
const [chatPanels, setChatPanels] = createSignal<Readonly<Record<string, unknown>>>({});
export function chatPanel(name: string): unknown {
  return chatPanels()[name];
}
/** When this window saw the chat turn in flight start (its `runStarted`), in
 *  epoch ms, for the working row's elapsed clock. Null otherwise, including a
 *  turn adopted after a reload (its real start is unknown). */
export const [chatRunStartedAt, setChatRunStartedAt] = createSignal<number | null>(null);

export function resetChatState() {
  if (chatBusy()) return;
  rememberBeforeDraftReset();
  chatAgentOverridden = false;
  setChatConversationVersion(version => version + 1);
  setChatSessionLoading(false);
  setChatInput('');
  setChatAttachments([]);
  setChatPrefillRequest(null);
  setChatHistoryCursor(null);
  setChatTurns([]);
  setChatSessionId(null);
  setChatTurnUsage(null);
  setChatAgent(preferredAgent());
  lsSet('chatAgent', chatAgent());
  setChatModelPersistent(agentSettings().models[chatAgent()] ?? '');
  setChatAgentSessionKeyPersistent(null);
  // An explicit reset or New chat starts at the app default, even when the
  // key was already null (a level picked in an unsent chat).
  setChatEffort(null);
  setChatContinuedFrom([]);
  setChatQueuedTurn(null);
  setChatInteractions([]);
  transcript.setPendingNewTurn(false);
  activeChatEvidence = null;
  setActiveChatRecall([]);
  setChatLiveRunId(null);
  setChatTurnSteerSupport(null);
  setChatActivity(null);
  setChatPanels({});
  setChatRunStartedAt(null);
  lsSet('chatSessionId', null);
  // A fresh chat ALWAYS starts in the vault. A `/cwd` override (or the cwd
  // restored by opening a non-vault session) is scoped to that conversation
  // — without this reset it leaked into every later chat via localStorage.
  setChatCwdPersistent(null);
  // Artifacts belong to the conversation — a fresh chat starts clean.
  // (Hoisted function declaration from the chat-preview section below.)
  clearChatArtifacts();
  flushParkedHeld();
}

/** Each stored read of a conversation's transcript takes the next number
 *  for its key. A read applies only when it is newer than the last one that
 *  wrote the screen (the open's first read can land after the re-read at
 *  its turn's end); a newer read that failed or gave up never blocks an
 *  older one (Codex re-check). */
const storedTranscriptReads = new Map<string, number>();
const storedTranscriptApplied = new Map<string, number>();
export function beginStoredTranscriptRead(key: string): number {
  const generation = (storedTranscriptReads.get(key) ?? 0) + 1;
  storedTranscriptReads.set(key, generation);
  return generation;
}
export const storedTranscriptReadApplies = (key: string, generation: number) => generation > (storedTranscriptApplied.get(key) ?? 0);
/** The read numbered `generation` wrote the screen. */
export function markStoredTranscriptApplied(key: string, generation: number) {
  if (storedTranscriptReadApplies(key, generation)) storedTranscriptApplied.set(key, generation);
}

/** Show `key`'s stored transcript under what already arrived live: the
 *  stored turns first, then the live ones (an adopted turn's bubble, its
 *  streamed reply). A live question the store already has is not shown
 *  twice, and a tool both hold (same run, same id) stays in its stored
 *  place with its newer live state. Nothing else about the running turn
 *  changes. */
export function showStoredTranscript(key: string, stored: ChatTurn[]) {
  if (chatAgentSessionKey() !== key) return;
  const live = chatStoredTranscriptPending() === key ? clone(unwrap(chatTurns)) : [];
  const q = chatQueuedTurn();
  const turns = withLiveTurns(clone(stored), live, chatLiveRunId(), t => !!q && !q.started && turnIsQueuedTurn(t, q));
  batch(() => {
    setChatTurns(turns);
    rebuildChatArtifacts(turns);
    setChatStoredTranscriptPending(null);
  });
}

const FINISHED_TURN = new Set(['success', 'error', 'cancelled']);

/** `notStarted`: the queued turn's bubble before its run began. The store
 *  has no row for it, so it is never matched, even to the same words. */
function withLiveTurns(stored: ChatTurn[], live: ChatTurn[], liveRunId: string | null, notStarted: (t: ChatTurn) => boolean): ChatTurn[] {
  // Tool ids may repeat across runs (chatTranscript.ts), so a tool is the
  // same one only within the same run.
  const toolKey = (runId: string | undefined, id: string) => `${runId ?? ''} ${id}`;
  const storedTools = new Map<string, { tools: ChatToolUse[]; index: number }>();
  for (const turn of stored) for (const [i, tool] of turn.tools.entries()) {
    if (tool.id) storedTools.set(toolKey(turn.runId, tool.id), { tools: turn.tools, index: i });
  }
  // The started run's question the store already holds: by its run id, or,
  // with no run id on either side, the last stored question when the
  // stored transcript ends in that unfinished run (Codex re-check).
  let asked: ChatTurn | undefined;
  if (liveRunId) {
    asked = stored.find(t => t.role === 'user' && t.runId === liveRunId);
    const lastUser = stored.map(t => t.role).lastIndexOf('user');
    const tail = lastUser >= 0 ? stored.slice(lastUser) : [];
    if (!asked && tail.length > 0 && !tail[0].runId
      && tail.every(t => (!t.runId || t.runId === liveRunId) && !FINISHED_TURN.has(t.status ?? ''))) asked = tail[0];
  }
  let questionMatched = false;
  const fresh: ChatTurn[] = [];
  for (const turn of live) {
    if (turn.role === 'user' && !questionMatched && asked && !notStarted(turn) && !turn.runId
      && asked.text.trim() === turn.text.trim()) { questionMatched = true; continue; }
    const tools = turn.tools.filter(tool => {
      const at = tool.id ? storedTools.get(toolKey(turn.runId, tool.id)) : undefined;
      if (!at) return true;
      at.tools[at.index] = tool;
      return false;
    });
    if (turn.role === 'assistant' && !turn.text && tools.length === 0) continue;
    fresh.push({ ...turn, tools });
  }
  return [...stored, ...fresh];
}

export function pushChatTurn(t: ChatTurn) {
  setChatTurns(produce(arr => { arr.push(t); }));
}

// The runner's lines (`remoteRunNotice.ts`, about 7 KB of text) load on first use, outside
// the shell chunk, whose 340 KB budget was full (overnight 11-07, lane D3). A `remoteRun` line
// is pushed at once, in stream order; the session's first waits (`remoteRunPending`) and gets
// its text a moment later, on screen or in a chat sent to the background meanwhile (or goes,
// if the module never lands; the next line tries again).
type RemoteRunText = (e: RemoteRunEvent) => string;
let remoteRunText: RemoteRunText | null = null;
let remoteRunLoad: Promise<RemoteRunText | null> | null = null;
function loadRemoteRunText() {
  return (remoteRunLoad ??= import('../remoteRunNotice').then(
    m => (remoteRunText = m.remoteRunNotice),
    () => { remoteRunLoad = null; return null; },
  ));
}

function pushRemoteRunTurn(p: RemoteRunEvent) {
  // `back` is about the turn that just ended, so Retry still sees it.
  const turn: ChatTurn = { role: 'system', text: remoteRunText?.(p) ?? '', tools: [], notice: 'remoteRun', aboutEarlierTurn: p.phase === 'back' };
  if (!remoteRunText) {
    turn.remoteRunPending = p;
    void loadRemoteRunText().then(fillRemoteRunTurns);
  }
  pushChatTurn(turn);
}

function fillRemoteRunTurns(text: RemoteRunText | null) {
  const fill = (turns: ChatTurn[]) => {
    for (let i = turns.length - 1; i >= 0; i--) {
      const waiting = turns[i].remoteRunPending;
      if (!waiting) continue;
      if (!text) { turns.splice(i, 1); continue; }
      turns[i].text = text(waiting);
      turns[i].remoteRunPending = undefined;
    }
  };
  setChatTurns(produce(fill));
  for (const chat of backgroundChats.values()) fill(chat.snapshot.turns);
}

/** Reset a conversation whose persisted working directory disappeared.
 *  The composer calls this before consuming the pending message; the
 *  send-side race guard can call it after retracting the unsent user row. */
export function recoverChatContext(previousCwd: string) {
  // The window keeps the turns it shows; the next send names the left
  // conversation so the backend bundles them (agent effectiveness F4).
  setChatContinuedFrom(keys => continuedFromAfter(keys, chatAgentSessionKey()));
  setChatAgentSessionKeyPersistent(null);
  setChatCwdPersistent(null);
  setChatSessionIdPersistent(null);
  setChatTurnUsage(null);
  setChatContextRaw(null);
  pushChatTurn({
    role: 'system',
    text: `The previous working folder no longer exists (${previousCwd}). Switched to a fresh chat in the vault; the prior agent context cannot be resumed there. Make the message in the composer self-contained, then send when ready.`,
    tools: [],
  });
}

/** Remove the user row of one send. A turn that never started was never
 *  sent, so it must not stay in the transcript. */
export function removeChatUserTurn(q: Pick<ChatQueuedTurn, 'clientTurnId' | 'queueId'>) {
  setChatTurns(produce(arr => {
    for (let i = arr.length - 1; i >= 0; i -= 1) {
      const t = arr[i];
      if (t.role !== 'user') continue;
      if ((t.clientTurnId && t.clientTurnId === q.clientTurnId) || (q.queueId != null && t.queueId === q.queueId)) {
        arr.splice(i, 1);
        break;
      }
    }
  }));
}

// ── Queued chat turns (Paperclip PR 7b, D3) ──────────────────────────────────
// The decisions live in `chatQueue.ts` (pure, unit-tested); these apply them.

/** Record a send before `chat_send` is invoked, so a `runStarted` that beats
 *  the `SendAccepted` reply still finds its turn. */
export function beginQueuedChatTurn(turn: Pick<ChatQueuedTurn, 'clientTurnId' | 'text' | 'attachments' | 'requestedCwd' | 'sessionKey' | 'interactionId'>) {
  setChatQueuedTurn({
    ...turn,
    queueId: null,
    waitingFor: null,
    queuePosition: null,
    started: false,
    intent: null,
    adopted: false,
    recovered: false,
  });
}

/** Apply `chat_send`'s reply. Returns false when the turn already ended
 *  (its events outran the reply), in which case nothing is adopted: a
 *  recovered turn must not bring its dead session key back. */
export function acceptQueuedChatTurn(clientTurnId: string, accepted: SendAccepted): boolean {
  const q = chatQueuedTurn();
  if (!q || q.clientTurnId !== clientTurnId) return false;
  // Recovered at claim before this reply landed: recoverChatContext already
  // cleared the key, and the dead conversation's key must stay cleared.
  if (!q.recovered) {
    acquireFirstDraft(accepted.sessionKey, q);
    setChatAgentSessionKeyPersistent(accepted.sessionKey, { ownTurn: true });
  }
  setChatTurns(produce(arr => {
    const t = arr.find(turn => turn.role === 'user' && turn.clientTurnId === clientTurnId);
    if (t) t.queueId = accepted.queueId;
  }));
  // After runStarted the queue fields are history; keep them for matching
  // only. acceptedShowsBadge is the same rule the badge renders by.
  setChatQueuedTurn({
    ...q,
    queueId: accepted.queueId,
    sessionKey: accepted.sessionKey,
    waitingFor: acceptedShowsBadge(q, clientTurnId) ? accepted.waitingFor : null,
    queuePosition: accepted.queuePosition,
  });
  return true;
}

/** `chat_send` itself failed: nothing was enqueued. */
export function abandonQueuedChatTurn(clientTurnId: string) {
  if (chatQueuedTurn()?.clientTurnId === clientTurnId) setChatQueuedTurn(null);
}

function restoreQueuedTurnToComposer(q: ChatQueuedTurn) {
  removeChatUserTurn(q);
  // An ask-canvas's answers go back to their canvas, never the box (P1-33).
  if (answersBack(q.sessionKey, q.text)) return;
  setChatInput(restoredDraft(q.text, chatInput()));
  if (q.attachments.length > 0) {
    // A recovered turn's working folder is gone, and its saved images with
    // it: offer them as failed so they are removed, never re-sent as paths.
    // A shared page holds its text, not a file, so it comes back as it was.
    const back = q.attachments.map(a => q.recovered && !a.page ? { ...a, path: undefined, saving: false, error: true } : a);
    setChatAttachments(current => [...back.filter(a => !current.some(c => c.id === a.id)), ...current]);
  }
  requestChatFocus();
}

/** Apply a decision from `queuedTurnEnd`. Terminal decisions clear the
 *  queued turn BEFORE removing its bubble, in one batch, so ConversationHistory.tsx's
 *  ensureQueuedTurnVisible effect never sees the turn without its bubble and
 *  puts it back. `done` itself ends busy. */
function applyQueuedTurnDecision(decision: QueuedTurnDecision, q: ChatQueuedTurn) {
  batch(() => {
    switch (decision) {
      case 'started':
        setChatQueuedTurn({ ...q, started: true, waitingFor: null, waitingSession: null });
        break;
      case 'restore':
      case 'drop':
        setChatQueuedTurn(null);
        if (q.interactionId) {
          // An answer turn that never started: its card comes back, or its
          // answers go to the composer. Never silently dropped.
          removeChatUserTurn(q);
          void returnUnsentAnswers(q.interactionId, q.text);
        } else if (decision === 'restore') {
          restoreQueuedTurnToComposer(q);
        } else {
          removeChatUserTurn(q);
        }
        break;
      case 'none':
        break;
    }
  });
}

/** Turns ended locally (Edit or Discard answered) before their own
 *  `cancelled` + `done` arrived. Those late events must not end a newer send;
 *  the `done` handler drops them and forgets the entry. */
const endedQueuedTurns: Array<Pick<ChatQueuedTurn, 'clientTurnId' | 'queueId'>> = [];
const ENDED_QUEUED_TURNS_CAP = 8;

/** What a withdrawn queued turn leaves in the transcript: an ask-canvas's
 *  answers go back to their canvas (P1-33), so it says where. */
function withdrawnNotice(intent: QueuedTurnIntent, text: string, answers = false): string | null {
  // A question card's answers go back to their card (or, failing that, the
  // composer, with its own notice): not "your message is in the composer".
  if (answers && intent !== 'discard') return intent === 'edit' ? null : 'Stopped before it started.';
  if (intent === 'discard' || !answeredRevision(text)) return queuedCancelNotice(intent);
  return intent === 'edit' ? 'Your answers are back in the canvas.' : 'Stopped before it started. Your answers are still in the canvas.';
}

/** A withdrawn queued turn is over: its row is terminal and nothing ran. */
function endWithdrawnTurn(q: ChatQueuedTurn, intent: QueuedTurnIntent) {
  endedQueuedTurns.unshift({ clientTurnId: q.clientTurnId, queueId: q.queueId });
  endedQueuedTurns.length = Math.min(endedQueuedTurns.length, ENDED_QUEUED_TURNS_CAP);
  const notice = withdrawnNotice(intent, q.text, !!q.interactionId);
  applyQueuedTurnDecision(intent === 'discard' ? 'drop' : 'restore', q);
  if (notice) pushChatTurn({ role: 'system', text: notice, tools: [] });
  setChatBusy(false);
}

/** The composer's Edit and Discard on a queued turn. Both take the row out
 *  of the queue with `cancel_queued_run`; Edit puts the text back in the
 *  composer, Discard drops it. A turn the pump already started can only be
 *  stopped. */
export async function withdrawQueuedChatTurn(intent: 'edit' | 'discard'): Promise<void> {
  const q = chatQueuedTurn();
  if (!q || q.queueId == null || q.intent) return;
  if (q.started) { flashFooter('It already started — use Stop', 4000); return; }
  setChatQueuedTurn({ ...q, intent });
  const reset = () => setChatQueuedTurn(cur => cur && cur.clientTurnId === q.clientTurnId ? { ...cur, intent: null } : cur);
  try {
    const item = await api.cancelQueuedRun(q.queueId);
    const cur = chatQueuedTurn();
    if (!cur || cur.clientTurnId !== q.clientTurnId) return; // its events already ended it
    if (item.status === 'cancelled') {
      // Terminal and never started. The backend's cancelled + done events
      // follow; ending here keeps the composer right if they are missed.
      endWithdrawnTurn(cur, intent);
    } else if (item.status === 'running') {
      reset();
      flashFooter('It already started — use Stop', 4000);
    }
    // 'claimed': the pump saw the cancel request and ends it before it starts;
    // its cancelled{queued} + done events carry the intent set above.
  } catch (err) {
    reset();
    flashFooter(`× Couldn’t ${intent} the queued message: ${errMsg(err)}`, 4000);
  }
}

/** The session key Stop should name: the queued turn's own, which exists
 *  before `SendAccepted` has been adopted into the conversation. */
export function chatStopKey(): string | null {
  return chatQueuedTurn()?.sessionKey ?? chatAgentSessionKey();
}

/** Stop the turn on screen: a queued one comes out of the queue (its text
 *  comes back), a running one is stopped. Keyed, so a stale request can
 *  never stop another session. The composer's Stop button, `/stop`, Esc
 *  (frontier F6), the palette and the native menu all call this. */
export function stopChatRun() {
  if (!chatBusy()) return;
  // A send still getting ready has nothing for chat_cancel to find yet: it
  // takes the stop itself, before or once it queues (C10, chatQueue.ts).
  const q = chatQueuedTurn();
  if (stopPreparingSend(chatAgentSessionKey(), q && !q.started ? q.clientTurnId : null)) return;
  api.chatCancel(chatStopKey()).catch(err => flashFooter(`× Couldn’t stop: ${String(err)}`, 4000));
}

/** The conversation on screen is paused (P1a PR 2): its note, else null. */
export const chatPausedHere = () => chatPausedFor(chatAgentSessionKey());
/** The on-screen chat's Resume is in flight (its buttons are disabled). */
export const chatResumingHere = () => chatResumingFor(chatAgentSessionKey());
/** A pause of the turn on screen is asked and not reached yet. */
export const chatPausingHere = () => chatPausingFor(chatStopKey());

/** Whether the composer offers Pause (P1a PR 2): a turn of this conversation
 *  has started (a queued one has nothing to pause yet; Stop takes it back). */
export function canPauseChatRun(): boolean {
  const q = chatQueuedTurn();
  // A turn adopted after a reload is `started` before any `runStarted` here.
  return chatBusy() && !!chatStopKey() && !q?.intent && (!!chatLiveRunId() || !!q?.started);
}

/** Pause the running turn on screen at its next safe point (P1a): the
 *  composer's Pause, `/pause`. Keyed like Stop. While the pause is asked the
 *  button reads "Pausing…" and Stop still works and wins; the `paused`
 *  event ends it with a notice and Resume. */
export async function pauseChatRun(): Promise<void> {
  const key = chatStopKey();
  if (!key || !canPauseChatRun() || chatPausingFor(key)) return;
  setChatPausing(key, true);
  try {
    const reached = await api.chatPause(key);
    if (!reached) {
      setChatPausing(key, false);
      flashFooter('Nothing to pause: the turn has ended or is stopping.', 4000);
    }
  } catch (err) {
    setChatPausing(key, false);
    flashFooter(`× Couldn’t pause: ${errMsg(err)}`, 4000);
  }
}

/** Resume paused conversation `key` (default: the one on screen): the
 *  composer's Resume, Mission Control's, `/resume`. The backend queues an
 *  ordinary turn on the stored native session; on screen it is adopted like
 *  any send made while busy (`adoptBusySend`), so the composer, Stop and the
 *  queue badge follow it. True when it was queued. */
export async function resumePausedChat(key: string | null = chatAgentSessionKey()): Promise<boolean> {
  if (!key) return false;
  const onScreen = () => chatAgentSessionKey() === key;
  if (onScreen() && chatBusy()) {
    // Mission Control's Resume on the chat on screen, already running
    // again: say so rather than nothing (review fix 3).
    flashFooter('This chat is already running.', 4000);
    return false;
  }
  if (!beginResume(key)) return false;
  try {
    const accepted = await api.chatResumePaused(key);
    clearChatPaused(key, true);
    // Left meanwhile, or its `runStarted` already drew the turn (and set
    // busy): its own events carry it from here.
    if (!onScreen() || chatBusy() || chatTurns.some(t => t.role === 'user' && t.queueId === accepted.queueId)) return true;
    adoptBusySend(newClientTurnId(), RESUME_PROMPT, [], accepted);
    return true;
  } catch (err) {
    flashFooter(`× Couldn’t resume: ${errMsg(err)}`, 5000);
    // The backend refused (not paused any more, or a turn of it is live):
    // read its record again so the button follows.
    void refreshChatPaused(key);
    return false;
  } finally {
    endResume(key);
  }
}

/** What Stop did to a send it reached while `chat_send` was in flight:
 *  `withdrawn` before it ran; `stopped` after it started (its row's own
 *  cancel); `failed` its held send failed (the outcome stays parked for the
 *  reply's own handling); `sent` too late to stop, or not found. */
export type StoppedSendOutcome = 'withdrawn' | 'stopped' | 'failed' | 'sent';

/** How long a held send that `drain_held` took before Stop could cancel it
 *  is followed to the row it became (its `heldSent`). */
export const HELD_OUTCOME_WAIT_MS = 10_000;
/** Waiters for a held send's outcome, by held id (`heldOutcome`). */
const heldWaiters = new Map<string, (outcome: HeldOutcome) => void>();

/** Every held outcome seen, by held id, whatever its route did with it (a
 *  backgrounded chat buffers its events, never parking them): `heldOutcome`
 *  reads it so a `heldSent` that came before Stop asked is still found.
 *  Bounded, oldest dropped. Read only by `heldOutcome`. */
const heldSeen = new Map<string, HeldOutcome>();
function noteHeldSeen(p: HeldOutcome) {
  heldSeen.delete(p.heldId);
  heldSeen.set(p.heldId, p);
  if (heldSeen.size > 100) heldSeen.delete(heldSeen.keys().next().value!);
}

/** The outcome of held send `heldId`: one already parked or seen, else the
 *  next `heldSent` / `heldFailed` for it, on any route; null after `ms`. */
function heldOutcome(heldId: string, ms = HELD_OUTCOME_WAIT_MS): Promise<HeldOutcome | null> {
  const parked = earlyHeld.get(heldId) ?? heldSeen.get(heldId);
  if (parked) return Promise.resolve(parked);
  return new Promise(resolve => {
    const timer = setTimeout(() => { heldWaiters.delete(heldId); resolve(null); }, ms);
    heldWaiters.set(heldId, outcome => { clearTimeout(timer); heldWaiters.delete(heldId); resolve(outcome); });
  });
}

/** A send Stop reached while `chat_send` was in flight (C10): take its
 *  accepted row back with the row-checked cancel only (`cancel_queued_run`,
 *  which also stops a row already running), never a key-wide stop that
 *  could reach a later turn. A held send (after a forward) is cancelled by
 *  its id; when `drain_held` took it first, it is followed to the row its
 *  `heldSent` names and exactly that row is cancelled. Withdrawn and still
 *  the queued turn on screen, it ends as a withdrawn turn does (its text
 *  back in the box, its late events dropped). */
export async function withdrawStoppedSend(clientTurnId: string, accepted: SendAccepted): Promise<StoppedSendOutcome> {
  let outcome: StoppedSendOutcome;
  try {
    let row = accepted.queueId;
    if (accepted.heldId && await api.chatCancelHeld(accepted.sessionKey, accepted.heldId)) outcome = 'withdrawn';
    else {
      if (accepted.heldId) {
        const held = await heldOutcome(accepted.heldId);
        if (!held) return 'sent';
        if (held.type === 'heldFailed') return 'failed';
        takeEarlyHeld(accepted.heldId);
        row = held.queueId;
      }
      const item = await api.cancelQueuedRun(row);
      // 'claimed': the pump saw the cancel and ends it before it starts.
      outcome = item.status === 'cancelled' || item.status === 'claimed' ? 'withdrawn' : item.status === 'running' ? 'stopped' : 'sent';
    }
  } catch (err) {
    flashFooter(`× Couldn’t stop: ${errMsg(err)}`, 4000);
    return 'sent';
  }
  const q = chatQueuedTurn();
  if (outcome === 'withdrawn' && q?.clientTurnId === clientTurnId) endWithdrawnTurn(q, null);
  return outcome;
}

/** "Switch model…" from the palette (frontier F6): the composer's model menu
 *  opens itself when it sees this (and clears it), so the request survives
 *  the conversation mounting after the palette closes. */
export const [modelMenuRequested, setModelMenuRequested] = createSignal(false);

/** After a history restore (or an adoption), put the queued turn's bubble
 *  back at the end of the transcript: a turn that never started has no
 *  stored `turns` row, so history does not contain it. */
export function ensureQueuedTurnVisible() {
  const q = chatQueuedTurn();
  if (!q || q.started || !q.text.trim()) return;
  if (chatTurns.some(t => turnIsQueuedTurn(t, q))) return;
  pushChatTurn({
    role: 'user', text: q.text, tools: [], images: attachmentImages(q.attachments), ...turnFilesField(q.attachments), sharedPage: q.attachments.some(a => !!a.page) || undefined,
    clientTurnId: q.clientTurnId, queueId: q.queueId ?? undefined,
  });
}

function readRestoredQueuedTurns(): string[] {
  try {
    const raw = JSON.parse(localStorage.getItem(RESTORED_QUEUED_TURNS_KEY) ?? '[]');
    return Array.isArray(raw) ? raw.filter((k): k is string => typeof k === 'string') : [];
  } catch { return []; }
}

/** Reconcile the composer with a fresh run-queue read. PR 7a's
 *  `refreshRunQueue` calls this after every `list_run_queue`:
 *  - a live chat row with no local turn (sent before a reload) is adopted,
 *    keeping the composer busy so Resume session cannot replace its key;
 *  - an adopted row that ended without its events being seen is released;
 *  - the local turn's place in line and holder are refreshed;
 *  - a turn the previous Hollow process queued but never started is put back
 *    into an empty composer once, with a notice. It is never run. */
export function reconcileQueuedChatRows(items: RunQueueItem[]) {
  const q = chatQueuedTurn();
  const restoredKeys = readRestoredQueuedTurns();
  const actions = planQueuedChatRows(items, {
    local: q ? { clientTurnId: q.clientTurnId, queueId: q.queueId, started: q.started, adopted: q.adopted } : null,
    busy: chatBusy() || chatSessionLoading(),
    composerEmpty: !chatInput().trim() && chatAttachments().length === 0,
    restoredKeys,
    foregroundKey: chatAgentSessionKey(),
    backgroundKeys: [...backgroundChats.keys()],
  });
  for (const action of actions) {
    switch (action.kind) {
      case 'adopt': {
        const row = action.row;
        setChatQueuedTurn({
          clientTurnId: adoptedClientTurnId(row.id), queueId: row.id, sessionKey: row.sourceId,
          text: row.detail ?? '', attachments: [], requestedCwd: null,
          ...rowWait(row), started: row.status === 'running', intent: null, adopted: true, recovered: false,
        });
        if (row.sourceId) {
          const otherConversation = row.sourceId !== chatAgentSessionKey();
          setChatPermissionMode(conversationPermission(row.sourceId, chatPermissionMode()).mode);
          setChatAgentSessionKeyPersistent(row.sourceId);
          // What is on screen is not this conversation's stored transcript.
          if (otherConversation) setChatStoredTranscriptPending(row.sourceId);
          // Its actual recorded mode has not been loaded by this window.
          setChatSessionIdPersistent(null);
        }
        setChatBusy(true);
        ensureQueuedTurnVisible();
        break;
      }
      case 'update': {
        const cur = chatQueuedTurn();
        if (!cur) break;
        const started = cur.started || action.started;
        const { kind, ...wait } = action;
        setChatQueuedTurn({ ...cur, ...wait, started, ...(started ? { waitingFor: null, waitingSession: null } : {}) });
        break;
      }
      case 'unadopt': {
        const cur = chatQueuedTurn();
        if (!cur?.adopted) break;
        batch(() => {
          setChatQueuedTurn(null);
          if (!cur.started) removeChatUserTurn(cur);
          setChatBusy(false);
        });
        break;
      }
      case 'restore':
        setChatInput(action.row.detail ?? '');
        // A notice about an earlier turn (L5): an interrupted turn above it still
        // offers Continue (found by the `interrupted` fixture).
        pushChatTurn({ role: 'system', text: INTERRUPTED_TURN_NOTICE, tools: [], aboutEarlierTurn: true });
        try { localStorage.setItem(RESTORED_QUEUED_TURNS_KEY, JSON.stringify(mergeRestoredKeys(restoredKeys, action.seen))); } catch { /* ignore */ }
        break;
    }
  }
}

// ── Structured question cards (Paperclip PR 13) ──────────────────────────────
// The open cards (pending or stale) of the conversation on screen. A card is
// seeded by `list_pending_interactions` (so mock mode, which has no event bus,
// can show one) and kept current by `interaction` chat events.

export const [chatInteractions, setChatInteractions] = createSignal<ChatInteraction[]>([]);
const interactionReads = latestRequest();

/** Re-read the current conversation's open cards. A newer call or a change
 *  of conversation wins; a failure keeps what is shown. */
export async function refreshChatInteractions(): Promise<void> {
  const key = chatAgentSessionKey();
  const generation = interactionReads.start();
  if (!key) {
    setChatInteractions([]);
    return;
  }
  try {
    // The chats before it in its chain too: a card asked in a chat that has
    // since continued here is answered here (one Main conversation, 11-07).
    const list = await api.listPendingInteractions(key, true);
    if (!interactionReads.isLatest(generation) || chatAgentSessionKey() !== key) return;
    setChatInteractions(list);
  } catch {
    // Not in Tauri, or the store is unreadable: the next refresh retries.
  }
}

/** Apply one card update for the conversation `key` names. */
export function applyChatInteraction(card: ChatInteraction, key: string | null = chatStopKey()) {
  setChatInteractions(list => applyInteractionUpdate(list, card, key, chainCardKeysFor(key)));
}

export function removeChatInteraction(id: string) {
  setChatInteractions(list => list.filter(card => card.id !== id));
}

/** Forget a conversation's cards locally (after archiving it: the archive
 *  command expires them but emits no event). */
export function dropChatInteractionsFor(key: string) {
  setChatInteractions(list => list.filter(card => card.sessionKey !== key));
}

/** An answer turn could not be sent. Reopen its card; when that worked in the
 *  conversation on screen the card comes back (it can always be sent again,
 *  since 2026-11-04). Otherwise the answers go into the composer with a
 *  notice, so they are never lost.
 *  The backend may already have reopened the card itself
 *  (`reopen_unstarted`), in which case the refresh shows it. */
export async function returnUnsentAnswers(interactionId: string, prompt: string): Promise<'restoreCard' | 'toComposer'> {
  let reopenOk = false;
  let sameChat = false;
  try {
    const card = await api.reopenInteraction(interactionId);
    reopenOk = card.status === 'pending';
    sameChat = !!card.sessionKey && card.sessionKey === chatAgentSessionKey();
  } catch {
    reopenOk = false;
  }
  const action = answerSendFailureAction({ reopenOk, sameChat });
  if (action === 'toComposer') {
    setChatInput(restoredDraft(prompt, chatInput()));
    pushChatTurn({ role: 'system', text: ANSWERS_IN_COMPOSER_NOTICE, tools: [] });
    requestChatFocus();
  }
  await refreshChatInteractions();
  return action;
}

createRoot(() => {
  createEffect(() => {
    chatAgentSessionKey();
    void refreshChatInteractions();
  });
});

// The transcript reducer is shared with every conversation engine
// (chatTranscript.ts, overnight 10-11 B3); these names stay for importers.
export function appendAssistantText(text: string) {
  transcript.appendText(text);
}

/** The live reply's outcome on its last bubble, as a restored turn carries
 *  it (P1-27): `success` from the run's own result, `error` (with its kind)
 *  from a failure, so Retry and Continue (chatContinue.ts) read one field.
 *  It also stamps when the reply finished (`at`, kept from the first mark; a success replaces an earlier error's),
 *  so its row shows a time before, or without, a receipt (overnight 11-12, C8). */
function markReply(status: 'success' | 'error', resultKind?: string) {
  setChatTurns(produce(arr => {
    for (let i = arr.length - 1; i >= 0 && arr[i].role !== 'user'; i--) {
      if (arr[i].role === 'assistant') { arr[i].at = markedReplyAt(arr[i], status, new Date().toISOString()); arr[i].status = status; if (resultKind) arr[i].resultKind = resultKind; return; }
    }
  }));
}
/** The live reply already reported success: an `error` after that without a
 *  result kind is the backend's bookkeeping (history, session record or
 *  receipt not written), not a failed reply. */
function replySucceeded(): boolean {
  for (let i = chatTurns.length - 1; i >= 0 && chatTurns[i].role !== 'user'; i--) {
    if (chatTurns[i].role === 'assistant' && chatTurns[i].status === 'success') return true;
  }
  return false;
}

/** Append a delta chunk (streaming) to the current assistant turn. */
export function appendAssistantDelta(delta: string) {
  transcript.appendDelta(delta);
}

export function appendToolUse(name: string, input?: unknown, id?: string, parentToolUseId?: string | null) {
  // An approval bound to this call may have arrived before the call itself
  // (the two travel on different channels): apply it now.
  transcript.appendToolUse(name, input, id, id ? pendingApprovalBadges.get(id) : undefined, parentToolUseId);
}

/** Attach a tool_result to its originating tool_use (matched by id). */
export function applyToolResult(id: string, content: string, isError: boolean) {
  transcript.applyToolResult(id, content, isError);
}

/** Attach data-URL images (a teammate's generated artifacts) to their
 *  originating tool_use, matched by id. */
export function applyToolImages(id: string, images: string[]) {
  transcript.applyToolImages(id, images);
}

export function toggleToolExpanded(turnIdx: number, toolIdx: number) {
  transcript.toggleToolExpanded(turnIdx, toolIdx);
  // WD1: a row the lean cold open sent as a preview brings its result
  // whole the first time it opens (only into the same chat and call).
  const tool = chatTurns[turnIdx]?.tools[toolIdx];
  const key = chatAgentSessionKey();
  if (!tool?.expanded || tool.resultChars == null || tool.resultTurn == null || !tool.id || !key) return;
  const { id, resultTurn: ord } = tool;
  void api.readToolResult(key, ord, id).then(full => {
    const now = chatTurns[turnIdx]?.tools[toolIdx];
    if (chatAgentSessionKey() !== key || now?.id !== id || now.resultTurn !== ord) return;
    setChatTurns(turnIdx, 'tools', toolIdx, { result: full ?? undefined, resultChars: undefined, resultLines: undefined, resultTurn: undefined });
  }, e => flashFooter(`× The full output didn’t load: ${errMsg(e)}`));
}

/** Attach a backend receipt only within the live run that announced its id. */
function applyEvidenceReceipt(wire: EvidenceReceiptWire) {
  const context = activeChatEvidence?.runId === wire.runId ? activeChatEvidence : null;
  const receipt = executionReceiptFromWire(wire, context);
  transcript.applyReceipt(receipt, wire.runId, context?.turnStartIndex ?? chatTurns.length);
  commitReceiptVideos(receipt);
}

// Finalize all partial turns when a `done` event arrives. Any tool still
// marked 'running' (cancelled mid-flight, or a result that never arrived)
// settles to 'done' so it doesn't spin forever.
function finalizePartials() {
  transcript.finalize();
}

// ─────────────────────────────────────────────────────────────
// CHAT PREVIEW — the Chat tab's artifact pane (Claude-desktop-style).
// We watch the event stream for Write/Edit tool calls and collect the files the
// agent touched (committed only when the tool call succeeds). They are listed
// under the reply and in the Preview pane's "Conversation files" menu; the
// pane opens only when the person clicks one (run 15 P1: it never opens by
// itself when a turn ends). While a file is previewed, every successful write
// to it bumps the preview nonce, so the person (and the iterating agent) see
// each edit land live.
// ─────────────────────────────────────────────────────────────

export { chatArtifactKind, type ChatArtifactKind };

export interface ChatArtifact {
  /** Absolute path on disk. */
  path: string;
  /** Basename, for display. */
  name: string;
  kind: ChatArtifactKind;
  /** Monotonic touch counter — higher = touched more recently. */
  touchedAt: number;
}

/** Tools whose input names a file the agent is creating/changing. */
const WRITE_TOOLS = new Set(['Write', 'Edit', 'MultiEdit', 'NotebookEdit']);

/** Extract the written file's absolute path from a write-tool input, or null
 *  when the tool isn't a write / the path is relative / it's tool-internal
 *  housekeeping (any dot-prefixed component: .git, .hollow-attachments, …). */
function artifactPathFromTool(name: string, input: unknown): string | null {
  if (!WRITE_TOOLS.has(name)) return null;
  if (!input || typeof input !== 'object') return null;
  const i = input as Record<string, unknown>;
  const p =
    typeof i.file_path === 'string' ? i.file_path :
    typeof i.notebook_path === 'string' ? i.notebook_path : null;
  if (!p || !p.startsWith('/')) return null;
  if (p.split('/').some(seg => seg.startsWith('.'))) return null;
  return p;
}

const ARTIFACT_CAP = 24;
let artifactSeq = 0;
/** tool_use id → path, held until the matching tool_result confirms success. */
const pendingArtifactWrites = new Map<string, string>();
let artifactsTouchedThisRun = false;

/** Files the agent created/edited this session, newest-touched first. */
export const [chatArtifacts, setChatArtifacts] = createSignal<ChatArtifact[]>([]);
/** The file the Preview pane shows (store/preview.ts), or null. */
export const chatPreviewPath = (): string | null => {
  const target = previewTarget();
  return target?.kind === 'file' ? target.path : null;
};
/** The lines a citation opened the shown file at, if any. */
export const chatPreviewLines = (): { path: string; line: number; end?: number } | null => {
  const target = previewTarget();
  return target?.kind === 'file' && target.line ? { path: target.path, line: target.line, ...(target.end ? { end: target.end } : {}) } : null;
};
/** Bumped to make the preview pane re-read the file from disk. */
export const chatPreviewNonce = previewNonce;
export const setChatPreviewNonce = setPreviewNonce;

/** Superseded whenever the artifact list is reset (another conversation, a
 *  reload, a background switch), so a receipt video committed after
 *  `vaultRoot()` resolves never lands in the wrong conversation's list. */
const artifactLists = latestRequest();

/** `touch: false` (a receipt's video) lists the file without marking the run
 *  as having written one: the receipt arrives before `done`, and a touched
 *  run with only a video would auto-open an older artifact. */
function commitArtifact(path: string, opts: { touch?: boolean } = {}) {
  if (opts.touch !== false) artifactsTouchedThisRun = true;
  const name = path.slice(path.lastIndexOf('/') + 1);
  setChatArtifacts(prev => {
    const next = prev.filter(a => a.path !== path);
    next.unshift({ path, name, kind: chatArtifactKind(name), touchedAt: ++artifactSeq });
    return next.slice(0, ARTIFACT_CAP);
  });
  // Live-refresh while the agent iterates on the file being previewed.
  if (chatPreviewOpen() && chatPreviewPath() === path) setChatPreviewNonce(n => n + 1);
}

// ── The Preview pane (run 15 P1) replaced the side panel: one app-wide right
// pane, hidden until the person opens something (store/preview.ts). Canvases
// and HTML previews render in the reply; a canvas's Expand opens it there.
/** Show a visual of the open conversation (a canvas's Expand) in the Preview pane. */
export function showChatVisual(key: string) { openPreview({ kind: 'visual', key }); }
/** Every settled visual in the current conversation (see chatVisuals.ts). */
export const chatVisuals = createRoot(() => createMemo(() => collectVisuals(chatTurns)));
// A Subagents tab is one chat's (COO rebuild L3): when another conversation
// comes on screen, the tabs of chats outside its chain go. A handover or the
// Main agent's day roll stays in the chain, so the tab being watched stays.
createRoot(() => {
  let last = chatAgentSessionKey();
  createEffect(() => {
    const key = chatAgentSessionKey();
    if (key === last) return;
    last = key;
    const tabs = subagentTabParents();
    if (!tabs.length || (key && tabs.every(p => p === key))) return;
    if (!key) { dropSubagentPreviews(new Set()); return; }
    void api.conversationChain(key).catch(() => null).then(chain => {
      if (chatAgentSessionKey() !== key) return;
      dropSubagentPreviews(new Set([key, ...(chain ? [chain.key, chain.current, ...chain.earlier.map(l => l.key)] : [])]));
    });
  });
});
/** A draft edit from a workspace or preview surface: appended to the
 *  composer and focused, never sent. */
export function appendToChatDraft(text: string) {
  setChatInput(addToDraft(chatInput(), text));
  requestChatFocus();
}

/** True while the Preview pane shows a file. */
export const chatPreviewOpen = () => previewTarget()?.kind === 'file';

/** Preview a file (`path` defaults to the conversation's newest previewable
 *  file, falling back to the newest of any kind), at cited lines if given. */
export function openChatPreview(path?: string, lines?: { line?: number; end?: number }) {
  const target = path
    ?? chatArtifacts().find(a => a.kind !== 'binary')?.path
    ?? chatArtifacts()[0]?.path;
  if (!target) return;
  openPreview({ kind: 'file', path: target, line: lines?.line, end: lines?.end });
}
export function closeChatPreview() { closePreview(); }

/** A file click's identity, taken in its handler before any chunk loads
 *  (2026-11-04 review): its conversation, and its ask of the Preview pane,
 *  which any later open or close supersedes (`store/fileOpen.ts`). */
export interface FileClick { version: number; cwd: string | null; intent: number }
export function fileClick(): FileClick {
  return { version: chatConversationVersion(), cwd: chatCwd(), intent: newPreviewIntent() };
}

/** Open a file a chat widget names (a tool or reply-footer chip, a file link
 *  in a reply, an attached image's data URL) in the Preview pane, beside
 *  whatever page is showing. Kept for callers written before `openPreview`. */
export function openFileInSidePane(target: string, lines?: { line?: number; end?: number }) {
  if (!target) return;
  openChatPreview(target, lines);
}

let vaultRootLookup: Promise<string | null> | null = null;
/** The vault's absolute path, for vault-relative receipt rows. Looked up once;
 *  a failed lookup is retried next time. */
export function vaultRoot(): Promise<string | null> {
  vaultRootLookup ??= api.setupStatus()
    .then(s => s.vaultPath || null)
    .catch(() => { vaultRootLookup = null; return null; });
  return vaultRootLookup;
}

/** The videos one receipt lists (a render through Bash included), at most
 *  `REPLY_VIDEO_CAP`, so a batch render can't push the conversation's other
 *  files out of the capped list; the reply's strip lists the rest. */
function commitVideosOf(receipt: ExecutionReceipt | undefined, root: string | null) {
  for (const path of receiptVideos(receipt, root).slice(0, REPLY_VIDEO_CAP)) commitArtifact(path, { touch: false });
}

/** A live receipt's videos join the Files picker, untouched (see
 *  `commitArtifact`). Vault rows need the vault's path, so they land when it
 *  resolves, and only while the same artifact list is showing. */
function commitReceiptVideos(receipt: ExecutionReceipt) {
  const ticket = artifactLists.peek();
  void vaultRoot().then(root => {
    if (artifactLists.isLatest(ticket)) commitVideosOf(receipt, root);
  });
}

/** Rebuild the artifact list from a loaded session transcript so a resumed
 *  session still offers its files for preview. Never auto-opens the pane.
 *  Turn by turn, each turn's written files then its receipt's videos, so
 *  the list keeps the conversation's order: at once without the vault's
 *  path (repo rows only), then again in full when it resolves (or, if a run
 *  started meanwhile, only its videos added). */
export function rebuildChatArtifacts(turns: { tools: { name: string; input?: unknown }[]; receipt?: ExecutionReceipt }[]) {
  const ticket = artifactLists.start();
  const fill = (root: string | null) => {
    setChatArtifacts([]);
    for (const t of turns) {
      for (const tu of t.tools) {
        const p = artifactPathFromTool(tu.name, tu.input);
        if (p) commitArtifact(p);
      }
      commitVideosOf(t.receipt, root);
    }
    artifactsTouchedThisRun = false;
  };
  fill(null);
  if (turns.some(t => t.receipt?.files.length)) {
    void vaultRoot().then(root => {
      if (!root || !artifactLists.isLatest(ticket)) return;
      // A run started meanwhile has its own files in the list: add the
      // videos without clearing it.
      if (chatBusy()) for (const t of turns) commitVideosOf(t.receipt, root);
      else fill(root);
    });
  }
}

function clearChatArtifacts() {
  artifactLists.cancel();
  setChatArtifacts([]);
  // A previewed file stays (it is a file on disk, opened by the person); the
  // old conversation's visuals can't be shown any more.
  dropConversationPreviews();
  pendingArtifactWrites.clear();
  artifactsTouchedThisRun = false;
}

/** Turn finished: the files it wrote are listed under the reply and in the
 *  Preview pane's "Conversation files" menu. Nothing opens by itself (run 15
 *  P1, Alex: the pane shows what is clicked, "otherwise it should hide
 *  itself"). */
function finalizeArtifactsForRun() {
  pendingArtifactWrites.clear();
  artifactsTouchedThisRun = false;
}

/** One `chat-event` payload: generated from `src-tauri/src/chat_event.rs`
 *  (AG2), so a field the backend adds, renames or drops changes this type.
 *  The fields' meanings are documented on the Rust structs. */
type ChatEventPayload =
  | Exclude<ChatEventEnvelope, { type: 'heldSent' | 'agentContinued' }>
  // Always tagged with their queue row (and the continued send's id); the
  // backend's schema check (`chat_event::conformance_error`) requires it.
  | (Extract<ChatEventEnvelope, { type: 'heldSent' }> & { queueId: number })
  | (Extract<ChatEventEnvelope, { type: 'agentContinued' }> & { queueId: number; clientTurnId: string });

// ── Mid-turn steering (2026-10-21) ─────────────────────────────────────────

/** What Steer now does per agent (`chat_steer_support`), read once. */
export const [chatSteerSupport, setChatSteerSupport] = createSignal<SteerSupportRow[] | null>(null);
export const [chatTurnSteerSupport, setChatTurnSteerSupport] = createSignal<SteerSupportRow | null>(null);
let steerSupportAsked = false;
const steerSupportReads = latestRequest();
export function ensureChatSteerSupport(refresh = false) {
  if (steerSupportAsked && !refresh) return;
  steerSupportAsked = true;
  const ticket = steerSupportReads.start();
  void api.chatSteerSupport().then(rows => {
    if (steerSupportReads.isLatest(ticket)) setChatSteerSupport(rows);
  }, () => { if (steerSupportReads.isLatest(ticket)) steerSupportAsked = false; });
}
/** The agent a mid-turn message reaches: the running turn's own, which a
 *  turn continued on another agent when one ran out (repo ideas 10-27 #8)
 *  makes differ from `picked`, the one chosen for the next send. */
export function steeringAgent(picked: string): string {
  return (chatBusy() ? chatTurnSteerSupport()?.agent : null) ?? picked;
}
/** Whether `agent` takes a message mid-turn; unknown (not read yet) is no. */
export function steersNatively(agent: string, turn = chatBusy() ? chatTurnSteerSupport() : null): boolean {
  return (turn?.agent === agent ? turn : chatSteerSupport()?.find(row => row.agent === agent))?.support === 'native';
}
/** The backend's one-sentence note on what Steer now does for `agent`. */
export function steerSupportNote(agent: string, turn = chatBusy() ? chatTurnSteerSupport() : null): string {
  return (turn?.agent === agent ? turn : chatSteerSupport()?.find(row => row.agent === agent))?.note ?? '';
}

/** Sends held for after a conversation's running turn, by conversation. */
const [heldChips, setHeldChips] = createSignal<Record<string, HeldChip[]>>({});
export function heldChipsFor(key: string | null): HeldChip[] {
  return key ? heldChips()[key] ?? [] : [];
}
/** False (no chip) for a held send whose words already came back. */
export function addHeldChip(key: string, chip: HeldChip): boolean {
  if (heldBack.has(chip.id)) return false;
  setHeldChips(all => ({ ...all, [key]: [...(all[key] ?? []), chip] }));
  return true;
}
export function removeHeldChip(key: string, id: string): HeldChip | null {
  const chip = heldChipsFor(key).find(c => c.id === id) ?? null;
  setHeldChips(all => ({ ...all, [key]: (all[key] ?? []).filter(c => c.id !== id) }));
  return chip;
}
/** Take a held send's chip by its id, under whichever conversation it was
 *  added: a `heldSent` after a handover names the successor's key, not the
 *  one the chip is under (C8b), and an idle chat not on screen gets its
 *  outcome on the orphan route (C8c). Null when no chip has that id. */
function takeHeldChip(id: string): HeldChip | null {
  const key = Object.keys(heldChips()).find(k => heldChips()[k].some(c => c.id === id));
  return key === undefined ? null : removeHeldChip(key, id);
}

/** Words (and images) that came back for a conversation not on screen,
 *  waiting for it to open (review: never into another chat's box). */
const stashed = new Map<string, { text: string; atts: ChatAttachment[] }>();
/** Put words and images back in conversation `key`'s box, beside anything
 *  typed since: now when it is on screen (with `notice` in its chat), else
 *  when it next opens (the footer says so now). */
export function returnToConversation(key: string | null, text: string, atts: ChatAttachment[], notice: string | null) {
  if (!key || key === chatAgentSessionKey()) {
    if (text) setChatInput(restoredDraft(text, chatInput()));
    if (atts.length) setChatAttachments(current => [...atts.filter(a => !current.some(c => c.id === a.id)), ...current]);
    if (notice) pushChatTurn({ role: 'system', text: notice, tools: [] });
    return;
  }
  // Nothing for the box (an ask-canvas's answers stay in their canvas).
  if (!text && !atts.length) { if (notice) flashFooter(notice, 8000); return; }
  const prior = stashed.get(key);
  stashed.set(key, { text: prior ? `${prior.text}\n\n${text}`.trim() : text, atts: [...(prior?.atts ?? []), ...atts] });
  flashFooter(`${notice ?? 'A message came back'} It's in that chat's box when you open it.`, 8000);
}
/** Drain any stashed returned message for the conversation now on screen.
 *  Driven by the `createEffect` below in the browser; exported so node unit
 *  tests (where Solid's SSR build stubs `createEffect`) can invoke it directly. */
export function flushStashedForCurrentChat() {
  const key = chatAgentSessionKey();
  if (!key || chatSessionLoading() || !stashed.has(key)) return;
  const back = stashed.get(key)!;
  stashed.delete(key);
  queueMicrotask(() => returnToConversation(key, back.text, back.atts, null));
}
createRoot(() => {
  createEffect(flushStashedForCurrentChat);
});

/** Sends made while a turn ran, by client id, from before their call until
 *  their turn starts or ends: a turn that ends before it starts hands its
 *  words back (review). */
const busyPending = new Map<string, { key: string | null; text: string; atts: ChatAttachment[]; started: boolean; error: string | null }>();
export function registerBusySend(clientTurnId: string, key: string | null, text: string, atts: ChatAttachment[]) {
  busyPending.set(clientTurnId, { key, text, atts, started: false, error: null });
  if (busyPending.size > 50) busyPending.delete(busyPending.keys().next().value!);
}
/** Its call failed or it was held instead: nothing to reconcile. */
export function forgetBusySend(clientTurnId: string) { busyPending.delete(clientTurnId); }
function busyPendingEvent(p: ChatEventPayload) {
  const id = p.clientTurnId;
  const pending = id ? busyPending.get(id) : undefined;
  if (!id || !pending) return;
  if (p.type === 'runStarted') pending.started = true;
  else if (p.type === 'error' && !pending.started) pending.error = p.message;
  else if (p.type === 'done') {
    busyPending.delete(id);
    if (!pending.started && answersBack(pending.key, pending.text)) {
      returnToConversation(pending.key, '', [], `Your answers didn't start${pending.error ? `: ${pending.error.trim().replace(/[.\s]+$/, '')}` : ''}. They're still in the canvas.`);
    } else if (!pending.started) returnToConversation(pending.key, pending.text, pending.atts, `Your message didn't start${pending.error ? `: ${pending.error.trim().replace(/[.\s]+$/, '')}` : ''}. It's back in the box.`);
  }
}

/** What this window saw of each turn it sent, by client id: a send whose
 *  reply arrives after its `runStarted` (or its `done`) is reconciled with
 *  that, never drawn twice or left busy (review). Bounded. */
const seenTurns = new Map<string, 'started' | 'done'>();
function noteTurnSeen(id: string | null | undefined, what: 'started' | 'done') {
  if (!id) return;
  seenTurns.set(id, what);
  if (seenTurns.size > 200) seenTurns.delete(seenTurns.keys().next().value!);
}
/** A held send's outcome that arrived before its `chat_send` reply named
 *  it, kept for that reply (`takeEarlyHeld`). */
type HeldOutcome = Extract<ChatEventPayload, { type: 'heldSent' | 'heldFailed' }>;
const earlyHeld = new Map<string, HeldOutcome>();
/** Parked failures' grace timers, by held id. */
const heldTimers = new Map<string, ReturnType<typeof setTimeout>>();
/** Held ids whose words already came back from the event: a `chat_send`
 *  reply that names one later draws no chip (`addHeldChip`). Bounded. */
const heldBack = new Set<string>();
/** The window now holds a failed held send's words (the box, a notice or
 *  the canvas): tell Rust it may drop its saved copy. Rust keeps a failed
 *  held send until this (R8b), so a quit between the event and the words
 *  landing leaves it to be reported again at the next launch. The
 *  acknowledgement of an id Rust no longer holds answers `false`, which
 *  is harmless. */
function ackHeldFailed(p: Extract<HeldOutcome, { type: 'heldFailed' }>) {
  void Promise.resolve(api.chatAckHeldFailed(p.conversationKey ?? p.sessionKey, p.heldId)).catch(() => {});
}
export function takeEarlyHeld(heldId: string) {
  const outcome = earlyHeld.get(heldId) ?? null;
  earlyHeld.delete(heldId);
  clearTimeout(heldTimers.get(heldId));
  heldTimers.delete(heldId);
  // Whoever takes a parked failure keeps its words in the same step
  // (`back(...)`, `returnToConversation`, or the canvas that holds answers).
  if (outcome?.type === 'heldFailed') ackHeldFailed(outcome);
  return outcome;
}
/** How long a failed held send with no chip waits for a `chat_send` reply
 *  to claim it before its own words go back (lane Q send-path read). */
export const HELD_FAILED_GRACE_MS = 5000;
/** The person's own words in a held send's prompt: `composePayload`
 *  (chatSendController.ts) put the page context, shared pages and Hollow's
 *  intent line before them and the attached images and files after. */
export function heldWords(prompt: string): string {
  return prompt
    .replace(/^(?:<(page-context|shared-page)>[\s\S]*?<\/\1>\n\n)*<hollow-chat-intent>[\s\S]*?<\/hollow-chat-intent>\n\n/, '')
    .replace(HELD_FILES, '').replace(HELD_IMAGES, '')
    .trim();
}
// composePayload's trailing blocks exactly as it writes them (images, then
// files), at the very end only: words typed to look like one stay.
const HELD_IMAGES = /\n\n<attached-images count="\d+">\nThe user attached \d+ images? to this message\. Open (?:them|it) using your available image-viewing tool:(?:\n\d+\. [^\n]*)+\n<\/attached-images>$/;
const HELD_FILES = /\n\n<attached-files count="\d+">\nThe user attached \d+ files? to this message\. Each line gives the file's original name, then the absolute path of its saved copy\. Open (?:them|it) with your file-reading tools:(?:\n\d+\. [^\n]*)+\n<\/attached-files>$/;
/** A parked failure no reply claimed: its words back, once. */
function restoreParked(p: Extract<HeldOutcome, { type: 'heldFailed' }>) {
  takeEarlyHeld(p.heldId);
  heldBack.add(p.heldId);
  if (heldBack.size > 100) heldBack.delete(heldBack.values().next().value!);
  const words = heldWords(p.prompt);
  // An ask-canvas's answers never go to the box: their canvas keeps them.
  if (answeredRevision(words)) returnToConversation(p.conversationKey ?? p.sessionKey, '', [], `Your queued answers weren't sent: ${p.error.trim().replace(/[.\s]+$/, '')}. They're still in the canvas.`);
  else returnHeldToBox(p.conversationKey ?? p.sessionKey, null, p.error, words);
}
/** Park a held send's outcome for the reply that will name it. A failure
 *  no reply claims (a reload or relaunch lost its chip; the send was made
 *  before) puts the event's own words back in its conversation's box; so
 *  does one pushed out of a full map. */
function parkEarlyHeld(p: HeldOutcome) {
  // A later outcome for the same send replaces the parked one and its timer
  // (a heldSent after a parked failure restores nothing).
  clearTimeout(heldTimers.get(p.heldId));
  heldTimers.delete(p.heldId);
  earlyHeld.set(p.heldId, p);
  if (earlyHeld.size > 50) {
    const oldest = earlyHeld.values().next().value!;
    if (oldest.type === 'heldFailed') restoreParked(oldest);
    else earlyHeld.delete(oldest.heldId);
  }
  if (p.type === 'heldFailed') heldTimers.set(p.heldId, setTimeout(() => restoreParked(p), HELD_FAILED_GRACE_MS));
}
/** A reset hands every parked failure back now (stashed for its chat),
 *  timers cleared, rather than leaving them to fire later. */
function flushParkedHeld() {
  for (const p of [...earlyHeld.values()]) if (p.type === 'heldFailed') restoreParked(p);
}

/** A send made while a turn ran turned out to be this conversation's next
 *  turn (sent at once, or a held send that went): draw and track it as if
 *  just sent, unless its events already did (`runStarted` drew its bubble
 *  from `asked`; `done` ended it). */
export function adoptBusySend(clientTurnId: string, text: string, attachments: ChatAttachment[], accepted: SendAccepted) {
  const seen = seenTurns.get(clientTurnId);
  const shown = chatTurns.some(t => t.role === 'user' && t.clientTurnId === clientTurnId);
  if (!shown && seen !== 'done') pushChatTurn({ role: 'user', text, tools: [], images: attachmentImages(attachments), ...turnFilesField(attachments), sharedPage: attachments.some(a => !!a.page) || undefined, clientTurnId, queueId: accepted.queueId, at: new Date().toISOString() });
  if (seen) {
    if (seen === 'started') setChatBusy(true);
    return;
  }
  // The queued-turn record owns it from here (Edit, Discard and an end
  // before start), so the busy-send registry lets go.
  busyPending.delete(clientTurnId);
  beginQueuedChatTurn({ clientTurnId, text, attachments, requestedCwd: chatCwd(), sessionKey: accepted.sessionKey, interactionId: null });
  acceptQueuedChatTurn(clientTurnId, accepted);
  setChatBusy(true);
}

/** Draw or move a steer row in the transcript on screen. A new one ends
 *  the reply's bubble so the agent's later words go below it. */
function applySteer(steer: SteerWire) {
  const at = chatTurns.findIndex(t => t.notice === 'steer' && t.steer?.id === steer.id);
  if (at >= 0) {
    // A refusal brings the agent's reason (P1-30).
    setChatTurns(at, 'steer', s => (s ? { ...s, state: steer.state, ...(steer.reason ? { reason: steer.reason } : {}) } : s));
    return;
  }
  finalizePartials();
  transcript.setPendingNewTurn(false);
  pushChatTurn(steerTurn(steer));
}
/** A held send that won't go: its words and images back in the box (beside
 *  anything typed since), and a notice saying why. */
export function returnHeldToBox(key: string | null, chip: HeldChip | null, error: string | null, prompt = '') {
  // An ask-canvas's queued answers (P1-33) go back to their canvas, never
  // the box: a failure shows its Send now and Take them back there.
  if (chip && settleHeldAnswers(chip.id, error === null ? null : chip.text)) {
    if (error) returnToConversation(key, '', [], `Your queued answers weren't sent: ${error.trim().replace(/[.\s]+$/, '')}. They're still in the canvas.`);
    return;
  }
  // No chip (gone with a reload or relaunch): the event's own words.
  returnToConversation(key, chip?.text ?? prompt, chip?.attachments ?? [], error ? `Your waiting message wasn't sent: ${error.trim().replace(/[.\s]+$/, '')}. It's back in the box.` : null);
}

/** The composer's own steer, from `chat_steer`'s reply (the same event also
 *  arrives on the bus; whichever comes first draws it). */
export function applyOwnSteer(steer: SteerWire) {
  applySteer(steer);
}

/** Settle the local queued turn against an event; returns the decision. */
function settleQueuedTurn(p: ChatEventPayload): QueuedTurnDecision {
  const q = chatQueuedTurn();
  if (!q) return 'none';
  const decision = queuedTurnEnd(p, q, q.intent);
  applyQueuedTurnDecision(decision, q);
  return decision;
}

function acquireFirstDraft(key: string, q: ChatQueuedTurn | null) {
  if (!chatAgentSessionKey() && q && !q.adopted && !q.recovered && !q.sessionKey
    && q.requestedCwd === chatCwd()) acceptFirstDraftSession(chatCwd(), key);
}

/** The paused and pausing state follows every chat's events, whichever
 *  conversation is on screen (the sidebar and Mission Control read it):
 *  `paused` marks it; a turn starting clears it (the backend clears its
 *  record when any turn of the chat is admitted); a stop or the end of a
 *  turn ends a pause still being asked. */
function notePauseEvent(p: ChatEventPayload) {
  const key = p.type === 'paused' ? p.sessionKey : (p.conversationKey ?? ('sessionKey' in p ? p.sessionKey : null) ?? null);
  if (!key) return;
  // An if-chain, not a switch: source-pin tests read handleChatEvent's switch
  // by the first place this file names the run-start event kind in a case.
  if (p.type === 'paused') markChatPaused(key, p.note, p.at, { reason: p.reason ?? 'user', resumeAt: p.resumeAt ?? null });
  else if (p.type === 'runStarted') clearChatPaused(key);
  else if (p.type === 'cancelled') { if (!p.queued) setChatPausing(key, false); }
  else if (p.type === 'done') setChatPausing(key, false);
}

function handleChatEvent(p: ChatEventPayload) {
  // Recorded before any route decides (a background route only buffers it),
  // for a Stop that follows this held id to its row (C10, `heldOutcome`).
  if (p.type === 'heldSent' || p.type === 'heldFailed') noteHeldSeen(p);
  // Several conversations can run at once (build programme PR B4): another
  // conversation's events never touch the one on screen.
  const route = routeChatEvent(p);
  // A held send that went (or couldn't) leaves its conversation's chips; a
  // backgrounded conversation's, when its buffer replays.
  let held: HeldChip | null = null;
  // A stopped send following its held id to the row it became (C10).
  if (p.type === 'heldSent' || p.type === 'heldFailed') heldWaiters.get(p.heldId)?.(p);
  if ((p.type === 'heldSent' || p.type === 'heldFailed') && route === 'foreground') {
    held = takeHeldChip(p.heldId);
    // Its chip isn't drawn yet (the reply that names it is still on its
    // way): the reply finishes the job.
    if (!held) { parkEarlyHeld(p); return; }
  }
  // A failure for a conversation this window never showed (after a reload,
  // or a send forwarded elsewhere) is never dropped: its chip's words if this
  // window drew one, else the event's own once no reply claims it.
  if (p.type === 'heldFailed' && route === 'orphan') {
    const chip = takeHeldChip(p.heldId);
    if (chip) { returnHeldToBox(p.sessionKey, chip, p.error); ackHeldFailed(p); }
    else parkEarlyHeld(p);
    return;
  }
  // A held send that went from a chat not on screen and not running (C8c):
  // its "waiting" chip goes, and an ask-canvas's hold on it ends as sent.
  // No chip yet: kept for the reply that names it, so it draws none.
  if (p.type === 'heldSent' && route === 'orphan') {
    const chip = takeHeldChip(p.heldId);
    if (chip) settleHeldAnswers(chip.id, null);
    else parkEarlyHeld(p);
    return;
  }
  // Whichever route: the registry knows its own sends (review), and a
  // background replay finds the entry already settled.
  busyPendingEvent(p);
  notePauseEvent(p);
  if (route === 'foreground' && (p.type === 'runStarted' || p.type === 'done')) noteTurnSeen(p.clientTurnId, p.type === 'done' ? 'done' : 'started');
  switch (route) {
    case 'background':
    // An event no conversation here claims (neither the one on screen nor a
    // backgrounded one). It refreshed Claude history's list until that tab was
    // removed (run 15 P4); nothing else reads it here.
    case 'orphan': return;
  }
  switch (p.type) {
    case 'session':
      setChatSessionIdPersistent(p.sessionId);
      break;
    case 'runStarted': {
      // Both key-adoption points (here and `done`) ignore a turn that already
      // ended locally; see isStaleTurnEvent.
      if (isStaleTurnEvent(p, chatQueuedTurn(), endedQueuedTurns)) break;
      const starting = chatQueuedTurn();
      if (p.sessionKey && starting && eventMatchesTurn(p, starting)) acquireFirstDraft(p.sessionKey, starting);
      settleQueuedTurn(p);
      setChatTurnUsage(null);
      setChatTurnSteerSupport(p.steerSupport ?? chatSteerSupport()?.find(row => row.agent === chatAgent()) ?? null);
      pendingApprovalBadges.clear();
      // A turn this window did not send (adopted after a reload, before its
      // bubble was restored) still shows the question it answers.
      const tagged = !!p.clientTurnId || p.queueId != null;
      const shown = chatTurns.some(t => t.role === 'user' && (
        (!!p.clientTurnId && t.clientTurnId === p.clientTurnId) || (p.queueId != null && t.queueId === p.queueId)));
      if (tagged && !shown && p.asked?.trim()) {
        pushChatTurn({ role: 'user', text: p.asked, tools: [], clientTurnId: p.clientTurnId ?? undefined, queueId: p.queueId ?? undefined, at: new Date().toISOString() });
      }
      activeChatEvidence = {
        runId: p.runId,
        recall: p.recall,
        turnStartIndex: chatTurns.length,
      };
      // The mode it runs in (2026-11-04), kept on the turn itself so it goes
      // with the conversation into the background and back: an adopted
      // turn's end decides from it whether its session carries on.
      const running = chatQueuedTurn();
      if (running && eventMatchesTurn(p, running) && typeof p.permissionMode === 'string') {
        setChatQueuedTurn({ ...running, permissionMode: p.permissionMode });
      }
      // Adopt the key the backend minted (or confirmed) for this turn, so the
      // next send continues the same session instead of starting a new one.
      if (p.sessionKey) setChatAgentSessionKeyPersistent(p.sessionKey, { ownTurn: true });
      setActiveChatRecall(p.recall);
      setTrace(p.rt ?? undefined);
      // After the stale check, so a withdrawn queued turn never claims it.
      setChatLiveRunId(p.runId);
      // A turn running here is busy, whoever sent it (a held send, a turn
      // replayed from the background buffer).
      setChatBusy(true);
      // A new turn starts with no activity (Paperclip PR 15b).
      setChatActivity(null);
      setChatRunStartedAt(Date.now());
      break;
    }
    case 'assistantText':
      appendAssistantText(p.text);
      break;
    case 'assistantDelta':
      appendAssistantDelta(p.text);
      break;
    case 'turnBoundary':
      transcript.markBoundary();
      break;
    case 'toolUse': {
      appendToolUse(p.name, p.input, p.id, p.parentToolUseId);
      const artifactPath = artifactPathFromTool(p.name, p.input);
      if (artifactPath) {
        // Commit when the result confirms success; commit immediately only
        // for id-less events (older agents) where no result will match.
        if (p.id) pendingArtifactWrites.set(p.id, artifactPath);
        else commitArtifact(artifactPath);
      }
      break;
    }
    case 'toolResult': {
      applyToolResult(p.id, p.content, p.isError);
      const pendingPath = pendingArtifactWrites.get(p.id);
      if (pendingPath) {
        pendingArtifactWrites.delete(p.id);
        if (!p.isError) commitArtifact(pendingPath);
      }
      break;
    }
    case 'toolImages':
      applyToolImages(p.id, p.images);
      break;
    case 'evidenceReceipt':
      applyEvidenceReceipt(p.receipt);
      break;
    case 'result':
      if (p.sessionId) setChatSessionIdPersistent(p.sessionId);
      {
        // A refusal is not an error the user caused: a neutral notice.
        const added = resultEventTurn(p);
        if (added && 'notice' in added) pushChatTurn({ role: 'system', text: added.notice, tools: [] });
        else if (added) appendAssistantText(added.append);
        if (!p.isError) markReply('success');
      }
      break;
    case 'usage':
      setChatTurnUsage(p.usage);
      if (p.usage?.contextTokens != null) {
        setChatContextRaw({ version: chatConversationVersion(), tokens: p.usage.contextTokens, model: p.usage.model ?? null, window: p.usage.contextWindow ?? null });
      }
      break;
    case 'activity':
      // Only the turn in flight's (its runStarted run id); a foreign or late
      // tick changes nothing (Paperclip PR 15b).
      setChatActivity(current => nextChatActivity(current, chatLiveRunId(), p));
      break;
    case 'interaction':
      // Not gated by isStaleTurnEvent: a reopened card arrives on the answer
      // turn that failed, and must still show. Another conversation's card is
      // ignored (its own list is read when it is opened).
      if (p.sessionKey && p.sessionKey === chatStopKey()) applyChatInteraction(p.interaction, p.sessionKey);
      break;
    case 'budgetWarning':
      // Only into the conversation it was claimed for; another conversation
      // (or none) drops it, and Settings, Usage still shows the state.
      if (p.sessionKey && p.sessionKey === chatStopKey()) {
        pushChatTurn({ role: 'system', text: budgetNoticeText(p.notice), tools: [] });
      }
      break;
    case 'panel': {
      // AG6: a snapshot replaces the last one of that panel, only for the
      // conversation on screen. `hollow:context` (PS1) also shows as the
      // system row it always was. Never a turn's lifecycle.
      // A plan proposed in a chat that is not on screen still marks its sidebar
      // row (3.3); the card itself only exists for the open chat.
      if (p.panel === 'hollow:proposedPlan' && p.sessionKey && p.sessionKey !== chatStopKey()) {
        const s = p.snapshot as { runId?: unknown; text?: unknown } | null;
        if (s?.runId && typeof s.text === 'string' && s.text.trim()) void import('./rowStatus').then(m => m.markPlanReady(p.sessionKey as string));
        break;
      }
      if (!p.sessionKey || p.sessionKey !== chatStopKey()) break;
      if (p.panel === 'hollow:proposedPlan') {
        if (isStaleTurnEvent(p, chatQueuedTurn(), endedQueuedTurns)) break;
        const s = p.snapshot as { runId?: unknown; text?: unknown } | null;
        if (!s || !s.runId || s.runId !== chatLiveRunId() || typeof s.text !== 'string' || !s.text.trim()) break;
        const text = Array.from(s.text).slice(0, 16_001).join('');
        const at = chatTurns.findIndex(t => t.role === 'assistant' && t.runId === s.runId);
        if (at >= 0) setChatTurns(at, 'proposedPlan', text);
        else pushChatTurn({ role: 'assistant', text: '', tools: [], runId: s.runId as string, proposedPlan: text });
        break;
      }
      if (p.panel === 'hollow:handoff') {
        if (isStaleTurnEvent(p, chatQueuedTurn(), endedQueuedTurns)) break;
        const s = p.snapshot as { runId?: unknown; state?: unknown; entries?: unknown } | null;
        if (!s || !s.runId || s.runId !== chatLiveRunId() || !['pending', 'complete', 'unavailable'].includes(String(s.state))) break;
        const entries = typeof s.entries === 'number' && Number.isSafeInteger(s.entries) && s.entries > 0 ? s.entries : 0;
        const state = s.state === 'complete' && !entries ? 'unavailable' : s.state as 'pending' | 'complete' | 'unavailable';
        const text = state === 'pending' ? 'Context handoff pending' : state === 'complete' ? `Context handoff ready: ${entries} earlier entries prepared for the new session.` : 'Context handoff unavailable: earlier entries were not included.';
        const at = chatTurns.findIndex(t => t.runId === s.runId && !!t.handoff);
        if (at < 0) pushChatTurn({ role: 'system', text, tools: [], runId: s.runId as string, handoff: { state, entries } });
        else setChatTurns(at, { text, handoff: { state, entries } });
        break;
      }
      if (p.panel === 'hollow:reasoning') {
        const snapshot = p.snapshot as { text?: unknown; runId?: unknown } | null;
        if (!snapshot || typeof snapshot.text !== 'string' || !snapshot.runId || snapshot.runId !== chatLiveRunId()
          || isStaleTurnEvent(p, chatQueuedTurn(), endedQueuedTurns)) break;
        const text = Array.from(snapshot.text).slice(0, 16_001).join('');
        const at = chatTurns.findIndex(t => t.role === 'assistant' && t.runId === snapshot.runId);
        if (at >= 0) setChatTurns(at, 'reasoning', text);
        else pushChatTurn({ role: 'assistant', text: '', tools: [], runId: String(snapshot.runId), reasoning: text });
        break;
      }
      setChatPanels(cur => ({ ...cur, [p.panel]: p.snapshot }));
      const message = p.panel === 'hollow:context' && p.snapshot && typeof p.snapshot === 'object'
        ? (p.snapshot as { message?: unknown }).message : undefined;
      if (typeof message === 'string' && message) pushChatTurn({ role: 'system', text: message, tools: [] });
      break;
    }
    case 'loginLow':
      // Like a budget warning: only into the conversation it came from.
      if (p.sessionKey && p.sessionKey === chatStopKey()) {
        pushChatTurn({ role: 'system', text: p.message, tools: [] });
      }
      break;
    case 'cancelled': {
      if (p.queued) {
        // Taken out of the queue before it started: it was never sent, so
        // this is not a stop. The local turn decides what, if anything, the
        // transcript says (Edit says nothing; the text is back).
        const withdrawn = chatQueuedTurn();
        const intent = withdrawn?.intent ?? null;
        const decision = settleQueuedTurn(p);
        const notice = decision === 'none' ? null : withdrawnNotice(intent, withdrawn?.text ?? '', !!withdrawn?.interactionId);
        if (notice) pushChatTurn({ role: 'system', text: notice, tools: [] });
        break;
      }
      // Neutral system notice, not assistant text or an error. `done` still
      // ends the turn. Flagged so Continue (Paperclip PR 14b) is offered,
      // except after a stop before spawn: that request never reached the
      // native session, so "continue" would resume past it.
      pushChatTurn({ role: 'system', text: cancelledNotice(p.stop), tools: [], stopNotice: stopOffersContinue(p) });
      break;
    }
    case 'paused':
      // Neutral, like a stop's notice, but not one: Continue isn't offered
      // (Resume is, from the paused state `notePauseEvent` recorded).
      // `done` still ends the turn.
      if (p.sessionKey !== chatStopKey()) break;
      finalizePartials();
      pushChatTurn({ role: 'system', text: pausedNotice(p.note), tools: [] });
      break;
    case 'sessionReset': {
      // Only the current conversation's turn: a stale or foreign event must
      // never clear another conversation's native session.
      if (isStaleTurnEvent(p, chatQueuedTurn(), endedQueuedTurns)) break;
      if (p.sessionKey && p.sessionKey !== chatStopKey()) break;
      // The old native id must never be resumed again; the fresh session's
      // `session` event (or `done`) sets the new one.
      setChatSessionIdPersistent(null);
      setChatContextRaw(null);
      if (p.sessionKey) dropChatInteractionsFor(p.sessionKey);
      pushChatTurn({ role: 'system', text: sessionResetNotice(p.reason, AGENT_LABELS[p.agent as ChatAgent] ?? p.agent, p.changed, p.away), tools: [] });
      break;
    }
    case 'steer':
      if (p.sessionKey === chatStopKey()) applySteer(p.steer);
      break;
    case 'heldSent':
      if (!held) break;
      settleHeldAnswers(held.id, null);
      // After a handover it runs in the successor, not here (C8b).
      if (p.sessionKey !== chatStopKey()) break;
      // The held send is now this conversation's next turn.
      adoptBusySend(p.clientTurnId ?? `held-${p.heldId}`, held.text, held.attachments ?? [], { sessionKey: p.sessionKey, queueId: p.queueId, queuePosition: 1, phase: 'queued', waitingFor: null });
      break;
    case 'heldFailed':
      returnHeldToBox(p.sessionKey, held, p.error);
      ackHeldFailed(p);
      break;
    case 'agentContinued':
      // Repo ideas 2026-10-27 #8: the turn ran out having changed nothing,
      // and the person's setting continues it on another agent: the
      // conversation's next turn, drawn as sent.
      if (p.sessionKey !== chatStopKey()) break;
      pushChatTurn({ role: 'system', text: agentContinuedNotice(p.from, p.to), tools: [] });
      adoptBusySend(p.clientTurnId, p.prompt, [], { sessionKey: p.sessionKey, queueId: p.queueId, queuePosition: 1, phase: 'queued', waitingFor: null });
      break;
    case 'accountSwitched': {
      if (isStaleTurnEvent(p, chatQueuedTurn(), endedQueuedTurns)) break;
      if (p.sessionKey && p.sessionKey !== chatStopKey()) break;
      // The dead account's reply is over; the resumed one starts a new bubble.
      finalizePartials();
      transcript.setPendingNewTurn(false);
      pushChatTurn({ role: 'system', text: accountSwitchNotice(p.from, p.to, p.reason), tools: [], notice: 'accountSwitch' });
      break;
    }
    case 'remoteRun': {
      if (isStaleTurnEvent(p, chatQueuedTurn(), endedQueuedTurns)) break;
      if (p.sessionKey && p.sessionKey !== chatStopKey()) break;
      // The reply so far is over (`back` comes after it); the next starts fresh.
      finalizePartials();
      pushRemoteRunTurn(p);
      break;
    }
    case 'error': {
      const q = chatQueuedTurn();
      if (p.message.startsWith(CHAT_CONTEXT_RECOVERED_PREFIX) && eventMatchesTurn(p, q)) {
        // The pump found the working folder gone when it claimed the turn.
        // Same reset as the send-time recovery; the recovered `done` that
        // follows puts the text back in the composer.
        recoverChatContext(q!.requestedCwd ?? 'the selected folder');
        setChatQueuedTurn({ ...q!, recovered: true });
        break;
      }
      if (q && !q.started && eventMatchesTurn(p, q)) {
        // Failed before it started (never admitted): nothing ran and nothing
        // was sent, so no assistant bubble. Its `done` restores the composer
        // (an answer turn's `done` puts its card back instead, PR 13).
        // An unsafe working folder (Paperclip PR 19) reads without its prefix.
        const failure = unsafeCwdText(p.message) ?? p.message;
        pushChatTurn({
          role: 'system',
          text: q.interactionId ? `Your answers were not sent: ${failure.trim().replace(/[.\s]+$/, '')}.`
            : answeredRevision(q.text) ? `Your answers weren't sent: ${failure.trim().replace(/[.\s]+$/, '')}. They're still in the canvas.` : queuedFailureNotice(failure),
          tools: [],
        });
        break;
      }
      appendAssistantText(`(error: ${p.message})`);
      if (p.resultKind || !replySucceeded()) markReply('error', p.resultKind);
      break;
    }
    case 'done': {
      const q = chatQueuedTurn();
      if (isStaleTurnEvent(p, q, endedQueuedTurns)) {
        // The end of a turn Edit or Discard already closed, arriving after a
        // newer send began: it must not end that send.
        const i = endedQueuedTurns.findIndex(turn => eventMatchesTurn(p, turn));
        if (i >= 0) endedQueuedTurns.splice(i, 1);
        break;
      }
      if (p.recovered && q && eventMatchesTurn(p, q) && !q.recovered) {
        // The recovery error was missed; reset the same way.
        recoverChatContext(q.requestedCwd ?? 'the selected folder');
        setChatQueuedTurn({ ...q, recovered: true });
      }
      settleQueuedTurn(p);
      // Whatever the decision, the turn is over: only one chat turn exists.
      setChatQueuedTurn(null);
      finalizePartials();
      transcript.setPendingNewTurn(false);
      activeChatEvidence = null;
      setActiveChatRecall([]);
      // Only the current turn's `done` gets here (a stale one broke above).
      setChatLiveRunId(null);
      setChatTurnSteerSupport(null);
      setChatActivity(null);
      setChatRunStartedAt(null);
      if (!p.recovered) {
        if (p.sessionId) setChatSessionIdPersistent(p.sessionId);
        if (p.sessionKey && q && eventMatchesTurn(p, q)) acquireFirstDraft(p.sessionKey, q);
        if (p.sessionKey) setChatAgentSessionKeyPersistent(p.sessionKey, { ownTurn: true });
      }
      // An adopted turn (a check-in, a wake) ran in a mode this window didn't
      // choose: its session carries on into the person's next turn only
      // when that mode is in the composer's mode's class, as reopening the
      // conversation would decide (`conversationPermission`). Every turn
      // after a Main check-in used to start a fresh session (2026-11-04).
      if (q?.adopted && !(p.sessionId && !p.recovered && q.permissionMode
        && modeClass(chatAgent(), q.permissionMode) === modeClass(chatAgent(), chatPermissionMode()))) {
        setChatSessionIdPersistent(null);
      }
      setChatBusy(false);
      finalizeArtifactsForRun();
      // A finished turn never drops an open card (overnight 11-08 E1).
      setChatInteractions(list => keepOpenCards(list));
      if (q?.adopted && !p.recovered) void rereadAdoptedConversation(chatAgentSessionKey());
      break;
    }
  }
}

/** A turn re-attached after a reload or reopen (adopted) was hydrated with
 *  its tools but none of the reply streamed so far, since the store keeps
 *  the reply text only at finish, so its next delta began a second reply.
 *  Once it ends the store has the whole answer: show that as one reply
 *  (overnight 11-13 c-11). A read behind what streamed live changes nothing. */
async function rereadAdoptedConversation(key: string | null) {
  if (!key) return;
  const version = chatConversationVersion();
  const generation = beginStoredTranscriptRead(key);
  const replyText = (turns: readonly ChatTurn[]) => turns.reduce((n, t) => n + (t.role === 'assistant' ? t.text.length : 0), 0);
  try {
    const conversation = await api.readAgentConversation(key);
    if (!conversation || !storedTranscriptReadApplies(key, generation) || chatBusy() || chatAgentSessionKey() !== key || chatConversationVersion() !== version) return;
    const turns = turnsFromConversation(conversation);
    if (replyText(turns) < replyText(chatTurns)) return;
    batch(() => {
      setChatTurns(turns);
      rebuildChatArtifacts(turns);
      setChatStoredTranscriptPending(null);
      markStoredTranscriptApplied(key, generation);
    });
  } catch { /* the live transcript stays */ }
}

// ── Conversations in the background (build programme PR B4) ────────────────
// Leaving a conversation whose turn is still running keeps it running: its
// on-screen state is snapshotted, its events are buffered while another
// conversation is shown, and coming back restores the snapshot and replays
// the buffer through `handleChatEvent`, so the same reducer draws it.

interface ChatSnapshot {
  steerSupport: SteerSupportRow | null;
  key: string;
  turns: ChatTurn[];
  continuedFrom: string[];
  sessionId: string | null;
  cwd: string | null;
  busy: boolean;
  queued: ChatQueuedTurn | null;
  liveRunId: string | null;
  activity: ChatActivity | null;
  panels: Readonly<Record<string, unknown>>;
  runStartedAt: number | null;
  turnUsage: TurnUsage | null;
  contextTokens: ChatContextUse | null;
  agent: ChatAgent;
  permissionMode: ChatPermissionMode;
  model: string;
  agentOverridden: boolean;
  interactions: ChatInteraction[];
  artifacts: ChatArtifact[];
  evidence: (EvidenceRunContextWire & { turnStartIndex: number }) | null;
  recall: EvidenceRunContextWire['recall'];
  pendingNewTurn: boolean;
  pendingWrites: Array<[string, string]>;
  touched: boolean;
}

interface BackgroundChat {
  snapshot: ChatSnapshot;
  events: ChatEventPayload[];
  /** Its turn ended while it was in the background. */
  done: boolean;
}

/** Max finished (`done: true`) backgrounded chats kept in memory for instant
 *  restore; older finished entries are evicted LRU and reload from the session
 *  store when reopened. Running (`done: false`) chats are never evicted. */
export const BACKGROUND_DONE_CAP = 12;

const backgroundChats = new Map<string, BackgroundChat>();
function touchBackgroundChat(key: string, chat: BackgroundChat) {
  backgroundChats.delete(key);
  backgroundChats.set(key, chat);
}
function evictFinishedBackgroundChats() {
  let doneCount = 0;
  for (const chat of backgroundChats.values()) if (chat.done) doneCount += 1;
  if (doneCount <= BACKGROUND_DONE_CAP) return;
  for (const [key, chat] of backgroundChats) {
    if (!chat.done) continue;
    backgroundChats.delete(key);
    doneCount -= 1;
    if (doneCount <= BACKGROUND_DONE_CAP) break;
  }
}
/** Conversations whose turn is still running in the background, for the
 *  rail's working marks. */
export const [backgroundChatKeys, setBackgroundChatKeys] = createSignal<string[]>([]);
function syncBackgroundKeys() {
  setBackgroundChatKeys([...backgroundChats].filter(([, chat]) => !chat.done).map(([key]) => key));
}

function clone<T>(value: T): T {
  try { return structuredClone(value); } catch { return JSON.parse(JSON.stringify(value)) as T; }
}

/** Where an event goes: the conversation on screen, a backgrounded one, or
 *  nowhere (a conversation this window never showed, e.g. after a reload). */
function routeChatEvent(p: ChatEventPayload): Route {
  const route = routeEvent(p, {
    isBackground: key => backgroundChats.has(key),
    foregroundKey: chatAgentSessionKey(),
    queued: chatQueuedTurn(),
  });
  if (route === 'background') {
    const key = p.conversationKey!;
    const chat = backgroundChats.get(key)!;
    bufferEvent(chat.events, p);
    if (p.type === 'done') {
      chat.done = true;
      touchBackgroundChat(key, chat);
      evictFinishedBackgroundChats();
      syncBackgroundKeys();
    }
    // A turn that follows (a held send) runs it again (review).
    else if ((p.type === 'runStarted' || p.type === 'agentContinued') && chat.done) {
      chat.done = false;
      touchBackgroundChat(key, chat);
      syncBackgroundKeys();
    }
  }
  return route;
}

// Composer sends still getting ready (the agent check, the working-folder
// preflight, a card's answers): busy, but nothing is queued yet, so the
// conversation stays on screen until the turn is. Left meanwhile, the send
// would carry on into the conversation opened next (run 15 P3 second-pass
// review). A count: each hold is released once.
const [chatSendsPreparing, setChatSendsPreparing] = createSignal(0);

/** Hold the conversation on screen while a send gets ready; call the
 *  returned release once the turn is queued or the send gave up (a second
 *  call does nothing). */
export function holdConversationForSend(): () => void {
  setChatSendsPreparing(n => n + 1);
  let held = true;
  return () => {
    if (!held) return;
    held = false;
    setChatSendsPreparing(n => n - 1);
  };
}

/** The conversation on screen may be left: nothing is loading or getting
 *  ready to send, and a running turn can be sent to the background (it has
 *  its key). */
export function canLeaveConversation(): boolean {
  return !chatSessionLoading() && chatSendsPreparing() === 0 && (!chatBusy() || !!chatAgentSessionKey());
}

/** Send the conversation on screen to the background when its turn is still
 *  running; afterwards `chatBusy()` is false and the caller may reset the
 *  chat state. True when nothing was running or it was backgrounded; false
 *  while a send is still getting ready (`holdConversationForSend`). */
export function backgroundRunningChat(): boolean {
  if (chatSendsPreparing() > 0) return false;
  if (!chatBusy()) return true;
  const key = chatAgentSessionKey();
  if (!key) return false;
  const raw = chatContextRaw();
  touchBackgroundChat(key, {
    snapshot: {
      key,
      turns: clone(unwrap(chatTurns)),
      continuedFrom: [...chatContinuedFrom()],
      sessionId: chatSessionId(),
      cwd: chatCwd(),
      busy: true,
      queued: clone(chatQueuedTurn()),
      liveRunId: chatLiveRunId(),
      steerSupport: chatTurnSteerSupport(),
      activity: chatActivity(),
      panels: clone(chatPanels()),
      runStartedAt: chatRunStartedAt(),
      turnUsage: chatTurnUsage(),
      contextTokens: raw && raw.version === chatConversationVersion() ? { tokens: raw.tokens, model: raw.model, window: raw.window } : null,
      agent: chatAgent(),
      permissionMode: chatPermissionMode(),
      model: chatModel(),
      agentOverridden: chatAgentOverridden,
      interactions: clone(chatInteractions()),
      artifacts: chatArtifacts(),
      evidence: activeChatEvidence ? { ...activeChatEvidence } : null,
      recall: activeChatRecall(),
      pendingNewTurn: transcript.pendingNewTurn(),
      pendingWrites: [...pendingArtifactWrites],
      touched: artifactsTouchedThisRun,
    },
    events: [],
    done: false,
  });
  syncBackgroundKeys();
  setChatBusy(false);
  setChatQueuedTurn(null);
  return true;
}

/** Bring a backgrounded conversation back on screen: its snapshot, then
 *  every event it received meanwhile. False when `key` is not in the
 *  background (load it from the session store instead). */
export function restoreBackgroundChat(key: string): boolean {
  const chat = backgroundChats.get(key);
  if (!chat || chatBusy()) return false;
  backgroundChats.delete(key);
  syncBackgroundKeys();
  const s = chat.snapshot;
  batch(() => {
    // The conversation on screen until now goes: so do its visuals in the
    // Preview pane, as resetChatState drops them (run 15 P1 review).
    dropConversationPreviews();
    setChatConversationVersion(version => version + 1);
    setChatTurns(s.turns);
    setChatSessionIdPersistent(s.sessionId);
    setChatAgentSessionKeyPersistent(s.key);
    setChatContinuedFrom(s.continuedFrom ?? []);
    setChatCwdPersistent(s.cwd);
    setChatAgent(s.agent);
    lsSet('chatAgent', s.agent);
    setChatModelPersistent(s.model);
    setChatPermissionMode(s.permissionMode);
    lsSet('chatPermissionMode', s.permissionMode);
    chatAgentOverridden = s.agentOverridden;
    setChatQueuedTurn(s.queued);
    setChatLiveRunId(s.liveRunId);
    setChatTurnSteerSupport(s.steerSupport);
    setChatActivity(s.activity);
    setChatPanels(s.panels ?? {});
    setChatRunStartedAt(s.runStartedAt);
    setChatTurnUsage(s.turnUsage);
    setChatContextRaw(s.contextTokens ? { version: chatConversationVersion(), ...s.contextTokens } : null);
    setChatInteractions(s.interactions);
    artifactLists.cancel();
    setChatArtifacts(s.artifacts);
    activeChatEvidence = s.evidence;
    setActiveChatRecall(s.recall);
    transcript.setPendingNewTurn(s.pendingNewTurn);
    pendingArtifactWrites.clear();
    for (const [id, path] of s.pendingWrites) pendingArtifactWrites.set(id, path);
    artifactsTouchedThisRun = s.touched;
    setChatBusy(s.busy);
  });
  batch(() => {
    for (const event of chat.events) handleChatEvent(event);
  });
  return true;
}

/** Forget `key`'s background entry without showing it, for a path that
 *  takes the conversation over from its stored record instead (a native
 *  Continue from an artifact's Sessions panel). Only for a turn that has
 *  finished: left in place, the entry would capture every event of the new
 *  on-screen turn (routeEvent checks it first) and could later be restored
 *  over the newer turns (review A·D4). */
export function discardBackgroundChat(key: string): void {
  if (backgroundChats.delete(key)) syncBackgroundKeys();
}

/** A composer send whose `chat_send` reply came back after the person left
 *  its conversation (`backgroundRunningChat` took it while the call was in
 *  flight; C8a): the reply is that conversation's, never the one on screen.
 *  Its queued turn learns its queue id, so Edit and Discard work once back. */
export function acceptBackgroundSend(key: string, clientTurnId: string, accepted: SendAccepted): void {
  const chat = backgroundChats.get(key);
  const q = chat?.snapshot.queued;
  if (!chat || !q || q.clientTurnId !== clientTurnId) return;
  chat.snapshot.queued = { ...q, queueId: accepted.queueId, sessionKey: accepted.sessionKey, waitingFor: q.started ? null : accepted.waitingFor, queuePosition: accepted.queuePosition };
  const t = chat.snapshot.turns.find(turn => turn.role === 'user' && turn.clientTurnId === clientTurnId);
  if (t) t.queueId = accepted.queueId;
}

/** The same for a send that queued nothing there (its call failed, or the
 *  message went to the chain's current conversation): its bubble goes and
 *  the conversation stops working (a send only starts from an idle one).
 *  `notice` joins its transcript. */
export function endBackgroundSend(key: string, clientTurnId: string, notice: string | null): void {
  const chat = backgroundChats.get(key);
  if (!chat) return;
  const s = chat.snapshot;
  for (let i = s.turns.length - 1; i >= 0; i -= 1) {
    if (s.turns[i].role === 'user' && s.turns[i].clientTurnId === clientTurnId) { s.turns.splice(i, 1); break; }
  }
  if (notice) s.turns.push({ role: 'system', text: notice, tools: [] });
  // Only while this send is still its queued turn: a later send's (this
  // chat reopened, sent again and left) keeps running (PR #1391 review).
  if (s.queued?.clientTurnId !== clientTurnId) return;
  s.queued = null;
  s.busy = false;
  chat.done = true;
  touchBackgroundChat(key, chat);
  evictFinishedBackgroundChats();
  syncBackgroundKeys();
}

/** An answer turn that failed after its conversation went to the background
 *  (C8a): its card is reopened there (it shows when that chat opens), else
 *  the answers wait in that chat's box. Never the chat on screen. */
export async function returnUnsentAnswersTo(key: string, interactionId: string, prompt: string, notice: string): Promise<void> {
  let reopened = false;
  try {
    const card = await api.reopenInteraction(interactionId);
    reopened = card.status === 'pending' && card.sessionKey === key;
  } catch {
    reopened = false;
  }
  if (!reopened) { returnToConversation(key, prompt, [], notice); return; }
  if (key === chatAgentSessionKey()) void refreshChatInteractions();
  else flashFooter(`${notice} Its card is open again in that chat.`, 8000);
}

/** Whether `key`'s conversation is held in the background (running, or
 *  finished there and not shown since). */
export const isBackgrounded = (key: string) => backgroundChats.has(key);
/** The saved context used to validate an unlisted live session before opening it. */
export const backgroundChatCwd = (key: string) => backgroundChats.get(key)?.snapshot.cwd;

/** The newest backgrounded conversation that runs in `cwd`, if any: opening
 *  a workspace brings back the conversation left running there. */
export function backgroundedKeyForCwd(cwd: string): string | null {
  let found: string | null = null;
  for (const [key, chat] of backgroundChats) if (chat.snapshot.cwd === cwd) found = key;
  return found;
}

/** Whether `key`'s conversation has a turn running in the background. */
export const isBackgroundChat = (key: string) => backgroundChatKeys().includes(key);

// Start the listener once at module init so streaming events keep arriving
// even when Conversation history is unmounted (e.g. user navigates to Vault while
// claude is still generating).
listen<ChatEventPayload>('chat-event', e => handleChatEvent(e.payload)).catch(() => {/* not in tauri */});
// Hollow asks for this window's whole mode record after a launch once a phone
// request relies on its copy, every 250 ms for 30 seconds until this window
// answers (the phone chat API; no launch call). An ask sent before this
// listener registers is lost; a later one is heard: within 250 ms during
// those 30 seconds, else with the next phone request.
listen('phone-access-modes-wanted', () => reportPhoneModes()).catch(() => {/* not in tauri */});
